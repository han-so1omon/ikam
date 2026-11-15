//! A recorded walk-through of the experiment-loop net, for a demo page.
//!
//! Parses the loop net from its text form (E270), stores it as a graph,
//! and runs three of the loop's next candidates through it on a database
//! (E269): proposals, the single cpu slot making the second candidate wait,
//! a retried firing that changes nothing, a reused key that is refused, and
//! the owner's step that the scheduler is never offered. Then the replay
//! check and the gap list (E275). Prints one JSON document.
//!
//!   cargo run --release --example net_demo > trace.json

#[path = "loop_common/mod.rs"]
mod loop_common;

use std::collections::BTreeMap;
use std::time::Instant;

use ikam_kernel::net::{Marking, Net, marking_id};
use ikam_kernel::{Db, Error, MemStore, Repo};
use loop_common::*;

const LOOP: &str = "\
# the experiment loop: one cpu slot, the owner merges
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

fn marking_json(m: &Marking) -> String {
    let places: Vec<String> = m
        .iter()
        .map(|(p, toks)| {
            let t: Vec<String> = toks
                .iter()
                .map(|(t, c)| format!("{}:{c}", json_str(t)))
                .collect();
            format!("{}:{{{}}}", json_str(p), t.join(","))
        })
        .collect();
    format!("{{{}}}", places.join(","))
}

fn enabled_json(v: &[(String, Option<String>)]) -> String {
    let e: Vec<String> = v
        .iter()
        .map(|(t, k)| {
            format!(
                "[{},{}]",
                json_str(t),
                k.as_deref().map(json_str).unwrap_or("null".into())
            )
        })
        .collect();
    format!("[{}]", e.join(","))
}

struct Demo {
    repo: Repo<MemStore>,
    net: Net,
    db: Db,
    steps: Vec<String>,
}

impl Demo {
    fn step(&mut self, transition: &str, token: &str, key: &str, note: &str) {
        let before = self.repo.net_marking(&self.db, "loop").unwrap();
        let seq_before = self.repo.db_head(&self.db.log).unwrap();
        let start = Instant::now();
        let r = self
            .repo
            .net_fire(&self.db, &self.net, transition, Some(token), key);
        let us = start.elapsed().as_micros();
        let (outcome, rec) = match r {
            Ok((db, f)) => {
                let same = db == self.db;
                self.db = db;
                (
                    if same { "retry, no change" } else { "fired" }.to_string(),
                    format!(
                        "{{\"before\":\"{}\",\"after\":\"{}\"}}",
                        &f.before.to_string()[..12],
                        &f.after.to_string()[..12]
                    ),
                )
            }
            Err(Error::IdempotencyConflict(_)) => (
                "refused: key already used for another firing".into(),
                "null".into(),
            ),
            Err(Error::Exec(m)) => (format!("refused: {m}"), "null".into()),
            Err(e) => panic!("{e}"),
        };
        let after = self.repo.net_marking(&self.db, "loop").unwrap();
        let auto = self.repo.net_enabled(&self.db, &self.net, true).unwrap();
        let all = self.repo.net_enabled(&self.db, &self.net, false).unwrap();
        let owner: Vec<(String, Option<String>)> =
            all.into_iter().filter(|e| !auto.contains(e)).collect();
        self.steps.push(format!(
            "{{\"transition\":{},\"token\":{},\"key\":{},\"note\":{},\"outcome\":{},\"record\":{rec},\"us\":{us},\"seq_before\":{seq_before},\"seq_after\":{},\"changed\":{},\"marking\":{},\"scheduler_may_fire\":{},\"owner_may_fire\":{}}}",
            json_str(transition),
            json_str(token),
            json_str(key),
            json_str(note),
            json_str(&outcome),
            self.repo.db_head(&self.db.log).unwrap(),
            before != after,
            marking_json(&after),
            enabled_json(&auto),
            enabled_json(&owner),
        ));
    }
}

fn main() {
    let net = Net::parse(LOOP).unwrap();
    let mut repo = Repo::new(MemStore::default());
    let graph = repo.put_net(&net).unwrap();
    assert_eq!(repo.net_at(&graph).unwrap().transitions, net.transitions);
    let db = repo.db_empty().unwrap();
    let db = repo.net_init(&db, &net).unwrap();
    let initial = repo.net_marking(&db, "loop").unwrap();
    let mut d = Demo {
        repo,
        net,
        db,
        steps: vec![],
    };
    let (a, b, c) = ("E271", "E278", "E279");
    d.step(
        "propose",
        a,
        "propose-E271",
        "The loop proposes the top three candidates from its gap list.",
    );
    d.step("propose", b, "propose-E278", "");
    d.step("propose", c, "propose-E279", "");
    d.step("start", a, "start-E271", "E271 takes the one cpu slot.");
    d.step(
        "start",
        b,
        "start-E278",
        "E278 cannot start: the slot is taken.",
    );
    d.step(
        "finish",
        a,
        "finish-E271",
        "E271's run ends and frees the slot.",
    );
    d.step(
        "finish",
        a,
        "finish-E271",
        "The same firing again, as after a crash: nothing changes.",
    );
    d.step(
        "start",
        b,
        "finish-E271",
        "A reused key for a different firing is refused.",
    );
    d.step("start", b, "start-E278", "");
    d.step(
        "ask",
        a,
        "ask-E271",
        "E271 waits for the owner. The scheduler is never offered merge.",
    );
    d.step("finish", b, "finish-E278", "");
    d.step("ask", b, "ask-E278", "");
    d.step("start", c, "start-E279", "");
    d.step("finish", c, "finish-E279", "");
    d.step(
        "ask",
        c,
        "ask-E279",
        "All three wait on the owner; nothing merges by itself.",
    );

    let stored = d.repo.net_marking(&d.db, "loop").unwrap();
    let start = Instant::now();
    let replayed = d.repo.net_replay(&d.db, &d.net).unwrap();
    let replay_us = start.elapsed().as_micros();
    let rebuilt = d.repo.db_rebuild(&d.db.log).unwrap();
    let firings = d.repo.net_firings(&d.db, "loop").unwrap();

    let docs = docs();
    let rows = rows(&docs);
    let entries = entries(&docs);
    let notes = open_hypotheses(&docs);
    let gaps = gaps(&rows, &entries, &notes, None);
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for g in &gaps {
        *by.entry(g.source.name()).or_default() += 1;
    }
    let by_json: Vec<String> = by
        .iter()
        .map(|(k, v)| format!("{}:{v}", json_str(k)))
        .collect();
    let sample: Vec<String> = gaps
        .iter()
        .filter(|g| matches!(g.source, Source::Stale | Source::OpenHypothesis))
        .take(8)
        .map(|g| format!("[{},{}]", json_str(g.source.name()), json_str(&g.text)))
        .collect();

    println!(
        "{{\"net_text\":{},\"canonical\":{},\"graph\":\"{}\",\"initial\":{},\"steps\":[{}],\"replay\":{{\"firings\":{},\"equal\":{},\"rebuild_equal\":{},\"us\":{replay_us},\"marking_hash\":\"{}\"}},\"gaps\":{{\"total\":{},\"by_source\":{{{}}},\"sample\":[{}]}}}}",
        json_str(LOOP),
        json_str(&d.net.to_text()),
        &graph.to_string()[..12],
        marking_json(&initial),
        d.steps.join(","),
        firings.len(),
        replayed == stored,
        rebuilt.kv == d.db.kv,
        &marking_id(&stored).to_string()[..12],
        gaps.len(),
        by_json.join(","),
        sample.join(","),
    );
}
