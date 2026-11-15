//! E263: temperature by age. `cargo run --release --example temperature -- [FILES] [EDITS]`
//!
//! A file history on an FsStore: FILES generated files (default 200, half
//! markdown and half CSV), then EDITS one-line edits (default 2,000), a
//! commit after every 10, so 200 commits deep. Writes are planned at the
//! heavy read weight (about 48.8, `READ_WEIGHT` in the bench, E069) and a
//! cold pass (`repack_incremental`, E268) runs after the session at that
//! weight. Then, on copies of that store:
//!
//! - `all heavy`: nothing more.
//! - `today` (its own store): the same history written and passed at the
//!   kernel's default weight (0.001, about the archive weight).
//! - `all archive`: every live content re-encoded at the archive weight
//!   (0.00097, the north score's `ARCHIVE_READ_WEIGHT`, E099): a
//!   temperature pass that counts nothing as hot.
//! - `temperature D`: `temperature_pass` with the newest D commits hot
//!   (D = 1, 10, 50), the rest at the archive weight.
//!
//! For each: stored bytes, and the time to read, from a freshly opened
//! store, the newest 10% of versions written and the oldest 50%. Then
//! promotion, on the `temperature 10` copy: 20 files reverted to versions
//! from the first half of the history in one commit, the reverted files
//! read before and after the next pass, and that pass's time and bytes.
//! Every version is read back and checked, and fsck runs, at the end of
//! each case. Prints one JSON line per step. Fixtures are synthetic.

mod db_common;

use std::path::Path;
use std::time::Instant;

use db_common::Lcg;
use ikam_kernel::temperature::TemperatureOpts;
use ikam_kernel::{Edit, FsStore, Id, Object, Repo, Store};

type R = Repo<FsStore>;

const HEAVY: f64 = 48.8;
const ARCHIVE: f64 = 0.00097;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

const WORDS: &[&str] = &[
    "account",
    "batch",
    "client",
    "delivery",
    "estimate",
    "freight",
    "invoice",
    "ledger",
    "margin",
    "notice",
    "order",
    "payment",
    "quote",
    "receipt",
    "schedule",
    "total",
    "update",
    "vendor",
    "warehouse",
    "yield",
];

fn line(rng: &mut Lcg, file: usize, csv: bool) -> String {
    if csv {
        format!(
            "{file},{},{},{}.{:02},{}",
            rng.next() % 10_000,
            WORDS[(rng.next() % 20) as usize],
            rng.next() % 1000,
            rng.next() % 100,
            WORDS[(rng.next() % 20) as usize]
        )
    } else {
        let n = 6 + rng.next() % 8;
        (0..n)
            .map(|_| WORDS[(rng.next() % 20) as usize])
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn render(lines: &[String]) -> Vec<u8> {
    let mut v = lines.join("\n").into_bytes();
    v.push(b'\n');
    v
}

struct History {
    /// Every version written, in order, with its file and bytes.
    versions: Vec<(usize, Id, Vec<u8>)>,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.metadata().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn open(dir: &Path) -> R {
    open_at(dir, HEAVY)
}

fn open_at(dir: &Path, weight: f64) -> R {
    let mut repo = Repo::new(FsStore::open(dir).unwrap());
    repo.read_weight = weight;
    repo
}

fn stored(repo: &R) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn build(dir: &Path, files: usize, edits: usize, weight: f64, label: &str) -> History {
    let mut repo = open_at(dir, weight);
    let mut rng = Lcg(11);
    let mut docs: Vec<Vec<String>> = (0..files)
        .map(|f| (0..40).map(|_| line(&mut rng, f, f % 2 == 1)).collect())
        .collect();
    let mut versions = Vec::new();
    let mut puts = Vec::new();
    for (f, d) in docs.iter().enumerate() {
        let bytes = render(d);
        let id = repo.put_content(&bytes).unwrap().id;
        versions.push((f, id, bytes));
        puts.push(Edit::Put {
            path: format!(
                "d{:02}/f{f:04}.{}",
                f % 10,
                if f % 2 == 1 { "csv" } else { "md" }
            ),
            id,
        });
    }
    let mut tree = repo.edit_tree(None, &puts).unwrap();
    repo.commit("main", tree, "files").unwrap();
    let t = Instant::now();
    for e in 0..edits {
        let f = (rng.next() as usize) % files;
        let at = (rng.next() as usize) % docs[f].len();
        docs[f][at] = line(&mut rng, f, f % 2 == 1);
        let bytes = render(&docs[f]);
        let id = repo.put_content(&bytes).unwrap().id;
        versions.push((f, id, bytes));
        let path = format!(
            "d{:02}/f{f:04}.{}",
            f % 10,
            if f % 2 == 1 { "csv" } else { "md" }
        );
        tree = repo
            .edit_tree(Some(tree), &[Edit::Put { path, id }])
            .unwrap();
        if e % 10 == 9 {
            repo.commit("main", tree, "edits").unwrap();
        }
    }
    let session_ms = ms(t);
    let before = stored(&repo);
    let t = Instant::now();
    repo.repack_incremental().unwrap();
    println!(
        "{{\"step\":\"{label}\",\"files\":{files},\"edits\":{edits},\"session_ms\":{session_ms:.0},\"stored\":{before},\"pass_ms\":{:.0},\"stored_after_pass\":{}}}",
        ms(t),
        stored(&repo)
    );
    History { versions }
}

/// Read a set of versions from a freshly opened store; checked.
fn read_ms(dir: &Path, h: &History, which: &[usize]) -> f64 {
    let repo = open(dir);
    let t = Instant::now();
    for &i in which {
        let (_, id, bytes) = &h.versions[i];
        assert_eq!(&repo.read_content(id).unwrap(), bytes);
    }
    ms(t)
}

fn check_all(repo: &R, h: &History) {
    for (_, id, bytes) in &h.versions {
        assert_eq!(&repo.read_content(id).unwrap(), bytes);
    }
    assert!(repo.fsck().unwrap().is_empty(), "fsck");
}

fn sets(h: &History) -> (Vec<usize>, Vec<usize>) {
    let n = h.versions.len();
    // Distinct ids only: a version may repeat an earlier one.
    let newest = (n - n / 10..n).collect();
    let oldest = (0..n / 2).collect();
    (newest, oldest)
}

fn case(src: &Path, h: &History, label: &str, hot: Option<usize>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(src, dir.path());
    let mut repo = open(dir.path());
    let t = Instant::now();
    let pass = hot.map(|d| {
        repo.temperature_pass(TemperatureOpts {
            hot_commits: d,
            cold_read_weight: ARCHIVE,
        })
        .unwrap()
    });
    let pass_ms = ms(t);
    check_all(&repo, h);
    let bytes = stored(&repo);
    let (newest, oldest) = sets(h);
    let new_ms = read_ms(dir.path(), h, &newest);
    let old_ms = read_ms(dir.path(), h, &oldest);
    let extra = match pass {
        Some(p) => format!(
            ",\"pass_ms\":{pass_ms:.0},\"hot\":{},\"cold\":{},\"cooled\":{},\"cooled_before\":{},\"cooled_after\":{}",
            p.hot, p.cold, p.cooled, p.before, p.after
        ),
        None => String::new(),
    };
    println!(
        "{{\"step\":\"{label}\",\"stored\":{bytes},\"read_newest_10pct_ms\":{new_ms:.1},\"read_oldest_50pct_ms\":{old_ms:.1},\"newest\":{},\"oldest\":{}{extra}}}",
        newest.len(),
        oldest.len()
    );
    dir
}

fn promotion(dir: &Path, h: &History) {
    let mut repo = open(dir);
    // 20 files back to a version from the first half of the history.
    let early = h.versions.len() / 2;
    let mut picked: Vec<usize> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for i in (0..early).rev() {
        if seen.insert(h.versions[i].0) {
            picked.push(i);
        }
        if picked.len() == 20 {
            break;
        }
    }
    let head = repo.resolve("main").unwrap();
    let tree = match repo.get(&head).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    };
    let edits: Vec<Edit> = picked
        .iter()
        .map(|&i| {
            let f = h.versions[i].0;
            Edit::Put {
                path: format!(
                    "d{:02}/f{f:04}.{}",
                    f % 10,
                    if f % 2 == 1 { "csv" } else { "md" }
                ),
                id: h.versions[i].1,
            }
        })
        .collect();
    let tree = repo.edit_tree(Some(tree), &edits).unwrap();
    repo.commit("main", tree, "revert 20 files").unwrap();
    let before_ms = read_ms(dir, h, &picked);
    let bytes_before = stored(&repo);
    let t = Instant::now();
    let p = repo
        .temperature_pass(TemperatureOpts {
            hot_commits: 10,
            cold_read_weight: ARCHIVE,
        })
        .unwrap();
    let pass_ms = ms(t);
    check_all(&repo, h);
    let after_ms = read_ms(dir, h, &picked);
    println!(
        "{{\"step\":\"promotion (temperature 10)\",\"reverted\":{},\"promoted\":{},\"cooled\":{},\"pass_ms\":{pass_ms:.0},\"read_reverted_before_ms\":{before_ms:.2},\"read_reverted_after_ms\":{after_ms:.2},\"stored_before\":{bytes_before},\"stored_after\":{}}}",
        picked.len(),
        p.promoted,
        p.cooled,
        stored(&repo)
    );
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let files = args.first().copied().unwrap_or(200);
    let edits = args.get(1).copied().unwrap_or(2_000);
    let src = tempfile::tempdir().unwrap();
    // Today: the kernel's default weight (about the archive weight) at
    // write and in the pass.
    let today = tempfile::tempdir().unwrap();
    let h0 = build(
        today.path(),
        files,
        edits,
        ikam_kernel::DEFAULT_READ_WEIGHT,
        "today: default weight, session and pass",
    );
    let (newest, oldest) = sets(&h0);
    println!(
        "{{\"step\":\"today\",\"read_newest_10pct_ms\":{:.1},\"read_oldest_50pct_ms\":{:.1}}}",
        read_ms(today.path(), &h0, &newest),
        read_ms(today.path(), &h0, &oldest)
    );
    let h = build(
        src.path(),
        files,
        edits,
        HEAVY,
        "heavy weight: session and pass",
    );
    case(src.path(), &h, "all heavy", None);
    case(src.path(), &h, "all archive", Some(0));
    for d in [1, 50] {
        case(src.path(), &h, &format!("temperature {d}"), Some(d));
    }
    let dir = case(src.path(), &h, "temperature 10", Some(10));
    promotion(dir.path(), &h);
}
