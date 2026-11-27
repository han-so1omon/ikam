//! E421: answers pinned to a commit are flagged stale exactly when a full
//! recheck finds their result changed (or, for recorded answers, when a
//! row they read changed).

use std::collections::BTreeSet;

use ikam_kernel::answers::{Answer, AnswerKind, Query, ReadSet, field};
use ikam_kernel::{Event, MemStore, Repo};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn row(i: usize, word: u64, x: u64, run: usize) -> Vec<u8> {
    format!("[{i}, \"w{word}\", {x}.5, \"{}\"]", "x".repeat(run)).into_bytes()
}

fn key(i: usize) -> String {
    format!("t/r/p{i:06}")
}

#[test]
fn fields_are_read_from_arrays_and_objects() {
    let r = br#"[1, "a,]b\"", {"x": [1, 2]}, 2.5e3 ]"#;
    assert_eq!(field(r, "0"), Some(&b"1"[..]));
    assert_eq!(field(r, "1"), Some(&br#""a,]b\"""#[..]));
    assert_eq!(field(r, "2"), Some(&br#"{"x": [1, 2]}"#[..]));
    assert_eq!(field(r, "3"), Some(&b"2.5e3"[..]));
    assert_eq!(field(r, "4"), None);
    let o = br#"{"a": null, "b" : "x}", "c": true}"#;
    assert_eq!(field(o, "b"), Some(&br#""x}""#[..]));
    assert_eq!(field(o, "c"), Some(&b"true"[..]));
    assert_eq!(field(o, "d"), None);
    assert_eq!(field(b"not json", "0"), None);
}

#[test]
fn records_round_trip() {
    let mut repo = Repo::new(MemStore::default());
    let db = repo.db_empty().unwrap();
    let c = repo.db_commit("db/t", &db, "empty").unwrap();
    let q = Query::CountEq {
        prefix: "t/r/".into(),
        field: "1".into(),
        value: b"\"w\n1\"".to_vec(),
    };
    assert_eq!(Query::parse(&q.text()), Some(q.clone()));
    let a = Answer {
        question: "how many\nrows?".into(),
        query: q.text(),
        kind: AnswerKind::Recorded,
        definitions: Some(c),
        commit: c,
        read: q.read_set(),
        result: b"0\nwith a newline".to_vec(),
    };
    assert_eq!(Answer::decode(&a.encode()).unwrap(), a);
    let p = repo.answer_pure("q", &q, &c, None).unwrap();
    assert_eq!(Answer::decode(&p.encode()).unwrap(), p);
    assert_eq!(p.result, b"0");
}

#[test]
fn stale_flags_equal_a_full_recheck() {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Lcg(7);
    let n = 3_000;
    let db = repo.db_empty().unwrap();
    let events: Vec<Event> = (0..n)
        .map(|i| {
            let v = row(i, rng.next() % 40, rng.next() % 1000, 3 + i % 5);
            Event::upsert(&key(i), &v, &format!("r{i}"))
        })
        .collect();
    let db = repo.db_append(&db, &events).unwrap();
    let c1 = repo.db_commit("db/t", &db, "load").unwrap();

    let mut answers = Vec::new();
    for j in 0..60 {
        let i = (rng.next() as usize) % n;
        let q = match j % 3 {
            0 => Query::Get { key: key(i) },
            1 => Query::Sum {
                start: key(i),
                end: key(i + 50),
                field: "2".into(),
            },
            _ => Query::CountEq {
                prefix: "t/r/".into(),
                field: "1".into(),
                value: format!("\"w{}\"", j % 40).into_bytes(),
            },
        };
        let a = repo.answer_pure(&format!("q{j}"), &q, &c1, None).unwrap();
        answers.push((format!("p{j:03}"), a));
    }
    // A recorded answer over a range's text column: not rerun, flagged by
    // its read set alone.
    answers.push((
        "rec".into(),
        Answer {
            question: "describe rows 100 to 199".into(),
            query: "an agent's summary".into(),
            kind: AnswerKind::Recorded,
            definitions: None,
            commit: c1,
            read: ReadSet {
                ranges: vec![(key(100), key(200))],
                fields: Some(vec!["3".into()]),
                filter: None,
            },
            result: b"mostly short runs of x".to_vec(),
        },
    ));
    let set = repo.answers_put(&answers).unwrap();
    // Every pure answer reruns exactly at its own commit.
    for (_, a) in &answers {
        assert_ne!(repo.answer_recheck(a, &c1).unwrap(), Some(false));
    }

    // Change rows: a word, a number, the run of x's (read by nothing but
    // the recorded answer and the gets), the same bytes again, deletes and
    // inserts between existing keys.
    let mut changes = Vec::new();
    let mut touched = BTreeSet::new();
    for t in 0..120 {
        let i = (rng.next() as usize) % n;
        if !touched.insert(i) {
            continue;
        }
        let old = repo.db_get(&db, &key(i)).unwrap().unwrap();
        let w = std::str::from_utf8(field(&old, "1").unwrap()).unwrap();
        let w: u64 = w.trim_matches('"')[1..].parse().unwrap();
        let x: u64 = std::str::from_utf8(field(&old, "2").unwrap())
            .unwrap()
            .trim_end_matches(".5")
            .parse()
            .unwrap();
        let run = field(&old, "3").unwrap().len() - 2;
        let id = format!("c{t}");
        changes.push(match t % 6 {
            0 => Event::upsert(&key(i), &row(i, (w + 1) % 40, x, run), &id),
            1 => Event::upsert(&key(i), &row(i, w, x + 1, run), &id),
            2 => Event::upsert(&key(i), &row(i, w, x, run + 1), &id),
            3 => Event::upsert(&key(i), &old, &id),
            4 => Event::remove(&key(i), false, &id),
            _ => Event::upsert(&format!("{}a", key(i)), &row(i, w, 3, 2), &id),
        });
    }
    let db2 = repo.db_append(&db, &changes).unwrap();
    let c2 = repo.db_commit("db/t", &db2, "change").unwrap();

    let flagged = repo.answers_flag(&set, &c1, &c2).unwrap();
    let flags: BTreeSet<String> = flagged.stale.iter().cloned().collect();
    let mut changed = BTreeSet::new();
    for (name, a) in &answers {
        if repo.answer_recheck(a, &c2).unwrap() == Some(false) {
            changed.insert(name.clone());
        }
    }
    let pure_flags: BTreeSet<String> = flags.iter().filter(|f| *f != "rec").cloned().collect();
    assert_eq!(pure_flags, changed);
    assert!(!changed.is_empty() && changed.len() < answers.len() - 1);
    // Same-bytes rewrites are not changes.
    assert!(flagged.rows_changed < touched.len());

    // The marks are kept beside the unchanged records; a second flag over
    // the same change adds nothing, and the records still read back.
    let root = flagged.root.unwrap();
    let read = repo.answers_read(&root).unwrap();
    for (name, a) in &answers {
        let (b, mark) = &read[name];
        assert_eq!(b, a);
        assert_eq!(mark.is_some(), flags.contains(name));
        if mark.is_some() {
            assert_eq!(mark.unwrap(), c2);
        }
    }
    let again = repo.answers_flag(&root, &c1, &c2).unwrap();
    assert!(again.stale.is_empty());
    assert_eq!(again.root, Some(root));
    // No change, no flags.
    let none = repo.answers_flag(&set, &c2, &c2).unwrap();
    assert!(none.stale.is_empty() && none.rows_changed == 0);
}

#[test]
fn a_recorded_answer_flags_only_on_its_fields_in_its_range() {
    let rs = ReadSet {
        ranges: vec![("a".into(), "b".into())],
        fields: Some(vec!["1".into()]),
        filter: None,
    };
    assert!(!rs.touched_by(Some(b"[1, 2]"), Some(b"[9, 2]")));
    assert!(rs.touched_by(Some(b"[1, 2]"), Some(b"[1, 3]")));
    assert!(rs.touched_by(Some(b"[1, 2]"), None));
    assert!(!rs.touched_by(None, Some(b"[1]")));
    let f = ReadSet {
        filter: Some(("0".into(), b"1".to_vec())),
        ..rs.clone()
    };
    assert!(!f.touched_by(Some(b"[2, 2]"), Some(b"[2, 3]")));
    assert!(f.touched_by(Some(b"[1, 2]"), Some(b"[2, 2]")));
    let whole = ReadSet { fields: None, ..rs };
    assert!(whole.touched_by(Some(b"[1, 2]"), Some(b"[1, 2] ")));
    assert!(!whole.touched_by(Some(b"[1, 2]"), Some(b"[1, 2]")));
}
