mod args;
mod dispatch;
mod mutation;

use anyhow::{Result, ensure};
use fastnode::Store;
use serde_json::{Value, json};
use std::{
    env,
    fs::File,
    io::{self, BufRead, Read, Write},
};

const HELP: &str = r#"FastNode — transactional JSON nodes + bitmap queries

Usage: fastnode <database> <command> [arguments]

Node JSON: {"type":"person","summary":"one sentence","attrs":{...}}

  create <node|@file|->
  create-many <node-array|@file|->
  get <id> [--links none|summary|full] [--link-limit 100]
  find <value> [--links …] [--limit 100] [--after 0] [--order <field>] [--direction asc|desc] [--cursor c]
  view <tree|event|state> <id> [--links …] [--link-limit 100]
  replace <id> <node|@file|->
  patch <id> <merge-patch over type/summary/attrs|@file|->
  delete <id>
  link <from> <relation> <to>
  unlink <from> <relation> <to>
  links <id>
  query <query-JSON|@file|->
  batch <operations-array|@file|->
  import <file|-> [batch-size=5000]
  stats
  rpc  (one JSON request/response per line)

Query example: {"predicate":{"op":"overlap","field":"/time","start":0,"end":100},"order_by":{"field":"/time/start"},"include_data":true}
"#;

fn main() {
    if let Err(error) = run() {
        eprintln!("{}", json!({"ok":false,"error":format!("{error:#}")}));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        println!("{HELP}");
        return Ok(());
    }
    ensure!(args.len() >= 2, "expected database and command; see --help");
    let mut store = Store::open(&args[0])?;
    let result = match args[1].as_str() {
        "rpc" => return rpc(&mut store),
        "import" => import(&mut store, &args[2..])?,
        command => dispatch::dispatch(&mut store, args::request(command, &args[2..])?)?,
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn rpc(store: &mut Store) -> Result<()> {
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let result = serde_json::from_str(&line)
            .map_err(anyhow::Error::from)
            .and_then(|request| dispatch::dispatch(store, request));
        let output = match result {
            Ok(result) => json!({"ok":true,"result":result}),
            Err(error) => json!({"ok":false,"error":format!("{error:#}")}),
        };
        writeln!(stdout, "{output}")?;
        stdout.flush()?;
    }
    Ok(())
}

fn import(store: &mut Store, args: &[String]) -> Result<Value> {
    let path = args.first().map(String::as_str).unwrap_or("-");
    let batch = args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(5000);
    let reader: Box<dyn Read> = if path == "-" {
        Box::new(io::stdin())
    } else {
        Box::new(File::open(path)?)
    };
    Ok(serde_json::to_value(store.import(reader, batch)?)?)
}
