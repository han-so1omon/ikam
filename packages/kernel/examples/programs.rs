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
    let mut lines = vec![
        "# function argument output-id output-bytes decode-work (paths under functions/)".into(),
    ];
    let mut price = |repo: &Repo<MemStore>, func: &Id, args: &[Arg], label: String| match repo
        .evaluate(func, args)
    {
        Ok((out, work)) => {
            println!("{label}: {} B, work {work}", out.len());
            lines.push(format!(
                "{label} {} {} {work}",
                Id::of_content(&out),
                out.len()
            ));
        }
        Err(e) => println!("{label}: {e}"),
    };
    for path in programs {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let program = repo.put_content(&std::fs::read(&path).unwrap()).unwrap().id;
        price(
            &repo,
            &interp,
            &[Arg::Whole(program)],
            format!("interp.wasm programs/{name}"),
        );
        let wat = path.with_extension("wat");
        if let Ok(text) = std::fs::read_to_string(&wat) {
            let module = wat::parse_str(&text).unwrap();
            let wasm = path.with_extension("wasm");
            std::fs::write(&wasm, &module).unwrap();
            let func = repo.put_content(&module).unwrap().id;
            let file = wasm.file_name().unwrap().to_string_lossy();
            println!("programs/{file}: module {} B", module.len());
            price(&repo, &func, &[], format!("programs/{file} -"));
        }
    }
    std::fs::write(dir.join("proposals.txt"), lines.join("\n") + "\n").unwrap();
}
