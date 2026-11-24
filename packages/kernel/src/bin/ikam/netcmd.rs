//! `ikam net`: run any Petri net from a net file on a store (E401).
//!
//! A loop is configuration, not a built-in: its net is a text file in the
//! `Net::parse` format, its steps are whatever program fires the net's
//! transitions, and its whole history is the net's firings on a ref. The
//! self-improvement loop (`experiments/self-loop`) is one such net; the
//! night net (E386) could be another. Every call opens the store, does one
//! thing and exits:
//!
//!   ikam [--store DIR] net NETFILE [--ref REF] fire T TOKEN [OUTFILE]
//!   ikam [--store DIR] net NETFILE [--ref REF] history
//!   ikam [--store DIR] net NETFILE [--ref REF] marking
//!   ikam [--store DIR] net NETFILE [--ref REF] enabled
//!   ikam [--store DIR] net NETFILE [--ref REF] check
//!
//! REF defaults to `net/<name>`. `fire` fires T for TOKEN (`-` for none)
//! under the key `T-TOKEN`, so a repeated call with the same output changes
//! nothing and one with other output is refused (E271); OUTFILE's bytes are
//! the firing's output. A human transition is never fired here: it is the
//! owner's, done outside the program (for a loop, a merge). `history` prints
//! every firing in order as a JSON line with its output; `marking` the
//! tokens per place as JSON; `enabled` what a scheduler may fire now;
//! `check` reads the store alone: replay equal to the marking, every output
//! readable, no human firing, fsck clean.

use std::time::{SystemTime, UNIX_EPOCH};

use ikam_kernel::net::{Kind, Net};
use ikam_kernel::{Db, FsStore, Repo};

use crate::corpus::err;
use crate::json::quote;

pub fn net_cmd(store_dir: &str, args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    let refname = crate::take_flag(&mut args, "--ref");
    let usage = "usage: ikam net NETFILE [--ref REF] fire T TOKEN [OUTFILE] | history | marking | enabled | check";
    let (file, cmd) = match (args.first(), args.get(1)) {
        (Some(f), Some(c)) => (f.clone(), c.clone()),
        _ => return Err(usage.into()),
    };
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?;
    let net = Net::parse(&text).map_err(|e| format!("{file}: {e:?}"))?;
    let refname = refname.unwrap_or_else(|| format!("net/{}", net.name));
    std::fs::create_dir_all(store_dir).map_err(err)?;
    let mut repo = Repo::new(FsStore::open(store_dir).map_err(err)?);
    let db = match repo.resolve(&refname) {
        Ok(c) => repo.db_at(&c).map_err(err)?,
        Err(_) => {
            let db = repo.db_empty().map_err(err)?;
            repo.net_init_sum(&db, &net).map_err(err)?
        }
    };
    match cmd.as_str() {
        "fire" => {
            let (t, tok) = match (args.get(2), args.get(3)) {
                (Some(t), Some(k)) => (t.clone(), k.clone()),
                _ => return Err(usage.into()),
            };
            let tr = net
                .transitions
                .iter()
                .find(|x| x.name == t)
                .ok_or_else(|| format!("no transition {t} in {}", net.name))?;
            if tr.kind == Kind::Human {
                return Err(format!(
                    "{t} is a human transition: ikam net never fires it"
                ));
            }
            let token = (tok != "-").then_some(tok.as_str());
            let key = format!("{t}-{tok}");
            let out = args.get(4).map(std::fs::read).transpose().map_err(err)?;
            let (db, _) = match &out {
                Some(b) => repo.net_fire_output(&db, &net, &t, token, &key, b),
                None => repo.net_fire(&db, &net, &t, token, &key),
            }
            .map_err(|e| format!("{key}: {e:?}"))?;
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let c = repo
                .db_commit(&refname, &db, &format!("{key} at {at}"))
                .map_err(err)?;
            println!("{key} -> {}", &c.to_string()[..12]);
        }
        "history" => {
            for f in repo.net_firings(&db, &net.name).map_err(err)? {
                let out = match repo.net_output(&db, &net.name, &f.key).map_err(err)? {
                    Some((_, b)) => quote(&String::from_utf8_lossy(&b)),
                    None => "null".into(),
                };
                println!(
                    "{{\"key\":{},\"transition\":{},\"token\":{},\"output\":{out}}}",
                    quote(&f.key),
                    quote(&f.transition),
                    quote(f.token.as_deref().unwrap_or(""))
                );
            }
        }
        "marking" => println!("{}", marking_json(&repo, &db, &net)?),
        "enabled" => {
            for (t, tok) in repo.net_enabled(&db, &net, true).map_err(err)? {
                println!("{t} {}", tok.as_deref().unwrap_or("-"));
            }
        }
        "check" => {
            let m = repo.net_marking(&db, &net.name).map_err(err)?;
            if repo.net_replay(&db, &net).map_err(err)? != m {
                return Err("replay differs from the marking".into());
            }
            let firings = repo.net_firings(&db, &net.name).map_err(err)?;
            for f in &firings {
                let human = net
                    .transitions
                    .iter()
                    .any(|x| x.name == f.transition && x.kind == Kind::Human);
                if human {
                    return Err(format!("{}: a human transition was fired", f.key));
                }
                if f.output.is_some()
                    && repo
                        .net_output(&db, &net.name, &f.key)
                        .map_err(err)?
                        .is_none()
                {
                    return Err(format!("{}: output unreadable", f.key));
                }
            }
            let bad = repo.fsck().map_err(err)?;
            if !bad.is_empty() {
                return Err(format!("fsck: {} objects fail", bad.len()));
            }
            println!(
                "{refname}: {} firings, replay equal, outputs readable, fsck clean; {}",
                firings.len(),
                marking_json(&repo, &db, &net)?
            );
        }
        _ => return Err(usage.into()),
    }
    Ok(())
}

fn marking_json(repo: &Repo<FsStore>, db: &Db, net: &Net) -> Result<String, String> {
    let m = repo.net_marking(db, &net.name).map_err(err)?;
    let places: Vec<String> = net
        .places
        .iter()
        .map(|p| {
            let n: u64 = m.get(&p.name).map_or(0, |t| t.values().sum());
            format!("{}:{n}", quote(&p.name))
        })
        .collect();
    Ok(format!("{{{}}}", places.join(",")))
}
