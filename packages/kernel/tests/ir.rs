//! E192: the IR codec returns every file byte-exact, for any bytes and any
//! options.

use ikam_kernel::ir::{Options, decode, encode, pack, unpack};
use proptest::prelude::*;

fn all_options() -> Vec<Options> {
    (0..16)
        .map(|b| Options {
            columns: b & 1 != 0,
            numbers: b & 2 != 0,
            siblings: b & 4 != 0,
            rows: b & 8 != 0,
        })
        .collect()
}

fn roundtrip(files: &[(String, Vec<u8>)]) {
    for o in all_options() {
        let parts = encode(files, o);
        let back = unpack(&pack(&parts)).unwrap();
        assert_eq!(back, parts);
        assert_eq!(decode(&parts).unwrap(), files, "{o:?}");
    }
}

#[test]
fn tables_reports_and_edge_cases_roundtrip() {
    let files: Vec<(String, Vec<u8>)> = [
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
        ("empty.txt", ""),
        ("nl.txt", "\n\n\n"),
        ("bin.json", "\"\u{0}\\"),
    ]
    .iter()
    .map(|(n, b)| (n.to_string(), b.as_bytes().to_vec()))
    .collect();
    roundtrip(&files);
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
