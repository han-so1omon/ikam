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
            route: false,
            tune: false,
            literal: false,
            sort: b % 3 == 0,
            transpose: b % 5 == 1,
            sample: if b % 7 == 2 { 600 } else { 0 },
            bases: b % 4 != 3,
        })
        .collect()
}

/// Every option set, and each again with `route` (E208), with `tune`
/// (E209) and with both.
fn all_options_route() -> Vec<Options> {
    all_options()
        .into_iter()
        .flat_map(|o| {
            [
                o,
                Options { route: true, ..o },
                Options { tune: true, ..o },
                Options {
                    route: true,
                    tune: true,
                    literal: true,
                    ..o
                },
            ]
        })
        .collect()
}

fn roundtrip(files: &[(String, Vec<u8>)]) {
    roundtrip_with(files, &all_options());
}

fn roundtrip_with(files: &[(String, Vec<u8>)], options: &[Options]) {
    for &o in options {
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
    roundtrip_with(&sample(), &all_options_route());
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
        // E208: a spread of option sets with `route`, which may store any
        // kind plain
        let routed: Vec<Options> = all_options_route().into_iter().skip(1).step_by(37).collect();
        roundtrip_with(&files, &routed);
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

/// E203, E205: a column read from its stream alone (and with `predict`, the
/// streams of the line's first cells) equals the column read from the
/// files, for every option set the query takes; the others are refused.
#[test]
fn query_column_and_row_match_the_files() {
    query_matches(&sample());
    // runs where columns follow from the id and a counter steps, so the
    // predict and step tokens are read too
    let mut runs = Vec::new();
    for run in 0..12 {
        let mut s = String::from("id,region,count,n\n");
        for i in 0..6 {
            if (run + i) % 5 == 0 {
                continue;
            }
            let region = ["north", "south", "east"][i % 3];
            s += &format!("link-{i},{region},{},{}.5\n", run * 2 + i, run * 7 + i);
        }
        runs.push((format!("runs/r{run:02}.csv"), s.into_bytes()));
    }
    query_matches(&runs);
}

fn query_matches(files: &[(String, Vec<u8>)]) {
    use ikam_kernel::ir::query;
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
    for o in all_options_route() {
        let parts = ir::encode(files, o);
        let ix = query::index(&parts, &parts.skeletons);
        if !o.columns || o.siblings || o.nearest > 0 {
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

/// E211: tables long enough to sort, in the shapes a real export takes.
fn big_tables() -> Vec<(String, Vec<u8>)> {
    let kinds = [
        "\"Retail, food\"",
        "Services",
        "Construction",
        "\"Rentals\"",
    ];
    let cities = ["SAN DIEGO", "LA JOLLA", "POWAY"];
    let mut a = String::from("ID,NAME,KIND,CITY,AMOUNT\n");
    let mut b = String::from("ID,NAME,KIND,CITY,AMOUNT\r\n");
    let mut c = String::from("ID,NOTE\n");
    let mut x: u64 = 7;
    for i in 0..300u32 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let k = ((x >> 33) % 4) as usize;
        let row = format!(
            "{},Name {},{},{},{} {} {},{}.{:02}",
            1000 + i,
            (x >> 40) % 101,
            kinds[k],
            cities[((x >> 50) % 3) as usize],
            kinds[k],
            "licensed under the city's business tax for this kind of work",
            k * 17,
            (x >> 20) % 997,
            i % 100
        );
        a.push_str(&row);
        a.push('\n');
        if i % 50 == 7 {
            // a short row, and an empty line
            b.push_str("1,short\r\n\r\n");
        }
        b.push_str(&row);
        b.push_str("\r\n");
        c.push_str(&format!("{i},\"line {}\n", i % 5));
        c.push_str("continued\"\n");
    }
    // no newline at the end of b
    b.truncate(b.len() - 2);
    vec![
        ("t/a.csv".into(), a.into_bytes()),
        ("t/b.csv".into(), b.into_bytes()),
        ("t/c.csv".into(), c.into_bytes()),
    ]
}

#[test]
fn sorted_tables_roundtrip() {
    let files = big_tables();
    let o = Options {
        sort: true,
        ..Options::default()
    };
    let keys = ir::sort_keys(&files, o);
    assert!(keys[0].is_some() && keys[1].is_some(), "{keys:?}");
    // a field spans lines: kept in its own order
    assert_eq!(keys[2], None);
    let options: Vec<Options> = all_options_route()
        .into_iter()
        .filter(|o| o.sort)
        .step_by(5)
        .collect();
    roundtrip_with(&files, &options);
    for o in &options {
        if o.columns {
            let parts = encode(&files, *o);
            assert!(ir::query::index(&parts, &parts.skeletons).is_err());
        }
    }
}

/// E212: the same tables laid out by column when stored as they are, with
/// and without `sort`.
#[test]
fn transposed_tables_roundtrip() {
    let mut files = big_tables();
    // E213: columns quoted in every row (one with empty and inner quotes),
    // one quoted in most rows, and a short row
    let mut q = String::from("\"ID\",\"NAME\",\"CODE\",NOTE\n");
    for i in 0..300u32 {
        let name = match i % 7 {
            0 => String::new(),
            1 => format!("a \"\"b\"\" {i}"),
            _ => format!("name, {}", i % 11),
        };
        let code = if i % 50 == 3 {
            "7".to_string()
        } else {
            format!("\"{}\"", i % 4)
        };
        if i == 99 {
            q.push_str(&format!("\"{i}\"\n"));
        } else {
            q.push_str(&format!("\"{i}\",\"{name}\",{code},n{}\n", i % 3));
        }
    }
    files.push(("t/q.csv".into(), q.into_bytes()));
    let options: Vec<Options> = all_options_route()
        .into_iter()
        .filter(|o| o.route || o.prose)
        .map(|o| Options {
            transpose: true,
            ..o
        })
        .step_by(7)
        .collect();
    assert!(options.iter().any(|o| o.sort) && options.iter().any(|o| !o.sort));
    roundtrip_with(&files, &options);
    // stored plain, a table is laid out by column: the raw part holds no
    // row of it as written (a row of the multi-line table may stay)
    let o = Options {
        route: true,
        transpose: true,
        ..Options::default()
    };
    let parts = encode(&files, o);
    if !parts.raw.is_empty() {
        let row = files[0].1.split(|&c| c == b'\n').nth(5).unwrap();
        assert!(!parts.raw.windows(row.len()).any(|w| w == row));
    }
    for o in &options {
        if o.columns && !o.sort && !o.siblings && o.nearest == 0 {
            let parts = encode(&files, *o);
            let ix = ir::query::index(&parts, &parts.skeletons).unwrap();
            let _ = ix;
        }
    }
}

/// E218: a damaged pack or block never panics the decoder: it decodes to
/// something or returns an error. Every byte position of a pack is not
/// tried; a fixed stream of changes (a byte set, a byte dropped, the pack
/// cut short) is, over packs of every new layout (sorted and transposed
/// tables, plain files, unbased files, tuned streams).
#[test]
fn damaged_packs_never_panic() {
    let mut files = sample();
    files.extend(big_tables());
    let one = Options {
        columns: true,
        numbers: true,
        siblings: true,
        nearest: 16,
        predict: true,
        step: true,
        prose: true,
        route: true,
        sort: true,
        transpose: true,
        bases: true,
        tune: true,
        ..Options::default()
    };
    let settings = [
        one,
        Options {
            route: false,
            ..one
        },
        Options {
            rows: true,
            group: true,
            literal: true,
            ..one
        },
        Options {
            columns: false,
            ..Options::default()
        },
    ];
    let mut seed: u64 = 0x5eed;
    let mut next = |n: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % n.max(1)
    };
    for o in settings {
        let packed = pack(&encode(&files, o));
        assert_eq!(decode(&unpack(&packed).unwrap()).unwrap(), files);
        for _ in 0..3000 {
            let mut b = packed.clone();
            match next(4) {
                0 | 1 => {
                    let at = next(b.len());
                    b[at] = next(256) as u8;
                }
                2 => {
                    b.remove(next(b.len()));
                }
                _ => b.truncate(next(b.len())),
            }
            if let Ok(p) = unpack(&b) {
                let _ = decode(&p);
            }
        }
        // the parts as stored by blocks, each changed alone
        let blocks = ir::encode_blocks(&files, o, 3, true);
        for _ in 0..1000 {
            let mut bl = blocks.clone();
            let i = next(bl.blocks.len());
            let part = &mut bl.blocks[i];
            let target = match next(4) {
                0 => &mut part.files,
                1 => &mut part.raw,
                2 => &mut part.directory,
                _ => match part.streams.len() {
                    0 => &mut part.files,
                    n => &mut part.streams[next(n)],
                },
            };
            if target.is_empty() {
                continue;
            }
            let at = next(target.len());
            target[at] = next(256) as u8;
            let _ = ir::decode_blocks(&bl);
            let _ = ir::reader(&bl).block(i);
        }
    }
}

/// E218: a files part that claims huge row and line counts for a sorted,
/// transposed table returns an error and allocates nothing for them.
#[test]
fn huge_table_counts_are_refused() {
    fn var(out: &mut Vec<u8>, mut v: u128) {
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }
    let raw = b"h\n-\n1\n1\nx\ny\n".to_vec();
    for (rows, lines, len) in [
        (u64::MAX as u128, 2, raw.len() as u128),
        (1, u64::MAX as u128, raw.len() as u128),
        (usize::MAX as u128 / 2 + 1, 2, raw.len() as u128),
        (1, 2, u64::MAX as u128),
        (u128::MAX >> 2, u128::MAX >> 2, u128::MAX >> 2),
    ] {
        let mut f = vec![4 | 32 | 64];
        var(&mut f, 1);
        var(&mut f, 5);
        f.extend_from_slice(b"a.csv");
        f.push(7);
        var(&mut f, rows);
        f.push(1);
        var(&mut f, lines);
        f.push(1);
        var(&mut f, len);
        let parts = ir::Parts {
            files: f,
            raw: raw.clone(),
            ..ir::Parts::default()
        };
        assert!(decode(&parts).is_err(), "{rows} {lines} {len}");
    }
}
