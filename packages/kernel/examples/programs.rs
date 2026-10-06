//! Price the reconstruction programs in `functions/programs/` (an offline
//! proposer step, like `semantic/embed.py`): run each with the shared
//! interpreter and write `functions/proposals.txt`, one line per program:
//! `<program file> <output content id> <output bytes> <decode work>`.
//! The benchmark reads that file; no program runs there unless offered.
//!
//!   cargo run --release --example programs

use std::path::Path;

use ikam_kernel::{Arg, MemStore, Repo};

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("functions");
    let mut repo = Repo::new(MemStore::default());
    let interp = repo
        .put_content(&std::fs::read(dir.join("interp.wasm")).unwrap())
        .unwrap()
        .id;
    let mut programs: Vec<_> = std::fs::read_dir(dir.join("programs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "bin"))
        .collect();
    programs.sort();
    let mut lines =
        vec!["# program output-id output-bytes decode-work (functions/interp.wasm)".to_string()];
    for path in programs {
        let program = repo.put_content(&std::fs::read(&path).unwrap()).unwrap().id;
        let name = path.file_name().unwrap().to_string_lossy();
        match repo.evaluate(&interp, &[Arg::Whole(program)]) {
            Ok((out, work)) => {
                let id = ikam_kernel::Id::of_content(&out);
                println!("{name}: {} B, work {work}", out.len());
                lines.push(format!("{name} {id} {} {work}", out.len()));
            }
            Err(e) => println!("{name}: {e}"),
        }
    }
    std::fs::write(dir.join("proposals.txt"), lines.join("\n") + "\n").unwrap();
}
