//! E302: removing the temporary files dead writer processes leave.

use ikam_kernel::{FsStore, Repo};

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[test]
fn only_dead_processes_files_are_removed() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open(dir.path()).unwrap());
    let id = repo.put_content(b"kept content").unwrap().id;
    let mut dead = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = dead.id();
    dead.wait().unwrap();
    let mut alive = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let sub = dir.path().join("objects").join("ab");
    std::fs::create_dir_all(&sub).unwrap();
    let names = [
        format!(".tmp-{dead_pid}-0"),
        format!(".pending-{dead_pid}-3-cdef"),
        format!(".tmp-{dead_pid}"),
        format!(".tmp-{}-1", alive.id()),
        format!(".pending-{}-2-cdef", std::process::id()),
        ".tmp-notapid".to_string(),
    ];
    for n in &names {
        std::fs::write(sub.join(n), b"partial").unwrap();
    }
    let removed = repo.store().clean_temporaries().unwrap();
    let left: Vec<bool> = names.iter().map(|n| sub.join(n).exists()).collect();
    alive.kill().unwrap();
    alive.wait().unwrap();
    assert_eq!(removed, 3);
    assert_eq!(left, [false, false, false, true, true, true]);
    assert_eq!(repo.read_content(&id).unwrap(), b"kept content");
    assert!(repo.fsck().unwrap().is_empty());
}
