//! `ikam` CLI over a filesystem store.

mod corpus;
mod serve;

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use ikam_kernel::{Arg, FsStore, Id, Object, Repo, snapshot};

use corpus::{err, lookup};

const USAGE: &str = "usage: ikam [--store DIR] [--short-records] [--slow-codec-max BYTES] <command>
  ingest DIR [-m MSG] [--ref REF]  commit DIR, repack, verify; print what it cost
  ls [REV[:PATH]] [-r]             list a directory of REV (default main)
  grep TEXT [REV]                  files of REV containing TEXT, with lines
  stats [REV]                      objects and bytes the store holds
  serve [PORT]                     browse, read and search in a local web page
                                   (default port 8737, 127.0.0.1 only)
  REV, ID and PATH arguments take a ref, a hex id or unique prefix, or REV:PATH.
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
    quiet_on_closed_stdout();
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

/// `ikam ls -r | head` ends with stdout closed: exit quietly, as other
/// command-line tools do, instead of printing a panic.
fn quiet_on_closed_stdout() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .unwrap_or("");
        if msg.starts_with("failed printing to stdout") && msg.contains("Broken pipe") {
            std::process::exit(141);
        }
        default(info)
    }));
}

fn run(store_dir: &str, args: &mut Vec<String>) -> Result<(), String> {
    let mut repo = Repo::new(FsStore::open(store_dir).map_err(err)?);
    if let Some(i) = args.iter().position(|a| a == "--short-records") {
        args.remove(i);
        repo.short_records = true;
    }
    if let Some(n) = take_flag(args, "--slow-codec-max") {
        repo.slow_codec_max = n
            .parse()
            .map_err(|_| "--slow-codec-max BYTES".to_string())?;
    }
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
                .read_content(&lookup(&repo, &arg(1)?).map_err(err)?)
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
                snapshot::tree_of(&repo, &lookup(&repo, &arg(1)?).map_err(err)?).map_err(err)?;
            snapshot::checkout(&repo, &tree, Path::new(&arg(2)?)).map_err(err)?;
        }
        Some("show") => match repo
            .get(&lookup(&repo, &arg(1)?).map_err(err)?)
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
            let func = lookup(&repo, &arg(1)?).map_err(err)?;
            let args = args[2..]
                .iter()
                .map(|a| a.parse::<Id>().map(Arg::Whole))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            println!("{}", repo.apply(func, args).map_err(err)?);
        }
        Some("links") => {
            for (label, id) in repo
                .links(&lookup(&repo, &arg(1)?).map_err(err)?)
                .map_err(err)?
            {
                println!("{id} {label}");
            }
        }
        Some("used-by") => {
            for (id, label) in repo
                .used_by(&lookup(&repo, &arg(1)?).map_err(err)?)
                .map_err(err)?
            {
                println!("{id} {label}");
            }
        }
        Some("fsck") => {
            let failures = repo.fsck_deep().map_err(err)?;
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
            let by = by.map(|b| lookup(&repo, &b)).transpose().map_err(err)?;
            let (s, o) = (
                lookup(&repo, &arg(1)?).map_err(err)?,
                lookup(&repo, &arg(3)?).map_err(err)?,
            );
            let id = repo
                .claim(Arg::Whole(s), &arg(2)?, Arg::Whole(o), by)
                .map_err(err)?;
            println!("{id}");
        }
        Some("claims") => {
            for (id, c) in repo
                .claims(&lookup(&repo, &arg(1)?).map_err(err)?)
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
                .relate(&lookup(&repo, &arg(1)?).map_err(err)?, k)
                .map_err(err)?
            {
                println!("{id} gain_bits={gain}");
            }
        }
        Some("promote") => match repo
            .promote(&lookup(&repo, &arg(1)?).map_err(err)?)
            .map_err(err)?
        {
            Some(put) => println!("{} now derived", put.id),
            None => println!("no smaller derivation found"),
        },
        Some("ingest") => ingest(&mut repo, store_dir, &arg(1)?, &refname, &message)?,
        Some("ls") => {
            let recursive = args.iter().any(|a| a == "-r");
            let spec = args
                .get(1)
                .filter(|a| *a != "-r")
                .map_or("main", String::as_str);
            list(&repo, spec, recursive)?;
        }
        Some("grep") => {
            let root = corpus::tree(
                &repo,
                &lookup(&repo, args.get(2).map_or("main", String::as_str))?,
            )?;
            for hit in corpus::grep(&repo, &root, &arg(1)?).map_err(err)? {
                println!("{}", hit.path);
                for (n, line) in hit.lines {
                    println!("  {n}: {line}");
                }
            }
        }
        Some("stats") => {
            let rev = lookup(&repo, args.get(1).map_or("main", String::as_str)).ok();
            let s = corpus::stats(&repo, rev.as_ref()).map_err(err)?;
            println!(
                "files={} input_bytes={} stored_bytes={}",
                s.files, s.input, s.stored
            );
            for (k, (n, b)) in s.kinds {
                println!("{k:<22} {n:>8} objects {b:>12} bytes");
            }
        }
        Some("serve") => {
            let port = args.get(1).map_or(Ok(8737), |p| p.parse()).map_err(err)?;
            serve::serve(&repo, port)?;
        }
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

/// Commit `dir`, repack, check every object and that each file reads back
/// exactly, and print what it took.
fn ingest(
    repo: &mut Repo<FsStore>,
    store_dir: &str,
    dir: &str,
    refname: &str,
    message: &str,
) -> Result<(), String> {
    let t = Instant::now();
    let tree = snapshot::snapshot(repo, Path::new(dir), Path::new(store_dir)).map_err(err)?;
    let message = if message.is_empty() { dir } else { message };
    let commit = repo.commit(refname, tree, message).map_err(err)?;
    let ingest_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let r = repo.repack().map_err(err)?;
    let repack_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let failures = repo.fsck_deep().map_err(err)?;
    let mut mismatched = 0;
    for f in corpus::files(repo, &tree).map_err(err)? {
        let on_disk = std::fs::read(Path::new(dir).join(&f.path)).map_err(err)?;
        if repo.read_content(&f.id).map_err(err)? != on_disk {
            mismatched += 1;
        }
    }
    let verify_ms = t.elapsed().as_millis();
    let s = corpus::stats(repo, Some(&commit)).map_err(err)?;
    println!("{commit}");
    println!(
        "files={} input_bytes={} stored_bytes={} ratio={:.4} repack_applied={} ingest_ms={ingest_ms} repack_ms={repack_ms} verify_ms={verify_ms} fsck_failures={} mismatched={mismatched}",
        s.files,
        s.input,
        s.stored,
        s.stored as f64 / s.input.max(1) as f64,
        r.applied,
        failures.len()
    );
    if failures.is_empty() && mismatched == 0 {
        Ok(())
    } else {
        Err("verification failed".into())
    }
}

fn list(repo: &Repo<FsStore>, spec: &str, recursive: bool) -> Result<(), String> {
    let mut id = lookup(repo, spec)?;
    if let Ok(Object::Commit(c)) = repo.get(&id) {
        id = c.tree;
    }
    if recursive {
        for f in corpus::files(repo, &id).map_err(err)? {
            let len = repo.read_content(&f.id).map_err(err)?.len();
            println!("{len:>10} {}", f.path);
        }
        return Ok(());
    }
    let Object::Tree(entries) = repo.get(&id).map_err(err)? else {
        return Err(format!("{spec}: not a directory"));
    };
    for e in entries {
        match e.kind {
            ikam_kernel::Kind::File => {
                let len = repo.read_content(&e.id).map_err(err)?.len();
                println!("{len:>10} {}", e.name);
            }
            _ => println!("{:>10} {}/", "-", e.name),
        }
    }
    Ok(())
}
