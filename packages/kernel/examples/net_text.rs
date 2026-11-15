//! E270: the net text form round-trips and refuses malformed nets.
//!
//! 1. Round trip: N seeded random nets (2 to 12 places, plain or colored,
//!    1 to 12 transitions of every kind, weights 1 to 3, optional
//!    capabilities) are printed with `Net::to_text`, parsed back and
//!    compared; the canonical text must be a fixed point.
//! 2. Mutation: each printed text has one random edit (a character deleted,
//!    duplicated or replaced from a small alphabet of the grammar's own
//!    symbols, or two lines swapped). Every result must either be refused
//!    with an error naming a line, or parse to a net that passes
//!    `Net::check` and round-trips; no input may panic.
//! 3. The parse time of the experiment-loop net.
//!
//!   cargo run --release --example net_text [-- N]
//!
//! Deterministic (fixed seed). N defaults to 10000.

use std::time::Instant;

use ikam_kernel::net::{Arc, Kind, Net, Place, Transition};

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

const KINDS: [Kind; 4] = [Kind::Pure, Kind::Cmd, Kind::Agent, Kind::Human];

fn random_net(r: &mut Rng, i: usize) -> Net {
    let np = 2 + r.below(11);
    let places: Vec<Place> = (0..np)
        .map(|p| {
            let colored = r.below(2) == 0;
            Place {
                name: format!("p{p}"),
                colored,
                initial: if colored { 0 } else { r.below(4) as u32 },
            }
        })
        .collect();
    let side = |r: &mut Rng| -> Vec<Arc> {
        (0..r.below(4))
            .map(|_| Arc {
                place: format!("p{}", r.below(np)),
                weight: 1 + r.below(3) as u32,
            })
            .collect()
    };
    let nt = 1 + r.below(12);
    let transitions = (0..nt)
        .map(|t| {
            let kind = KINDS[r.below(4)];
            let mut inputs = side(r);
            let outputs = side(r);
            if inputs.is_empty() && outputs.is_empty() {
                inputs.push(Arc {
                    place: "p0".into(),
                    weight: 1,
                });
            }
            Transition {
                name: format!("t{t}"),
                inputs,
                outputs,
                kind,
                capability: (kind != Kind::Human && r.below(2) == 0).then(|| format!("cap-{t}")),
            }
        })
        .collect();
    Net {
        name: format!("n{i}"),
        version: 1 + r.below(9) as u32,
        places,
        transitions,
    }
}

const SYMBOLS: &[u8] = b" ,:*->#0123ptc\nnet place transition colored pure human";

fn mutate(r: &mut Rng, text: &str) -> String {
    let mut b = text.as_bytes().to_vec();
    match r.below(4) {
        0 if !b.is_empty() => {
            b.remove(r.below(b.len()));
        }
        1 if !b.is_empty() => {
            let i = r.below(b.len());
            b.insert(i, b[i]);
        }
        2 if !b.is_empty() => {
            let i = r.below(b.len());
            b[i] = SYMBOLS[r.below(SYMBOLS.len())];
        }
        _ => {
            let mut lines: Vec<&str> = text.lines().collect();
            let (i, j) = (r.below(lines.len()), r.below(lines.len()));
            lines.swap(i, j);
            return lines.join("\n") + "\n";
        }
    }
    String::from_utf8(b).unwrap_or_default()
}

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

fn main() {
    let n: usize = (std::env::args().nth(1))
        .and_then(|a| a.parse().ok())
        .unwrap_or(10000);
    let mut r = Rng(0x5DEE_CE66_D1CE_4E5B);
    let (mut refused_by_check, mut round_trips) = (0, 0);
    let (mut refused, mut accepted, mut changed_net) = (0, 0, 0);
    let mut lines_named = 0;
    for i in 0..n {
        let net = random_net(&mut r, i);
        // The generator should only make valid nets; count any that are not.
        if net.check().is_err() {
            refused_by_check += 1;
            continue;
        }
        let text = net.to_text();
        let back = Net::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(back, net, "round trip differs:\n{text}");
        assert_eq!(back.to_text(), text, "canonical text is not a fixed point");
        round_trips += 1;

        let bad = mutate(&mut r, &text);
        let out = std::panic::catch_unwind(|| Net::parse(&bad));
        match out {
            Err(_) => panic!("parse panicked on:\n{bad}"),
            Ok(Err(e)) => {
                refused += 1;
                lines_named += e.to_string().contains("line ") as usize;
            }
            Ok(Ok(m)) => {
                accepted += 1;
                m.check().expect("an accepted net fails check");
                assert_eq!(Net::parse(&m.to_text()).unwrap(), m);
                changed_net += (m != net) as usize;
            }
        }
    }
    let reps = 10_000;
    let start = Instant::now();
    for _ in 0..reps {
        std::hint::black_box(Net::parse(std::hint::black_box(LOOP)).unwrap());
    }
    let parse_us = start.elapsed().as_secs_f64() * 1e6 / reps as f64;
    println!(
        "round trip: {round_trips} of {n} random nets equal after print and parse, canonical text a fixed point ({refused_by_check} refused by check before printing)"
    );
    println!(
        "mutation: {refused} refused ({lines_named} naming a line), {accepted} accepted (all pass check and round-trip; {changed_net} differ from the original net), 0 panics"
    );
    println!("parse of the experiment-loop net: {parse_us:.1} us");
}
