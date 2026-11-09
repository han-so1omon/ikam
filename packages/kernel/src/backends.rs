//! Alternative [`Store`] backends, each a swappable option (E122 bake-off).
//!
//! Every backend keeps the same contract as [`FsStore`](crate::FsStore):
//! objects are written once under their id, and refs move by compare-and-swap.
//! What differs is the layout on disk and when bytes become durable:
//!
//! - [`FsStore`](crate::FsStore) (the default) writes one file per object and
//!   fsyncs every object.
//! - [`PackStore`] appends objects to a pack file and makes them durable, with
//!   one fsync, when a ref moves. Sealed packs get a sorted index with a
//!   256-way fanout, as in git's pack index v2.
//! - The key-value engines ([`KvStore`] over SQLite, LMDB, redb, fjall or
//!   RocksDB, each behind a cargo feature) buffer new objects and commit them
//!   in the same transaction as the ref move, so a ref never points at an
//!   object that is not durable.
//!
//! None of these changes the bytes of any object: a store only keeps them.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use crate::store::check_ref_name;
use crate::{Error, Id, Store};

#[cfg(any(
    feature = "store-fjall",
    feature = "store-lmdb",
    feature = "store-redb",
    feature = "store-rocksdb",
    feature = "store-sqlite"
))]
fn other<E: std::fmt::Display>(e: E) -> Error {
    Error::Io(io::Error::other(e.to_string()))
}

// ---------------------------------------------------------------------------
// Key-value engines behind one adapter.

/// The two tables every key-value backend keeps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Table {
    Objects,
    Refs,
}

/// One write in an atomic batch: `None` deletes the key.
pub struct KvOp<'a> {
    pub table: Table,
    pub key: &'a [u8],
    pub value: Option<&'a [u8]>,
}

/// One key and its value.
pub type Entry = (Vec<u8>, Vec<u8>);

/// The few operations a key-value engine must offer to back a [`Store`].
pub trait Kv {
    fn get(&self, table: Table, key: &[u8]) -> Result<Option<Vec<u8>>, Error>;
    fn contains(&self, table: Table, key: &[u8]) -> Result<bool, Error> {
        Ok(self.get(table, key)?.is_some())
    }
    /// Keys of `table` starting with `prefix`, in order.
    fn keys(&self, table: Table, prefix: &[u8]) -> Result<Vec<Vec<u8>>, Error>;
    fn entries(&self, table: Table) -> Result<Vec<Entry>, Error>;
    /// Apply `ops` atomically and durably. With a `guard`, first check inside
    /// the same transaction that ref `guard.0` currently holds `guard.1`, and
    /// fail with [`Error::RefConflict`] (applying nothing) if it does not.
    fn apply(&mut self, ops: &[KvOp], guard: Option<(&str, Option<&[u8]>)>) -> Result<(), Error>;
}

/// New objects wait in memory until a ref moves or this many bytes are
/// waiting; then they are committed in one transaction.
pub const KV_FLUSH_BYTES: usize = 64 << 20;

/// A [`Store`] over any [`Kv`] engine. Objects written since the last commit
/// are readable at once, and become durable together with the next ref move.
pub struct KvStore<K: Kv> {
    kv: K,
    pending: BTreeMap<Id, Option<Vec<u8>>>,
    pending_bytes: usize,
}

impl<K: Kv> KvStore<K> {
    pub fn new(kv: K) -> KvStore<K> {
        KvStore {
            kv,
            pending: BTreeMap::new(),
            pending_bytes: 0,
        }
    }

    pub fn engine(&self) -> &K {
        &self.kv
    }

    /// Commit waiting objects without moving a ref.
    pub fn sync(&mut self) -> Result<(), Error> {
        self.commit(None)
    }

    fn commit(&mut self, guard_and_ref: Option<(&str, Option<Id>, Id)>) -> Result<(), Error> {
        if self.pending.is_empty() && guard_and_ref.is_none() {
            return Ok(());
        }
        let expected = guard_and_ref.and_then(|(_, e, _)| e).map(|e| *e.as_bytes());
        let new = guard_and_ref.map(|(_, _, n)| *n.as_bytes());
        let mut ops: Vec<KvOp> = self
            .pending
            .iter()
            .map(|(id, v)| KvOp {
                table: Table::Objects,
                key: id.as_bytes(),
                value: v.as_deref(),
            })
            .collect();
        if let (Some((name, _, _)), Some(new)) = (guard_and_ref, new.as_ref()) {
            ops.push(KvOp {
                table: Table::Refs,
                key: name.as_bytes(),
                value: Some(new),
            });
        }
        let guard = guard_and_ref.map(|(name, _, _)| (name, expected.as_ref().map(|e| &e[..])));
        self.kv.apply(&ops, guard)?;
        self.pending.clear();
        self.pending_bytes = 0;
        Ok(())
    }

    fn stage(&mut self, id: Id, value: Option<Vec<u8>>) -> Result<(), Error> {
        self.pending_bytes += value.as_ref().map_or(0, Vec::len) + 32;
        self.pending.insert(id, value);
        if self.pending_bytes >= KV_FLUSH_BYTES {
            self.sync()?;
        }
        Ok(())
    }
}

impl<K: Kv> Drop for KvStore<K> {
    fn drop(&mut self) {
        let _ = self.sync();
    }
}

fn parse_id(key: &[u8]) -> Option<Id> {
    <[u8; 32]>::try_from(key).ok().map(Id::from_bytes)
}

impl<K: Kv> Store for KvStore<K> {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        if self.has(&id) {
            return Ok(false);
        }
        self.stage(id, Some(encoded.to_vec()))?;
        Ok(true)
    }

    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.stage(id, Some(encoded.to_vec()))
    }

    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        match self.pending.get(id) {
            Some(Some(v)) => Ok(v.clone()),
            Some(None) => Err(Error::NotFound(*id)),
            None => self
                .kv
                .get(Table::Objects, id.as_bytes())?
                .ok_or(Error::NotFound(*id)),
        }
    }

    fn has(&self, id: &Id) -> bool {
        match self.pending.get(id) {
            Some(v) => v.is_some(),
            None => self
                .kv
                .contains(Table::Objects, id.as_bytes())
                .unwrap_or(false),
        }
    }

    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.ids_with_prefix(&[])
    }

    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        let mut out: BTreeMap<Id, bool> = BTreeMap::new();
        for key in self.kv.keys(Table::Objects, prefix)? {
            if let Some(id) = parse_id(&key) {
                out.insert(id, true);
            }
        }
        for (id, v) in &self.pending {
            if id.as_bytes().starts_with(prefix) {
                out.insert(*id, v.is_some());
            }
        }
        Ok(out
            .into_iter()
            .filter(|(_, live)| *live)
            .map(|(id, _)| id)
            .collect())
    }

    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        if !self.has(id) {
            return Err(Error::NotFound(*id));
        }
        self.stage(*id, None)
    }

    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        check_ref_name(name)?;
        match self.kv.get(Table::Refs, name.as_bytes())? {
            None => Ok(None),
            Some(v) => parse_id(&v).map(Some).ok_or(Error::Decode("ref value")),
        }
    }

    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        check_ref_name(name)?;
        self.commit(Some((name, expected, new)))
    }

    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        let mut out = Vec::new();
        for (k, v) in self.kv.entries(Table::Refs)? {
            let name = String::from_utf8(k).map_err(|_| Error::Decode("ref name"))?;
            out.push((name, parse_id(&v).ok_or(Error::Decode("ref value"))?));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// SQLite (WAL).

#[cfg(feature = "store-sqlite")]
pub mod sqlite {
    use super::*;
    use rusqlite::{Connection, OptionalExtension, params};

    /// `synchronous=FULL` syncs the WAL on every commit (durable); `NORMAL`
    /// syncs only at checkpoints, so a power cut can lose the last commits.
    #[derive(Clone, Copy, Debug)]
    pub enum Sync {
        Full,
        Normal,
    }

    pub struct Sqlite {
        conn: Connection,
    }

    fn table(t: Table) -> &'static str {
        match t {
            Table::Objects => "objects",
            Table::Refs => "refs",
        }
    }

    impl Sqlite {
        pub fn open(dir: impl AsRef<Path>, sync: Sync) -> Result<Sqlite, Error> {
            fs::create_dir_all(dir.as_ref())?;
            let conn = Connection::open(dir.as_ref().join("store.sqlite")).map_err(other)?;
            let sync = match sync {
                Sync::Full => "FULL",
                Sync::Normal => "NORMAL",
            };
            // On macOS, SQLite's fsync does not flush the drive's cache unless
            // asked (fullfsync is off by default); the other backends use
            // F_FULLFSYNC, so Full asks for it too, for a like-for-like test.
            let full = if cfg!(target_os = "macos") && sync == "FULL" {
                "PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;"
            } else {
                ""
            };
            conn.execute_batch(&format!(
                "{full} PRAGMA journal_mode=WAL; PRAGMA synchronous={sync};
                 CREATE TABLE IF NOT EXISTS objects(k BLOB PRIMARY KEY, v BLOB NOT NULL);
                 CREATE TABLE IF NOT EXISTS refs(k BLOB PRIMARY KEY, v BLOB NOT NULL);"
            ))
            .map_err(other)?;
            Ok(Sqlite { conn })
        }
    }

    impl Kv for Sqlite {
        fn get(&self, t: Table, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
            let sql = format!("SELECT v FROM {} WHERE k = ?1", table(t));
            let mut st = self.conn.prepare_cached(&sql).map_err(other)?;
            st.query_row(params![key], |r| r.get(0))
                .optional()
                .map_err(other)
        }

        fn contains(&self, t: Table, key: &[u8]) -> Result<bool, Error> {
            let sql = format!("SELECT 1 FROM {} WHERE k = ?1", table(t));
            let mut st = self.conn.prepare_cached(&sql).map_err(other)?;
            Ok(st
                .query_row(params![key], |_| Ok(()))
                .optional()
                .map_err(other)?
                .is_some())
        }

        fn keys(&self, t: Table, prefix: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
            let sql = format!("SELECT k FROM {} WHERE k >= ?1 ORDER BY k", table(t));
            let mut st = self.conn.prepare_cached(&sql).map_err(other)?;
            let rows = st
                .query_map(params![prefix], |r| r.get::<_, Vec<u8>>(0))
                .map_err(other)?;
            let mut out = Vec::new();
            for k in rows {
                let k = k.map_err(other)?;
                if !k.starts_with(prefix) {
                    break;
                }
                out.push(k);
            }
            Ok(out)
        }

        fn entries(&self, t: Table) -> Result<Vec<Entry>, Error> {
            let sql = format!("SELECT k, v FROM {} ORDER BY k", table(t));
            let mut st = self.conn.prepare_cached(&sql).map_err(other)?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(other)?;
            rows.collect::<Result<_, _>>().map_err(other)
        }

        fn apply(
            &mut self,
            ops: &[KvOp],
            guard: Option<(&str, Option<&[u8]>)>,
        ) -> Result<(), Error> {
            let tx = self
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(other)?;
            if let Some((name, expected)) = guard {
                let now: Option<Vec<u8>> = tx
                    .query_row(
                        "SELECT v FROM refs WHERE k = ?1",
                        params![name.as_bytes()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(other)?;
                if now.as_deref() != expected {
                    return Err(Error::RefConflict(name.into()));
                }
            }
            for op in ops {
                match op.value {
                    Some(v) => {
                        let sql = format!(
                            "INSERT OR REPLACE INTO {} (k, v) VALUES (?1, ?2)",
                            table(op.table)
                        );
                        tx.prepare_cached(&sql)
                            .map_err(other)?
                            .execute(params![op.key, v])
                            .map_err(other)?;
                    }
                    None => {
                        let sql = format!("DELETE FROM {} WHERE k = ?1", table(op.table));
                        tx.prepare_cached(&sql)
                            .map_err(other)?
                            .execute(params![op.key])
                            .map_err(other)?;
                    }
                }
            }
            tx.commit().map_err(other)
        }
    }

    pub fn open(dir: impl AsRef<Path>, sync: Sync) -> Result<KvStore<Sqlite>, Error> {
        Ok(KvStore::new(Sqlite::open(dir, sync)?))
    }
}

// ---------------------------------------------------------------------------
// LMDB (through heed).

#[cfg(feature = "store-lmdb")]
pub mod lmdb {
    use super::*;
    use heed::types::Bytes;
    use heed::{Database, Env, EnvOpenOptions};

    pub struct Lmdb {
        env: Env,
        objects: Database<Bytes, Bytes>,
        refs: Database<Bytes, Bytes>,
    }

    /// The largest the map may grow. The file is sparse and grows as used.
    pub const MAP_SIZE: usize = 1 << 40;

    impl Lmdb {
        pub fn open(dir: impl AsRef<Path>) -> Result<Lmdb, Error> {
            fs::create_dir_all(dir.as_ref())?;
            // Safety: one Env per directory per process (the bake-off opens
            // each store once), and none of LMDB's unsafe flags is set.
            let env = unsafe {
                EnvOpenOptions::new()
                    .map_size(MAP_SIZE)
                    .max_dbs(2)
                    .open(dir.as_ref())
            }
            .map_err(other)?;
            let mut w = env.write_txn().map_err(other)?;
            let objects = env
                .create_database(&mut w, Some("objects"))
                .map_err(other)?;
            let refs = env.create_database(&mut w, Some("refs")).map_err(other)?;
            w.commit().map_err(other)?;
            Ok(Lmdb { env, objects, refs })
        }

        fn db(&self, t: Table) -> Database<Bytes, Bytes> {
            match t {
                Table::Objects => self.objects,
                Table::Refs => self.refs,
            }
        }
    }

    impl Kv for Lmdb {
        fn get(&self, t: Table, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
            let r = self.env.read_txn().map_err(other)?;
            Ok(self.db(t).get(&r, key).map_err(other)?.map(<[u8]>::to_vec))
        }

        fn keys(&self, t: Table, prefix: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
            let r = self.env.read_txn().map_err(other)?;
            let mut out = Vec::new();
            // LMDB rejects an empty key, so an empty prefix walks everything.
            if prefix.is_empty() {
                for kv in self.db(t).iter(&r).map_err(other)? {
                    out.push(kv.map_err(other)?.0.to_vec());
                }
            } else {
                for kv in self.db(t).prefix_iter(&r, prefix).map_err(other)? {
                    out.push(kv.map_err(other)?.0.to_vec());
                }
            }
            Ok(out)
        }

        fn entries(&self, t: Table) -> Result<Vec<Entry>, Error> {
            let r = self.env.read_txn().map_err(other)?;
            let mut out = Vec::new();
            for kv in self.db(t).iter(&r).map_err(other)? {
                let (k, v) = kv.map_err(other)?;
                out.push((k.to_vec(), v.to_vec()));
            }
            Ok(out)
        }

        fn apply(
            &mut self,
            ops: &[KvOp],
            guard: Option<(&str, Option<&[u8]>)>,
        ) -> Result<(), Error> {
            let mut w = self.env.write_txn().map_err(other)?;
            if let Some((name, expected)) = guard
                && self.refs.get(&w, name.as_bytes()).map_err(other)? != expected
            {
                return Err(Error::RefConflict(name.into()));
            }
            for op in ops {
                let db = self.db(op.table);
                match op.value {
                    Some(v) => db.put(&mut w, op.key, v).map_err(other)?,
                    None => {
                        db.delete(&mut w, op.key).map_err(other)?;
                    }
                }
            }
            w.commit().map_err(other)
        }
    }

    pub fn open(dir: impl AsRef<Path>) -> Result<KvStore<Lmdb>, Error> {
        Ok(KvStore::new(Lmdb::open(dir)?))
    }
}

// ---------------------------------------------------------------------------
// redb (copy-on-write B-trees, pure Rust).

#[cfg(feature = "store-redb")]
pub mod redb_store {
    use super::*;
    use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

    const OBJECTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("objects");
    const REFS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("refs");

    fn def(t: Table) -> TableDefinition<'static, &'static [u8], &'static [u8]> {
        match t {
            Table::Objects => OBJECTS,
            Table::Refs => REFS,
        }
    }

    pub struct Redb {
        db: Database,
    }

    impl Redb {
        pub fn open(dir: impl AsRef<Path>) -> Result<Redb, Error> {
            fs::create_dir_all(dir.as_ref())?;
            let db = Database::create(dir.as_ref().join("store.redb")).map_err(other)?;
            let w = db.begin_write().map_err(other)?;
            w.open_table(OBJECTS).map_err(other)?;
            w.open_table(REFS).map_err(other)?;
            w.commit().map_err(other)?;
            Ok(Redb { db })
        }
    }

    impl Kv for Redb {
        fn get(&self, t: Table, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
            let r = self.db.begin_read().map_err(other)?;
            let table = r.open_table(def(t)).map_err(other)?;
            Ok(table.get(key).map_err(other)?.map(|v| v.value().to_vec()))
        }

        fn keys(&self, t: Table, prefix: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
            let r = self.db.begin_read().map_err(other)?;
            let table = r.open_table(def(t)).map_err(other)?;
            let mut out = Vec::new();
            for kv in table.range(prefix..).map_err(other)? {
                let k = kv.map_err(other)?.0.value().to_vec();
                if !k.starts_with(prefix) {
                    break;
                }
                out.push(k);
            }
            Ok(out)
        }

        fn entries(&self, t: Table) -> Result<Vec<Entry>, Error> {
            let r = self.db.begin_read().map_err(other)?;
            let table = r.open_table(def(t)).map_err(other)?;
            let mut out = Vec::new();
            for kv in table.iter().map_err(other)? {
                let (k, v) = kv.map_err(other)?;
                out.push((k.value().to_vec(), v.value().to_vec()));
            }
            Ok(out)
        }

        fn apply(
            &mut self,
            ops: &[KvOp],
            guard: Option<(&str, Option<&[u8]>)>,
        ) -> Result<(), Error> {
            let w = self.db.begin_write().map_err(other)?;
            {
                if let Some((name, expected)) = guard {
                    let refs = w.open_table(REFS).map_err(other)?;
                    let now = refs
                        .get(name.as_bytes())
                        .map_err(other)?
                        .map(|v| v.value().to_vec());
                    if now.as_deref() != expected {
                        drop(refs);
                        w.abort().map_err(other)?;
                        return Err(Error::RefConflict(name.into()));
                    }
                }
                let mut objects = w.open_table(OBJECTS).map_err(other)?;
                let mut refs = w.open_table(REFS).map_err(other)?;
                for op in ops {
                    let table = if op.table == Table::Objects {
                        &mut objects
                    } else {
                        &mut refs
                    };
                    match op.value {
                        Some(v) => {
                            table.insert(op.key, v).map_err(other)?;
                        }
                        None => {
                            table.remove(op.key).map_err(other)?;
                        }
                    }
                }
            }
            w.commit().map_err(other)
        }
    }

    pub fn open(dir: impl AsRef<Path>) -> Result<KvStore<Redb>, Error> {
        Ok(KvStore::new(Redb::open(dir)?))
    }
}

// ---------------------------------------------------------------------------
// fjall (a Rust LSM tree), with or without key-value separation.

#[cfg(feature = "store-fjall")]
pub mod fjall_store {
    use super::*;
    use fjall::{Database, Keyspace, KeyspaceCreateOptions, KvSeparationOptions, PersistMode};

    pub struct Fjall {
        db: Database,
        objects: Keyspace,
        refs: Keyspace,
    }

    impl Fjall {
        /// `separate`: keep values of 1 KiB or more in blob files beside the
        /// tree (WiscKey-style key-value separation), so compaction moves only keys.
        pub fn open(dir: impl AsRef<Path>, separate: bool) -> Result<Fjall, Error> {
            let db = Database::builder(dir.as_ref()).open().map_err(other)?;
            let objects = db
                .keyspace("objects", || {
                    let o = KeyspaceCreateOptions::default();
                    if separate {
                        o.with_kv_separation(Some(KvSeparationOptions::default()))
                    } else {
                        o
                    }
                })
                .map_err(other)?;
            let refs = db
                .keyspace("refs", KeyspaceCreateOptions::default)
                .map_err(other)?;
            Ok(Fjall { db, objects, refs })
        }

        fn ks(&self, t: Table) -> &Keyspace {
            match t {
                Table::Objects => &self.objects,
                Table::Refs => &self.refs,
            }
        }
    }

    impl Kv for Fjall {
        fn get(&self, t: Table, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
            Ok(self.ks(t).get(key).map_err(other)?.map(|v| v.to_vec()))
        }

        fn contains(&self, t: Table, key: &[u8]) -> Result<bool, Error> {
            self.ks(t).contains_key(key).map_err(other)
        }

        fn keys(&self, t: Table, prefix: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
            let mut out = Vec::new();
            for g in self.ks(t).prefix(prefix) {
                out.push(g.key().map_err(other)?.to_vec());
            }
            Ok(out)
        }

        fn entries(&self, t: Table) -> Result<Vec<Entry>, Error> {
            let mut out = Vec::new();
            for g in self.ks(t).iter() {
                let (k, v) = g.into_inner().map_err(other)?;
                out.push((k.to_vec(), v.to_vec()));
            }
            Ok(out)
        }

        fn apply(
            &mut self,
            ops: &[KvOp],
            guard: Option<(&str, Option<&[u8]>)>,
        ) -> Result<(), Error> {
            // fjall locks its directory to one process, and `&mut self` makes
            // this the only writer, so checking before the batch is a true CAS.
            if let Some((name, expected)) = guard
                && self.refs.get(name.as_bytes()).map_err(other)?.as_deref() != expected
            {
                return Err(Error::RefConflict(name.into()));
            }
            let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
            for op in ops {
                match op.value {
                    Some(v) => batch.insert(self.ks(op.table), op.key, v),
                    None => batch.remove(self.ks(op.table), op.key),
                }
            }
            batch.commit().map_err(other)
        }
    }

    pub fn open(dir: impl AsRef<Path>, separate: bool) -> Result<KvStore<Fjall>, Error> {
        Ok(KvStore::new(Fjall::open(dir, separate)?))
    }
}

// ---------------------------------------------------------------------------
// RocksDB, with or without BlobDB (key-value separation).

#[cfg(feature = "store-rocksdb")]
pub mod rocks {
    use super::*;
    use rocksdb::{
        ColumnFamilyDescriptor, DB, DBCompressionType, Options, WriteBatch, WriteOptions,
    };

    pub struct Rocks {
        db: DB,
    }

    fn cf(t: Table) -> &'static str {
        match t {
            Table::Objects => "objects",
            Table::Refs => "refs",
        }
    }

    impl Rocks {
        /// Objects are already compressed by the kernel, so the objects column
        /// family stores them uncompressed. `blobs`: BlobDB for values of 1 KiB or more.
        pub fn open(dir: impl AsRef<Path>, blobs: bool) -> Result<Rocks, Error> {
            let mut db_opts = Options::default();
            db_opts.create_if_missing(true);
            db_opts.create_missing_column_families(true);
            let mut objects = Options::default();
            objects.set_compression_type(DBCompressionType::None);
            if blobs {
                objects.set_enable_blob_files(true);
                objects.set_min_blob_size(1024);
                objects.set_blob_compression_type(DBCompressionType::None);
            }
            let cfs = vec![
                ColumnFamilyDescriptor::new("objects", objects),
                ColumnFamilyDescriptor::new("refs", Options::default()),
            ];
            let db = DB::open_cf_descriptors(&db_opts, dir.as_ref(), cfs).map_err(other)?;
            Ok(Rocks { db })
        }
    }

    impl Kv for Rocks {
        fn get(&self, t: Table, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
            let h = self.db.cf_handle(cf(t)).unwrap();
            self.db.get_cf(h, key).map_err(other)
        }

        fn keys(&self, t: Table, prefix: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
            let h = self.db.cf_handle(cf(t)).unwrap();
            let mode = rocksdb::IteratorMode::From(prefix, rocksdb::Direction::Forward);
            let mut out = Vec::new();
            for kv in self.db.iterator_cf(h, mode) {
                let (k, _) = kv.map_err(other)?;
                if !k.starts_with(prefix) {
                    break;
                }
                out.push(k.to_vec());
            }
            Ok(out)
        }

        fn entries(&self, t: Table) -> Result<Vec<Entry>, Error> {
            let h = self.db.cf_handle(cf(t)).unwrap();
            let mut out = Vec::new();
            for kv in self.db.iterator_cf(h, rocksdb::IteratorMode::Start) {
                let (k, v) = kv.map_err(other)?;
                out.push((k.to_vec(), v.to_vec()));
            }
            Ok(out)
        }

        fn apply(
            &mut self,
            ops: &[KvOp],
            guard: Option<(&str, Option<&[u8]>)>,
        ) -> Result<(), Error> {
            // RocksDB locks its directory to one process; `&mut self` makes this
            // the only writer, so checking before the batch is a true CAS.
            if let Some((name, expected)) = guard
                && self.get(Table::Refs, name.as_bytes())?.as_deref() != expected
            {
                return Err(Error::RefConflict(name.into()));
            }
            let mut batch = WriteBatch::default();
            for op in ops {
                let h = self.db.cf_handle(cf(op.table)).unwrap();
                match op.value {
                    Some(v) => batch.put_cf(h, op.key, v),
                    None => batch.delete_cf(h, op.key),
                }
            }
            let mut w = WriteOptions::default();
            w.set_sync(true);
            self.db.write_opt(batch, &w).map_err(other)
        }
    }

    pub fn open(dir: impl AsRef<Path>, blobs: bool) -> Result<KvStore<Rocks>, Error> {
        Ok(KvStore::new(Rocks::open(dir, blobs)?))
    }
}

// ---------------------------------------------------------------------------
// Pack files: append-only, one fsync per ref move, git-style sorted index.

/// A pack is sealed (indexed, then never written again) once it reaches this size.
pub const PACK_SEAL_BYTES: u64 = 512 << 20;
/// The open pack's index is checkpointed after this many new bytes, and on close.
pub const PACK_CKPT_BYTES: u64 = 64 << 20;
const TOMBSTONE: u32 = u32::MAX;
const RECORD_HEAD: u64 = 32 + 4;
const IDX_MAGIC: &[u8; 8] = b"ikamidx1";
const IDX_ENTRY: u64 = 32 + 8 + 4;
const IDX_HEAD: u64 = 8 + 8 + 256 * 4;

/// One sealed pack and its index, opened lazily: only the fanout is read at open.
struct Sealed {
    pack: File,
    idx: File,
    fanout: [u32; 256],
}

impl Sealed {
    fn entry(&self, i: u32) -> Result<(Id, u64, u32), Error> {
        let mut e = [0u8; IDX_ENTRY as usize];
        self.idx
            .read_exact_at(&mut e, IDX_HEAD + i as u64 * IDX_ENTRY)?;
        let id = Id::from_bytes(e[..32].try_into().unwrap());
        Ok((
            id,
            u64::from_le_bytes(e[32..40].try_into().unwrap()),
            u32::from_le_bytes(e[40..44].try_into().unwrap()),
        ))
    }

    /// Offset and length of `id` (length `TOMBSTONE` if deleted here).
    fn find(&self, id: &Id) -> Result<Option<(u64, u32)>, Error> {
        let b = id.as_bytes()[0] as usize;
        let (mut lo, mut hi) = (if b == 0 { 0 } else { self.fanout[b - 1] }, self.fanout[b]);
        let mut key = [0u8; 32];
        // Ids are hashes, so uniform: guess the position from the id's next
        // bytes (interpolation search, about log log n reads), then fall
        // back to halving if the guesses stop closing in.
        let pos = |k: &[u8]| u64::from_be_bytes(k[1..9].try_into().unwrap());
        let target = pos(id.as_bytes());
        let (mut lo_key, mut hi_key) = (0u64, u64::MAX);
        let mut guesses = 0;
        while lo < hi {
            let mid = if guesses < 6 && hi - lo > 8 && lo_key < hi_key {
                guesses += 1;
                let span = (hi - lo) as u128;
                let frac =
                    (target.saturating_sub(lo_key)) as u128 * span / (hi_key - lo_key) as u128;
                lo + (frac as u32).min(hi - lo - 1)
            } else {
                lo + (hi - lo) / 2
            };
            self.idx
                .read_exact_at(&mut key, IDX_HEAD + mid as u64 * IDX_ENTRY)?;
            match key[..].cmp(id.as_bytes()) {
                std::cmp::Ordering::Less => {
                    lo = mid + 1;
                    lo_key = pos(&key);
                }
                std::cmp::Ordering::Greater => {
                    hi = mid;
                    hi_key = pos(&key);
                }
                std::cmp::Ordering::Equal => {
                    let (_, off, len) = self.entry(mid)?;
                    return Ok(Some((off, len)));
                }
            }
        }
        Ok(None)
    }

    fn range(&self, prefix: &[u8]) -> (u32, u32) {
        match prefix.first() {
            None => (0, self.fanout[255]),
            Some(&b) => (
                if b == 0 {
                    0
                } else {
                    self.fanout[b as usize - 1]
                },
                self.fanout[b as usize],
            ),
        }
    }
}

/// Objects appended to pack files, refs as files (as in [`FsStore`](crate::FsStore)).
///
/// Layout: `packs/<n>.pack` holds records `id (32) | len u32 | bytes | crc32`;
/// a sealed pack has `packs/<n>.idx` (sorted ids, offsets and lengths, with a
/// 256-entry fanout). The open pack's index lives in memory and is rebuilt on
/// open by scanning it, stopping at the first torn or corrupt record, which is
/// cut off. New records reach disk with one `sync_data` when a ref moves, before
/// the ref file is renamed into place, so a ref never points past durable data.
/// One process writes at a time (a lock file is held while open).
pub struct PackStore {
    root: PathBuf,
    sealed: Vec<Sealed>, // oldest first; lookups go newest first
    active: File,
    active_no: u32,
    active_index: HashMap<Id, (u64, u32)>,
    buf: Vec<u8>,
    flushed: u64,
    ckpt_at: u64, // where the open pack's last checkpoint ends
    _lock: File,
}

fn crc(id: &Id, len: u32, bytes: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(id.as_bytes());
    h.update(&len.to_le_bytes());
    h.update(bytes);
    h.finalize()
}

impl PackStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<PackStore, Error> {
        let root = root.into();
        fs::create_dir_all(root.join("packs"))?;
        fs::create_dir_all(root.join("refs"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("LOCK"))?;
        lock.try_lock()
            .map_err(|_| Error::RefConflict("LOCK".into()))?;
        let mut nos: Vec<u32> = fs::read_dir(root.join("packs"))?
            .filter_map(|e| {
                e.ok()?
                    .file_name()
                    .to_str()?
                    .strip_suffix(".pack")?
                    .parse()
                    .ok()
            })
            .collect();
        nos.sort();
        let mut sealed = Vec::new();
        let last = nos.last().copied();
        for &n in &nos {
            let idx_path = root.join(format!("packs/{n:08}.idx"));
            if !idx_path.exists() && Some(n) != last {
                // Sealing was cut short: rebuild the index from the pack.
                let (index, _) = scan(&root.join(format!("packs/{n:08}.pack")), 0, HashMap::new())?;
                write_idx(&idx_path, &index)?;
            }
            if idx_path.exists() {
                sealed.push(open_sealed(&root, n)?);
            }
        }
        let active_no = match last {
            Some(n) if !root.join(format!("packs/{n:08}.idx")).exists() => n,
            Some(n) => n + 1,
            None => 0,
        };
        let path = root.join(format!("packs/{active_no:08}.pack"));
        let ckpt = root.join(format!("packs/{active_no:08}.ckpt"));
        let (active_index, good) = if path.exists() {
            let len = fs::metadata(&path)?.len();
            let (index, start) = read_ckpt(&ckpt, len).unwrap_or_default();
            scan(&path, start, index)?
        } else {
            (HashMap::new(), 0)
        };
        let active = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        if active.metadata()?.len() != good {
            active.set_len(good)?; // cut a torn tail
            active.sync_all()?;
        }
        Ok(PackStore {
            root,
            sealed,
            active,
            active_no,
            active_index,
            buf: Vec::new(),
            flushed: good,
            ckpt_at: good,
            _lock: lock,
        })
    }

    fn append(&mut self, id: Id, bytes: Option<&[u8]>) -> Result<(), Error> {
        let len = bytes.map_or(TOMBSTONE, |b| b.len() as u32);
        let body = bytes.unwrap_or(&[]);
        let off = self.flushed + self.buf.len() as u64;
        self.buf.extend_from_slice(id.as_bytes());
        self.buf.extend_from_slice(&len.to_le_bytes());
        self.buf.extend_from_slice(body);
        self.buf
            .extend_from_slice(&crc(&id, len, body).to_le_bytes());
        self.active_index.insert(id, (off, len));
        if self.buf.len() >= 1 << 20 {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        if !self.buf.is_empty() {
            self.active.write_all_at(&self.buf, self.flushed)?;
            self.flushed += self.buf.len() as u64;
            self.buf.clear();
        }
        Ok(())
    }

    /// Make everything appended so far durable, and seal the pack if it is full.
    pub fn sync(&mut self) -> Result<(), Error> {
        self.flush()?;
        self.active.sync_data()?;
        if self.flushed >= PACK_SEAL_BYTES {
            self.seal()?;
        } else if self.flushed - self.ckpt_at >= PACK_CKPT_BYTES {
            self.checkpoint()?;
        }
        Ok(())
    }

    /// Save the open pack's index, so the next open scans only what follows.
    fn checkpoint(&mut self) -> Result<(), Error> {
        let path = self.root.join(format!("packs/{:08}.ckpt", self.active_no));
        write_ckpt(&path, &self.active_index, self.flushed)?;
        self.ckpt_at = self.flushed;
        Ok(())
    }

    fn seal(&mut self) -> Result<(), Error> {
        let n = self.active_no;
        write_idx(
            &self.root.join(format!("packs/{n:08}.idx")),
            &self.active_index,
        )?;
        let _ = fs::remove_file(self.root.join(format!("packs/{n:08}.ckpt")));
        self.sealed.push(open_sealed(&self.root, n)?);
        self.active_no += 1;
        let path = self.root.join(format!("packs/{:08}.pack", self.active_no));
        self.active = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        self.active_index.clear();
        self.flushed = 0;
        self.ckpt_at = 0;
        Ok(())
    }

    /// Where `id` lives: `(sealed pack or None for the open one, offset, len)`.
    fn locate(&self, id: &Id) -> Result<Option<(Option<usize>, u64, u32)>, Error> {
        if let Some(&(off, len)) = self.active_index.get(id) {
            return Ok(Some((None, off, len)));
        }
        for (i, s) in self.sealed.iter().enumerate().rev() {
            if let Some((off, len)) = s.find(id)? {
                return Ok(Some((Some(i), off, len)));
            }
        }
        Ok(None)
    }

    pub fn packs(&self) -> usize {
        self.sealed.len() + 1
    }
}

fn open_sealed(root: &Path, n: u32) -> Result<Sealed, Error> {
    let pack = File::open(root.join(format!("packs/{n:08}.pack")))?;
    let mut idx = File::open(root.join(format!("packs/{n:08}.idx")))?;
    let mut head = vec![0u8; IDX_HEAD as usize];
    idx.read_exact(&mut head)?;
    if &head[..8] != IDX_MAGIC {
        return Err(Error::Decode("pack index magic"));
    }
    let mut fanout = [0u32; 256];
    for (i, f) in fanout.iter_mut().enumerate() {
        let at = 16 + i * 4;
        *f = u32::from_le_bytes(head[at..at + 4].try_into().unwrap());
    }
    Ok(Sealed { pack, idx, fanout })
}

/// Where each object of a pack starts and how long its record is.
type PackIndex = HashMap<Id, (u64, u32)>;

/// Read a pack's records from `start`, stopping at the first torn or
/// corrupt one. Returns the index and the length of the good prefix.
fn scan(path: &Path, start: u64, mut index: PackIndex) -> Result<(PackIndex, u64), Error> {
    let mut f = File::open(path)?;
    let total = f.metadata()?.len();
    f.seek(SeekFrom::Start(start))?;
    // Records are read in order, so one buffered pass with no seeks.
    let mut f = io::BufReader::with_capacity(1 << 20, f);
    let mut off = start;
    let mut head = [0u8; RECORD_HEAD as usize];
    let mut body = Vec::new();
    while off + RECORD_HEAD + 4 <= total {
        f.read_exact(&mut head)?;
        let id = Id::from_bytes(head[..32].try_into().unwrap());
        let len = u32::from_le_bytes(head[32..36].try_into().unwrap());
        let n = if len == TOMBSTONE { 0 } else { len as u64 };
        if off + RECORD_HEAD + n + 4 > total {
            break;
        }
        body.resize(n as usize, 0);
        f.read_exact(&mut body)?;
        let mut c = [0u8; 4];
        f.read_exact(&mut c)?;
        if u32::from_le_bytes(c) != crc(&id, len, &body) {
            break;
        }
        index.insert(id, (off, len));
        off += RECORD_HEAD + n + 4;
    }
    Ok((index, off))
}

/// A checkpoint of the open pack's index: the index file's bytes for the
/// records before `covers`, then `covers` (u64) and a BLAKE3 hash of all of
/// it. Open loads it and scans only what came after; a missing, torn or
/// stale checkpoint only means a longer scan.
fn write_ckpt(path: &Path, index: &HashMap<Id, (u64, u32)>, covers: u64) -> Result<(), Error> {
    let mut out = idx_bytes(index);
    out.extend_from_slice(&covers.to_le_bytes());
    let h = blake3::hash(&out);
    out.extend_from_slice(h.as_bytes());
    let tmp = path.with_extension("ckpt.tmp");
    fs::write(&tmp, &out)?;
    fs::rename(&tmp, path)?; // no fsync: losing it costs a scan, nothing more
    Ok(())
}

fn read_ckpt(path: &Path, pack_len: u64) -> Option<(PackIndex, u64)> {
    let b = fs::read(path).ok()?;
    if b.len() < (IDX_HEAD + 8 + 32) as usize || &b[..8] != IDX_MAGIC {
        return None;
    }
    let (body, hash) = b.split_at(b.len() - 32);
    if blake3::hash(body).as_bytes() != hash {
        return None;
    }
    let count = u64::from_le_bytes(body[8..16].try_into().ok()?) as usize;
    if body.len() != IDX_HEAD as usize + count * IDX_ENTRY as usize + 8 {
        return None;
    }
    let covers = u64::from_le_bytes(body[body.len() - 8..].try_into().ok()?);
    if covers > pack_len {
        return None; // the pack lost records the checkpoint names
    }
    let mut index = HashMap::with_capacity(count);
    for i in 0..count {
        let e = &body[IDX_HEAD as usize + i * IDX_ENTRY as usize..][..IDX_ENTRY as usize];
        let id = Id::from_bytes(e[..32].try_into().ok()?);
        let off = u64::from_le_bytes(e[32..40].try_into().ok()?);
        let len = u32::from_le_bytes(e[40..44].try_into().ok()?);
        index.insert(id, (off, len));
    }
    Some((index, covers))
}

fn idx_bytes(index: &HashMap<Id, (u64, u32)>) -> Vec<u8> {
    let mut entries: Vec<(&Id, &(u64, u32))> = index.iter().collect();
    entries.sort_by_key(|(id, _)| **id);
    let mut fanout = [0u32; 256];
    for (id, _) in &entries {
        fanout[id.as_bytes()[0] as usize] += 1;
    }
    for i in 1..256 {
        fanout[i] += fanout[i - 1];
    }
    let mut out = Vec::with_capacity(IDX_HEAD as usize + entries.len() * IDX_ENTRY as usize);
    out.extend_from_slice(IDX_MAGIC);
    out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    for f in fanout {
        out.extend_from_slice(&f.to_le_bytes());
    }
    for (id, (off, len)) in entries {
        out.extend_from_slice(id.as_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
    }
    out
}

fn write_idx(path: &Path, index: &HashMap<Id, (u64, u32)>) -> Result<(), Error> {
    let out = idx_bytes(index);
    let tmp = path.with_extension("idx.tmp");
    let mut f = File::create(&tmp)?;
    f.write_all(&out)?;
    f.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

impl PackStore {
    /// Up to `n` bytes of the record for `id`, from its start.
    fn read_prefix(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        match self.locate(id)? {
            None => Err(Error::NotFound(*id)),
            Some((_, _, TOMBSTONE)) => Err(Error::NotFound(*id)),
            Some((pack, off, len)) => {
                let len = (len as usize).min(n) as u32;
                let mut out = vec![0u8; len as usize];
                let at = off + RECORD_HEAD;
                match pack {
                    Some(i) => self.sealed[i].pack.read_exact_at(&mut out, at)?,
                    None if at >= self.flushed => {
                        let s = (at - self.flushed) as usize;
                        out.copy_from_slice(&self.buf[s..s + len as usize]);
                    }
                    None => self.active.read_exact_at(&mut out, at)?,
                }
                Ok(out)
            }
        }
    }
}

impl Drop for PackStore {
    fn drop(&mut self) {
        let _ = self
            .flush()
            .and_then(|_| self.active.sync_data().map_err(Error::Io));
        if self.flushed > self.ckpt_at {
            let _ = self.checkpoint();
        }
    }
}

impl Store for PackStore {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        if self.has(&id) {
            return Ok(false);
        }
        self.append(id, Some(encoded))?;
        Ok(true)
    }

    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.append(id, Some(encoded))
    }

    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        match self.locate(id)? {
            None => Err(Error::NotFound(*id)),
            Some((_, _, TOMBSTONE)) => Err(Error::NotFound(*id)),
            Some((pack, off, len)) => {
                let at = off + RECORD_HEAD;
                if pack.is_none() && at >= self.flushed {
                    let s = (at - self.flushed) as usize;
                    return Ok(self.buf[s..s + len as usize].to_vec());
                }
                // On disk: the whole record in one read, its CRC checked, so
                // a record a checkpoint or index names is still verified.
                let mut rec = vec![0u8; RECORD_HEAD as usize + len as usize + 4];
                match pack {
                    Some(i) => self.sealed[i].pack.read_exact_at(&mut rec, off)?,
                    None => self.active.read_exact_at(&mut rec, off)?,
                }
                let (head, rest) = rec.split_at(RECORD_HEAD as usize);
                let (body, c) = rest.split_at(len as usize);
                if &head[..32] != id.as_bytes()
                    || u32::from_le_bytes(c.try_into().unwrap()) != crc(id, len, body)
                {
                    return Err(Error::Corrupt(*id));
                }
                Ok(body.to_vec())
            }
        }
    }

    // E139: the first `n` bytes of a record, read from the pack alone.
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        self.read_prefix(id, n)
    }

    // E147: from the index alone.
    fn stored_len(&self, id: &Id) -> Result<usize, Error> {
        match self.locate(id)? {
            Some((_, _, len)) if len != TOMBSTONE => Ok(len as usize),
            _ => Err(Error::NotFound(*id)),
        }
    }

    fn has(&self, id: &Id) -> bool {
        matches!(self.locate(id), Ok(Some((_, _, len))) if len != TOMBSTONE)
    }

    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.ids_with_prefix(&[])
    }

    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        // Oldest first, so newer packs overwrite: a later tombstone wins.
        let mut live: BTreeMap<Id, bool> = BTreeMap::new();
        for s in &self.sealed {
            let (lo, hi) = s.range(prefix);
            for i in lo..hi {
                let (id, _, len) = s.entry(i)?;
                if id.as_bytes().starts_with(prefix) {
                    live.insert(id, len != TOMBSTONE);
                }
            }
        }
        for (id, &(_, len)) in &self.active_index {
            if id.as_bytes().starts_with(prefix) {
                live.insert(*id, len != TOMBSTONE);
            }
        }
        Ok(live
            .into_iter()
            .filter(|(_, l)| *l)
            .map(|(id, _)| id)
            .collect())
    }

    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        if !self.has(id) {
            return Err(Error::NotFound(*id));
        }
        self.append(*id, None)
    }

    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        check_ref_name(name)?;
        match fs::read_to_string(self.root.join("refs").join(name)) {
            Ok(s) => Ok(Some(s.trim().parse()?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        check_ref_name(name)?;
        if self.get_ref(name)? != expected {
            return Err(Error::RefConflict(name.into()));
        }
        self.sync()?; // objects first, then the ref that reaches them
        let path = self.root.join("refs").join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_file_name(format!(
            "{}.lock",
            path.file_name().unwrap().to_string_lossy()
        ));
        let mut f = File::create(&tmp)?;
        f.write_all(format!("{new}\n").as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        let mut out = Vec::new();
        let base = self.root.join("refs");
        let mut stack = vec![base.clone()];
        while let Some(dir) = stack.pop() {
            for e in fs::read_dir(&dir)? {
                let e = e?;
                let p = e.path();
                if e.file_type()?.is_dir() {
                    stack.push(p);
                } else if !p.to_string_lossy().ends_with(".lock") {
                    let name = p
                        .strip_prefix(&base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    out.push((name, fs::read_to_string(&p)?.trim().parse()?));
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod pack_index_tests {
    use super::*;

    #[test]
    fn sealed_lookup_finds_every_id_and_no_other() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("packs")).unwrap();
        File::create(dir.path().join("packs/00000000.pack")).unwrap();
        let mut index = HashMap::new();
        for n in 0..200_000u32 {
            index.insert(Id::of(&n.to_le_bytes()), (n as u64 * 7, n));
        }
        write_idx(&dir.path().join("packs/00000000.idx"), &index).unwrap();
        let s = open_sealed(dir.path(), 0).unwrap();
        for (id, at) in &index {
            assert_eq!(s.find(id).unwrap(), Some(*at));
        }
        for n in 200_000..210_000u32 {
            assert_eq!(s.find(&Id::of(&n.to_le_bytes())).unwrap(), None);
        }
        // Clustered ids (not hash-like) still work through the fallback.
        let mut index = HashMap::new();
        for n in 0..5_000u32 {
            let mut b = [0u8; 32];
            b[0] = 7;
            b[28..].copy_from_slice(&n.to_be_bytes());
            index.insert(Id::from_bytes(b), (n as u64, 1));
        }
        write_idx(&dir.path().join("packs/00000000.idx"), &index).unwrap();
        let s = open_sealed(dir.path(), 0).unwrap();
        for (id, at) in &index {
            assert_eq!(s.find(id).unwrap(), Some(*at));
        }
    }
}
