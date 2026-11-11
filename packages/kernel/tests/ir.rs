//! E192: the IR codec returns every file byte-exact, for any bytes and any
//! options. E197: so do blocks, each read on its own.

use ikam_kernel::ir::{self, Options, decode, encode, pack, unpack};
use proptest::prelude::*;

fn all_options() -> Vec<Options> {
    (0..512)
        .map(|b| Options {
            columns: b & 1 != 0,
            numbers: b & 2 != 0,
            siblings: b & 4 != 0,
            rows: b & 8 != 0,
            prose: b & 16 != 0,
            nearest: if b & 32 != 0 { 3 } else { 0 },
            predict: b & 64 != 0,
            group: b & 128 != 0,
            step: b & 256 != 0,
        })
        .collect()
}

fn roundtrip(files: &[(String, Vec<u8>)]) {
    for o in all_options() {
        let parts = encode(files, o);
        let back = unpack(&pack(&parts)).unwrap();
        assert_eq!(back, parts);
        assert_eq!(decode(&parts).unwrap(), files, "{o:?}");
        for keyframe in [false, true] {
            let b = ir::encode_blocks(files, o, 2, keyframe);
            assert_eq!(ir::decode_blocks(&b).unwrap(), files, "{o:?} blocks");
        }
    }
}

fn sample() -> Vec<(String, Vec<u8>)> {
    [
        (
            "a.csv",
            "\"Client\",\"Amount\"\n\"Vista \"\"A\"\"\",\"$1,650,000\"\n,x,\r\n\"open",
        ),
        (
            "a.json",
            "[\n  {\"Client\": \"Vista \\\"A\\\"\", \"Amount\": \"$1,650,000\", \"n\": -1.5e3}\n]",
        ),
        (
            "a.md",
            "# t\n\n| Client | Amount |\n|---|---|\n| Vista \"A\" | $1,650,000 |\n|x",
        ),
        (
            "r1.md",
            "run 12 at 2026-11-10 03:00, rho -0.31 and 007 and 1.\n",
        ),
        (
            "r2.md",
            "run 13 at 2026-11-10 04:00, rho -0.29 and 008 and 2.\n",
        ),
        (
            "c1.csv",
            "id,n,x\na,1,-1\nb,9,0.9\na,2,0\nb,10,1.00\na,3,1\nb,11,$1,999\na,4,2%\nb,12,$2,000\n",
        ),
        (
            "c2.csv",
            "id,n,x\na,5,3%\nb,13,$2,001\na,6,4%\nb,14,007\na,7,5%\nb,15,008\n",
        ),
        ("empty.txt", ""),
        ("nl.txt", "\n\n\n"),
        ("bin.json", "\"\u{0}\\"),
    ]
    .iter()
    .map(|(n, b)| (n.to_string(), b.as_bytes().to_vec()))
    .collect()
}

#[test]
fn tables_reports_and_edge_cases_roundtrip() {
    roundtrip(&sample());
}

proptest! {
    #[test]
    fn any_bytes_roundtrip(
        files in prop::collection::vec(
            (prop::sample::select(vec!["x.csv", "x.json", "x.md", "y.md", "z.txt"]),
             prop::collection::vec(prop::sample::select(
                 b"0123456789-.,$%|\"\\:{}[] ab\n\r\x00".to_vec()), 0..80)),
            0..6)
    ) {
        let mut files: Vec<(String, Vec<u8>)> =
            files.into_iter().map(|(n, b)| (n.to_string(), b)).collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files.dedup_by(|a, b| a.0 == b.0);
        roundtrip(&files);
    }
}

#[test]
fn blocks_roundtrip_and_read_alone() {
    let files = sample();
    for per in [1, 2, 3, 7, 100] {
        for o in all_options() {
            for keyframe in [false, true] {
                let b = ir::encode_blocks(&files, o, per, keyframe);
                assert_eq!(ir::decode_blocks(&b).unwrap(), files, "per {per} {o:?}");
                // backwards, so no block is read after the one before it
                for (i, chunk) in files.chunks(per).enumerate().rev() {
                    assert_eq!(ir::decode_block(&b, i).unwrap(), chunk);
                }
            }
        }
    }
}

/// E202: a column that follows from the id is read out, and a blip, a
/// change and a conflict along it are found where they were put.
#[test]
fn schema_dependencies_and_events() {
    use ikam_kernel::ir::schema::{self, Kind};
    let regions = ["north", "south", "east", "west"];
    let mut files = Vec::new();
    for run in 0..30 {
        let mut s = String::from("id,region,count\n");
        for i in 0..8 {
            let mut region = regions[i % 4];
            if i == 3 && run == 12 {
                region = "south"; // a blip: back to east next run
            }
            if i == 5 && run >= 20 {
                region = "east"; // a change that holds
            }
            s += &format!("link-{i},{region},{}\n", run * 7 + i * 3);
        }
        if run == 25 {
            s += "link-1,west,1\n"; // a second row for link-1, another region
        }
        files.push((format!("runs/r{run:02}.csv"), s.into_bytes()));
    }
    let deps = schema::dependencies(&files, 20, 0.9);
    let region = deps
        .iter()
        .position(|d| d.from == 0 && d.to == 1)
        .expect("region follows from id");
    assert!(deps[region].strength() > 0.9);
    assert!(
        !deps.iter().any(|d| d.to == 2),
        "count changes every run: no key"
    );
    let ev: Vec<_> = schema::events(&files, &deps)
        .into_iter()
        .filter(|e| e.dep == region)
        .collect();
    let at = |k: Kind| -> Vec<(usize, Vec<u8>)> {
        ev.iter()
            .filter(|e| e.kind == k)
            .map(|e| (e.at.0, e.key.clone()))
            .collect()
    };
    assert_eq!(at(Kind::Blip), vec![(12, b"link-3".to_vec())]);
    assert_eq!(at(Kind::Change), vec![(20, b"link-5".to_vec())]);
    assert_eq!(at(Kind::Conflict), vec![(25, b"link-1".to_vec())]);
}

/// E203: a column read from its stream alone equals the column read from the
/// files, for every option set the query takes; the others are refused.
#[test]
fn query_column_and_row_match_the_files() {
    use ikam_kernel::ir::query;
    let files = sample();
    // the truth, from the files
    type Column = Vec<((usize, usize), Vec<u8>)>;
    let mut truth: std::collections::HashMap<(Vec<u8>, usize), Column> = Default::default();
    for (fi, (n, b)) in files.iter().enumerate() {
        for (li, (sk, cells)) in ir::lex_lines(n, b).into_iter().enumerate() {
            for (k, c) in cells.into_iter().enumerate() {
                truth
                    .entry((sk.clone(), k))
                    .or_default()
                    .push(((fi, li), c));
            }
        }
    }
    for o in all_options() {
        let parts = ir::encode(&files, o);
        let ix = query::index(&parts, &parts.skeletons);
        if !o.columns || o.siblings || o.nearest > 0 || o.predict {
            // refused, unless nothing was copied or predicted after all
            if !o.columns {
                assert!(ix.is_err(), "{o:?}");
            }
            continue;
        }
        if o.prose && ix.is_err() {
            panic!("{o:?}: {:?}", ix.err());
        }
        let ix = ix.unwrap();
        let mut fetch = |i: usize| Ok(parts.streams[i].clone());
        for (id, sk) in parts.skeletons.iter().enumerate() {
            let slots = ir::segments(sk).unwrap().len() - 1;
            for k in 0..slots {
                let mut want = truth.get(&(sk.clone(), k)).cloned().unwrap_or_default();
                // files stored as they are have no lines in the index
                want.retain(|(p, _)| !ix.lines[p.0].is_empty());
                let got = query::column(&ix, id, k, &mut fetch).unwrap();
                assert_eq!(got, want, "{o:?} skeleton {id} slot {k}");
                if let Some((_, key)) = want.first().map(|(p, _)| {
                    (
                        p,
                        truth[&(sk.clone(), 0)]
                            .iter()
                            .find(|x| x.0 == *p)
                            .unwrap()
                            .1
                            .clone(),
                    )
                }) {
                    let keys = &truth[&(sk.clone(), 0)];
                    let mine: Vec<_> = want
                        .iter()
                        .filter(|(p, _)| keys.iter().any(|x| x.0 == *p && x.1 == key))
                        .cloned()
                        .collect();
                    assert_eq!(query::row(&ix, id, k, &key, &mut fetch).unwrap(), mine);
                }
            }
        }
    }
}
