//! Price the reconstruction programs in `functions/programs/` (an offline
//! proposer step, like `semantic/embed.py`). Each program is priced in two
//! forms: interpreted (`interp.wasm` with the bytecode as argument) and
//! compiled (its own module, assembled here from `compile.py`'s WAT). Writes
//! `functions/proposals.txt`, one line per working form:
//! `<function file> <argument file or -> <output id> <output bytes> <decode work>`.
//!
//!   cargo run --release --example programs

use std::path::Path;

use ikam_kernel::{Arg, Id, MemStore, Repo};

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("functions");
    let interp = std::fs::read(dir.join("interp.wasm")).unwrap();
    let mut programs: Vec<_> = std::fs::read_dir(dir.join("programs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "bin"))
        .collect();
    programs.sort();
    let mut lines = vec![
        "# function argument output-id output-bytes decode-work (paths under functions/)".into(),
    ];
    // Each form is priced in a fresh store, so no function or argument is
    // stored as a derivation of an earlier one (whose read would add work)
    // and the result does not depend on file order (E027).
    let mut price = |func: &[u8], arg: Option<Vec<u8>>, label: String| {
        let mut repo = Repo::new(MemStore::default());
        let func = repo.put_content(func).unwrap().id;
        let args: Vec<Arg> = arg
            .map(|a| Arg::Whole(repo.put_content(&a).unwrap().id))
            .into_iter()
            .collect();
        match repo.evaluate(&func, &args) {
            Ok((out, work)) => {
                println!("{label}: {} B, work {work}", out.len());
                lines.push(format!(
                    "{label} {} {} {work}",
                    Id::of_content(&out),
                    out.len()
                ));
            }
            Err(e) => println!("{label}: {e}"),
        }
    };
    for path in programs {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let program = std::fs::read(&path).unwrap();
        price(
            &interp,
            Some(program),
            format!("interp.wasm programs/{name}"),
        );
        let wat = path.with_extension("wat");
        if let Ok(text) = std::fs::read_to_string(&wat) {
            let module = wat::parse_str(&text).unwrap();
            let wasm = path.with_extension("wasm");
            std::fs::write(&wasm, &module).unwrap();
            let file = wasm.file_name().unwrap().to_string_lossy();
            println!("programs/{file}: module {} B", module.len());
            price(&module, None, format!("programs/{file} -"));
        }
    }
    std::fs::write(dir.join("proposals.txt"), lines.join("\n") + "\n").unwrap();
}
