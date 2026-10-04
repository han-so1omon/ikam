//! `ikam` CLI over a filesystem store.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use ikam_kernel::{Arg, FsStore, Id, Object, Repo, snapshot};

const USAGE: &str = "usage: ikam [--store DIR] <command>
  put FILE                         store a file, print its content id
  cat ID                           write a file's bytes to stdout
  commit DIR [-m MSG] [--ref REF]  snapshot DIR and advance REF (default main)
  log [REV]                        first-parent history of REV (default main)
  checkout REV DEST                write REV's tree into new directory DEST
  show ID                          print a tree, commit or derivation record
  apply FUNC ARG...                run function FUNC (a stored WASM module id) on
                                   stored contents, print the output id (memoized)
  links ID                         outgoing graph edges of an object or content
  used-by ID                       incoming graph edges (objects that link to ID)
  fsck                             reconstruct and verify every stored object
  gc                               delete objects unreachable from any ref
  claim SUBJ PRED OBJ [--by ID]    record a semantic claim; its weight (bits the
                                   object saves on the subject) is measured
  claims ID                        claims about ID, with measured gains
  relate ID [K]                    the K stored contents most informative about ID
  promote CLAIM                    derive the claim's subject from its object, if smaller
  repack                           re-plan storage of all live content; applied
                                   only if smaller and fully verified";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let store_dir = take_flag(&mut args, "--store").unwrap_or_else(|| ".ikam".into());
    match run(&store_dir, &mut args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(store_dir: &str, args: &mut Vec<String>) -> Result<(), String> {
    let mut repo = Repo::new(FsStore::open(store_dir).map_err(err)?);
    let message = take_flag(args, "-m").unwrap_or_default();
    let refname = take_flag(args, "--ref").unwrap_or_else(|| "main".into());
    let by = take_flag(args, "--by");
    let arg = |i: usize| args.get(i).cloned().ok_or_else(|| USAGE.to_string());
    match args.first().map(String::as_str) {
        Some("put") => {
            let input = std::fs::read(arg(1)?).map_err(err)?;
            let put = repo.put_content(&input).map_err(err)?;
            println!("{}", put.id);
            eprintln!(
                "form={:?} input_bytes={} new_bytes={}",
                put.form,
                input.len(),
                repo.bytes_written
            );
        }
        Some("cat") => {
            let bytes = repo
                .read_content(&repo.resolve(&arg(1)?).map_err(err)?)
                .map_err(err)?;
            std::io::stdout().write_all(&bytes).map_err(err)?;
        }
        Some("commit") => {
            let tree = snapshot::snapshot(&mut repo, Path::new(&arg(1)?), Path::new(store_dir))
                .map_err(err)?;
            let id = repo.commit(&refname, tree, &message).map_err(err)?;
            println!("{id}");
            eprintln!("new_bytes={}", repo.bytes_written);
        }
        Some("log") => {
            let start = repo
                .resolve(args.get(1).map_or("main", String::as_str))
                .map_err(err)?;
            for (id, c) in repo.log(start).map_err(err)? {
                println!("{} tree={} {}", id, &c.tree.to_hex()[..12], c.message);
            }
        }
        Some("checkout") => {
            let tree =
                snapshot::tree_of(&repo, &repo.resolve(&arg(1)?).map_err(err)?).map_err(err)?;
            snapshot::checkout(&repo, &tree, Path::new(&arg(2)?)).map_err(err)?;
        }
        Some("show") => match repo
            .get(&repo.resolve(&arg(1)?).map_err(err)?)
            .map_err(err)?
        {
            Object::Tree(entries) => entries
                .iter()
                .for_each(|e| println!("{:?} {} {}", e.kind, e.id, e.name)),
            Object::Commit(c) => {
                println!("tree {}\nparents {:?}\n\n{}", c.tree, c.parents, c.message)
            }
            Object::Derivation(d) => {
                println!("output {}\nfunc {}\nargs {:?}", d.output, d.func, d.args)
            }
            _ => unreachable!(),
        },
        Some("apply") => {
            let func = repo.resolve(&arg(1)?).map_err(err)?;
            let args = args[2..]
                .iter()
                .map(|a| a.parse::<Id>().map(Arg::Whole))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            println!("{}", repo.apply(func, args).map_err(err)?);
        }
        Some("links") => {
            for (label, id) in repo
                .links(&repo.resolve(&arg(1)?).map_err(err)?)
                .map_err(err)?
            {
                println!("{id} {label}");
            }
        }
        Some("used-by") => {
            for (id, label) in repo
                .used_by(&repo.resolve(&arg(1)?).map_err(err)?)
                .map_err(err)?
            {
                println!("{id} {label}");
            }
        }
        Some("fsck") => {
            let failures = repo.fsck().map_err(err)?;
            for (id, e) in &failures {
                println!("{id} {e}");
            }
            if !failures.is_empty() {
                return Err(format!("{} objects failed verification", failures.len()));
            }
            println!("ok");
        }
        Some("repack") => {
            let r = repo.repack().map_err(err)?;
            println!(
                "before={} after={} applied={}",
                r.before, r.after, r.applied
            );
        }
        Some("claim") => {
            let by = by.map(|b| repo.resolve(&b)).transpose().map_err(err)?;
            let (s, o) = (
                repo.resolve(&arg(1)?).map_err(err)?,
                repo.resolve(&arg(3)?).map_err(err)?,
            );
            let id = repo
                .claim(Arg::Whole(s), &arg(2)?, Arg::Whole(o), by)
                .map_err(err)?;
            println!("{id}");
        }
        Some("claims") => {
            for (id, c) in repo
                .claims(&repo.resolve(&arg(1)?).map_err(err)?)
                .map_err(err)?
            {
                println!(
                    "{id} {} {} {} gain_bits={}",
                    c.subject.id(),
                    c.predicate,
                    c.object.id(),
                    c.gain_bits
                );
            }
        }
        Some("relate") => {
            let k = args.get(2).map_or(Ok(5), |k| k.parse()).map_err(err)?;
            for (id, gain) in repo
                .relate(&repo.resolve(&arg(1)?).map_err(err)?, k)
                .map_err(err)?
            {
                println!("{id} gain_bits={gain}");
            }
        }
        Some("promote") => match repo
            .promote(&repo.resolve(&arg(1)?).map_err(err)?)
            .map_err(err)?
        {
            Some(put) => println!("{} now derived", put.id),
            None => println!("no smaller derivation found"),
        },
        Some("gc") => println!("deleted {} objects", repo.gc().map_err(err)?),
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    let value = args.get(i + 1).cloned();
    args.drain(i..(i + 2).min(args.len()));
    value
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
