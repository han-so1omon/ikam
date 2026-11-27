//! `--disk-guard` (E404, off): before `ingest`, `commit DIR` and `repack`,
//! compare the store disk's free space with an estimate of what the command
//! writes, and refuse with a plain message when it is short. When a guarded
//! command still runs out of space, `gc` the objects it left (content no ref
//! reaches) so the disk is not left full, and say what happened.
//!
//! Nothing half-built is ever read: objects are named by their content and a
//! ref moves only after everything it reaches is written (commits, and kept
//! views and search views in `db serve`), so a command that stops part way
//! leaves only unreachable objects. That is why no completion marker is
//! needed; the leftovers cost disk until `gc`, which this frees at once.
//! That `gc` is the store's own, so like `ikam gc` it must not run while
//! another process writes the same store (an FsStore has no store-wide
//! lock): use the guard where the command is the store's only writer.

use std::path::Path;

/// Room kept free beyond the estimate: a tenth of it, at least 1 MiB and at
/// most 64 MiB.
pub fn slack(need: u64) -> u64 {
    (need / 10).clamp(1 << 20, 64 << 20)
}

/// Bytes free for this user on the filesystem holding `path` (its nearest
/// existing ancestor).
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `s` is a valid buffer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some(s.f_bavail as u64 * s.f_frsize as u64)
}

#[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
pub fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// Bytes and number of the files under `path`, leaving out `skip` (the
/// store, when it is inside the folder being ingested). Links are counted
/// as themselves, not followed.
pub fn tree_bytes(path: &Path, skip: &Path) -> (u64, u64) {
    let skip = skip.canonicalize().ok();
    let (mut total, mut files) = (0, 0);
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        if skip
            .as_deref()
            .is_some_and(|s| p.canonicalize().ok().as_deref() == Some(s))
        {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
        } else {
            total += meta.len();
            files += 1;
        }
    }
    (total, files)
}

/// The filesystem's block size at `path` (4096 if unknown): each stored
/// object takes at least one.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
fn block_size(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let mut p = path.to_path_buf();
    while !p.exists() {
        match p.parent() {
            Some(q) => p = q.to_path_buf(),
            None => return 4096,
        }
    }
    std::fs::metadata(&p).map_or(4096, |m| m.blksize().max(512))
}

#[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
fn block_size(_path: &Path) -> u64 {
    4096
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1e6)
}

/// What `cmd` (with its folder argument, if any) is estimated to write to
/// the store: an ingest about twice its input (the objects, then repack's
/// rewrite of the store, E404's runs), a commit about its input, a repack
/// about the store's size. `None` for commands not guarded.
pub fn estimate(cmd: &str, dir: Option<&str>, store: &Path) -> Option<(u64, String)> {
    let (store_bytes, _) = tree_bytes(store, Path::new("/nonexistent"));
    // Input as stored: at least a block for each file (E404: 3,000 files of
    // 100 bytes filled 12 MB of a small disk).
    let input = |d: &str| {
        let (bytes, files) = tree_bytes(Path::new(d), store);
        (bytes.max(files * block_size(store)), bytes)
    };
    match (cmd, dir) {
        ("ingest", Some(d)) => {
            let (stored, bytes) = input(d);
            Some((
                2 * stored + store_bytes,
                format!(
                    "ingest of {} with a store of {}",
                    mb(bytes),
                    mb(store_bytes)
                ),
            ))
        }
        ("commit", Some(d)) => {
            let (stored, bytes) = input(d);
            Some((stored, format!("commit of {}", mb(bytes))))
        }
        ("repack", _) => Some((
            store_bytes,
            format!("repack of a {} store", mb(store_bytes)),
        )),
        _ => None,
    }
}

/// The commands the guard checks before they run, and cleans up after.
pub fn guarded(cmd: &str) -> bool {
    matches!(cmd, "ingest" | "commit" | "repack")
}

/// Refuse when the store's disk has less free than `need` + `SLACK`.
pub fn check(store: &Path, need: u64, what: &str) -> Result<(), String> {
    let Some(free) = free_bytes(store) else {
        return Ok(()); // unknown: go ahead, as without the guard
    };
    if free < need + slack(need) {
        return Err(format!(
            "not enough free space on the store's disk: {what} needs about {} and {} more kept free, and {} is free; nothing was stored",
            mb(need),
            mb(slack(need)),
            mb(free)
        ));
    }
    Ok(())
}

/// True if `msg` reports a full disk (ENOSPC, or a quota).
pub fn out_of_space(msg: &str) -> bool {
    msg.contains("os error 28") || msg.contains("No space left") || msg.contains("os error 122")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guard_refuses_what_does_not_fit_and_passes_what_does() {
        let dir = tempfile::tempdir().unwrap();
        let free = free_bytes(dir.path()).unwrap();
        assert!(free > 0);
        assert!(check(dir.path(), 1, "a small write").is_ok());
        let e = check(dir.path(), free, "a write the size of the disk").unwrap_err();
        assert!(
            e.contains("not enough free space") && e.contains("nothing was stored"),
            "{e}"
        );
        // An estimate counts files once, leaves the store out, and follows
        // no links.
        std::fs::write(dir.path().join("a"), vec![0; 1000]).unwrap();
        std::fs::create_dir(dir.path().join("store")).unwrap();
        std::fs::write(dir.path().join("store/x"), vec![0; 500]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("a"), dir.path().join("link")).unwrap();
        let (need, _) = estimate("commit", dir.path().to_str(), &dir.path().join("store")).unwrap();
        let block = block_size(dir.path());
        let files = if cfg!(unix) { 2 } else { 1 };
        assert_eq!(need, files * block.max(1000)); // two files, each a block
        let (need, _) = estimate("ingest", dir.path().to_str(), &dir.path().join("store")).unwrap();
        assert!(need >= 2 * files * block + 500);
        assert!(estimate("log", None, dir.path()).is_none());
        assert!(guarded("ingest") && guarded("repack") && !guarded("log"));
        assert!(out_of_space("No space left on device (os error 28)"));
        assert!(!out_of_space("not found"));
    }
}
