//! E287: planning a night away from the keyboard by simulating the loop
//! net with times.
//!
//! A ranked queue of candidates goes through a timed loop net: the agent
//! codes a candidate, the cpu benches it, the agent writes the entry. Some
//! candidates need the owner's decision first (a human transition, which
//! never fires while the owner is away). Each step that takes time is a
//! start transition (it takes its resources) and an end transition fired
//! when its time is up (it gives them back); the simulation fires them on
//! the net's own interpreter (`Net::fire`), so the plan is the net.
//!
//! Durations come from tonight's run: the minutes between the commits of
//! E279 to E286 on their exp/ branches, resampled with a seeded jitter
//! (x0.5 to x1.5). How much of each is bench time was not recorded, so the
//! bench share is swept (20, 50 and 80 percent; the write-up is 15 percent
//! and coding the rest). The night is 8 hours, and nothing is started that
//! its estimate says cannot finish before the end; a bench or write-up
//! that would run past the end is not started (the candidate is left
//! half-done).
//!
//! Four plans, the same queue:
//! - `serial, strict order`: the agent waits on its own bench, and takes
//!   candidates in rank order only, so it stops at the first one that
//!   needs the owner.
//! - `serial, skip asks`: as above, passing over candidates that need the
//!   owner.
//! - `pipelined`: the agent codes the next candidate while the cpu benches,
//!   with no limit on how far it codes ahead.
//! - `pipelined, 4 in flight`: as above, with a `slot` place of 4 tokens
//!   that coding takes and the bench's end gives back (a work-in-progress
//!   limit, as on a kanban board).
//! - `2 cpus, 6 in flight`.
//!
//! Reported: candidates finished, those left half-done, the first that
//! stalled, agent and cpu busy shares. Then the best plan's firings are replayed on an IKAM store
//! (`net_fire`, MemStore): replay equal, all finished ones waiting on the
//! owner. No kernel change.
//!
//!   cargo run --release --example net_night_plan [-- QUEUE]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use ikam_kernel::net::{Marking, Net};
use ikam_kernel::{MemStore, Repo};

/// Minutes between the commits of E279..E286 tonight (from E278's).
const TONIGHT: [f64; 8] = [22.0, 9.4, 9.3, 7.2, 2.0, 12.9, 2.5, 2.8];
const NIGHT_S: u64 = 8 * 3600;
const WRITE_SHARE: f64 = 0.15;

fn net_text(cpus: u32, serial: bool, wip: u32) -> String {
    let (take, give) = if serial {
        ("built, cpu, agent", "measured, cpu, agent, slot")
    } else {
        ("built, cpu", "measured, cpu, slot")
    };
    format!(
        "\
net plan 1
place agent 1
place cpu {cpus}
place slot {wip}
place ready colored
place asks colored
place coding colored
place built colored
place running colored
place measured colored
place writing colored
place waiting colored
transition propose agent propose: -> ready
transition ask agent propose: -> asks
transition decide human: asks -> ready
transition code agent code: ready, agent, slot -> coding
transition coded pure: coding -> built, agent
transition bench cmd bench: {take} -> running
transition benched pure: running -> {give}
transition write agent write: measured, agent -> writing
transition written pure: writing -> waiting, agent
"
    )
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[derive(Clone)]
struct Cand {
    tok: String,
    asks: bool,
    code: u64,
    bench: u64,
    write: u64,
}

fn queue(n: usize, bench_share: f64, ask_share: f64) -> Vec<Cand> {
    let mut rng = Rng(0x0287_5EED_1234_ABCD);
    (0..n)
        .map(|i| {
            let mins = TONIGHT[(rng.next() % 8) as usize] * (0.5 + rng.unit());
            let s = mins * 60.0;
            let bench = (s * bench_share) as u64;
            let write = (s * WRITE_SHARE) as u64;
            Cand {
                tok: format!("C{i:03}"),
                asks: rng.unit() < ask_share,
                code: (s as u64).saturating_sub(bench + write).max(1),
                bench: bench.max(1),
                write: write.max(1),
            }
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Plan {
    SerialStrict,
    SerialSkip,
    Pipelined,
    PipelinedWip,
    Pipelined2Wip,
}

impl Plan {
    fn name(self) -> &'static str {
        match self {
            Plan::SerialStrict => "serial, strict order",
            Plan::SerialSkip => "serial, skip asks",
            Plan::Pipelined => "pipelined",
            Plan::PipelinedWip => "pipelined, 4 in flight",
            Plan::Pipelined2Wip => "2 cpus, 6 in flight",
        }
    }
    fn cpus(self) -> u32 {
        if self == Plan::Pipelined2Wip { 2 } else { 1 }
    }
    fn net(self) -> Net {
        let (serial, wip) = match self {
            Plan::SerialStrict | Plan::SerialSkip => (true, 1),
            Plan::Pipelined => (false, 1000),
            Plan::PipelinedWip => (false, 4),
            Plan::Pipelined2Wip => (false, 6),
        };
        Net::parse(&net_text(self.cpus(), serial, wip)).unwrap()
    }
}

struct Outcome {
    finished: usize,
    unfinished: usize,
    stalled_at: Option<String>,
    agent_busy: u64,
    cpu_busy: u64,
    firings: Vec<(String, String)>,
    marking: Marking,
}

fn has(m: &Marking, place: &str, tok: &str) -> bool {
    m.get(place).is_some_and(|p| p.contains_key(tok))
}

fn simulate(plan: Plan, q: &[Cand]) -> Outcome {
    let net = plan.net();
    let mut m = net.initial_marking();
    let mut firings = Vec::new();
    let fire = |m: &mut Marking, t: &str, tok: &str, firings: &mut Vec<(String, String)>| {
        let tr = net.transition(t).unwrap();
        *m = net.fire(m, tr, Some(tok)).expect(t);
        firings.push((t.to_string(), tok.to_string()));
    };
    for c in q {
        fire(
            &mut m,
            if c.asks { "ask" } else { "propose" },
            &c.tok,
            &mut firings,
        );
    }
    // (end time, sequence, end transition, candidate index)
    let mut events: BinaryHeap<Reverse<(u64, u64, &str, usize)>> = BinaryHeap::new();
    let (mut now, mut seq) = (0u64, 0u64);
    let (mut agent_busy, mut cpu_busy) = (0u64, 0u64);
    let mut next_rank = 0usize; // strict order: the next candidate to code
    let mut stalled_at = None;
    loop {
        // Start whatever can start now: write-ups first, then benches, then
        // coding the best-ranked candidate whose estimate fits the night.
        loop {
            let mut started = false;
            for (i, c) in q.iter().enumerate() {
                for (t, end, place, dur) in [
                    ("write", "written", "measured", c.write),
                    ("bench", "benched", "built", c.bench),
                ] {
                    let tr = net.transition(t).unwrap();
                    if has(&m, place, &c.tok)
                        && now + dur <= NIGHT_S
                        && net.is_enabled(&m, tr, Some(&c.tok))
                    {
                        fire(&mut m, t, &c.tok, &mut firings);
                        if t == "write" {
                            agent_busy += dur;
                        } else {
                            cpu_busy += dur;
                            if matches!(plan, Plan::SerialStrict | Plan::SerialSkip) {
                                agent_busy += dur;
                            }
                        }
                        events.push(Reverse((now + dur, seq, end, i)));
                        seq += 1;
                        started = true;
                    }
                }
            }
            let code = net.transition("code").unwrap();
            let pick = if plan == Plan::SerialStrict {
                q.get(next_rank).map(|c| (next_rank, c))
            } else {
                q.iter().enumerate().find(|(_, c)| {
                    has(&m, "ready", &c.tok)
                        && c.code + c.bench + c.write <= NIGHT_S.saturating_sub(now)
                })
            };
            if let Some((i, c)) = pick {
                if !has(&m, "ready", &c.tok) {
                    if stalled_at.is_none() && has(&m, "asks", &c.tok) {
                        stalled_at = Some(c.tok.clone());
                    }
                } else if c.code + c.bench + c.write <= NIGHT_S.saturating_sub(now)
                    && net.is_enabled(&m, code, Some(&c.tok))
                {
                    fire(&mut m, "code", &c.tok, &mut firings);
                    agent_busy += c.code;
                    events.push(Reverse((now + c.code, seq, "coded", i)));
                    seq += 1;
                    next_rank = i + 1;
                    started = true;
                }
            }
            if !started {
                break;
            }
        }
        let Some(Reverse((t, _, end, i))) = events.pop() else {
            break;
        };
        now = t;
        fire(&mut m, end, &q[i].tok, &mut firings);
    }
    assert!(now <= NIGHT_S, "nothing runs past the night");
    Outcome {
        finished: m.get("waiting").map_or(0, |w| w.len()),
        unfinished: ["coding", "built", "running", "measured", "writing"]
            .iter()
            .map(|p| m.get(*p).map_or(0, |w| w.len()))
            .sum(),
        stalled_at,
        agent_busy,
        cpu_busy,
        firings,
        marking: m,
    }
}

fn main() {
    let n: usize = (std::env::args().nth(1))
        .and_then(|a| a.parse().ok())
        .unwrap_or(150);
    let total: f64 = TONIGHT.iter().sum();
    println!(
        "queue of {n} ranked candidates; tonight's 8 experiments took {total:.0} min ({:.1} min each); night {} h",
        total / 8.0,
        NIGHT_S / 3600
    );
    let mut best: Option<(usize, usize, Plan, Vec<Cand>)> = None;
    for ask_share in [0.0, 0.25] {
        for bench_share in [0.2, 0.5, 0.8] {
            let q = queue(n, bench_share, ask_share);
            println!(
                "owner decisions needed by {:.0}% of candidates, bench {:.0}% of each:",
                ask_share * 100.0,
                bench_share * 100.0
            );
            for plan in [
                Plan::SerialStrict,
                Plan::SerialSkip,
                Plan::Pipelined,
                Plan::PipelinedWip,
                Plan::Pipelined2Wip,
            ] {
                let o = simulate(plan, &q);
                println!(
                    "  {:<22} {:>3} finished, {} left half-done, agent busy {:>3.0}%, cpu busy {:>3.0}%{}",
                    plan.name(),
                    o.finished,
                    o.unfinished,
                    100.0 * o.agent_busy as f64 / NIGHT_S as f64,
                    100.0 * o.cpu_busy as f64 / (NIGHT_S * plan.cpus() as u64) as f64,
                    o.stalled_at
                        .map(|s| format!(", stalled at {s} (needs the owner)"))
                        .unwrap_or_default()
                );
                if ask_share > 0.0
                    && bench_share == 0.5
                    && best.as_ref().is_none_or(|(f, u, _, _)| {
                        (o.finished, Reverse(o.unfinished)) > (*f, Reverse(*u))
                    })
                {
                    best = Some((o.finished, o.unfinished, plan, q.clone()));
                }
            }
        }
    }

    // Replay the best plan (asks at 25%, bench at 50%) on an IKAM store.
    let (_, _, plan, q) = best.unwrap();
    let o = simulate(plan, &q);
    let net = plan.net();
    let mut repo = Repo::new(MemStore::default());
    let db = repo.db_empty().unwrap();
    let mut db = repo.net_init(&db, &net).unwrap();
    for (k, (t, tok)) in o.firings.iter().enumerate() {
        db = repo
            .net_fire(&db, &net, t, Some(tok), &format!("{k}-{t}-{tok}"))
            .unwrap()
            .0;
    }
    let stored = repo.net_marking(&db, "plan").unwrap();
    assert_eq!(stored, o.marking, "store equals the simulation");
    assert_eq!(repo.net_replay(&db, &net).unwrap(), stored, "replay");
    let waiting = repo.net_enabled(&db, &net, false).unwrap();
    let decisions = waiting.iter().filter(|(t, _)| t == "decide").count();
    println!(
        "best plan ({}) replayed on an IKAM store: {} firings, replay equal, {} waiting on the owner, {} owner decisions open",
        plan.name(),
        o.firings.len(),
        o.finished,
        decisions
    );
}
