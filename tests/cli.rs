use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fastnode"))
        .args(args)
        .output()
        .unwrap()
}

fn json_of(args: &[&str]) -> Value {
    let output = run(args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn rpc_preserves_large_numbers_and_recovers_after_invalid_request() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fastnode"))
        .args([":memory:", "rpc"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in [
        r#"{"op":"create","node":{"type":"place","summary":"上海","attrs":{"n":-18446744073709551615,"city":"上海"}}}"#,
        r#"{"op":"query","query":{"predicate":{"op":"eq","field":"/n","value":-18446744073709551615},"include_data":true}}"#,
        r#"{"op":"batch","ops":[{"op":"patch","id":1,"patch":{"attrs":{"city":"北京"}}},{"op":"link","from":1,"relation":"bad","to":999}]}"#,
        r#"{"op":"get","id":1,"links":"none"}"#,
        r#"{"op":"query","query":{"predicate":{"op":"eq","field":"/city","value":"上海","typo":true}}}"#,
        r#"{"op":"create","node":{"type":"place","attrs":{}}}"#,
        r#"{"op":"stats"}"#,
    ] {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(lines.len(), 7);
    assert_eq!(lines[0]["result"]["id"], 1);
    assert_eq!(lines[1]["result"]["total"], 1);
    let node = &lines[1]["result"]["nodes"][0];
    assert_eq!(node["attrs"]["n"].to_string(), "-18446744073709551615");
    assert_eq!(
        (&node["type"], &node["summary"]),
        (&json!("place"), &json!("上海"))
    );
    assert_eq!(lines[2]["ok"], false);
    assert_eq!(lines[3]["result"]["attrs"]["city"], "上海");
    assert_eq!(lines[3]["result"]["links"]["out"], json!([]));
    assert_eq!(lines[4]["ok"], false);
    assert_eq!(lines[5]["ok"], false);
    assert!(lines[5]["error"].as_str().unwrap().contains("summary"));
    assert_eq!(lines[6]["result"]["nodes"], 1);
    assert_eq!(lines[6]["result"]["links"], 0);
}

#[test]
fn cli_import_find_order_view_and_errors() {
    let path = std::env::temp_dir().join(format!("fastnode-cli-{}.db", std::process::id()));
    let db = path.to_str().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_fastnode"))
        .args([db, "import", "-", "2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let records = r#"[{"type":"person","summary":"x1","attrs":{"jobs":[{"company":"A"}],"age":26}},
        {"type":"person","summary":"x2","attrs":{"tags":["26"],"age":19,"company":"A"}},
        {"type":"tree","summary":"A 公司","attrs":{"name":"A"}}]"#;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(records.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        (report["imported"].clone(), report["batches"].clone()),
        (json!(3), json!(2))
    );
    assert!(run(&[db, "link", "1", "parent", "3"]).status.success());

    let found = json_of(&[db, "find", "A"]);
    assert_eq!(found["ids"], json!([1, 2, 3]));
    assert_eq!(found["nodes"][0]["links"]["out"][0]["summary"], "A 公司");
    assert_eq!(json_of(&[db, "find", "26"])["ids"], json!([1]));
    assert_eq!(json_of(&[db, "find", "\"26\""])["ids"], json!([2]));

    let by_age = json_of(&[db, "find", "A", "--order", "/age", "--limit", "1"]);
    assert_eq!(by_age["ids"], json!([2]));
    let cursor = by_age["next_cursor"].as_str().unwrap();
    let rest = json_of(&[db, "find", "A", "--order", "/age", "--cursor", cursor]);
    assert_eq!(rest["ids"], json!([1, 3]));
    let desc = json_of(&[db, "find", "A", "--order", "/age", "--direction", "desc"]);
    assert_eq!(desc["ids"], json!([1, 2, 3]));

    let got = json_of(&[db, "get", "3", "--links", "full"]);
    assert_eq!(got["links"]["in"][0]["attrs"]["age"], 26);
    let view = json_of(&[db, "view", "tree", "3"]);
    assert_eq!(
        (view["name"].clone(), view["children"][0]["id"].clone()),
        (json!("A"), json!(1))
    );

    let output = run(&[db, "create", "42"]);
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["ok"],
        json!(false)
    );
    assert!(!run(&[db, "get", "1", "--links", "all"]).status.success());
    assert!(!run(&[db, "view", "event", "3"]).status.success());
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{db}{suffix}"));
    }
}
