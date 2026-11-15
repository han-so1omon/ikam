//! E285: colored nets in the table, by unfolding colors.
//!
//! The loop net (candidates as colors, one cpu slot, the owner's merge)
//! with K candidates: `Net::unfold` makes one plain place per colored place
//! and color and one transition per binding transition and color, then
//! `Net::lower` makes the table (E278). A seeded random walk runs on the
//! colored interpreter (`Net::is_enabled` and `Net::fire` with tokens) and
//! on the table, and must take the same firings and end in the same counts.
//! Reported: unfolded size, unfold and lower time, ns per step for each.
//!
//!   cargo run --release --example net_unfold

use std::time::Instant;

use ikam_kernel::net::{ANON, Marking, Net, Table};

const LOOP: &str = "\
net loop 1
place cpu 1
place queued colored
place running colored
place measured colored
place waiting colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
transition ask pure: measured -> waiting
transition merge human: waiting ->
";

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn walk_interp(net: &Net, cs: &[&str], steps: u64, seed: u64) -> (Marking, Vec<String>) {
    let mut m = net.initial_marking();
    let mut r = Rng(seed);
    let mut fired = Vec::new();
    for _ in 0..steps {
        let mut list = Vec::new();
        for t in &net.transitions {
            if net.binds_color(t) {
                for c in cs {
                    if net.is_enabled(&m, t, Some(c)) {
                        list.push((t, Some(*c)));
                    }
                }
            } else if net.is_enabled(&m, t, None) {
                list.push((t, None));
            }
        }
        if list.is_empty() {
            break;
        }
        let (t, c) = list[(r.next() % list.len() as u64) as usize];
        m = net.fire(&m, t, c).unwrap();
        fired.push(match c {
            Some(c) => format!("{}.{c}", t.name),
            None => t.name.clone(),
        });
    }
    (m, fired)
}

fn dense(t: &Table, m: &Marking) -> Vec<u64> {
    (t.places.iter())
        .map(|p| {
            let (base, tok) = p.split_once('.').unwrap_or((p, ANON));
            m.get(base).and_then(|x| x.get(tok)).copied().unwrap_or(0)
        })
        .collect()
}

fn main() {
    let net = Net::parse(LOOP).unwrap();
    let seed = 0x0285_0285_0285_0285;
    println!(
        "candidates | places x transitions | unfold+lower ms | interp ns/step | table ns/step | x"
    );
    for k in [10usize, 100, 1000] {
        let colors: Vec<String> = (0..k).map(|i| format!("E{i:04}")).collect();
        let cs: Vec<&str> = colors.iter().map(String::as_str).collect();
        let start = Instant::now();
        let t = net.unfold(&cs).unwrap().lower().unwrap();
        let build_ms = start.elapsed().as_secs_f64() * 1000.0;

        let isteps = (200_000 / k as u64).max(200);
        let start = Instant::now();
        let (im, ifired) = walk_interp(&net, &cs, isteps, seed);
        let interp_ns = start.elapsed().as_nanos() as f64 / ifired.len() as f64;

        // Same walk on the table: same firings, same counts.
        let mut tm = t.initial.clone();
        let mut r = Rng(seed);
        let mut names = Vec::new();
        let mut en = Vec::new();
        for _ in 0..isteps {
            t.enabled_into(&tm, &mut en);
            if en.is_empty() {
                break;
            }
            let i = en[(r.next() % en.len() as u64) as usize];
            t.fire(&mut tm, i);
            names.push(t.transitions[i].clone());
        }
        assert_eq!(names, ifired, "firings differ");
        assert_eq!(tm, dense(&t, &im), "counts differ");

        let steps = (50_000_000 / t.transitions.len() as u64).max(10_000);
        let mut tm = t.initial.clone();
        let mut r = Rng(seed);
        let start = Instant::now();
        let done = t.walk(&mut tm, steps, |n| (r.next() % n as u64) as usize);
        let table_ns = start.elapsed().as_nanos() as f64 / done as f64;
        println!(
            "{k:>10} | {} x {} | {build_ms:.1} | {interp_ns:.0} | {table_ns:.0} | {:.0}",
            t.places.len(),
            t.transitions.len(),
            interp_ns / table_ns
        );
    }
}
