//! E244: the word index kept by updates equals one built from scratch, and
//! its answers equal a scan.

use ikam_kernel::textindex::words;
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

const VOCAB: &[&str] = &[
    "Alpha", "beta", "gamma", "delta", "x-ray", "naïve", "Été", "42", "a1b2", "the", "of", "data",
];

fn text(rng: &mut Lcg) -> String {
    (0..1 + rng.next() % 8)
        .map(|_| VOCAB[rng.next() as usize % VOCAB.len()])
        .collect::<Vec<_>>()
        .join([" ", ", ", ". ", "--"][rng.next() as usize % 4])
}

#[test]
fn words_split_like_a_simple_tokenizer() {
    let w: Vec<String> = words("Hello, WORLD! naïve--Été x-ray 42".as_bytes())
        .into_iter()
        .collect();
    assert_eq!(w, ["42", "hello", "naïve", "ray", "world", "x", "été"]);
    assert!(words("a".repeat(40).as_bytes()).is_empty());
    assert_eq!(words("a".repeat(39).as_bytes()).len(), 1);
}

#[test]
fn updates_equal_a_build_and_searches_equal_a_scan() {
    let mut rng = Lcg(11);
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let start: Vec<Event> = (0..800)
        .map(|i| {
            Event::upsert(
                &format!("doc/{i:04}"),
                text(&mut rng).as_bytes(),
                &format!("s{i}"),
            )
        })
        .chain([Event::upsert("other/1", b"alpha beta", "o1")])
        .collect();
    let mut db = repo.db_append(&empty, &start).unwrap();
    let mut index = repo.text_index_build(&db, "doc/").unwrap();
    for round in 0..30 {
        let batch: Vec<Event> = (0..1 + rng.next() % 25)
            .map(|j| {
                let k = format!("doc/{:04}", rng.next() % 900);
                let idem = format!("r{round}-{j}");
                match rng.next() % 4 {
                    0 => Event::remove(&k, false, &idem),
                    _ => Event::upsert(&k, text(&mut rng).as_bytes(), &idem),
                }
            })
            .collect();
        let next = repo.db_append(&db, &batch).unwrap();
        index = repo.text_index_update(&index, &db, &next, "doc/").unwrap();
        db = next;
        assert_eq!(
            index,
            repo.text_index_build(&db, "doc/").unwrap(),
            "round {round}"
        );
        for q in [
            "alpha",
            "beta gamma",
            "x ray",
            "ÉTÉ data",
            "missing",
            "42 the of",
        ] {
            let want: Vec<String> = repo
                .db_scan(&db)
                .unwrap()
                .into_iter()
                .filter(|(k, v)| k.starts_with("doc/") && words(q.as_bytes()).is_subset(&words(v)))
                .map(|(k, _)| k)
                .collect();
            assert_eq!(repo.text_search(&index, q).unwrap(), want, "{q}");
        }
    }
}
