//! E241: column blocks give back their rows byte for byte.

use ikam_kernel::{MemStore, Repo, colblock};

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

const ODD: &[&str] = &[
    "",
    "0",
    "-0",
    "007",
    "+5",
    "1e5",
    "1.50",
    "-0.00",
    "-0.50",
    "0.5",
    "1.",
    ".5",
    "9223372036854775807",
    "-9223372036854775808",
    "99999999999999999999",
    "1.2.3",
    "naïve",
    "a\"b",
    " 12",
    "12 ",
    "\t",
    "x,y",
];

fn round_trip(rows: &[Vec<u8>], sep: u8) {
    let refs: Vec<&[u8]> = rows.iter().map(Vec::as_slice).collect();
    let block = colblock::encode(&refs, sep).unwrap();
    assert_eq!(colblock::decode(&block).unwrap(), rows);
}

#[test]
fn odd_values_round_trip() {
    let mut rng = Lcg(3);
    for trial in 0..300 {
        let n = (rng.next() % 60) as usize;
        let rows: Vec<Vec<u8>> = (0..n)
            .map(|_| {
                let fields = 1 + rng.next() % 5;
                let parts: Vec<String> = (0..fields)
                    .map(|_| match rng.next() % 4 {
                        0 => ODD[rng.next() as usize % ODD.len()].to_string(),
                        1 => (rng.next() as i64 - (1 << 30)).to_string(),
                        2 => format!("{}.{:02}", rng.next() % 1000, rng.next() % 100),
                        _ => ["ok", "warn", "idle"][rng.next() as usize % 3].to_string(),
                    })
                    .collect();
                parts.join(",").into_bytes()
            })
            .collect();
        round_trip(&rows, b',');
        if trial % 10 == 0 {
            round_trip(&rows, b';');
        }
    }
    round_trip(&[], b',');
    round_trip(&[b"".to_vec(), b",".to_vec(), b",,".to_vec()], b',');
    // Binary rows, and bytes that are not UTF-8.
    let bin: Vec<Vec<u8>> = (0..50u8).map(|i| vec![i, 0xff, b',', 0x80, i]).collect();
    round_trip(&bin, b',');
}

#[test]
fn typed_columns_round_trip_and_one_column_reads_alone() {
    let rows: Vec<Vec<u8>> = (0..5000)
        .map(|i| {
            format!(
                "{},{}.{:02},{}",
                1000 + i * 7,
                i % 37,
                i % 100,
                ["a", "b"][i % 2]
            )
            .into_bytes()
        })
        .collect();
    round_trip(&rows, b',');
    let refs: Vec<&[u8]> = rows.iter().map(Vec::as_slice).collect();
    let block = colblock::encode(&refs, b',').unwrap();
    let col = colblock::column(&block, 1).unwrap();
    assert_eq!(col.len(), 5000);
    assert_eq!(col[123], b"12.23");
    assert!(block.len() * 20 < rows.iter().map(|r| r.len() + 1).sum::<usize>());
}

#[test]
fn blocks_in_the_database_read_back_exactly() {
    let mut repo = Repo::new(MemStore::default());
    let rows: Vec<Vec<u8>> = (0..1000)
        .map(|i| format!("{i},{}", i * i).into_bytes())
        .collect();
    let refs: Vec<&[u8]> = rows.iter().map(Vec::as_slice).collect();
    let db = repo.db_empty().unwrap();
    let db = repo
        .db_append_block(&db, "t/blocks/0", &refs, b',', "b0")
        .unwrap();
    let value = repo.db_get(&db, "t/blocks/0").unwrap().unwrap();
    assert_eq!(colblock::decode(&value).unwrap(), rows);
    assert_eq!(repo.db_rebuild(&db.log).unwrap(), db);
    let id = repo.put_column_block(&refs, b',').unwrap();
    assert_eq!(
        colblock::decode(&repo.read_content(&id).unwrap()).unwrap(),
        rows
    );
}
