//! `ikam` CLI over a filesystem store.

/// `eprintln!` that never panics (E374): a server whose stderr went away
/// (the terminal closed, a pipe's reader quit) carries on instead of losing
/// the thread that logged, which was a request (500s with `--log-requests`)
/// or the stop watcher (SIGTERM no longer stopped it).
macro_rules! say {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}

mod ask;
mod auth;
mod corpus;
mod dav;
mod dbsearch;
mod dbserve;
mod dbviews;
mod host;
mod http;
mod json;
mod live;
mod mcp;
mod serve;
mod text;
mod tokens;
mod ui;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ikam_kernel::{Arg, FsStore, Id, Object, Repo, snapshot};

use corpus::{err, lookup};

const USAGE: &str = "usage: ikam [--store DIR] [--short-records] [--slow-codec-max BYTES] [--group-size BYTES] [--needed-index] [--read-memo] <command>
  ingest DIR [-m MSG] [--ref REF]  commit DIR, repack, verify; print what it cost
  ls [REV[:PATH]] [-r]             list a directory of REV (default main)
  grep TEXT [REV]                  files of REV containing TEXT, with lines
  stats [REV]                      objects and bytes the store holds
  db serve [--live] [--packs] [--plain-events] [--no-object-templates] [--concurrent-reads] [--edit-index] [--plain-writes] [--hnsw] [--keep-search] [--no-keep-search] [--shared-store] [--skip-unchanged] [--keep-counts N] [--defer-kv N] [--views FILE] [--keep-views] [--keep-on-hangup] [PORT]
                                   the store's databases over HTTP, for programs
                                   (default port 8740, 127.0.0.1 only; dbserve.rs);
                                   --live turns on the options a live database
                                   wants (E356, E372: with --defer-kv 20000 and
                                   --keep-search; --defer-kv 0 or --no-keep-search
                                   turns those off); --keep-on-hangup
                                   ignores SIGHUP (E374); --skip-unchanged drops
                                   upserts of a key's current value (E376, off)
  host --config FILE [PORT]        several separate projects in one process, each
                                   its own store, users and limits, chosen by Host
                                   name; crossing only through scoped shares (host.rs)
  serve [PORT]                     browse, read and search in a local web page
                                   (default port 8737, 127.0.0.1 only)
  REV, ID and PATH arguments take a ref, a hex id or unique prefix, or REV:PATH.
  put FILE                         store a file, print its content id
  cat ID                           write a file's bytes to stdout
  commit DIR [-m MSG] [--ref REF]  snapshot DIR and advance REF (default main)
  log [REV]                        first-parent history of REV (default main)
  checkout REV DEST                write REV's tree into new directory DEST
  show ID                          print a tree, commit or derivation record
  apply FUNC ARG...                run function FUNC (a stored WASM module id) on
                                   stored contents, print the output id (memoized)
  links ID                         outgoing graph edges of an object or content
  used-by ID                       incoming graph edges (objects that link to ID)
  fsck                             reconstruct and verify every stored object
  gc                               delete objects unreachable from any ref
  claim SUBJ PRED OBJ [--by ID]    record a semantic claim; its weight (bits the
                                   object saves on the subject) is measured
  claims ID                        claims about ID, with measured gains
  relate ID [K]                    the K stored contents most informative about ID
  promote CLAIM                    derive the claim's subject from its object, if smaller
  repack                           re-plan storage of all live content; applied
                                   only if smaller and fully verified
  live [PORT] [--watch DIR[:PREFIX]]... [--llm] [--commit-after MS]
       [--allow-host NAME]... [--log-requests] [--public]
       [--max-body BYTES] [--max-store BYTES] [--max-conns N] [--gc-every N]
       [--deferred-sync] [--no-object-templates] [--passage-index] [--keep-on-hangup]
                                   run REF (default main) as a live filesystem:
                                   WebDAV at http://127.0.0.1:PORT/dav/ (mount it
                                   in Finder: Go > Connect to Server), a web page
                                   at /, and each --watch folder mirrored into the
                                   tree as it changes. Edits commit after MS of
                                   quiet (default 1500). --llm lets questions be
                                   answered by Claude (needs ANTHROPIC_API_KEY).
                                   --deferred-sync makes edits durable when they
                                   commit, with one sync, not one per object;
                                   --no-object-templates plans trees only as
                                   slices of earlier versions (E227, E228).
                                   --allow-host also answers to NAME (such as the
                                   machine's Tailscale name behind Tailscale
                                   Serve); it still listens on 127.0.0.1 only. With
                                   IKAM_AUTH set (from passwd), every request needs
                                   that user and password. SIGTERM, Ctrl-C or
                                   SIGHUP (the terminal closed) commits pending
                                   edits, logs which signal, then exits;
                                   --keep-on-hangup ignores SIGHUP instead.
                                   --log-requests prints one line per request
                                   (time, method, path, status, ms, client).
                                   For a copy open to anyone: --max-body refuses
                                   larger uploads (413), --max-store refuses new
                                   content once the store reaches BYTES (507),
                                   --max-conns serves N connections at once (503
                                   past that); BYTES takes K, M or G. --public
                                   (needs IKAM_AUTH) lets anyone without the
                                   password read (browse, search, download, mount
                                   read-only) but not write, edit, delete, run
                                   repack, fsck or commit, or ask Claude; it sets
                                   those limits to 25M, 1G and 32 unless given.
                                   --gc-every deletes what no commit needs (file
                                   versions and trees written between commits)
                                   after every N commits; POST /api/gc does it
                                   once. Each walks the whole store while edits
                                   wait, so it is off by default.
                                   --passage-index answers questions (the page's
                                   Ask, /api/ask, the passages tool) from an
                                   index of words kept as files change: same
                                   answers, more memory (E378, off).
  passwd USER                      read a password from stdin, print the
                                   IKAM_AUTH line for it
  token                            a new host token (E363): the config's
                                   ID:HASH and the Bearer token for a client
  write PATH [FILE|-]              store FILE (or stdin) at PATH in REF, commit
  rm PATH | mkdir PATH             remove a file or folder / make a folder in REF
  mv FROM TO | cp FROM TO          rename or copy a file or folder in REF
  mcp [--passage-index]            the store as tools for an AI agent: a Model
                                   Context Protocol server on stdin/stdout (list,
                                   read, search, write, edit, commit with a
                                   message, diff, history, drafts) (E359)
                                   --passage-index answers `passages` from an
                                   index of words kept as files change, not a
                                   scan of all text: same answers, more memory
                                   (E378, off)
  search TEXT                      files of REF whose text contains TEXT (also the
                                   text inside docx, xlsx, pdf and tar members)
  ask QUESTION [--llm]             the passages of REF that answer QUESTION best;
                                   with --llm, an answer from them by Claude";

fn main() -> ExitCode {
    quiet_on_closed_stdout();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let store_dir = take_flag(&mut args, "--store").unwrap_or_else(|| ".ikam".into());
    if args.first().map(String::as_str) == Some("passwd") {
        return match passwd(args.get(1).map(String::as_str)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                say!("{msg}");
                ExitCode::FAILURE
            }
        };
    }
    if args.first().map(String::as_str) == Some("token") {
        // E363: a host token; the config gets its hash, the client the token.
        return match tokens::make() {
            Ok((config, token)) => {
                println!("CONFIG={config}");
                println!("TOKEN={token}");
                ExitCode::SUCCESS
            }
            Err(msg) => {
                say!("{msg}");
                ExitCode::FAILURE
            }
        };
    }
    if args.first().map(String::as_str) == Some("host") {
        return match host::host_cmd(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                say!("{msg}");
                ExitCode::FAILURE
            }
        };
    }
    if args.first().map(String::as_str) == Some("db") {
        return match db_cmd(&store_dir, &args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                say!("{msg}");
                ExitCode::FAILURE
            }
        };
    }
    match run(&store_dir, &mut args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            say!("{msg}");
            ExitCode::FAILURE
        }
    }
}

/// `ikam db serve [--live] [--packs] [--plain-events] [--no-object-templates]
/// [--concurrent-reads] [--edit-index] [--plain-writes] [--hnsw] [--keep-search] [--no-keep-search] [--shared-store] [--skip-unchanged] [--keep-counts N] [--defer-kv N]
/// [--views FILE] [--keep-views]
/// [--allow-host NAME]...
/// [--log-requests] [--keep-on-hangup] [PORT]`:
/// the store's databases over HTTP (`dbserve.rs`), on 127.0.0.1:PORT (default
/// 8740). With `--packs` the store is a pack store (E122), which keeps many
/// small events in a few files; a store keeps the backend it was made with.
/// `--plain-events` stores row events without a plan search and
/// `--no-object-templates` plans view chunks as slices only (E229: both
/// together make a bulk load about five times faster; both off by default).
/// `--concurrent-reads` (E232, off by default) gives reads their own `Repo`
/// on a shared store, so they wait neither for an append nor for each other;
/// `--edit-index` (E232, off) plans each write against the chunks it read
/// instead of an index of the whole store, which a restarted server builds
/// on its first write. `--plain-writes` (E357, off) stores the view chunks
/// an append rewrites compressed as they are, without the plan search
/// (`Repo::plain_hot_writes`, E261): appends to a table with keys spread
/// over it (a secondary index) are 3 to 5 times faster, and the store grows
/// faster, since a rewritten chunk is no longer a slice of the one before.
/// `--keep-counts N` (E233, off) keeps the number of
/// keys under each prefix of up to N segments, edited by every append, so
/// `count?prefix=` and `span` read one node per prefix. `--defer-kv N`
/// (E368, off; 20000 under `--live`) logs each append at once but folds appends into the kv view
/// only once N events wait, or before a read of the head, so a view chunk
/// that many batches touch is rewritten once (`Repo::db_append_deferred`).
///
/// `search` and `vsearch` (E358) rank rows by BM25, by a vector the client
/// sends, or both fused; their indexes are built on a prefix's first query
/// and kept in memory. Vectors are scored exactly unless `--hnsw` (off)
/// searches them with a graph (E332: about 0.93 of the exact top 10).
/// `--keep-search` (E369, off; on under `--live`) keeps each BM25 view on a ref, so a
/// restarted server reads it back instead of building it on the first
/// query (E358: 8.6 s for 64 MB of text); the ref keeps the view's data
/// through `gc`.
///
/// `--views FILE` (E361) keeps the groups and indexes FILE defines for
/// every database (`dbviews.rs`), read with `view?name=V`. They are built on
/// a database's first view request after each start (E379: 3 s for an index
/// of 200k rows); `--keep-views` (E380, off) keeps them on a ref, as
/// `--keep-search` keeps search views, so a restarted server reads them back
/// and brings them to the head from the changes since.
///
/// `--shared-store` (E370, off) is for a store other processes write too
/// (another server, `ikam mcp`, a script): each handle lists the store
/// again when it misses an object another process stored
/// (`Repo::refresh_ledger`, E298), and an append commits only over the
/// head it read, applying its events again over a newer head, so neither
/// process's reads fail nor its events get lost. A read of content that
/// really is missing pays a listing of the store. Not for a pack store
/// (`--packs`, `--live`), which one process opens.
///
/// `--skip-unchanged` (E376, off) drops from each batch the upserts that
/// would write a key's current value again, so importing the same rows
/// again logs nothing (E376: re-sending 200k unchanged rows under `--live`
/// grew the store 161 to 348 MB). A dropped event's idempotency key is not
/// recorded. Keys written by deferred appends not yet folded are not
/// checked, nor any key after a subtree or range removal in a batch, and
/// nothing is skipped with `--shared-store`.
///
/// `--live` (E356) is the options a database kept by a running program
/// wants, together: `--packs --plain-events --no-object-templates
/// --concurrent-reads --edit-index`, and since E372 `--defer-kv 20000
/// --keep-search` (E371: a 200k-row load 78.6 to 28.3 s, store 276 to
/// 161 MB, a restart's first search 8.8 s to 4 ms; reads wait for a fold,
/// p50 6.6 to 17.6 ms). Without `--edit-index` a restarted server's first
/// write builds the whole store's index (E354: 0.22 s at 10k rows, 5.06 s at
/// 200k; 0.11-0.13 s with it). It is still an option, as every trade-off is:
/// `--defer-kv N` sets another N (0 folds every append at once) and
/// `--no-keep-search` keeps search views in memory only. It implies
/// `--packs`, so use it on a new store or a pack store.
///
/// SIGTERM, SIGINT and SIGHUP stop the server with a line naming the signal
/// (E374: a SIGHUP, sent when the terminal that started it closes, used to
/// end it with no line logged). `--keep-on-hangup` (off) ignores SIGHUP, as
/// `nohup` would. Every answered append is committed, so a stop, or a kill,
/// loses none (E374).
fn db_cmd(store_dir: &str, args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    if args.first().map(String::as_str) != Some("serve") {
        return Err("usage: ikam [--store DIR] db serve [--live] [--packs] [--plain-events] [--no-object-templates] [--concurrent-reads] [--edit-index] [--plain-writes] [--hnsw] [--keep-search] [--no-keep-search] [--shared-store] [--skip-unchanged] [--keep-counts N] [--defer-kv N] [--views FILE] [--keep-views] [--allow-host NAME]... [--log-requests] [--keep-on-hangup] [PORT]".into());
    }
    args.remove(0);
    let keep_on_hangup = take_switch(&mut args, "--keep-on-hangup");
    let mut allowed = Vec::new();
    while let Some(h) = take_flag(&mut args, "--allow-host") {
        allowed.push(h);
    }
    let keep_counts: u32 = match take_flag(&mut args, "--keep-counts") {
        Some(n) => n.parse().map_err(err)?,
        None => 0,
    };
    let defer_kv: Option<u64> = match take_flag(&mut args, "--defer-kv") {
        Some(n) => Some(n.parse().map_err(err)?),
        None => None,
    };
    let keep_views = {
        let before = args.len();
        args.retain(|a| a != "--keep-views");
        args.len() != before
    };
    let views = match take_flag(&mut args, "--views") {
        Some(path) => {
            let text = std::fs::read_to_string(&path).map_err(err)?;
            Some((dbviews::parse(&text)?, keep_views))
        }
        None if keep_views => return Err("--keep-views needs --views FILE".into()),
        None => None,
    };
    let (packs, log_requests, serve_opts) = db_flags(&mut args, keep_counts, defer_kv);
    if packs && serve_opts.shared_store {
        return Err(
            "--shared-store is for a store on disk (FsStore), not --packs or --live".into(),
        );
    }
    let port: u16 = args.first().map_or(Ok(8740), |p| p.parse()).map_err(err)?;
    auth::catch_stop_signals(keep_on_hangup);
    std::thread::spawn(|| {
        // Every answered append is committed already, so a stop has
        // nothing to flush.
        loop {
            std::thread::sleep(Duration::from_millis(250));
            if auth::stopping() {
                say!("{} stopped ({})", live::stamp(), auth::stop_signal());
                std::process::exit(0);
            }
        }
    });
    if packs {
        let opts = ikam_kernel::backends::PackOptions {
            lazy_checkpoint: true,
            ref_log: false,
            filters: true,
        };
        let store = ikam_kernel::backends::PackStore::open_with(store_dir, opts).map_err(err)?;
        serve_db(store, serve_opts, views, port, allowed, log_requests)
    } else {
        let store = FsStore::open(store_dir).map_err(err)?;
        serve_db(store, serve_opts, views, port, allowed, log_requests)
    }
}

/// Takes `db serve`'s switches out of `args`: whether the store is a pack
/// store, whether to log requests, and the serving options. `--live` turns
/// on `--packs`, `--plain-events`, `--no-object-templates`,
/// `--concurrent-reads` and `--edit-index` (E356), and `--keep-search` and
/// `--defer-kv` [`LIVE_DEFER_KV`] (E372); naming any of them as well
/// changes nothing. `defer_kv` is the `--defer-kv` value, if one was given:
/// it wins over the preset's, so `--defer-kv 0` turns deferring off, and
/// `--no-keep-search` turns kept search views off.
fn db_flags(
    args: &mut Vec<String>,
    keep_counts: u32,
    defer_kv: Option<u64>,
) -> (bool, bool, DbServeOpts) {
    let mut take = |name: &str| {
        let before = args.len();
        args.retain(|a| a != name);
        args.len() != before
    };
    let live = take("--live");
    let packs = take("--packs") || live;
    let log_requests = take("--log-requests");
    let plain_events = take("--plain-events") || live;
    let object_templates = !(take("--no-object-templates") || live);
    let concurrent_reads = take("--concurrent-reads") || live;
    let edit_index = take("--edit-index") || live;
    let plain_writes = take("--plain-writes");
    let hnsw = take("--hnsw");
    let keep_search = take("--keep-search") || live;
    let keep_search = !take("--no-keep-search") && keep_search;
    let defer_kv = defer_kv.unwrap_or(if live { LIVE_DEFER_KV } else { 0 });
    let shared_store = take("--shared-store");
    let skip_unchanged = take("--skip-unchanged");
    let opts = DbServeOpts {
        concurrent_reads,
        edit_index,
        plain_writes,
        plain_events,
        object_templates,
        keep_counts,
        defer_kv,
        hnsw,
        keep_search,
        shared_store,
        skip_unchanged,
    };
    (packs, log_requests, opts)
}

/// `--defer-kv` under `--live` (E372): the N measured in E371.
const LIVE_DEFER_KV: u64 = 20_000;

#[derive(Clone, Copy, Debug, PartialEq)]
struct DbServeOpts {
    concurrent_reads: bool,
    edit_index: bool,
    plain_writes: bool,
    plain_events: bool,
    object_templates: bool,
    keep_counts: u32,
    /// E368: fold appends into the kv view once this many events wait.
    defer_kv: u64,
    hnsw: bool,
    keep_search: bool,
    shared_store: bool,
    /// E376: drop upserts of a key's current value.
    skip_unchanged: bool,
}

/// `db serve` on `store`; with `concurrent_reads` the store is shared, so
/// reads get `Repo`s of their own (E232).
fn search(o: DbServeOpts) -> dbsearch::SearchOpts {
    let index = match o.hnsw {
        true => dbsearch::VectorIndex::Hnsw,
        false => dbsearch::VectorIndex::Exact,
    };
    dbsearch::SearchOpts {
        index,
        keep: o.keep_search,
    }
}

fn serve_db<S: ikam_kernel::Store + Send + Sync + 'static>(
    store: S,
    o: DbServeOpts,
    views: Option<(Vec<dbviews::Rule>, bool)>,
    port: u16,
    allowed: Vec<String>,
    log_requests: bool,
) -> Result<(), String> {
    let views = views
        .map(|(rules, keep)| dbviews::Served::new(rules).map(|v| v.keep(keep)))
        .transpose()?;
    fn tune<T: ikam_kernel::Store>(mut r: Repo<T>, o: DbServeOpts) -> Repo<T> {
        r.plain_db_events = o.plain_events;
        r.object_templates = o.object_templates;
        r.plain_hot_writes = o.plain_writes;
        r.refresh_ledger = o.shared_store;
        if o.edit_index {
            // E232: plan against what each edit reads (E133's scope and
            // hints), not an index of every stored object built on the
            // first write.
            r.set_seed_scope(Some(Vec::new()));
            r.replace_hints = true;
        }
        r
    }
    if o.concurrent_reads {
        let shared = ikam_kernel::SharedStore::new(store);
        let make: dbserve::ReaderFactory<_> = Box::new(|w: &Repo<_>| w.sibling());
        dbserve::serve(
            tune(Repo::new(shared), o),
            Some(make),
            (o.keep_counts, o.defer_kv, o.skip_unchanged),
            (search(o), views),
            port,
            allowed,
            log_requests,
        )
    } else {
        dbserve::serve(
            tune(Repo::new(store), o),
            None,
            (o.keep_counts, o.defer_kv, o.skip_unchanged),
            (search(o), views),
            port,
            allowed,
            log_requests,
        )
    }
}

/// Print `IKAM_AUTH=...` for USER and the password on stdin's first line.
/// Needs no store, so it opens none.
fn passwd(user: Option<&str>) -> Result<(), String> {
    let user = user.ok_or("usage: ikam passwd USER (password on stdin)")?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(err)?;
    let password = line.trim_end_matches(['\n', '\r']);
    if password.is_empty() {
        return Err("empty password".into());
    }
    println!("IKAM_AUTH={}", auth::Auth::line(user, password)?);
    Ok(())
}

/// `ikam ls -r | head` ends with stdout closed: exit quietly, as other
/// command-line tools do, instead of printing a panic.
fn quiet_on_closed_stdout() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .unwrap_or("");
        if msg.starts_with("failed printing to stdout") && msg.contains("Broken pipe") {
            std::process::exit(141);
        }
        default(info)
    }));
}

fn run(store_dir: &str, args: &mut Vec<String>) -> Result<(), String> {
    let mut repo = Repo::new(FsStore::open(store_dir).map_err(err)?);
    if let Some(i) = args.iter().position(|a| a == "--short-records") {
        args.remove(i);
        repo.short_records = true;
    }
    if let Some(i) = args.iter().position(|a| a == "--needed-index") {
        args.remove(i);
        repo.needed_index = true;
    }
    if let Some(i) = args.iter().position(|a| a == "--read-memo") {
        args.remove(i);
        repo.read_memo = true;
    }
    if let Some(n) = take_flag(args, "--slow-codec-max") {
        repo.slow_codec_max = n
            .parse()
            .map_err(|_| "--slow-codec-max BYTES".to_string())?;
    }
    if let Some(n) = take_flag(args, "--group-size") {
        repo.group_size = n
            .parse()
            .ok()
            .filter(|&n: &usize| n > 0)
            .ok_or_else(|| "--group-size BYTES".to_string())?;
    }
    let message = take_flag(args, "-m").unwrap_or_default();
    let refname = take_flag(args, "--ref").unwrap_or_else(|| "main".into());
    let by = take_flag(args, "--by");
    let arg = |i: usize| args.get(i).cloned().ok_or_else(|| USAGE.to_string());
    match args.first().map(String::as_str) {
        Some("put") => {
            let input = std::fs::read(arg(1)?).map_err(err)?;
            let put = repo.put_content(&input).map_err(err)?;
            println!("{}", put.id);
            say!(
                "form={:?} input_bytes={} new_bytes={}",
                put.form,
                input.len(),
                repo.bytes_written
            );
        }
        Some("cat") => {
            let bytes = repo
                .read_content(&lookup(&repo, &arg(1)?).map_err(err)?)
                .map_err(err)?;
            std::io::stdout().write_all(&bytes).map_err(err)?;
        }
        Some("commit") => {
            let tree = snapshot::snapshot(&mut repo, Path::new(&arg(1)?), Path::new(store_dir))
                .map_err(err)?;
            let id = repo.commit(&refname, tree, &message).map_err(err)?;
            println!("{id}");
            say!("new_bytes={}", repo.bytes_written);
        }
        Some("log") => {
            let start = repo
                .resolve(args.get(1).map_or("main", String::as_str))
                .map_err(err)?;
            for (id, c) in repo.log(start).map_err(err)? {
                println!("{} tree={} {}", id, &c.tree.to_hex()[..12], c.message);
            }
        }
        Some("checkout") => {
            let tree =
                snapshot::tree_of(&repo, &lookup(&repo, &arg(1)?).map_err(err)?).map_err(err)?;
            snapshot::checkout(&repo, &tree, Path::new(&arg(2)?)).map_err(err)?;
        }
        Some("show") => match repo
            .get(&lookup(&repo, &arg(1)?).map_err(err)?)
            .map_err(err)?
        {
            Object::Tree(entries) => entries
                .iter()
                .for_each(|e| println!("{:?} {} {}", e.kind, e.id, e.name)),
            Object::Commit(c) => {
                println!("tree {}\nparents {:?}\n\n{}", c.tree, c.parents, c.message)
            }
            Object::Derivation(d) => {
                println!("output {}\nfunc {}\nargs {:?}", d.output, d.func, d.args)
            }
            _ => unreachable!(),
        },
        Some("apply") => {
            let func = lookup(&repo, &arg(1)?).map_err(err)?;
            let args = args[2..]
                .iter()
                .map(|a| a.parse::<Id>().map(Arg::Whole))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            println!("{}", repo.apply(func, args).map_err(err)?);
        }
        Some("links") => {
            for (label, id) in repo
                .links(&lookup(&repo, &arg(1)?).map_err(err)?)
                .map_err(err)?
            {
                println!("{id} {label}");
            }
        }
        Some("used-by") => {
            for (id, label) in repo
                .used_by(&lookup(&repo, &arg(1)?).map_err(err)?)
                .map_err(err)?
            {
                println!("{id} {label}");
            }
        }
        Some("fsck") => {
            let failures = repo.fsck_deep().map_err(err)?;
            for (id, e) in &failures {
                println!("{id} {e}");
            }
            if !failures.is_empty() {
                return Err(format!("{} objects failed verification", failures.len()));
            }
            println!("ok");
        }
        Some("repack") => {
            let r = repo.repack().map_err(err)?;
            println!(
                "before={} after={} applied={}",
                r.before, r.after, r.applied
            );
        }
        Some("claim") => {
            let by = by.map(|b| lookup(&repo, &b)).transpose().map_err(err)?;
            let (s, o) = (
                lookup(&repo, &arg(1)?).map_err(err)?,
                lookup(&repo, &arg(3)?).map_err(err)?,
            );
            let id = repo
                .claim(Arg::Whole(s), &arg(2)?, Arg::Whole(o), by)
                .map_err(err)?;
            println!("{id}");
        }
        Some("claims") => {
            for (id, c) in repo
                .claims(&lookup(&repo, &arg(1)?).map_err(err)?)
                .map_err(err)?
            {
                println!(
                    "{id} {} {} {} gain_bits={}",
                    c.subject.id(),
                    c.predicate,
                    c.object.id(),
                    c.gain_bits
                );
            }
        }
        Some("relate") => {
            let k = args.get(2).map_or(Ok(5), |k| k.parse()).map_err(err)?;
            for (id, gain) in repo
                .relate(&lookup(&repo, &arg(1)?).map_err(err)?, k)
                .map_err(err)?
            {
                println!("{id} gain_bits={gain}");
            }
        }
        Some("promote") => match repo
            .promote(&lookup(&repo, &arg(1)?).map_err(err)?)
            .map_err(err)?
        {
            Some(put) => println!("{} now derived", put.id),
            None => println!("no smaller derivation found"),
        },
        Some("ingest") => ingest(&mut repo, store_dir, &arg(1)?, &refname, &message)?,
        Some("ls") => {
            let recursive = args.iter().any(|a| a == "-r");
            let spec = args
                .get(1)
                .filter(|a| *a != "-r")
                .map_or("main", String::as_str);
            list(&repo, spec, recursive)?;
        }
        Some("grep") => {
            let root = corpus::tree(
                &repo,
                &lookup(&repo, args.get(2).map_or("main", String::as_str))?,
            )?;
            for hit in corpus::grep(&repo, &root, &arg(1)?).map_err(err)? {
                println!("{}", hit.path);
                for (n, line) in hit.lines {
                    println!("  {n}: {line}");
                }
            }
        }
        Some("stats") => {
            let rev = lookup(&repo, args.get(1).map_or("main", String::as_str)).ok();
            let s = corpus::stats(&repo, rev.as_ref()).map_err(err)?;
            println!(
                "files={} input_bytes={} stored_bytes={}",
                s.files, s.input, s.stored
            );
            for (k, (n, b)) in s.kinds {
                println!("{k:<22} {n:>8} objects {b:>12} bytes");
            }
        }
        Some("serve") => {
            let port = args.get(1).map_or(Ok(8737), |p| p.parse()).map_err(err)?;
            serve::serve(&repo, port)?;
        }
        Some("gc") => println!("deleted {} objects", repo.gc().map_err(err)?),
        Some("mcp") => {
            drop(repo);
            let stdin = std::io::stdin();
            let passage_index = args.iter().any(|a| a == "--passage-index");
            mcp::run(
                store_dir,
                &refname,
                passage_index,
                stdin.lock(),
                std::io::stdout().lock(),
            )?;
        }
        Some("live") => {
            let needed_index = repo.needed_index;
            drop(repo);
            live_cmd(store_dir, &refname, &args[1..], needed_index)?;
        }
        Some(cmd @ ("write" | "rm" | "mkdir" | "mv" | "cp")) => {
            let op = match cmd {
                "write" => {
                    let mut bytes = Vec::new();
                    match args.get(2).map(String::as_str) {
                        None | Some("-") => {
                            std::io::stdin().read_to_end(&mut bytes).map_err(err)?;
                        }
                        Some(f) => bytes = std::fs::read(f).map_err(err)?,
                    }
                    live::Op::Put(arg(1)?, bytes)
                }
                "rm" => live::Op::Remove(arg(1)?),
                "mkdir" => live::Op::Mkdir(arg(1)?),
                "mv" => live::Op::Rename(arg(1)?, arg(2)?),
                _ => live::Op::Copy(arg(1)?, arg(2)?),
            };
            drop(repo);
            let mut l = live::Live::open(store_dir, &refname)?;
            if cmd == "rm" && l.stat(&live::norm(&arg(1)?))?.is_none() {
                return Err(format!("{}: no such file or folder", arg(1)?));
            }
            let t = Instant::now();
            let cost = l.apply(vec![op])?;
            let commit = l.flush(true)?;
            match commit {
                Some(c) => println!("{c}"),
                None => println!("no change"),
            }
            say!(
                "new_bytes={} store_ms={:.1} tree_ms={:.1} total_ms={:.1}",
                cost.new_bytes,
                cost.store_ms,
                cost.tree_ms,
                t.elapsed().as_secs_f64() * 1e3
            );
        }
        Some("search") => {
            let q = arg(1)?;
            drop(repo);
            let mut l = live::Live::open(store_dir, &refname)?;
            l.index_all(false)?;
            let t = Instant::now();
            let (hits, total) = text::search(&l.docs, &q, usize::MAX);
            for h in hits {
                if h.part.is_empty() {
                    println!("{}", h.path);
                } else {
                    println!("{} ({})", h.path, h.part);
                }
                for (n, line) in h.lines {
                    println!("  {n}: {line}");
                }
            }
            say!("{total} file(s), {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        }
        Some("ask") => {
            let llm = args.iter().any(|a| a == "--llm");
            let q = args[1..]
                .iter()
                .filter(|a| *a != "--llm")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            if q.is_empty() {
                return Err(USAGE.into());
            }
            drop(repo);
            let mut l = live::Live::open(store_dir, &refname)?;
            l.index_all(false)?;
            let passages = l.retrieve(&q, 8);
            if llm {
                println!("{}\n", ask::answer(&q, &passages)?);
            }
            for (i, p) in passages.iter().enumerate() {
                println!("{}", ask::source_label(i + 1, p));
                if !llm {
                    for line in p.text.lines().take(6) {
                        println!("    {line}");
                    }
                }
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

/// `ikam live`: serve the ref as a filesystem until stopped.
fn live_cmd(
    store_dir: &str,
    refname: &str,
    args: &[String],
    needed_index: bool,
) -> Result<(), String> {
    let mut args = args.to_vec();
    let mut watches = Vec::new();
    while let Some(w) = take_flag(&mut args, "--watch") {
        let (dir, prefix) = match w.rsplit_once(':') {
            Some((d, p)) if !d.is_empty() && Path::new(d).is_dir() => {
                (d.to_string(), live::norm(p))
            }
            _ => (w.clone(), String::new()),
        };
        if !Path::new(&dir).is_dir() {
            return Err(format!("--watch {dir}: not a folder"));
        }
        watches.push((PathBuf::from(dir), prefix));
    }
    let commit_after = take_flag(&mut args, "--commit-after")
        .map(|m| {
            m.parse::<u64>()
                .map_err(|_| "--commit-after MS".to_string())
        })
        .transpose()?;
    let mut allowed = Vec::new();
    while let Some(name) = take_flag(&mut args, "--allow-host") {
        if name.is_empty() || name.contains(['/', ' ']) {
            return Err(format!("--allow-host {name:?}: expected a host name"));
        }
        allowed.push(name);
    }
    let log_requests = take_switch(&mut args, "--log-requests");
    let keep_on_hangup = take_switch(&mut args, "--keep-on-hangup");
    let byte_flag = |args: &mut Vec<String>, flag: &str| {
        take_flag(args, flag)
            .map(|v| parse_bytes(&v).ok_or_else(|| format!("{flag} BYTES (such as 25M)")))
            .transpose()
    };
    let max_body = byte_flag(&mut args, "--max-body")?;
    let max_store = byte_flag(&mut args, "--max-store")?;
    let max_conns = take_flag(&mut args, "--max-conns")
        .map(|n| match n.parse::<usize>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err("--max-conns N (at least 1)".to_string()),
        })
        .transpose()?;
    let gc_every = take_flag(&mut args, "--gc-every")
        .map(|n| match n.parse::<usize>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err("--gc-every N (at least 1)".to_string()),
        })
        .transpose()?;
    let before = args.len();
    args.retain(|a| a != "--deferred-sync");
    let deferred_sync = args.len() != before;
    let before = args.len();
    args.retain(|a| a != "--no-object-templates");
    let object_templates = args.len() == before;
    let before = args.len();
    args.retain(|a| a != "--passage-index");
    let passage_index = args.len() != before;
    let before = args.len();
    args.retain(|a| a != "--public");
    let public = args.len() != before;
    let auth = auth::Auth::from_env()?;
    let llm = match args.iter().position(|a| a == "--llm") {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
    if public && auth.is_none() {
        return Err("--public lets guests read; writes need IKAM_AUTH, which is not set".into());
    }
    let limits = http::Limits {
        max_body: max_body
            .or(public.then_some(PUBLIC_MAX_BODY))
            .map(|n| n as usize),
        body_limit: None,
        max_conns: max_conns.or(public.then_some(PUBLIC_MAX_CONNS)),
        idle: if public {
            Duration::from_secs(30)
        } else {
            http::Limits::default().idle
        },
    };
    let port: u16 = args.first().map_or(Ok(8737), |p| p.parse()).map_err(err)?;
    let mut l = live::Live::open_with(store_dir, refname, deferred_sync)?;
    l.repo.needed_index = needed_index;
    l.repo.object_templates = object_templates;
    l.max_store = max_store.or(public.then_some(PUBLIC_MAX_STORE));
    l.gc_every = gc_every;
    if let Some(ms) = commit_after {
        l.commit_after = Duration::from_millis(ms);
    }
    l.llm = llm;
    if llm && !ask::key_available() {
        say!("--llm: ANTHROPIC_API_KEY is not set; questions will show sources only");
    }
    auth::catch_stop_signals(keep_on_hangup);
    l.index_all(true)?;
    if passage_index {
        l.index_passages();
    }
    let live = Arc::new(Mutex::new(l));
    {
        let live = live.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                if auth::stopping() {
                    // Holding the lock, so no edit or repack is half done;
                    // commit what is pending and leave.
                    let mut l = live.lock().unwrap();
                    let code = match l.flush(true) {
                        Ok(_) => 0,
                        Err(e) => {
                            say!("{} commit on stop failed: {e}", live::stamp());
                            1
                        }
                    };
                    say!("{} stopped ({})", live::stamp(), auth::stop_signal());
                    std::process::exit(code);
                }
                if let Err(e) = live.lock().unwrap().flush(false) {
                    say!("{} commit failed: {e}", live::stamp());
                }
            }
        });
    }
    for (dir, prefix) in watches {
        say!(
            "{} watching {} into /{prefix}",
            live::stamp(),
            dir.display()
        );
        let live = live.clone();
        std::thread::spawn(move || live::watch(live, dir, prefix, Duration::from_millis(500)));
    }
    let hosts = auth::Hosts::new(port, allowed.clone());
    let password = auth.is_some();
    let route = move |req: &mut http::Request| {
        // A page on another site can point a name it controls at
        // 127.0.0.1 (DNS rebinding); its requests carry that name.
        if !req.header("host").is_some_and(|h| hosts.ok(h)) {
            return http::Response::text(403, "unknown Host");
        }
        if req.path == "/robots.txt" && matches!(req.method.as_str(), "GET" | "HEAD") {
            return robots(public);
        }
        if let Some(refusal) = auth.as_ref().and_then(|a| a.check(req)) {
            // With --public, a guest (no password sent) may read; Claude
            // answers are dropped from what it asks.
            let guest = public && req.header("authorization").is_none() && !writes(req);
            if !guest {
                return refusal;
            }
            req.query = without_param(&req.query, "llm");
        }
        if req.path == dav::PREFIX || req.path.starts_with("/dav/") {
            dav::handle(&mut live.lock().unwrap(), req)
        } else {
            ui::handle(&live, req)
        }
    };
    let handler = Arc::new(move |req: http::Request| {
        let started = Instant::now();
        let line = log_requests.then(|| request_line(&req));
        let mut req = req;
        let resp = route(&mut req);
        if let Some(line) = line {
            say!(
                "{} {line} -> {} {:.0}ms",
                live::stamp(),
                resp.status,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        resp
    });
    let t = live::stamp();
    say!(
        "{t} live on http://127.0.0.1:{port}/ (WebDAV: http://127.0.0.1:{port}/dav/), ref {refname}"
    );
    for name in &allowed {
        say!("{t} also answering to {name}");
    }
    if password {
        say!("{t} password required (IKAM_AUTH)");
    }
    if log_requests {
        say!("{t} logging each request");
    }
    if public {
        say!("{t} public: guests may read; writes, admin and Claude answers need the password");
    }
    let show = |n: Option<u64>| n.map_or("no limit".to_string(), |n| format!("{n} bytes"));
    if limits.max_body.is_some() || limits.max_conns.is_some() || max_store.is_some() || public {
        say!(
            "{t} limits: uploads {}, store {}, connections {}",
            show(limits.max_body.map(|n| n as u64)),
            show(max_store.or(public.then_some(PUBLIC_MAX_STORE))),
            limits
                .max_conns
                .map_or("no limit".to_string(), |n| n.to_string())
        );
    }
    http::serve(port, handler, limits)
}

/// `/robots.txt`, answered before the password check: crawlers and web
/// readers that honour it may read a `--public` copy, and are asked to keep
/// out of any other (which needs the password anyway). Many treat a missing
/// or refused robots.txt as "keep out".
fn robots(public: bool) -> http::Response {
    let rules = if public { "Allow: /" } else { "Disallow: /" };
    http::Response::text(200, format!("User-agent: *\n{rules}\n"))
}

/// Whether a request can change anything: every WebDAV method but the
/// reading ones, and anything but GET or HEAD elsewhere (the API's POSTs).
fn writes(req: &http::Request) -> bool {
    !matches!(req.method.as_str(), "GET" | "HEAD" | "OPTIONS")
        && !(req.method == "PROPFIND" && (req.path == dav::PREFIX || req.path.starts_with("/dav/")))
}

/// `query` without `key` (and its value).
fn without_param(query: &str, key: &str) -> String {
    query
        .split('&')
        .filter(|kv| kv.split_once('=').map_or(*kv, |(k, _)| k) != key)
        .collect::<Vec<_>>()
        .join("&")
}

/// `--public` limits unless given: 25 MB per upload, 1 GB stored, 32
/// connections.
const PUBLIC_MAX_BODY: u64 = 25 << 20;
const PUBLIC_MAX_STORE: u64 = 1 << 30;
const PUBLIC_MAX_CONNS: usize = 32;

/// `25M`, `1G`, `512K` or plain bytes (powers of 1024).
fn parse_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, shift) = match s.char_indices().last()? {
        (i, 'K' | 'k') => (&s[..i], 10),
        (i, 'M' | 'm') => (&s[..i], 20),
        (i, 'G' | 'g') => (&s[..i], 30),
        _ => (s, 0),
    };
    let n: u64 = num.parse().ok()?;
    n.checked_mul(1u64 << shift)
}

/// What `--log-requests` shows of a request: method, path (no query, which
/// may hold a search), the Host it named, the client's first User-Agent word,
/// and whether it carried a password (never the password).
fn request_line(req: &http::Request) -> String {
    let client = req
        .header("user-agent")
        .and_then(|u| u.split_whitespace().next())
        .unwrap_or("-");
    format!(
        "{} {} host={} [{client}{}]",
        req.method,
        req.path,
        req.header("host").unwrap_or("-"),
        req.header("authorization").map_or(", no password", |_| "")
    )
}

/// Take a switch (a flag with no value) out of `args`: whether it was there.
fn take_switch(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != flag);
    args.len() != before
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    let value = args.get(i + 1).cloned();
    args.drain(i..(i + 2).min(args.len()));
    value
}

/// Commit `dir`, repack, check every object and that each file reads back
/// exactly, and print what it took.
fn ingest(
    repo: &mut Repo<FsStore>,
    store_dir: &str,
    dir: &str,
    refname: &str,
    message: &str,
) -> Result<(), String> {
    let t = Instant::now();
    let tree = snapshot::snapshot(repo, Path::new(dir), Path::new(store_dir)).map_err(err)?;
    let message = if message.is_empty() { dir } else { message };
    let commit = repo.commit(refname, tree, message).map_err(err)?;
    let ingest_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let r = repo.repack().map_err(err)?;
    let repack_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let failures = repo.fsck_deep().map_err(err)?;
    let mut mismatched = 0;
    for f in corpus::files(repo, &tree).map_err(err)? {
        let on_disk = std::fs::read(Path::new(dir).join(&f.path)).map_err(err)?;
        if repo.read_content(&f.id).map_err(err)? != on_disk {
            mismatched += 1;
        }
    }
    let verify_ms = t.elapsed().as_millis();
    let s = corpus::stats(repo, Some(&commit)).map_err(err)?;
    println!("{commit}");
    println!(
        "files={} input_bytes={} stored_bytes={} ratio={:.4} repack_applied={} ingest_ms={ingest_ms} repack_ms={repack_ms} verify_ms={verify_ms} fsck_failures={} mismatched={mismatched}",
        s.files,
        s.input,
        s.stored,
        s.stored as f64 / s.input.max(1) as f64,
        r.applied,
        failures.len()
    );
    if failures.is_empty() && mismatched == 0 {
        Ok(())
    } else {
        Err("verification failed".into())
    }
}

fn list(repo: &Repo<FsStore>, spec: &str, recursive: bool) -> Result<(), String> {
    let mut id = lookup(repo, spec)?;
    if let Ok(Object::Commit(c)) = repo.get(&id) {
        id = c.tree;
    }
    if recursive {
        for f in corpus::files(repo, &id).map_err(err)? {
            let len = repo.read_content(&f.id).map_err(err)?.len();
            println!("{len:>10} {}", f.path);
        }
        return Ok(());
    }
    let Object::Tree(entries) = repo.get(&id).map_err(err)? else {
        return Err(format!("{spec}: not a directory"));
    };
    for e in entries {
        match e.kind {
            ikam_kernel::Kind::File => {
                let len = repo.read_content(&e.id).map_err(err)?.len();
                println!("{len:>10} {}", e.name);
            }
            _ => println!("{:>10} {}/", "-", e.name),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_switch_is_taken_out_of_the_arguments() {
        let mut a: Vec<String> = ["--keep-on-hangup", "--live", "8740"]
            .map(String::from)
            .to_vec();
        assert!(take_switch(&mut a, "--keep-on-hangup"));
        assert!(!take_switch(&mut a, "--keep-on-hangup"));
        assert_eq!(a, ["--live", "8740"]);
    }

    #[test]
    fn db_live_turns_on_the_live_options() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let all = DbServeOpts {
            concurrent_reads: true,
            edit_index: true,
            plain_writes: false,
            plain_events: true,
            object_templates: false,
            keep_counts: 0,
            defer_kv: LIVE_DEFER_KV,
            hnsw: false,
            keep_search: true,
            shared_store: false,
            skip_unchanged: false,
        };
        let mut a = v(&["--live", "8752"]);
        assert_eq!(db_flags(&mut a, 0, None), (true, false, all));
        assert_eq!(a, v(&["8752"]));
        let mut a = v(&[
            "--packs",
            "--plain-events",
            "--no-object-templates",
            "--concurrent-reads",
            "--edit-index",
            "--keep-search",
        ]);
        assert_eq!(db_flags(&mut a, 0, Some(LIVE_DEFER_KV)), (true, false, all));
        let mut a = v(&["--live", "--edit-index", "--log-requests"]);
        let counted = DbServeOpts {
            keep_counts: 3,
            ..all
        };
        assert_eq!(db_flags(&mut a, 3, None), (true, true, counted));
        assert!(a.is_empty());
        let mut a = v(&["8740"]);
        let off = DbServeOpts {
            concurrent_reads: false,
            edit_index: false,
            plain_writes: false,
            plain_events: false,
            object_templates: true,
            keep_counts: 0,
            defer_kv: 0,
            hnsw: false,
            keep_search: false,
            shared_store: false,
            skip_unchanged: false,
        };
        assert_eq!(db_flags(&mut a, 0, None), (false, false, off));
        // E357: `--plain-writes` is its own option, not part of `--live`.
        let mut a = v(&["--live", "--plain-writes"]);
        let plain = DbServeOpts {
            plain_writes: true,
            ..all
        };
        assert_eq!(db_flags(&mut a, 0, None), (true, false, plain));
        assert!(a.is_empty());
        // E358: `--hnsw` is its own option too.
        let mut a = v(&["--live", "--hnsw"]);
        let graph = DbServeOpts { hnsw: true, ..all };
        assert_eq!(db_flags(&mut a, 0, None), (true, false, graph));
        assert!(a.is_empty());
        // E372: `--live` keeps search views and defers kv folds; each can
        // be turned off or set on its own.
        let mut a = v(&["--live", "--keep-search"]);
        assert_eq!(db_flags(&mut a, 0, None), (true, false, all));
        assert!(a.is_empty());
        let mut a = v(&["--live", "--no-keep-search"]);
        let unkept = DbServeOpts {
            keep_search: false,
            ..all
        };
        assert_eq!(db_flags(&mut a, 0, None), (true, false, unkept));
        assert!(a.is_empty());
        let mut a = v(&["--live"]);
        let folded = DbServeOpts { defer_kv: 0, ..all };
        assert_eq!(db_flags(&mut a, 0, Some(0)), (true, false, folded));
        let mut a = v(&["--live"]);
        let more = DbServeOpts {
            defer_kv: 100_000,
            ..all
        };
        assert_eq!(db_flags(&mut a, 0, Some(100_000)), (true, false, more));
        // Without `--live` neither is on.
        let mut a = v(&["--keep-search"]);
        let keep = DbServeOpts {
            keep_search: true,
            ..off
        };
        assert_eq!(db_flags(&mut a, 0, None), (false, false, keep));
        let mut a = v(&[]);
        let deferred = DbServeOpts {
            defer_kv: 500,
            ..off
        };
        assert_eq!(db_flags(&mut a, 0, Some(500)), (false, false, deferred));
        // E370: `--shared-store` too.
        let mut a = v(&["--shared-store"]);
        let shared = DbServeOpts {
            shared_store: true,
            skip_unchanged: false,
            ..off
        };
        assert_eq!(db_flags(&mut a, 0, None), (false, false, shared));
    }
    use super::*;

    #[test]
    fn robots_rules() {
        assert_eq!(robots(true).body, b"User-agent: *\nAllow: /\n");
        assert_eq!(robots(false).body, b"User-agent: *\nDisallow: /\n");
        assert_eq!(robots(true).status, 200);
    }

    #[test]
    fn guest_requests() {
        let req = |method: &str, path: &str| http::Request {
            method: method.into(),
            path: path.into(),
            query: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
        };
        for (m, p) in [
            ("GET", "/dav/a.txt"),
            ("HEAD", "/dav/a.txt"),
            ("PROPFIND", "/dav/"),
            ("OPTIONS", "/dav/"),
            ("GET", "/api/search"),
            ("GET", "/ask"),
        ] {
            assert!(!writes(&req(m, p)), "{m} {p}");
        }
        for (m, p) in [
            ("PUT", "/dav/a.txt"),
            ("DELETE", "/dav/a.txt"),
            ("MKCOL", "/dav/d"),
            ("MOVE", "/dav/a.txt"),
            ("COPY", "/dav/a.txt"),
            ("LOCK", "/dav/a.txt"),
            ("UNLOCK", "/dav/a.txt"),
            ("PROPPATCH", "/dav/a.txt"),
            ("POST", "/api/repack"),
            ("POST", "/api/commit"),
            ("POST", "/api/fsck"),
            ("PROPFIND", "/api/status"),
        ] {
            assert!(writes(&req(m, p)), "{m} {p}");
        }
        assert_eq!(without_param("q=x&llm=1", "llm"), "q=x");
        assert_eq!(without_param("llm&q=x", "llm"), "q=x");
        assert_eq!(without_param("q=llm", "llm"), "q=llm");
    }

    #[test]
    fn byte_sizes() {
        assert_eq!(parse_bytes("25M"), Some(25 << 20));
        assert_eq!(parse_bytes("1G"), Some(1 << 30));
        assert_eq!(parse_bytes("512k"), Some(512 << 10));
        assert_eq!(parse_bytes("1000"), Some(1000));
        assert_eq!(parse_bytes("M"), None);
        assert_eq!(parse_bytes("-1M"), None);
        assert_eq!(parse_bytes("99999999999G"), None);
    }

    #[test]
    fn request_lines_never_show_the_password() {
        let req = |auth: Option<&str>| http::Request {
            method: "PROPFIND".into(),
            path: "/dav/a%20b".into(),
            query: "q=secret+search".into(),
            headers: [
                Some(("Host".to_string(), "mac.ts.net".to_string())),
                Some((
                    "User-Agent".to_string(),
                    "WebDAVFS/3.0.0 (03008000) Darwin/25.0.0".to_string(),
                )),
                auth.map(|a| ("Authorization".to_string(), a.to_string())),
            ]
            .into_iter()
            .flatten()
            .collect(),
            body: Vec::new(),
        };
        assert_eq!(
            request_line(&req(Some("Basic ZXJpYzpodW50ZXIy"))),
            "PROPFIND /dav/a%20b host=mac.ts.net [WebDAVFS/3.0.0]"
        );
        assert_eq!(
            request_line(&req(None)),
            "PROPFIND /dav/a%20b host=mac.ts.net [WebDAVFS/3.0.0, no password]"
        );
    }
}
