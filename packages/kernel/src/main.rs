//! `ikam` CLI: put files through a codec, render them back, inspect trees.

use std::io::Write;
use std::process::ExitCode;

use ikam_kernel::{Error, FsStore, Id, Object, Store, codec, ingest, render};

const USAGE: &str = "usage: ikam [--store DIR] <command>
  put [--codec raw|cdc|md] FILE   ingest FILE, print its root id
  cat ID                          write the reconstructed bytes to stdout
  tree ID                         print the object tree";

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
    let mut store = FsStore::open(store_dir).map_err(err)?;
    match args.first().map(String::as_str) {
        Some("put") => {
            let codec_name = take_flag(args, "--codec").unwrap_or_else(|| "md".into());
            let codec = codec::by_name(&codec_name).ok_or(format!("unknown codec {codec_name}"))?;
            let path = args.get(1).ok_or(USAGE)?;
            let input = std::fs::read(path).map_err(err)?;
            let r = ingest(&mut store, codec.as_ref(), &input).map_err(err)?;
            println!("{}", r.root);
            eprintln!(
                "codec={} fell_back={} input_bytes={} new_objects={} new_bytes={}",
                codec.name(),
                r.fell_back,
                input.len(),
                r.objects_written,
                r.bytes_written
            );
            Ok(())
        }
        Some("cat") => {
            let bytes = render(&store, &parse_id(args)?).map_err(err)?;
            std::io::stdout().write_all(&bytes).map_err(err)
        }
        Some("tree") => print_tree(&store, &parse_id(args)?, "", 0).map_err(err),
        _ => Err(USAGE.into()),
    }
}

fn print_tree(store: &dyn Store, id: &Id, label: &str, depth: usize) -> Result<(), Error> {
    let pad = "  ".repeat(depth);
    let short = &id.to_hex()[..12];
    match store.get(id)? {
        Object::Blob(bytes) => println!("{pad}{short} blob {}B {label}", bytes.len()),
        Object::Node(entries) => {
            println!("{pad}{short} node {} {label}", entries.len());
            for e in entries {
                print_tree(store, &e.id, &e.label, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn parse_id(args: &[String]) -> Result<Id, String> {
    args.get(1).ok_or(USAGE)?.parse().map_err(err)
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
