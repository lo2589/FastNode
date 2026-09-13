"""Exercise the Rust CLI with one million JSONL documents and unique IDs.

Usage: python3 examples/jsonl_million.py [count=1000000]
Creates fresh data/jsonl-<count>.db and data/nodes-<count>.jsonl.
"""
import argparse
import json
import math
import pathlib
import resource
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("count", type=int, nargs="?", default=1_000_000)
parser.add_argument("--database", type=pathlib.Path)
parser.add_argument("--input", type=pathlib.Path)
parser.add_argument("--report", type=pathlib.Path)
args = parser.parse_args()
COUNT = args.count
BIN = ROOT / "target/release/fastnode"
DB = ROOT / (args.database or pathlib.Path(f"data/jsonl-{COUNT}.db"))
INPUT = ROOT / (args.input or pathlib.Path(f"data/nodes-{COUNT}.jsonl"))
OUTPUT = ROOT / (args.report or pathlib.Path(f"reports/jsonl-{COUNT}.json"))
assert COUNT > 0
assert not DB.exists(), f"Choose a fresh database: {DB} already exists"
assert not INPUT.exists(), f"Choose a fresh input path: {INPUT} already exists"
DB.parent.mkdir(parents=True, exist_ok=True)
INPUT.parent.mkdir(parents=True, exist_ok=True)
OUTPUT.parent.mkdir(parents=True, exist_ok=True)


def document(i):
    city = ["上海", "北京", "深圳", "杭州"][i // 7 % 4]
    return {
        "type": "person" if i % 5 == 0 else "event",
        "summary": f"node-{i:07}，{city}",
        "attrs": {
            "external_id": f"node-{i:07}",
            "person": f"p{i % 2500}",
            "city": city,
            "year": 2020 + i % 7,
            "age": 18 + i % 63,
            "skills": ["python", "cuda" if i % 3 == 0 else "rust"],
            "jobs": [{"company": f"c{i % 1000}"}],
        },
    }


def stored(node):
    return {"type": node["type"], "summary": node["summary"], "attrs": node["attrs"]}


started = time.perf_counter()
with INPUT.open("w", encoding="utf-8") as file:
    for i in range(COUNT):
        file.write(json.dumps(document(i), ensure_ascii=False, separators=(",", ":")) + "\n")
generation_seconds = time.perf_counter() - started
print(f"JSONL generated: {COUNT} objects in {generation_seconds:.2f}s", flush=True)
started = time.perf_counter()
result = subprocess.run([str(BIN), str(DB), "import", str(INPUT), "5000"], capture_output=True, text=True, check=True)
import_seconds = time.perf_counter() - started
import_peak_rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
# macOS reports bytes, Linux reports KiB.
if sys.platform != "darwin":
    import_peak_rss *= 1024
import_result = json.loads(result.stdout)
assert import_result["imported"] == COUNT
print(f"JSONL imported: {COUNT} objects in {import_seconds:.2f}s", flush=True)

rpc = subprocess.Popen([str(BIN), str(DB), "rpc"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding="utf-8")


def call(request):
    rpc.stdin.write(json.dumps(request, ensure_ascii=False) + "\n")
    rpc.stdin.flush()
    response = json.loads(rpc.stdout.readline())
    assert response["ok"], response
    return response["result"]


def eq(field, value):
    return {"op": "eq", "field": field, "value": value}


middle = COUNT // 2
key = f"node-{middle:07}"
request = {"op": "query", "query": {"predicate": eq("/external_id", key), "include_data": True}}
assert stored(call(request)["nodes"][0]) == document(middle)
for _ in range(3):
    call(request)
times = []
for _ in range(50):
    started = time.perf_counter()
    response = call(request)
    times.append((time.perf_counter() - started) * 1000)
    assert response["ids"] == [middle + 1] and response["total"] == 1

# Change an existing singleton posting and a numeric index in the large store.
call({"op": "patch", "id": middle + 1, "patch": {"attrs": {"external_id": "renamed", "age": 99}}})
assert call(request)["total"] == 0
changed = call({"op": "query", "query": {"predicate": {"op": "and", "args": [eq("/external_id", "renamed"), {"op": "range", "field": "/age", "gte": 99, "lte": 99}]}}})
assert changed["ids"] == [middle + 1]
call({"op": "delete", "id": middle + 1})
assert call({"op": "query", "query": {"predicate": eq("/external_id", "renamed")}})["total"] == 0
restored = call({"op": "create", "node": document(middle)})
assert restored["id"] > COUNT
stats = call({"op": "stats"})
assert stats["nodes"] == COUNT
assert call(request)["ids"] == [restored["id"]]
rpc.stdin.close()
assert rpc.wait() == 0
times.sort()
report = {
    "nodes": COUNT,
    "distinct_external_ids": COUNT,
    "input_bytes": INPUT.stat().st_size,
    "database_bytes": DB.stat().st_size,
    "generation_seconds": generation_seconds,
    "import_seconds": import_seconds,
    "nodes_per_second": COUNT / import_seconds,
    "import_peak_rss_bytes": import_peak_rss,
    "import_measurement": "Rust CLI process lifetime: open database + stream/parse JSONL + update all indexes + commit batches + clean shutdown; excludes input-file generation",
    "query_measurement": "50 sequential Python-to-Rust JSON RPC round trips after 3 warmups; includes JSON serialization, IPC, full payload (type, summary, attrs, links=summary) and JSON response parsing; warm cache",
    "unique_lookup_p50_ms": times[math.ceil(len(times) * 0.5) - 1],
    "unique_lookup_p95_ms": times[math.ceil(len(times) * 0.95) - 1],
    "stats": stats,
    "correctness": "Full JSON round-trip verified; unique lookup checked in every sample; existing Node renamed, numeric index updated, old posting removed, deleted, recreated under fresh ID and counted.",
}
OUTPUT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(json.dumps(report, ensure_ascii=False, indent=2), flush=True)
