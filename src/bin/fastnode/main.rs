mod args;
mod dispatch;
mod mutation;
mod session;

use anyhow::{Result, ensure};
use fastnode::Store;
use serde_json::{Value, json};
use std::{
    env,
    fs::File,
    io::{self, BufRead, Read},
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
  rpc  (one JSON request/response per line; begin / commit / rollback wrap requests in one transaction)

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
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut stdout = io::stdout().lock();
    while let Some(line) = lines.next() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Result<Value> = serde_json::from_str(&line).map_err(Into::into);
        if let Ok(begin) = &request
            && begin.get("op").and_then(Value::as_str) == Some("begin")
        {
            session::reply(&mut stdout, Ok(json!({"began": true})))?;
            session::run(store, &mut lines, &mut stdout)?;
            continue;
        }
        session::reply(
            &mut stdout,
            request.and_then(|r| dispatch::dispatch(store, r)),
        )?;
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
