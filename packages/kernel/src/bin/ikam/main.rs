//! `ikam` CLI over a filesystem store.

mod ask;
mod auth;
mod corpus;
mod dav;
mod http;
mod json;
mod live;
mod serve;
mod text;
mod ui;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ikam_kernel::{Arg, FsStore, Id, Object, Repo, snapshot};

use corpus::{err, lookup};

const USAGE: &str = "usage: ikam [--store DIR] [--short-records] [--slow-codec-max BYTES] [--needed-index] <command>
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
                                   only if smaller and fully verified
  live [PORT] [--watch DIR[:PREFIX]]... [--llm] [--commit-after MS]
       [--allow-host NAME]... [--log-requests] [--public]
       [--max-body BYTES] [--max-store BYTES] [--max-conns N] [--gc-every N]
                                   run REF (default main) as a live filesystem:
                                   WebDAV at http://127.0.0.1:PORT/dav/ (mount it
                                   in Finder: Go > Connect to Server), a web page
                                   at /, and each --watch folder mirrored into the
                                   tree as it changes. Edits commit after MS of
                                   quiet (default 1500). --llm lets questions be
                                   answered by Claude (needs ANTHROPIC_API_KEY).
                                   --allow-host also answers to NAME (such as the
                                   machine's Tailscale name behind Tailscale
                                   Serve); it still listens on 127.0.0.1 only. With
                                   IKAM_AUTH set (from passwd), every request needs
                                   that user and password. SIGTERM or Ctrl-C
                                   commits pending edits, then exits.
                                   --log-requests prints one line per request
                                   (time, method, path, status, ms, client).
                                   For a copy open to anyone: --max-body refuses
                                   larger uploads (413), --max-store refuses new
                                   content once the store reaches BYTES (507),
                                   --max-conns serves N connections at once (503
                                   past that); BYTES takes K, M or G. --public
                                   (needs IKAM_AUTH) lets anyone without the
                                   password read (browse, search, download, mount
                                   read-only) but not write, edit, delete, run
                                   repack, fsck or commit, or ask Claude; it sets
                                   those limits to 25M, 1G and 32 unless given.
                                   --gc-every deletes what no commit needs (file
                                   versions and trees written between commits)
                                   after every N commits; POST /api/gc does it
                                   once. Each walks the whole store while edits
                                   wait, so it is off by default.
  passwd USER                      read a password from stdin, print the
                                   IKAM_AUTH line for it
  write PATH [FILE|-]              store FILE (or stdin) at PATH in REF, commit
  rm PATH | mkdir PATH             remove a file or folder / make a folder in REF
  mv FROM TO | cp FROM TO          rename or copy a file or folder in REF
  search TEXT                      files of REF whose text contains TEXT (also the
                                   text inside docx, xlsx, pdf and tar members)
  ask QUESTION [--llm]             the passages of REF that answer QUESTION best;
                                   with --llm, an answer from them by Claude";

fn main() -> ExitCode {
    quiet_on_closed_stdout();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let store_dir = take_flag(&mut args, "--store").unwrap_or_else(|| ".ikam".into());
    if args.first().map(String::as_str) == Some("passwd") {
        return match passwd(args.get(1).map(String::as_str)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        };
    }
    match run(&store_dir, &mut args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

/// Print `IKAM_AUTH=...` for USER and the password on stdin's first line.
/// Needs no store, so it opens none.
fn passwd(user: Option<&str>) -> Result<(), String> {
    let user = user.ok_or("usage: ikam passwd USER (password on stdin)")?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(err)?;
    let password = line.trim_end_matches(['\n', '\r']);
    if password.is_empty() {
        return Err("empty password".into());
    }
    println!("IKAM_AUTH={}", auth::Auth::line(user, password)?);
    Ok(())
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
    if let Some(i) = args.iter().position(|a| a == "--needed-index") {
        args.remove(i);
        repo.needed_index = true;
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
        Some("live") => {
            let needed_index = repo.needed_index;
            drop(repo);
            live_cmd(store_dir, &refname, &args[1..], needed_index)?;
        }
        Some(cmd @ ("write" | "rm" | "mkdir" | "mv" | "cp")) => {
            let op = match cmd {
                "write" => {
                    let mut bytes = Vec::new();
                    match args.get(2).map(String::as_str) {
                        None | Some("-") => {
                            std::io::stdin().read_to_end(&mut bytes).map_err(err)?;
                        }
                        Some(f) => bytes = std::fs::read(f).map_err(err)?,
                    }
                    live::Op::Put(arg(1)?, bytes)
                }
                "rm" => live::Op::Remove(arg(1)?),
                "mkdir" => live::Op::Mkdir(arg(1)?),
                "mv" => live::Op::Rename(arg(1)?, arg(2)?),
                _ => live::Op::Copy(arg(1)?, arg(2)?),
            };
            drop(repo);
            let mut l = live::Live::open(store_dir, &refname)?;
            if cmd == "rm" && l.stat(&live::norm(&arg(1)?))?.is_none() {
                return Err(format!("{}: no such file or folder", arg(1)?));
            }
            let t = Instant::now();
            let cost = l.apply(vec![op])?;
            let commit = l.flush(true)?;
            match commit {
                Some(c) => println!("{c}"),
                None => println!("no change"),
            }
            eprintln!(
                "new_bytes={} store_ms={:.1} tree_ms={:.1} total_ms={:.1}",
                cost.new_bytes,
                cost.store_ms,
                cost.tree_ms,
                t.elapsed().as_secs_f64() * 1e3
            );
        }
        Some("search") => {
            let q = arg(1)?;
            drop(repo);
            let mut l = live::Live::open(store_dir, &refname)?;
            l.index_all(false)?;
            let t = Instant::now();
            let (hits, total) = text::search(&l.docs, &q, usize::MAX);
            for h in hits {
                if h.part.is_empty() {
                    println!("{}", h.path);
                } else {
                    println!("{} ({})", h.path, h.part);
                }
                for (n, line) in h.lines {
                    println!("  {n}: {line}");
                }
            }
            eprintln!("{total} file(s), {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        }
        Some("ask") => {
            let llm = args.iter().any(|a| a == "--llm");
            let q = args[1..]
                .iter()
                .filter(|a| *a != "--llm")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            if q.is_empty() {
                return Err(USAGE.into());
            }
            drop(repo);
            let mut l = live::Live::open(store_dir, &refname)?;
            l.index_all(false)?;
            let passages = text::retrieve(&l.docs, &q, 8);
            if llm {
                println!("{}\n", ask::answer(&q, &passages)?);
            }
            for (i, p) in passages.iter().enumerate() {
                println!("{}", ask::source_label(i + 1, p));
                if !llm {
                    for line in p.text.lines().take(6) {
                        println!("    {line}");
                    }
                }
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

/// `ikam live`: serve the ref as a filesystem until stopped.
fn live_cmd(
    store_dir: &str,
    refname: &str,
    args: &[String],
    needed_index: bool,
) -> Result<(), String> {
    let mut args = args.to_vec();
    let mut watches = Vec::new();
    while let Some(w) = take_flag(&mut args, "--watch") {
        let (dir, prefix) = match w.rsplit_once(':') {
            Some((d, p)) if !d.is_empty() && Path::new(d).is_dir() => {
                (d.to_string(), live::norm(p))
            }
            _ => (w.clone(), String::new()),
        };
        if !Path::new(&dir).is_dir() {
            return Err(format!("--watch {dir}: not a folder"));
        }
        watches.push((PathBuf::from(dir), prefix));
    }
    let commit_after = take_flag(&mut args, "--commit-after")
        .map(|m| {
            m.parse::<u64>()
                .map_err(|_| "--commit-after MS".to_string())
        })
        .transpose()?;
    let mut allowed = Vec::new();
    while let Some(name) = take_flag(&mut args, "--allow-host") {
        if name.is_empty() || name.contains(['/', ' ']) {
            return Err(format!("--allow-host {name:?}: expected a host name"));
        }
        allowed.push(name);
    }
    let before = args.len();
    args.retain(|a| a != "--log-requests");
    let log_requests = args.len() != before;
    let byte_flag = |args: &mut Vec<String>, flag: &str| {
        take_flag(args, flag)
            .map(|v| parse_bytes(&v).ok_or_else(|| format!("{flag} BYTES (such as 25M)")))
            .transpose()
    };
    let max_body = byte_flag(&mut args, "--max-body")?;
    let max_store = byte_flag(&mut args, "--max-store")?;
    let max_conns = take_flag(&mut args, "--max-conns")
        .map(|n| match n.parse::<usize>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err("--max-conns N (at least 1)".to_string()),
        })
        .transpose()?;
    let gc_every = take_flag(&mut args, "--gc-every")
        .map(|n| match n.parse::<usize>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err("--gc-every N (at least 1)".to_string()),
        })
        .transpose()?;
    let before = args.len();
    args.retain(|a| a != "--public");
    let public = args.len() != before;
    let auth = auth::Auth::from_env()?;
    let llm = match args.iter().position(|a| a == "--llm") {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
    if public && auth.is_none() {
        return Err("--public lets guests read; writes need IKAM_AUTH, which is not set".into());
    }
    let limits = http::Limits {
        max_body: max_body
            .or(public.then_some(PUBLIC_MAX_BODY))
            .map(|n| n as usize),
        max_conns: max_conns.or(public.then_some(PUBLIC_MAX_CONNS)),
        idle: if public {
            Duration::from_secs(30)
        } else {
            http::Limits::default().idle
        },
    };
    let port: u16 = args.first().map_or(Ok(8737), |p| p.parse()).map_err(err)?;
    let mut l = live::Live::open(store_dir, refname)?;
    l.repo.needed_index = needed_index;
    l.max_store = max_store.or(public.then_some(PUBLIC_MAX_STORE));
    l.gc_every = gc_every;
    if let Some(ms) = commit_after {
        l.commit_after = Duration::from_millis(ms);
    }
    l.llm = llm;
    if llm && !ask::key_available() {
        eprintln!("--llm: ANTHROPIC_API_KEY is not set; questions will show sources only");
    }
    auth::catch_stop_signals();
    l.index_all(true)?;
    let live = Arc::new(Mutex::new(l));
    {
        let live = live.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                if auth::stopping() {
                    // Holding the lock, so no edit or repack is half done;
                    // commit what is pending and leave.
                    let mut l = live.lock().unwrap();
                    let code = match l.flush(true) {
                        Ok(_) => 0,
                        Err(e) => {
                            eprintln!("{} commit on stop failed: {e}", live::stamp());
                            1
                        }
                    };
                    eprintln!("{} stopped", live::stamp());
                    std::process::exit(code);
                }
                if let Err(e) = live.lock().unwrap().flush(false) {
                    eprintln!("{} commit failed: {e}", live::stamp());
                }
            }
        });
    }
    for (dir, prefix) in watches {
        eprintln!(
            "{} watching {} into /{prefix}",
            live::stamp(),
            dir.display()
        );
        let live = live.clone();
        std::thread::spawn(move || live::watch(live, dir, prefix, Duration::from_millis(500)));
    }
    let hosts = auth::Hosts::new(port, allowed.clone());
    let password = auth.is_some();
    let route = move |req: &mut http::Request| {
        // A page on another site can point a name it controls at
        // 127.0.0.1 (DNS rebinding); its requests carry that name.
        if !req.header("host").is_some_and(|h| hosts.ok(h)) {
            return http::Response::text(403, "unknown Host");
        }
        if req.path == "/robots.txt" && matches!(req.method.as_str(), "GET" | "HEAD") {
            return robots(public);
        }
        if let Some(refusal) = auth.as_ref().and_then(|a| a.check(req)) {
            // With --public, a guest (no password sent) may read; Claude
            // answers are dropped from what it asks.
            let guest = public && req.header("authorization").is_none() && !writes(req);
            if !guest {
                return refusal;
            }
            req.query = without_param(&req.query, "llm");
        }
        if req.path == dav::PREFIX || req.path.starts_with("/dav/") {
            dav::handle(&mut live.lock().unwrap(), req)
        } else {
            ui::handle(&live, req)
        }
    };
    let handler = Arc::new(move |req: http::Request| {
        let started = Instant::now();
        let line = log_requests.then(|| request_line(&req));
        let mut req = req;
        let resp = route(&mut req);
        if let Some(line) = line {
            eprintln!(
                "{} {line} -> {} {:.0}ms",
                live::stamp(),
                resp.status,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        resp
    });
    let t = live::stamp();
    eprintln!(
        "{t} live on http://127.0.0.1:{port}/ (WebDAV: http://127.0.0.1:{port}/dav/), ref {refname}"
    );
    for name in &allowed {
        eprintln!("{t} also answering to {name}");
    }
    if password {
        eprintln!("{t} password required (IKAM_AUTH)");
    }
    if log_requests {
        eprintln!("{t} logging each request");
    }
    if public {
        eprintln!(
            "{t} public: guests may read; writes, admin and Claude answers need the password"
        );
    }
    let show = |n: Option<u64>| n.map_or("no limit".to_string(), |n| format!("{n} bytes"));
    if limits.max_body.is_some() || limits.max_conns.is_some() || max_store.is_some() || public {
        eprintln!(
            "{t} limits: uploads {}, store {}, connections {}",
            show(limits.max_body.map(|n| n as u64)),
            show(max_store.or(public.then_some(PUBLIC_MAX_STORE))),
            limits
                .max_conns
                .map_or("no limit".to_string(), |n| n.to_string())
        );
    }
    http::serve(port, handler, limits)
}

/// `/robots.txt`, answered before the password check: crawlers and web
/// readers that honour it may read a `--public` copy, and are asked to keep
/// out of any other (which needs the password anyway). Many treat a missing
/// or refused robots.txt as "keep out".
fn robots(public: bool) -> http::Response {
    let rules = if public { "Allow: /" } else { "Disallow: /" };
    http::Response::text(200, format!("User-agent: *\n{rules}\n"))
}

/// Whether a request can change anything: every WebDAV method but the
/// reading ones, and anything but GET or HEAD elsewhere (the API's POSTs).
fn writes(req: &http::Request) -> bool {
    !matches!(req.method.as_str(), "GET" | "HEAD" | "OPTIONS")
        && !(req.method == "PROPFIND" && (req.path == dav::PREFIX || req.path.starts_with("/dav/")))
}

/// `query` without `key` (and its value).
fn without_param(query: &str, key: &str) -> String {
    query
        .split('&')
        .filter(|kv| kv.split_once('=').map_or(*kv, |(k, _)| k) != key)
        .collect::<Vec<_>>()
        .join("&")
}

/// `--public` limits unless given: 25 MB per upload, 1 GB stored, 32
/// connections.
const PUBLIC_MAX_BODY: u64 = 25 << 20;
const PUBLIC_MAX_STORE: u64 = 1 << 30;
const PUBLIC_MAX_CONNS: usize = 32;

/// `25M`, `1G`, `512K` or plain bytes (powers of 1024).
fn parse_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, shift) = match s.char_indices().last()? {
        (i, 'K' | 'k') => (&s[..i], 10),
        (i, 'M' | 'm') => (&s[..i], 20),
        (i, 'G' | 'g') => (&s[..i], 30),
        _ => (s, 0),
    };
    let n: u64 = num.parse().ok()?;
    n.checked_mul(1u64 << shift)
}

/// What `--log-requests` shows of a request: method, path (no query, which
/// may hold a search), the Host it named, the client's first User-Agent word,
/// and whether it carried a password (never the password).
fn request_line(req: &http::Request) -> String {
    let client = req
        .header("user-agent")
        .and_then(|u| u.split_whitespace().next())
        .unwrap_or("-");
    format!(
        "{} {} host={} [{client}{}]",
        req.method,
        req.path,
        req.header("host").unwrap_or("-"),
        req.header("authorization").map_or(", no password", |_| "")
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_rules() {
        assert_eq!(robots(true).body, b"User-agent: *\nAllow: /\n");
        assert_eq!(robots(false).body, b"User-agent: *\nDisallow: /\n");
        assert_eq!(robots(true).status, 200);
    }

    #[test]
    fn guest_requests() {
        let req = |method: &str, path: &str| http::Request {
            method: method.into(),
            path: path.into(),
            query: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
        };
        for (m, p) in [
            ("GET", "/dav/a.txt"),
            ("HEAD", "/dav/a.txt"),
            ("PROPFIND", "/dav/"),
            ("OPTIONS", "/dav/"),
            ("GET", "/api/search"),
            ("GET", "/ask"),
        ] {
            assert!(!writes(&req(m, p)), "{m} {p}");
        }
        for (m, p) in [
            ("PUT", "/dav/a.txt"),
            ("DELETE", "/dav/a.txt"),
            ("MKCOL", "/dav/d"),
            ("MOVE", "/dav/a.txt"),
            ("COPY", "/dav/a.txt"),
            ("LOCK", "/dav/a.txt"),
            ("UNLOCK", "/dav/a.txt"),
            ("PROPPATCH", "/dav/a.txt"),
            ("POST", "/api/repack"),
            ("POST", "/api/commit"),
            ("POST", "/api/fsck"),
            ("PROPFIND", "/api/status"),
        ] {
            assert!(writes(&req(m, p)), "{m} {p}");
        }
        assert_eq!(without_param("q=x&llm=1", "llm"), "q=x");
        assert_eq!(without_param("llm&q=x", "llm"), "q=x");
        assert_eq!(without_param("q=llm", "llm"), "q=llm");
    }

    #[test]
    fn byte_sizes() {
        assert_eq!(parse_bytes("25M"), Some(25 << 20));
        assert_eq!(parse_bytes("1G"), Some(1 << 30));
        assert_eq!(parse_bytes("512k"), Some(512 << 10));
        assert_eq!(parse_bytes("1000"), Some(1000));
        assert_eq!(parse_bytes("M"), None);
        assert_eq!(parse_bytes("-1M"), None);
        assert_eq!(parse_bytes("99999999999G"), None);
    }

    #[test]
    fn request_lines_never_show_the_password() {
        let req = |auth: Option<&str>| http::Request {
            method: "PROPFIND".into(),
            path: "/dav/a%20b".into(),
            query: "q=secret+search".into(),
            headers: [
                Some(("Host".to_string(), "mac.ts.net".to_string())),
                Some((
                    "User-Agent".to_string(),
                    "WebDAVFS/3.0.0 (03008000) Darwin/25.0.0".to_string(),
                )),
                auth.map(|a| ("Authorization".to_string(), a.to_string())),
            ]
            .into_iter()
            .flatten()
            .collect(),
            body: Vec::new(),
        };
        assert_eq!(
            request_line(&req(Some("Basic ZXJpYzpodW50ZXIy"))),
            "PROPFIND /dav/a%20b host=mac.ts.net [WebDAVFS/3.0.0]"
        );
        assert_eq!(
            request_line(&req(None)),
            "PROPFIND /dav/a%20b host=mac.ts.net [WebDAVFS/3.0.0, no password]"
        );
    }
}
