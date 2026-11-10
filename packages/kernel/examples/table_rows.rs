//! E164: large tables stored with rows as fragments, so a one-cell edit
//! writes about one row instead of what the matcher writes for the file.
//!
//! `cargo run --release --example table_rows`
//!
//! Tables (synthetic, as E161's scaled run): `fixtures/tables`'
//! s-local-retail-v01.csv rows repeated with a row number and every digit
//! redrawn (so rows do not repeat), as csv and as a markdown table, at
//! 1,000, 10,000 and 100,000 rows. One edit: a digit
//! appended to the last cell of the middle row.
//!
//! Forms, each on a fresh store holding the table:
//! - `plain`: `put_content` (the matcher plans the edited file);
//! - `rows`: one render naming every row (`put_cut`, no fan-out);
//! - `groups-64`: one render over groups of 64 rows;
//! - `tree-64`: a tree of renders, 64 children each (`put_cut` fan-out);
//! - `tree-groups-64`: the same tree over groups of 64 rows.
//!
//! Arguments name forms to run (all when none).
//!
//! Per form: stored bytes at rest, put ms, the bytes of the objects the edit
//! adds and its ms
//! (median of 3), a whole read and a 4 KiB range read from the middle (ms,
//! and the decode work it took),
//! all checked byte-exact, with fsck clean after the edit and a repack.

use std::ops::Range;
use std::path::Path;
use std::time::Instant;

use ikam_kernel::{Edit, MemStore, Repo, Store};

/// Bytes of the objects not in `before`.
fn added(repo: &Repo<MemStore>, before: &std::collections::HashSet<ikam_kernel::Id>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .filter(|id| !before.contains(id))
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn stored(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn lines(b: &[u8]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut s = 0;
    for (i, &c) in b.iter().enumerate() {
        if c == b'\n' {
            out.push(s..i + 1);
            s = i + 1;
        }
    }
    if s < b.len() {
        out.push(s..b.len());
    }
    out
}

fn groups(rs: &[Range<usize>], n: usize) -> Vec<Range<usize>> {
    rs.chunks(n)
        .map(|g| g[0].start..g[g.len() - 1].end)
        .collect()
}

fn table(n: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let csv = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tables/s-local-retail-v01.csv"),
    )
    .unwrap();
    let mut rows: Vec<&str> = csv
        .lines()
        .filter(|l| !l.trim_matches(',').is_empty())
        .collect();
    let header = rows.remove(0);
    let mk = |edit: bool| {
        let mut c = format!("\"Row\",{header}\n");
        let mut m = format!(
            "| Row | {} |\n",
            header.replace('"', "").replace(',', " | ")
        );
        m.push_str(&"|---".repeat(header.split(',').count() + 1));
        m.push_str("|\n");
        let mut x: u64 = 0x2545f4914f6cdd1d;
        for i in 0..n {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            // Varied rows: every digit of the source row redrawn, so rows do
            // not repeat (real tables are rarely copies of a few rows).
            let mut y = x;
            let mut r: String = rows[i % rows.len()]
                .chars()
                .map(|c| {
                    if c.is_ascii_digit() {
                        y = y.wrapping_mul(6364136223846793005).wrapping_add(1);
                        (b'0' + ((y >> 59) % 10) as u8) as char
                    } else {
                        c
                    }
                })
                .collect();
            if edit && i == n / 2 {
                r.push('1');
            }
            c.push_str(&format!("{i},{r}\n"));
            m.push_str(&format!(
                "| {i} | {} |\n",
                r.replace('"', "").replace(',', " | ")
            ));
        }
        (c.into_bytes(), m.into_bytes())
    };
    let (c0, m0) = mk(false);
    let (c1, m1) = mk(true);
    (c0, c1, m0, m1)
}

fn put(repo: &mut Repo<MemStore>, form: &str, b: &[u8]) -> ikam_kernel::Id {
    let ls = lines(b);
    match form {
        "plain" => repo.put_content(b).unwrap().id,
        "rows" => repo.put_cut(b, &ls, None).unwrap(),
        "groups-64" => repo.put_cut(b, &groups(&ls, 64), None).unwrap(),
        "tree-64" => repo.put_cut(b, &ls, Some(64)).unwrap(),
        "tree-groups-64" => repo.put_cut(b, &groups(&ls, 64), Some(64)).unwrap(),
        _ => unreachable!(),
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn main() {
    for n in [1_000usize, 10_000, 100_000] {
        let (c0, c1, m0, m1) = table(n);
        for (fmt, old, new) in [("csv", &c0, &c1), ("md", &m0, &m1)] {
            let forms: Vec<String> = std::env::args().skip(1).collect();
            let all = ["plain", "rows", "groups-64", "tree-64", "tree-groups-64"];
            for form in all
                .into_iter()
                .filter(|f| forms.is_empty() || forms.iter().any(|x| x == f))
            {
                let mut runs = Vec::new();
                for _ in 0..3 {
                    let mut repo = Repo::new(MemStore::default());
                    let t = Instant::now();
                    let id0 = put(&mut repo, form, old);
                    let put_ms = ms(t);
                    let at_rest = stored(&repo);
                    let before: std::collections::HashSet<_> =
                        repo.store().ids().unwrap().into_iter().collect();
                    let t = Instant::now();
                    let id1 = put(&mut repo, form, new);
                    let edit_ms = ms(t);
                    let written = added(&repo, &before);
                    let t = Instant::now();
                    assert_eq!(&repo.read_content(&id1).unwrap(), new);
                    let read_ms = ms(t);
                    let mid = (new.len() / 2) as u64;
                    let t = Instant::now();
                    let (got, range_work) = repo.read_range(&id1, mid, 4096).unwrap();
                    let range_ms = ms(t);
                    assert_eq!(&got[..], &new[mid as usize..mid as usize + 4096]);
                    assert_eq!(&repo.read_content(&id0).unwrap(), old);
                    assert!(repo.fsck().unwrap().is_empty());
                    // After a repack: what is kept, and range reads then.
                    let edits = [
                        Edit::Put {
                            path: "old".into(),
                            id: id0,
                        },
                        Edit::Put {
                            path: "new".into(),
                            id: id1,
                        },
                    ];
                    let tree = repo.edit_tree(None, &edits).unwrap();
                    repo.commit("main", tree, "both versions").unwrap();
                    let t = Instant::now();
                    repo.repack().unwrap();
                    let repack_ms = ms(t);
                    let repacked = stored(&repo);
                    let (got, packed_work) = repo.read_range(&id1, mid, 4096).unwrap();
                    assert_eq!(&got[..], &new[mid as usize..mid as usize + 4096]);
                    assert_eq!(&repo.read_content(&id0).unwrap(), old);
                    assert!(repo.fsck().unwrap().is_empty());
                    let packed = (repack_ms, repacked, packed_work);
                    runs.push((
                        edit_ms, put_ms, at_rest, written, read_ms, range_ms, range_work, packed,
                    ));
                }
                runs.sort_by(|a, b| a.0.total_cmp(&b.0));
                let (edit_ms, put_ms, at_rest, written, read_ms, range_ms, range_work, packed) =
                    runs[1];
                println!(
                    "{{\"rows\":{n},\"format\":\"{fmt}\",\"form\":\"{form}\",\"bytes\":{},\"at_rest\":{at_rest},\"put_ms\":{put_ms:.2},\"edit_written\":{written},\"edit_ms\":{edit_ms:.2},\"read_ms\":{read_ms:.2},\"range_4k_ms\":{range_ms:.3},\"range_4k_work\":{range_work},\"repack_ms\":{:.1},\"repacked\":{},\"repacked_range_4k_work\":{}}}",
                    old.len(),
                    packed.0,
                    packed.1,
                    packed.2
                );
            }
        }
    }
}
