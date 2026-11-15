//! E278: the `net::Table` lowering (opt-in) against the interpreter.
//!
//! 1. Agreement: 2,000 seeded random place/transition nets (as E277's), each
//!    walked 300 steps by `Table::walk` and by `Net::is_enabled` and
//!    `Net::fire` with the same picks; every marking must be equal.
//! 2. Speed: ns per step for both, on E277's three sizes.
//!
//!   cargo run --release --example net_table

use std::time::Instant;

use ikam_kernel::net::{Arc, Kind, Marking, Net, Place, Transition};

#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A conservative random net: each transition moves as many tokens out as
/// it takes in, so the walk neither explodes nor drains, and a cycle of
/// single moves keeps it live.
fn random_net(r: &mut Rng, places: usize, transitions: usize) -> Net {
    let pick = |r: &mut Rng, k: usize| {
        let mut v: Vec<usize> = Vec::new();
        while v.len() < k {
            let p = r.below(places);
            if !v.contains(&p) {
                v.push(p);
            }
        }
        v
    };
    let ts = (0..transitions)
        .map(|t| {
            // The first `places` transitions move one token round a cycle,
            // so some transition is always enabled and the walk never
            // deadlocks.
            if t < places {
                let arc = |p: usize| {
                    vec![Arc {
                        place: format!("p{p}"),
                        weight: 1,
                    }]
                };
                return Transition {
                    name: format!("t{t}"),
                    inputs: arc(t),
                    outputs: arc((t + 1) % places),
                    kind: Kind::Pure,
                    capability: None,
                };
            }
            let k = 1 + r.below(3);
            let ins = pick(r, k);
            let inputs: Vec<Arc> = (ins.iter())
                .map(|&p| Arc {
                    place: format!("p{p}"),
                    weight: 1 + r.below(2) as u32,
                })
                .collect();
            let total: u32 = inputs.iter().map(|a| a.weight).sum();
            let k = 1 + r.below(3).min(total as usize - 1);
            let outs = pick(r, k);
            let mut weights = vec![1u32; outs.len()];
            for _ in 0..(total as usize - outs.len()) {
                weights[r.below(outs.len())] += 1;
            }
            Transition {
                name: format!("t{t}"),
                inputs,
                outputs: (outs.iter().zip(weights))
                    .map(|(&p, w)| Arc {
                        place: format!("p{p}"),
                        weight: w,
                    })
                    .collect(),
                kind: Kind::Pure,
                capability: None,
            }
        })
        .collect();
    Net {
        name: "walk".into(),
        version: 1,
        places: (0..places)
            .map(|p| Place {
                name: format!("p{p}"),
                colored: false,
                initial: 2 + r.below(5) as u32,
            })
            .collect(),
        transitions: ts,
    }
}

fn walk_interp(net: &Net, steps: u64, seed: u64) -> (Marking, u64) {
    let mut m = net.initial_marking();
    let mut rng = Rng(seed);
    for step in 0..steps {
        let enabled: Vec<&Transition> = (net.transitions.iter())
            .filter(|t| net.is_enabled(&m, t, None))
            .collect();
        if enabled.is_empty() {
            return (m, step);
        }
        let t = enabled[(rng.next() % enabled.len() as u64) as usize];
        m = net.fire(&m, t, None).unwrap();
    }
    (m, steps)
}

fn walk_table(net: &Net, steps: u64, seed: u64) -> (Marking, u64) {
    let t = net.lower().unwrap();
    let mut m = t.initial.clone();
    let mut rng = Rng(seed);
    let done = t.walk(&mut m, steps, |n| (rng.next() % n as u64) as usize);
    (t.marking(&m), done)
}

fn main() {
    let mut nets = 0;
    let mut steps = 0;
    for i in 0..2000u64 {
        let mut r = Rng(0x7AB1_E000 + i);
        let np = 3 + r.below(40);
        let nt = np + r.below(60);
        let net = random_net(&mut r, np, nt);
        let a = walk_interp(&net, 300, i);
        let b = walk_table(&net, 300, i);
        assert_eq!(a, b, "net {i} differs");
        nets += 1;
        steps += a.1;
    }
    println!("agreement: {nets} random nets, {steps} steps, every marking equal");
    println!("places x transitions | interp ns/step | table ns/step | speedup");
    for (i, &(np, nt)) in [(10usize, 20usize), (100, 200), (1000, 2000)]
        .iter()
        .enumerate()
    {
        let net = random_net(&mut Rng(0xA5A5_0000 + i as u64), np, nt);
        let isteps = (2_000_000 / nt as u64).clamp(1000, 200_000);
        let start = Instant::now();
        let (im, idone) = walk_interp(&net, isteps, 1);
        let ins = start.elapsed().as_nanos() as f64 / idone as f64;
        assert_eq!(walk_table(&net, isteps, 1), (im, idone));
        let tsteps = (200_000_000 / nt as u64).clamp(100_000, 10_000_000);
        let table = net.lower().unwrap();
        let mut m = table.initial.clone();
        let mut rng = Rng(1);
        let start = Instant::now();
        let tdone = table.walk(&mut m, tsteps, |n| (rng.next() % n as u64) as usize);
        let tns = start.elapsed().as_nanos() as f64 / tdone as f64;
        println!("{np} x {nt} | {ins:.0} | {tns:.0} | {:.0}x", ins / tns);
    }
}
