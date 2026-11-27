//! E417: `merge::merge_lines` on generated edits, against `git merge-file`.
//!
//! Each case is a base text of generated lines and two sides, each with a
//! few random edits (a line changed, deleted, or new lines inserted). The
//! case is merged by `merge_lines` and by `git merge-file -p` (Myers, git's
//! default), and the outcomes are counted: both clean with the same bytes,
//! both clean with different bytes, clean here only, clean in git only,
//! refused by both. Checks laws on every case too: a side merged with the
//! base is that side, a side merged with itself is that side. Then the time
//! to merge one large file with edits far apart.
//!
//! `cargo run --release --example line_merge -- WORK [CASES]` (WORK: a
//! scratch folder for git's three files).

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use ikam_kernel::merge::merge_lines;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn line(r: &mut Rng) -> String {
    // Few distinct lines, so repeats make the alignment ambiguous at times.
    format!("line {} of {}\n", r.below(40), r.below(4))
}

fn edit(r: &mut Rng, base: &[String]) -> Vec<String> {
    let mut out = base.to_vec();
    for _ in 0..1 + r.below(4) {
        let at = r.below(out.len() + 1);
        match r.below(3) {
            0 if at < out.len() => out[at] = format!("changed {}\n", r.next() % 1000),
            1 if at < out.len() => {
                out.remove(at);
            }
            _ => {
                for _ in 0..1 + r.below(2) {
                    out.insert(at, format!("new {}\n", r.next() % 1000));
                }
            }
        }
    }
    out
}

fn git(work: &Path, b: &[u8], o: &[u8], t: &[u8]) -> Option<Vec<u8>> {
    let (pb, po, pt) = (work.join("base"), work.join("ours"), work.join("theirs"));
    std::fs::write(&pb, b).unwrap();
    std::fs::write(&po, o).unwrap();
    std::fs::write(&pt, t).unwrap();
    let out = Command::new("git")
        .args(["merge-file", "-p"])
        .args([&po, &pb, &pt])
        .output()
        .expect("git merge-file");
    match out.status.code() {
        Some(0) => Some(out.stdout),
        Some(n) if n > 0 => None,
        _ => panic!(
            "git merge-file failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let work = Path::new(args.get(1).expect("WORK"));
    std::fs::create_dir_all(work).unwrap();
    let cases: usize = args.get(2).map_or(5000, |n| n.parse().unwrap());
    let mut r = Rng(0x2545_f491_4f6c_dd1d);
    let (mut same, mut differ, mut ours_only, mut git_only, mut both_refuse) = (0, 0, 0, 0, 0);
    let mut example_ours_only = None;
    for _ in 0..cases {
        let n = 5 + r.below(40);
        let base: Vec<String> = (0..n).map(|_| line(&mut r)).collect();
        let (o, t) = (edit(&mut r, &base), edit(&mut r, &base));
        let (b, o, t) = (base.concat(), o.concat(), t.concat());
        let (b, o, t) = (b.as_bytes(), o.as_bytes(), t.as_bytes());
        for (x, y, want) in [(o, b, o), (b, t, t), (o, o, o)] {
            assert_eq!(merge_lines(b, x, y), Some(Ok(want.to_vec())));
        }
        match (merge_lines(b, o, t).unwrap(), git(work, b, o, t)) {
            (Ok(m), Some(g)) if m == g => same += 1,
            (Ok(_), Some(_)) => differ += 1,
            (Ok(_), None) => {
                ours_only += 1;
                // Keep the shortest, to print.
                if example_ours_only
                    .as_ref()
                    .is_none_or(|(x, _, _): &(Vec<u8>, Vec<u8>, Vec<u8>)| b.len() < x.len())
                {
                    example_ours_only = Some((b.to_vec(), o.to_vec(), t.to_vec()));
                }
            }
            (Err(_), Some(_)) => git_only += 1,
            (Err(_), None) => both_refuse += 1,
        }
    }
    println!("{cases} cases: clean in both with the same bytes {same}, different bytes {differ}");
    println!(
        "clean here, refused by git {ours_only}; refused here, clean in git {git_only}; refused by both {both_refuse}"
    );
    if let Some((b, o, t)) = example_ours_only {
        println!(
            "an example clean here only:\n--- base\n{}--- ours\n{}--- theirs\n{}",
            String::from_utf8_lossy(&b),
            String::from_utf8_lossy(&o),
            String::from_utf8_lossy(&t)
        );
    }
    // One large file, each side editing lines far from the other's.
    for lines in [1_000usize, 10_000, 100_000] {
        let base: Vec<String> = (0..lines)
            .map(|i| format!("row {i} text {}\n", i * 7 % 13))
            .collect();
        let (mut o, mut t) = (base.clone(), base.clone());
        for k in 0..10 {
            o[k * lines / 20] = format!("ours {k}\n");
            t[k * lines / 20 + lines / 40] = format!("theirs {k}\n");
        }
        let (b, o, t) = (base.concat(), o.concat(), t.concat());
        let mut times = Vec::new();
        for _ in 0..5 {
            let at = Instant::now();
            let m = merge_lines(b.as_bytes(), o.as_bytes(), t.as_bytes());
            times.push(at.elapsed().as_secs_f64() * 1e3);
            assert!(matches!(m, Some(Ok(_))));
        }
        times.sort_by(f64::total_cmp);
        let at = Instant::now();
        let g = git(work, b.as_bytes(), o.as_bytes(), t.as_bytes());
        let git_ms = at.elapsed().as_secs_f64() * 1e3;
        let m = merge_lines(b.as_bytes(), o.as_bytes(), t.as_bytes())
            .unwrap()
            .unwrap();
        println!(
            "{lines} lines ({} KB), 10 edits a side: median {:.2} ms (git merge-file, with process start, {git_ms:.1} ms; same bytes: {})",
            b.len() / 1024,
            times[2],
            g.as_deref() == Some(m.as_slice())
        );
    }
    for f in ["base", "ours", "theirs"] {
        let _ = std::fs::remove_file(work.join(f));
    }
}
