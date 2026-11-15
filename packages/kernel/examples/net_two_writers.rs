//! E292: two writers firing one net on one database.
//!
//! Every net firing so far had one writer. A night with two workers has
//! two: each reads the head, fires on what it read, and commits. This
//! measures what each way of committing does with that.
//!
//! Part 1, two writers from one head, each starting a different candidate
//! of the loop net (`cpu` holds 1 or 2):
//! - `db_commit` both: the second builds on the first's commit but writes
//!   its own database, read before the first committed.
//! - a branch each, then `merge`: plain markings, and with the sum hash
//!   (E279) on.
//! - `db_commit_at` (E292, new): the second is refused, reads the new head
//!   and tries again.
//!
//! Part 2, W threads (siblings on one SharedStore) take N candidates through
//! start, WORK_MS (env, default 2) of work and finish, committing after every firing: with
//! `db_commit`, with `db_commit_at` and a retry, and with one lock held
//! from reading the head to the commit (the work outside it). Reported:
//! candidates measured in the head, work runs, runs whose start was lost
//! from the head, the most runs at once (against the cpu count), refused
//! commits, time (each run stops after 20 s), and replay.
//!
//!   cargo run --release --example net_two_writers [-- N]
//!   (PART1=0 skips part 1; WS=1,8 picks the thread counts; TUNED=1 runs
//!   part 2 with E293 and E294's options, as E295 does)

use std::sync::Arc as Shared;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

use ikam_kernel::merge::{Conflict, Merged};
use ikam_kernel::net::Net;
use ikam_kernel::{Db, Error, Id, MemStore, Repo, SharedStore};

const REF: &str = "loop";
const CPUS: u32 = 2;

/// TUNED=1 (E295): `read_weight` 1, a 64 MB `read_cache` (E294) and the
/// sum hash (E279), as E293 and E294 recommend for a loop runner.
fn tuned() -> bool {
    std::env::var("TUNED").as_deref() == Ok("1")
}

/// Milliseconds of work per candidate (WORK_MS, default 2).
fn work_ms() -> u64 {
    std::env::var("WORK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
}

fn loop_net(cpus: u32) -> Net {
    Net::parse(&format!(
        "\
net loop 1
place cpu {cpus}
place queued colored
place running colored
place measured colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
"
    ))
    .unwrap()
}

fn tokens(repo: &Repo<impl ikam_kernel::Store>, db: &Db, place: &str) -> Vec<String> {
    repo.net_marking(db, "loop")
        .unwrap()
        .get(place)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

fn count(repo: &Repo<impl ikam_kernel::Store>, db: &Db, place: &str) -> u64 {
    repo.net_marking(db, "loop")
        .unwrap()
        .get(place)
        .map(|m| m.values().sum())
        .unwrap_or(0)
}

fn replay_ok(repo: &Repo<impl ikam_kernel::Store>, db: &Db, net: &Net) -> String {
    match repo.net_replay(db, net) {
        Ok(m) if m == repo.net_marking(db, "loop").unwrap() => "equal".into(),
        Ok(_) => "differs".into(),
        Err(e) => format!("fails ({e})"),
    }
}

/// A head with candidates a and b proposed.
fn setup(net: &Net, sum: bool) -> (Repo<MemStore>, Id) {
    let mut repo = Repo::new(MemStore::default());
    let db = repo.db_empty().unwrap();
    let mut db = if sum {
        repo.net_init_sum(&db, net).unwrap()
    } else {
        repo.net_init(&db, net).unwrap()
    };
    for t in ["a", "b"] {
        db = repo
            .net_fire(&db, net, "propose", Some(t), &format!("p-{t}"))
            .unwrap()
            .0;
    }
    let head = repo.db_commit(REF, &db, "proposed").unwrap();
    (repo, head)
}

fn describe(repo: &Repo<MemStore>, db: &Db, net: &Net) -> String {
    format!(
        "running {:?}, cpu {}, firings {}, replay {}",
        tokens(repo, db, "running"),
        count(repo, db, "cpu"),
        repo.net_firings(db, "loop").unwrap().len(),
        replay_ok(repo, db, net)
    )
}

fn part1() {
    println!("Part 1: two writers read one head; A starts a, B starts b\n");
    for cpus in [1, 2] {
        let net = loop_net(cpus);
        println!("cpu {cpus} (correct: {} running)", cpus.min(2));

        // db_commit both.
        let (mut repo, head) = setup(&net, false);
        let base = repo.db_at(&head).unwrap();
        let a = repo
            .net_fire(&base, &net, "start", Some("a"), "s-a")
            .unwrap()
            .0;
        let b = repo
            .net_fire(&base, &net, "start", Some("b"), "s-b")
            .unwrap()
            .0;
        repo.db_commit(REF, &a, "A").unwrap();
        repo.db_commit(REF, &b, "B").unwrap();
        let db = repo.db_at(&repo.resolve(REF).unwrap()).unwrap();
        println!("  db_commit both:        {}", describe(&repo, &db, &net));

        // A branch each, then merge.
        for sum in [false, true] {
            let (mut repo, head) = setup(&net, sum);
            let base = repo.db_at(&head).unwrap();
            repo.branch("w2", head).unwrap();
            let a = repo
                .net_fire(&base, &net, "start", Some("a"), "s-a")
                .unwrap()
                .0;
            let b = repo
                .net_fire(&base, &net, "start", Some("b"), "s-b")
                .unwrap()
                .0;
            repo.db_commit(REF, &a, "A").unwrap();
            repo.db_commit("w2", &b, "B").unwrap();
            let label = if sum {
                "merge, sum hash:"
            } else {
                "merge, plain:   "
            };
            match repo.merge(REF, "w2", "merge B").unwrap() {
                Merged::Conflicts(cs) => {
                    let keys: Vec<String> = cs
                        .iter()
                        .map(|c| match c {
                            Conflict::Key(k) | Conflict::Idempotency(k) | Conflict::Path(k) => {
                                k.clone()
                            }
                        })
                        .collect();
                    println!("  {label}       refused, conflicts on {keys:?}");
                }
                other => {
                    let db = repo.db_at(&repo.resolve(REF).unwrap()).unwrap();
                    let kind = match other {
                        Merged::Commit(_) => "merged",
                        Merged::FastForward(_) => "fast-forward",
                        _ => "up to date",
                    };
                    println!("  {label}       {kind}: {}", describe(&repo, &db, &net));
                }
            }
        }

        // db_commit_at, with a retry.
        let (mut repo, head) = setup(&net, false);
        let base = repo.db_at(&head).unwrap();
        let a = repo
            .net_fire(&base, &net, "start", Some("a"), "s-a")
            .unwrap()
            .0;
        let b = repo
            .net_fire(&base, &net, "start", Some("b"), "s-b")
            .unwrap()
            .0;
        repo.db_commit_at(REF, head, &a, "A").unwrap();
        let refused = matches!(
            repo.db_commit_at(REF, head, &b, "B"),
            Err(Error::RefConflict(_))
        );
        let head = repo.resolve(REF).unwrap();
        let db = repo.db_at(&head).unwrap();
        let retry = match repo.net_fire(&db, &net, "start", Some("b"), "s-b2") {
            Ok((db2, _)) => {
                repo.db_commit_at(REF, head, &db2, "B again").unwrap();
                "fired"
            }
            Err(_) => "not enabled",
        };
        let db = repo.db_at(&repo.resolve(REF).unwrap()).unwrap();
        println!(
            "  db_commit_at:          B refused {refused}, B's retry {retry}; {}\n",
            describe(&repo, &db, &net)
        );
    }
}

#[derive(Default)]
struct Tally {
    runs: AtomicUsize,
    lost: AtomicUsize,
    retries: AtomicUsize,
    now: AtomicUsize,
    most: AtomicUsize,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// `db_commit`, as every runner so far.
    Commit,
    /// `db_commit_at`, rereading and retrying when refused.
    CommitAt,
    /// `db_commit` with one lock held from reading the head to the commit:
    /// one writer at a time, the work itself in parallel.
    Locked,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Commit => "db_commit",
            Mode::CommitAt => "db_commit_at",
            Mode::Locked => "one lock",
        }
    }
}

/// Commit as `mode` says; false when refused and the caller must reread.
fn commit(
    repo: &mut Repo<SharedStore<MemStore>>,
    mode: Mode,
    base: Id,
    db: &Db,
    t: &Tally,
) -> bool {
    let r = if mode == Mode::CommitAt {
        repo.db_commit_at(REF, base, db, "step")
    } else {
        repo.db_commit(REF, db, "step")
    };
    match r {
        Ok(_) => true,
        Err(Error::RefConflict(_)) => {
            t.retries.fetch_add(1, SeqCst);
            false
        }
        Err(e) => panic!("{e}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn worker(
    mut repo: Repo<SharedStore<MemStore>>,
    net: Shared<Net>,
    n: usize,
    w: usize,
    mode: Mode,
    t: Shared<Tally>,
    lock: Shared<std::sync::Mutex<()>>,
    stop: Instant,
) {
    let mut rng = Rng(0x9E37_79B9 ^ ((w as u64 + 1) * 0x1000_0001));
    let mut k = 0usize;
    let mut key = || {
        k += 1;
        format!("w{w}-{k}")
    };
    while Instant::now() < stop {
        let guard = (mode == Mode::Locked).then(|| lock.lock().unwrap());
        let head = repo.resolve(REF).unwrap();
        let db = repo.db_at(&head).unwrap();
        if count(&repo, &db, "measured") as usize >= n {
            return;
        }
        let starts: Vec<String> = repo
            .net_enabled_limit(&db, &net, true, 64)
            .unwrap()
            .into_iter()
            .filter(|(tr, _)| tr == "start")
            .filter_map(|(_, tok)| tok)
            .collect();
        if starts.is_empty() {
            drop(guard);
            std::thread::sleep(Duration::from_micros(200));
            continue;
        }
        let tok = &starts[(rng.next() % starts.len() as u64) as usize];
        let db = repo
            .net_fire(&db, &net, "start", Some(tok), &key())
            .unwrap()
            .0;
        let ok = commit(&mut repo, mode, head, &db, &t);
        drop(guard);
        if !ok {
            continue;
        }
        let now = t.now.fetch_add(1, SeqCst) + 1;
        t.most.fetch_max(now, SeqCst);
        std::thread::sleep(Duration::from_millis(work_ms()));
        t.now.fetch_sub(1, SeqCst);
        t.runs.fetch_add(1, SeqCst);
        loop {
            let _guard = (mode == Mode::Locked).then(|| lock.lock().unwrap());
            let head = repo.resolve(REF).unwrap();
            let db = repo.db_at(&head).unwrap();
            match repo.net_fire(&db, &net, "finish", Some(tok), &key()) {
                Ok((db, _)) => {
                    if commit(&mut repo, mode, head, &db, &t) {
                        break;
                    }
                }
                Err(_) => {
                    // The start this run followed is not in the head.
                    t.lost.fetch_add(1, SeqCst);
                    break;
                }
            }
        }
    }
}

fn part2(n: usize) {
    println!(
        "Part 2: W threads take {n} candidates through start, {} ms of work and finish; cpu {CPUS}{}\n",
        work_ms(),
        if tuned() {
            "; read_weight 1, read cache, sum hash"
        } else {
            ""
        }
    );
    println!(
        "{:<13} {:>2} {:>9} {:>5} {:>5} {:>8} {:>8} {:>7} {:>9}  replay",
        "commit", "W", "measured", "runs", "lost", "at once", "retries", "ms", "firings"
    );
    let net = Shared::new(loop_net(CPUS));
    for mode in [Mode::Commit, Mode::CommitAt, Mode::Locked] {
        let ws: Vec<usize> = std::env::var("WS")
            .ok()
            .map(|v| v.split(',').map(|x| x.parse().unwrap()).collect())
            .unwrap_or(vec![1, 2, 4, 8]);
        for &w in &ws {
            let mut repo = Repo::new(SharedStore::new(MemStore::default()));
            if tuned() {
                repo.read_weight = 1.0;
                repo.read_cache = 64 << 20;
            }
            {
                let db = repo.db_empty().unwrap();
                let mut db = if tuned() {
                    repo.net_init_sum(&db, &net).unwrap()
                } else {
                    repo.net_init(&db, &net).unwrap()
                };
                for i in 0..n {
                    db = repo
                        .net_fire(
                            &db,
                            &net,
                            "propose",
                            Some(&format!("c{i}")),
                            &format!("p{i}"),
                        )
                        .unwrap()
                        .0;
                }
                repo.db_commit(REF, &db, "proposed").unwrap();
            }
            let tally = Shared::new(Tally::default());
            let lock = Shared::new(std::sync::Mutex::new(()));
            let t0 = Instant::now();
            let stop = t0 + Duration::from_secs(20);
            let hs: Vec<_> = (0..w)
                .map(|i| {
                    let (r, net, t, l) = (repo.sibling(), net.clone(), tally.clone(), lock.clone());
                    std::thread::spawn(move || worker(r, net, n, i, mode, t, l, stop))
                })
                .collect();
            for h in hs {
                h.join().unwrap();
            }
            let ms = t0.elapsed().as_millis();
            let db = repo.db_at(&repo.resolve(REF).unwrap()).unwrap();
            println!(
                "{:<13} {:>2} {:>9} {:>5} {:>5} {:>8} {:>8} {:>7} {:>9}  {}",
                mode.name(),
                w,
                count(&repo, &db, "measured"),
                tally.runs.load(SeqCst),
                tally.lost.load(SeqCst),
                tally.most.load(SeqCst),
                tally.retries.load(SeqCst),
                ms,
                repo.net_firings(&db, "loop").unwrap().len(),
                replay_ok(&repo, &db, &net)
            );
        }
    }
}

fn main() {
    if std::env::var("PART1").as_deref() != Ok("0") {
        part1();
    }
    let n = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    part2(n);
}
