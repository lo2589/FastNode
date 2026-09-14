"""把 .codedendrite/anchors.json + relations.json 一键导入 FastNode。

模型：
  每个符号 → type="code" 的 Node（派生类型 CodeNode，src/types/code.rs）
  每条原始边 → type="relationship" 的 Node（RelationshipNode，边元数据全保留），
               再加 rel_from/rel_to 两条结构 link 指向两端符号；
  图遍历用的去重图边（call/dataflow/impact）照常写入。

一条命令完成：生成 jsonl → import → 生成 link 操作 → rpc → stats 校验。

用法：
    python3 examples/anchors_to_fastnode.py                 # 导入到 data/codedendrite.db
    python3 examples/anchors_to_fastnode.py --force         # 库已存在时删除重建
    python3 examples/anchors_to_fastnode.py --db data/x.db --bin target/release/fastnode
"""

import argparse
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BATCH = 500
EDGE_META = [
    "condition", "data_in", "data_out", "outer_hash", "dispatch_key",
    "dispatch_value", "is_conditional", "scope_label", "seq", "call_line",
    "call_column", "resolution", "evidence", "bridge_kind",
]


def write_nodes(anchors_path: pathlib.Path, out: pathlib.Path) -> list[str]:
    """每个符号写一行 type="code" 的 Node，返回有序 anchor id 列表。"""
    anchors = json.loads(anchors_path.read_text())
    ids = list(anchors)  # dict 保持插入序，import 后 id = first_id + 序号
    with out.open("w") as f:
        for a in anchors.values():
            node = {
                "type": "code",
                "summary": f'{a.get("qualified_name", a["symbol"])} {a.get("signature", "")} ({a["file"]}:{a.get("line_start", 0)})',
                "attrs": {
                    "anchor": a["id"],
                    "symbol": a["symbol"],
                    "qualified_name": a.get("qualified_name", a["symbol"]),
                    "symbol_type": a.get("symbol_type", "unknown"),
                    "file": a["file"],
                    "line_start": a.get("line_start", 0),
                    "line_end": a.get("line_end", 0),
                    "hash": a.get("hash", ""),
                    "imports": a.get("imports", []),
                    "callers": a.get("callers", []),
                    "inputs": a.get("inputs", []),
                },
            }
            f.write(json.dumps(node, ensure_ascii=False) + "\n")
    return ids


def write_edge_nodes(relations_path: pathlib.Path, out: pathlib.Path) -> list[dict]:
    """每条原始边（不去重）写一行 type="relationship" 的 Node，元数据全保留。"""
    edges = json.loads(relations_path.read_text())["edges"]
    with out.open("w") as f:
        for e in edges:
            node = {
                "type": "relationship",
                "summary": f'{e["from_id"]} -[{e["edge_type"]}]-> {e["to_id"]} @{e.get("call_line")}',
                "attrs": {
                    "edge_type": e["edge_type"],
                    "from_anchor": e["from_id"],
                    "to_anchor": e["to_id"],
                    **{k: e[k] for k in EDGE_META if e.get(k) is not None},
                },
            }
            f.write(json.dumps(node, ensure_ascii=False) + "\n")
    return edges


def run_import(binary: str, db: pathlib.Path, jsonl: pathlib.Path) -> int:
    """调 fastnode import，返回 first_id 作为 id 映射基准。"""
    r = subprocess.run(
        [binary, str(db), "import", str(jsonl)],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        sys.exit(f"import 失败：{r.stderr.strip()}")
    report = json.loads(r.stdout)
    print(f"import: {report['imported']} nodes, first_id={report['first_id']} <- {jsonl.name}")
    return report["first_id"]


def write_links(
    edges: list[dict], ids: list[str], first_id: int, edge_first_id: int, out: pathlib.Path
) -> tuple[int, int]:
    """图边（call/dataflow/impact，按三元组去重）+ 每条边的 rel_from/rel_to。"""
    ordinal = {aid: first_id + i for i, aid in enumerate(ids)}
    seen, ops, skipped = set(), [], 0
    for i, e in enumerate(edges):
        if e["from_id"] not in ordinal or e["to_id"] not in ordinal:
            skipped += 1
            continue
        edge_node = edge_first_id + i
        ops.append({"op": "link", "from": edge_node, "relation": "rel_from", "to": ordinal[e["from_id"]]})
        ops.append({"op": "link", "from": edge_node, "relation": "rel_to", "to": ordinal[e["to_id"]]})
        key = (e["from_id"], e["to_id"], e["edge_type"])
        if key in seen:
            continue
        seen.add(key)
        ops.append({
            "op": "link",
            "from": ordinal[e["from_id"]],
            "relation": e["edge_type"],
            "to": ordinal[e["to_id"]],
        })
    with out.open("w") as f:
        for i in range(0, len(ops), BATCH):
            f.write(json.dumps({"op": "batch", "ops": ops[i : i + BATCH]}) + "\n")
    return len(ops), skipped


def run_rpc(binary: str, db: pathlib.Path, links_jsonl: pathlib.Path) -> None:
    """把 batch link 操作喂给 fastnode rpc，逐行校验 ok。"""
    with links_jsonl.open() as stdin:
        r = subprocess.run(
            [binary, str(db), "rpc"],
            stdin=stdin, capture_output=True, text=True,
        )
    if r.returncode != 0:
        sys.exit(f"rpc 失败：{r.stderr.strip()}")
    errors = [json.loads(line)["error"] for line in r.stdout.splitlines() if not json.loads(line).get("ok")]
    if errors:
        sys.exit(f"rpc 有 {len(errors)} 个 batch 失败，首个错误：{errors[0]}")


def main() -> None:
    p = argparse.ArgumentParser(description="codedendrite anchors → FastNode 一键导入")
    p.add_argument("--db", type=pathlib.Path, default=ROOT / "data" / "codedendrite.db")
    p.add_argument("--bin", default=str(ROOT / "target" / "release" / "fastnode"))
    p.add_argument("--anchors", type=pathlib.Path, default=ROOT / ".codedendrite" / "anchors.json")
    p.add_argument("--relations", type=pathlib.Path, default=ROOT / ".codedendrite" / "relations.json")
    p.add_argument("--force", action="store_true", help="库已存在时删除重建")
    args = p.parse_args()

    if not pathlib.Path(args.bin).exists():
        sys.exit(f"找不到 {args.bin}，先执行 cargo build --release --locked")
    if args.db.exists():
        if not args.force:
            sys.exit(f"{args.db} 已存在，加 --force 删除重建")
        for suffix in ("", "-wal", "-shm"):
            pathlib.Path(str(args.db) + suffix).unlink(missing_ok=True)

    nodes_jsonl = args.db.with_suffix(".nodes.jsonl")
    edges_jsonl = args.db.with_suffix(".edges.jsonl")
    links_jsonl = args.db.with_suffix(".links.jsonl")

    ids = write_nodes(args.anchors, nodes_jsonl)
    first_id = run_import(args.bin, args.db, nodes_jsonl)
    edges = write_edge_nodes(args.relations, edges_jsonl)
    edge_first_id = run_import(args.bin, args.db, edges_jsonl)
    linked, skipped = write_links(edges, ids, first_id, edge_first_id, links_jsonl)
    run_rpc(args.bin, args.db, links_jsonl)
    print(f"links: {linked} 写入（{skipped} 条悬空边只保留 relationship 节点）")

    r = subprocess.run([args.bin, str(args.db), "stats"], capture_output=True, text=True)
    stats = json.loads(r.stdout)
    assert stats["nodes"] == len(ids) + len(edges), f"nodes 数不符：{stats['nodes']} != {len(ids) + len(edges)}"
    assert stats["links"] == linked, f"links 数不符：{stats['links']} != {linked}"
    print(f"校验通过：nodes={stats['nodes']}（code {len(ids)} + relationship {len(edges)}）, links={stats['links']} -> {args.db}")


if __name__ == "__main__":
    main()
