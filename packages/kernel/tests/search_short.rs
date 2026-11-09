//! E132: patterns of one or two bytes are answered from the trigram index,
//! with exactly the results of a scan.

use ikam_kernel::{MemStore, Repo};

fn check(contents: &[Vec<u8>], patterns: &[Vec<u8>]) {
    let mut repo = Repo::new(MemStore::default());
    let files: Vec<_> = contents
        .iter()
        .map(|c| repo.put_content(c).unwrap().id)
        .collect();
    repo.build_search_index(&files).unwrap();
    let index = repo.search_index().unwrap().unwrap();
    for p in patterns {
        let (mut scan, _) = repo.search(p, &files, None).unwrap();
        let (mut indexed, _) = repo.search(p, &files, Some(&index)).unwrap();
        scan.sort();
        indexed.sort();
        assert_eq!(scan, indexed, "pattern {:?}", String::from_utf8_lossy(p));
    }
}

#[test]
fn short_patterns_match_a_scan() {
    let contents: Vec<Vec<u8>> = [
        &b"Q3"[..],
        b"x",
        b"",
        b"the Q3 report",
        b"nothing here",
        b"Q",
        b"aQ3",
        b"Q3a",
        b"q3 lower case",
    ]
    .iter()
    .map(|c| c.to_vec())
    .collect();
    let patterns: Vec<Vec<u8>> = [&b"Q3"[..], b"Q", b"3", b"x", b"zz", b"a", b"q3", b" "]
        .iter()
        .map(|p| p.to_vec())
        .collect();
    check(&contents, &patterns);
}

#[test]
fn every_short_pattern_over_a_small_alphabet() {
    let alphabet = b"abQ3";
    let mut x = 7u64;
    let mut next = || {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (x >> 33) as usize
    };
    let contents: Vec<Vec<u8>> = (0..60)
        .map(|_| {
            let n = next() % 8;
            (0..n).map(|_| alphabet[next() % 4]).collect()
        })
        .collect();
    let mut patterns: Vec<Vec<u8>> = alphabet.iter().map(|&a| vec![a]).collect();
    for &a in alphabet {
        for &b in alphabet {
            patterns.push(vec![a, b]);
        }
    }
    check(&contents, &patterns);
}
