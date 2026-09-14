# FastNode

把 JSON 存成 Node，自动给每个路径建索引。支持任意深度的属性查询、范围与排序、时间区间、关系遍历，拉取一个 Node 时顺带拉取它引用的 Node。

FastNode 是一个 Rust 嵌入式库，也提供命令行和常驻 JSON 接口。底层用 SQLite（WAL）存数据，用 Roaring 位图做集合运算。核心层只提供基础能力；树、事件、状态这类类型在核心层之上派生，底层存储不变。

## 性能摘要

百万条实测，Apple M5 Pro，Rust 库调用、热缓存。复现方法见「测试与复现」。

| 操作 | p50 |
|---|---:|
| 等值 / 通配 / any，只返回 ID | 0.004–0.005 ms |
| 区间 at / overlap / contained_by，只返回 ID | 0.004–0.007 ms |
| 排序首页或翻过 10 万条后的页，每页 100 条 | 0.04–0.17 ms |
| 拉取一个 Node 及引用（none / summary / full） | 0.014–0.018 ms |
| 同上加载 100 个 Node 的内容 | 约 0.8–1.1 ms |
| 单条新建 / 修改 / 删除（独立事务） | 0.15–0.23 ms |
| 导入 100 万 Node（带区间） | 29.6 s |
| 数据库文件（100 万 Node + 100 万关系 + 100 万区间） | 755 MiB |
| 单条时间线 100 万个版本中定位 t 时刻的版本（seek） | 0.011 ms |

## 安装

**前置要求：**

- Rust 工具链（1.98+，用了 edition 2024）：`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- SQLite 无需单独安装——rusqlite 以 bundled 模式编译进二进制（见 Cargo.toml）。
- Python 3：仅当需要用 `examples/anchors_to_fastnode.py` 导入 codedendrite 数据时。

**从源码构建：**

```bash
git clone <仓库地址> && cd Node
cargo build --release --locked      # 产物：target/release/fastnode
cargo test --locked                 # 验证：52 项测试应全部通过
```

**作为 Rust 库使用：** 见下文「Rust 接入」一节，以 path 或 git 依赖引入即可。

## 快速上手

```bash
cargo build --release --locked
cargo run --release --example demo     # 核心能力演示
cargo run --release --example types    # 派生类型演示：tree / event / state
```

```bash
B=./target/release/fastnode
mkdir -p data

# 导入 5 个示例 Node：小明、小红、两场活动、一个会场
$B data/demo.db import examples/people.json

# 组合查询：@type=person 且 city=上海 且 skills 含 rust 且 20 ≤ age < 30 → 小明
$B data/demo.db query @examples/query.json

# 关系：会场 contains 2026 活动，2026 活动 contains 小明
$B data/demo.db link 5 contains 4
$B data/demo.db link 4 contains 1
$B data/demo.db query @examples/traversal.json          # 小明 ← 2026 活动 ← 会场

# 任意字段含"星云科技"的 Node，按 age 倒序
$B data/demo.db find 星云科技 --order /age --direction desc

# 拉取 Node 4，并带上它引用的 Node 的完整内容
$B data/demo.db get 4 --links full

# 时间区间：记下一件事，再查中午 12 点在进行的事
$B data/demo.db create '{"type":"event","summary":"今天吃了饭","attrs":{"what":"吃饭","time":{"text":"今天","start":1789228800000,"end":1789315200000}}}'
$B data/demo.db query '{"predicate":{"op":"at","field":"/time","value":1789272000000},"order_by":{"field":"/time/start"},"include_data":true}'

$B data/demo.db patch 1 '{"attrs":{"city":"深圳"}}'
$B data/demo.db stats
```

上面的固定 ID 对应第一次导入到新数据库的情况。

## Node

```text
Node
├─ id        自动分配的 u32，删除后不复用
├─ type      必填
├─ summary   必填，一句话说明这是什么；不进属性索引
├─ attrs     任意 JSON 对象，每个路径自动建索引
└─ links     (from, relation, to)，relation 是任意字符串
```

写入格式：

```json
{"type":"person","summary":"小明，上海的 Rust 工程师","attrs":{"name":"小明","age":26,"skills":["rust"],"jobs":[{"company":"星云科技"}]}}
```

读出时会多出 `id`、`version`，按需还有 `links`。

## 核心索引

| 索引 | 存什么 | 回答什么 |
|---|---|---|
| ExactIndex | `(路径, 带类型的值) → 位图`；只有一个 Node 持有的数字不建位图 | `eq` `in` `any` `exists` `prefix` |
| OrderedIndex | 数字：`(路径, 精确排序键, id)`；字符串：位图按值排列 | `range`、`order_by` |
| IntervalIndex | `(路径, start, end, id)`，每个区间一行 | `at` `overlap` `contains` `contained_by` |
| LinkIndex | `(from, relation, to)`，正反两个方向 | `has_link`、`traverse`、拉取引用 |

每个查询条件的结果都是一个位图，可以任意组合 `and` / `or` / `not`。AND 会先算预计最小的那个条件，后面的条件只在剩下的候选里查。

## 属性：任意路径

字段用 JSON Pointer 书写，`@type` 表示 type。

| 数据 | 查询 |
|---|---|
| `{"city":"上海"}` | `/city = 上海` |
| `{"g":{"k":{"d":["f","g","b","e"]}}}` | `/g/k/d = b`（数组包含 b）；`/g/k/d/2 = b`（指定位置） |
| `{"jobs":[{"company":"A"},{"company":"B"}]}` | `/jobs/*/company = B` |
| 不指定字段 | `{"op":"any","value":"b"}` |

- `*` 表示任意数组下标；对象的键本身就叫 `*` 时写成 `~2`，`/` 和 `~` 按 JSON Pointer 写成 `~1`、`~0`。
- `1`、`"1"`、`true` 是三个不同的值。`1`、`1.0`、`1e0` 数值相等，排序键保留完整的十进制精度，不经过 f64。
- `null` 和字段缺失是两回事；`exists` 能查到值为空对象、空数组的字段。

## 查询协议

```json
{
  "predicate": {"op":"and","args":[
    {"op":"eq","field":"@type","value":"person"},
    {"op":"eq","field":"/jobs/*/company","value":"星云科技"},
    {"op":"range","field":"/age","gte":20,"lt":30}
  ]},
  "limit": 100,
  "include_data": true,
  "links": "summary",
  "link_limit": 100
}
```

| op | 参数 | 含义 |
|---|---|---|
| `all` | | 所有 Node |
| `ids` | `ids` | 指定的 id，已删除的自动去掉 |
| `eq` / `in` | `field,value` / `field,values` | 等值，或数组成员 |
| `any` | `value` | attrs 任意路径含这个标量 |
| `exists` | `field` | 字段存在 |
| `prefix` | `field,prefix` | 字符串前缀，区分大小写 |
| `range` | `field,gt/gte,lt/lte` | 数字或字符串范围，所有边界必须同一类型 |
| `at` | `field,value` | 区间包含时刻 value |
| `overlap` / `contains` / `contained_by` | `field,start,end` | 区间与 [start, end) 有交集 / 完全覆盖它 / 完全落在它里面 |
| `has_link` | `relation,target,direction` | 有这条关系指向 target（`in`：被 target 指向） |
| `traverse` | `from,steps` | 从一个集合出发沿关系展开 |
| `and` / `or` / `not` | `args` / `arg` | 集合运算 |

结果：`{"total":400,"ids":[…],"nodes":[…],"next_after":29}`。

- `total` 是完整匹配数。
- 不排序时按 id 升序，把 `next_after` 作为下一次的 `after` 就能翻页。
- 每页最多 100,000 条；`limit: 0` 只返回计数。
- `include_data: false` 时只读索引，不读 Node 内容。

## 排序

```json
{"predicate":{"op":"eq","field":"@type","value":"event"},
 "order_by":{"field":"/time/start","direction":"desc"},
 "limit":20, "cursor":"上一页返回的 next_cursor"}
```

- 先按值排，值相同再按 id 排，所以结果稳定；翻页用 `next_cursor`。
- 升序时数字排在字符串前面，降序时反过来。这个路径上没有值的 Node 永远排在最后。
- 如果某个 Node 在这个路径上有多个值（数组，或通配路径命中多个元素），排序会报错，要求改用具体路径。数据库不替你决定取 min、max 还是第一个。
- 执行时不读 JSON：
  - 候选不超过 2048 个：逐个取排序键，在内存里排好。
  - 数字：顺序扫有序行。
  - 字符串：按值的顺序扫位图。
  - 没有值的 Node：直接由写入时维护的位图算出。

## 时间区间

核心层不认识"时间"，只认识区间的形状：一个对象里有数字 `start`，就是区间 `[start, end)`。

| 写法 | 含义 |
|---|---|
| `{"start":10,"end":20}` | [10, 20) |
| `{"start":15,"end":15}` | 瞬间 15 |
| `{"start":30}` 或 `"end":null` | 没有结束 |
| `start > end`，或 `end` 不是数字 | 不建区间索引，里面的字段照常建普通索引 |

- **数组里的区间**：每个区间单独一行。`/jobs/*/time` 不会把一份工作的开始和另一份工作的结束凑成一对。
- **时间短语**："今天""一年前"换算成数字由写入方负责，原话放进 `text` 字段。建议单位统一用 UTC 毫秒。
- **模糊时间**：存成一个较宽的区间；先后顺序用 `next` 这类关系表达，不靠时间排序。
- **查询速度**：每个路径会记住写入过的最长有限区间长度 L，查时刻 t 时只扫开始时间落在 `(t − L, t]` 的行；没有结束时间的区间走一个单独的小索引。

## 关系、拉取与多跳

拉取一个 Node 时，引用信息分三档，出向和入向都会返回：

| `links` | 每条引用带什么 |
|---|---|
| `none` | relation、id |
| `summary`（默认） | 再加 type、summary |
| `full` | 再加 attrs |

- 每个方向默认最多 100 条（上限 10,000）；超出时 `out_more` / `in_more` 为 true。
- summary 只存一份，所有引用读到的都是当前值。
- 在 Rust 里，`Store::neighbors` 可以只读某一种关系的某一个方向。

```json
{"op":"traverse",
 "from":{"op":"eq","field":"/name","value":"小明"},
 "steps":[
   {"relation":"contains","direction":"in","filter":{"op":"eq","field":"/year","value":2026}},
   {"relation":"next","min":1,"max":10}
 ]}
```

- 不写 `min` / `max`：这一步走一跳。
- 写了 `min` / `max`：沿同一种关系重复走，保留最短跳数在 `min..=max` 之间的 Node。
  - `max` 不写：一直走到没有新的 Node。
  - `min: 0`：结果包含出发点。
  - 关系里有环也会停下来。
- `filter` 只筛选最终到达的 Node，不影响途中经过哪些 Node。

## 复合索引与 seek

```rust
db.define_index("timeline", "state", &["/timeline", "/recorded/end"], "/valid/start")?;
```

```json
{"op":"seek","index":"timeline","group":["12#condition",null],"lte":1789228800000,"direction":"desc","limit":1}
```

- 按 type 声明：分组路径（等值）加一个有序路径。定义时回填已有 Node，之后写入、修改、删除自动维护。
- 只有每个分组路径恰好一个标量、有序路径恰好一个数字或字符串的 Node 进入索引。
- `seek` 在一个分组内按有序 key 走 B 树：边界 `gt / gte / lt / lte`（同为数字或同为字符串）、方向、条数。结果可以再和其他条件组合，条数先于组合生效。
- 单条时间线 100 万个版本，定位 t 时刻的版本 p50 0.011 ms。

## 写入策略、事务与时间戳

| 能力 | 用法 |
|---|---|
| 只追加 | `db.set_policy("state", &WritePolicy::append_only(&["/recorded/end"]))`：该 type 禁止 delete / replace，patch 只能改列出的 attrs 路径；违反即报错，所在事务回滚 |
| 事务内读取 | `Write::query` / `get_with` / `neighbors`，看得到本事务未提交的写入 |
| 事务时间戳 | `Write::now()`：事务内固定，跨事务严格递增，系统时钟回拨也不倒退 |
| rpc 事务 | `{"op":"begin"}` → 任意读写 → `{"op":"commit"}` 或 `{"op":"rollback"}`；其中一条失败后只接受 rollback，输入中断即回滚 |

rpc 另有 `define_index`、`define_type`、`typedefs`、`set_policy`、`now`、`neighbors`。

## 派生类型

派生类型把 attrs 路径和关系提到第一级，让调用方直接读 `node.parent`、`node.next`。存储仍然是 Node 加 links，所有读、查、写都翻译成核心层操作。

| 类型 | 第一级字段 | 取自 | 常用操作 |
|---|---|---|---|
| [TreeNode](src/types/tree.rs) | `name` `parent` `children` `next` `before` | `/name`；`parent` 关系的出向 / 入向；`next` 关系的出向 / 入向 | `create` `read` `set_next` `descendants` `ancestors` |
| [EventNode](src/types/event.rs) | `what` `time` `next` `before` | `/what`；`/time` 区间；`next` 关系的出向 / 入向 | `create` `then` `read` `during` `after` |
| [StateNode](src/types/state.rs) | `key` `value` `valid` `subject` | `/key` `/value`；`/valid` 区间；`state_of` 关系的出向 | `set` `at` `history` |
| [CodeNode](src/types/code.rs) | `symbol` `file` `line_start` `calls` `called_by` 等 | `/symbol` 等 attrs；`call` / `dataflow` / `impact` 关系的出向 / 入向 | `create` `read` |
| [RelationshipNode](src/types/code.rs) | `from` `to` `edge_type` `condition` `call_line` 等 | `rel_from` / `rel_to` 关系的出向；边元数据 attrs | `create` `read` |

```rust
use fastnode::types::{EventNode, Span, StateNode, TreeNode};
use fastnode::{NewNode, Result, Store};
use serde_json::json;

const DAY: i64 = 1_789_228_800_000; // 2026-09-13 00:00 +08:00
const DAY_MS: i64 = 86_400_000;

fn main() -> Result<()> {
    let mut db = Store::open(":memory:")?;

    let root = TreeNode::create(&mut db, "文档", "项目文档根目录", None)?;
    let api = TreeNode::create(&mut db, "API", "接口说明", Some(root))?;
    let node = TreeNode::read(&mut db, api)?.unwrap();
    assert_eq!(node.parent.unwrap().id, root);
    assert!(TreeNode::descendants(&mut db, root)?.contains(api));

    let today = Span::new(DAY, Some(DAY + DAY_MS)).with_text("今天");
    let eat = EventNode::create(&mut db, "吃饭", "今天吃了饭", today.clone())?;
    let shop = EventNode::create(&mut db, "逛商场", "然后逛商场", today)?;
    EventNode::then(&mut db, eat, shop)?;               // 先后顺序靠关系，不靠时间
    assert_eq!(EventNode::during(&mut db, DAY, DAY + DAY_MS, 20)?.len(), 2);

    let person = db.create(NewNode::new("person", "小明", json!({})))?;
    StateNode::set(&mut db, person, "city", json!("上海"), DAY - 400 * DAY_MS, "住在上海")?;
    StateNode::set(&mut db, person, "city", json!("北京"), DAY - 30 * DAY_MS, "搬到北京")?;   // 自动切开旧状态
    let then = StateNode::at(&mut db, person, "city", DAY - 365 * DAY_MS)?.unwrap();
    assert_eq!(then.value, json!("上海"));
    Ok(())
}
```

定义新的类型，只要写一个 `Schema`，说明它对应哪个 `type`、每个字段从哪里取：

```rust
use fastnode::Direction::Out;
use fastnode::types::{Field, Schema, Source};
use fastnode::{LinkOptions, NewNode, Result, Store};
use serde_json::json;

static PERSON: Schema = Schema {
    kind: "person",
    fields: &[
        Field { name: "name", source: Source::Attr("/name") },
        Field { name: "company", source: Source::Link { relation: "works_at", direction: Out, many: false } },
    ],
};

fn main() -> Result<()> {
    let mut db = Store::open(":memory:")?;
    let company = db.create(NewNode::new("company", "星云科技，做数据库", json!({})))?;
    let person = db.create(NewNode::new("person", "小明", json!({"name":"小明"})))?;
    db.write(|w| PERSON.link(w, person, "company", company))?;
    let view = PERSON.read(&mut db, person, &LinkOptions::default())?.unwrap();
    let name: Option<String> = view.get("name")?;
    assert_eq!(name.as_deref(), Some("小明"));
    println!("{}", serde_json::to_string(&view)?);   // {"id":2,"type":"person","summary":"小明","name":"小明","company":{…}}
    Ok(())
}
```

**派生类型目前的限制：**

- 字段只能取自本 Node 的 attrs 路径，或某种关系走一跳后的邻居。
- 还不支持：沿关系链取对方的属性（如 `company.city`）、多跳字段、沿关系继承 attrs。
- 命令行和 rpc 的 `view` 认识内置的 tree、event、state、code、relationship，以及 `define-type` 存进库里的运行时类型。

**code / relationship 用法**：这两个类型是为代码符号库准备的（见 `examples/anchors_to_fastnode.py`，把 codedendrite 的 anchors/relations 一键导入）。每个符号是 `code` 节点，每条调用/数据流边是 `relationship` 节点（边元数据全保留），图遍历仍走纯三元组的 `call` / `dataflow` / `impact` 关系：

```rust
use fastnode::types::{CodeNode, RelationshipNode};
use fastnode::{Result, Store};

fn main() -> Result<()> {
    let mut db = Store::open("data/codedendrite.db")?;
    let main_fn = CodeNode::read(&mut db, 1209)?.unwrap();
    println!("{} 调用了 {:?}", main_fn.summary, main_fn.calls.iter().map(|l| &l.summary).collect::<Vec<_>>());
    let edge = RelationshipNode::read(&mut db, 1392)?.unwrap();   // 一条 call 边
    println!("{:?} -[{}]-> {:?} @{:?}", edge.from.map(|l| l.id), edge.edge_type, edge.to.map(|l| l.id), edge.call_line);
    Ok(())
}
```

命令行同样可用：

```bash
$B data/codedendrite.db view code 1209           # main 的 calls / called_by 提升为第一级字段
$B data/codedendrite.db view relationship 1392   # 边的 from / to / condition / call_line
```

**运行时定义派生类型**（不用改代码、不用重编译）：`define-type` 把定义存进数据库，`view` 读取时动态解析；重复定义相同内容无副作用，同名不同定义会报错。定义里每个字段取 `attr`（JSON Pointer）或 `link`（关系名，可配 `direction`、`many`）之一：

```bash
$B data/demo.db define-type '{"kind":"brief","fields":[
  {"name":"symbol","attr":"/symbol"},
  {"name":"callees","link":"call","direction":"out","many":true}]}'
$B data/demo.db typedefs            # 列出所有运行时定义
$B data/demo.db view brief <id>     # 与内置类型同样使用
```

运行时定义的类型同样受上面的限制约束；`kind` 必须与节点的 `type` 一致，且不能覆盖内置类型。

## 命令行

格式为 `fastnode <database> <command>`。JSON 参数可以直接写，也可以写 `@文件路径`，或者写 `-` 从标准输入读。

| 命令 | 用法 |
|---|---|
| 新建 / 批量新建 | `create <node>` / `create-many <node-array>` |
| 读取 | `get <id> [--links none\|summary\|full] [--link-limit N]` |
| 派生视图 | `view <tree\|event\|state\|code\|relationship\|自定义类型> <id> [--links …]` |
| 按值查找 | `find <value> [--order <field>] [--direction asc\|desc] [--cursor c] [--limit N] [--after N] [--links …]` |
| 替换 / 修改 / 删除 | `replace <id> <node>` / `patch <id> <merge-patch>` / `delete <id>` |
| 关系 | `link <from> <relation> <to>` / `unlink …` / `links <id>` |
| 查询 / 批量写 / 导入 | `query <query>` / `batch <ops>` / `import <file\|-> [batch-size]` |
| 派生类型定义 | `define-type <def>` / `typedefs` |
| 统计 / 常驻 | `stats` / `rpc`（支持 begin / commit / rollback） |

- **find 的参数类型**：`find 26` 查数字，`find '"26"'` 查字符串，`find b` 查字符串 b。
- **patch**：对 `{type, summary, attrs}` 做 JSON Merge Patch。attrs 里写 null 表示删除这个属性，数组整体替换；type 和 summary 不能删除或置空。
- **事务**：`create-many`、`batch` 要么全部成功，要么整体回滚。
- **导入**：流式读取，默认每 5,000 条提交一次；出错时报告第几条记录出错、已经提交了多少条。

`rpc` 模式每行读一个请求，返回 `{"ok":true,"result":…}` 或 `{"ok":false,"error":"…"}`；单条请求出错不影响后面的请求：

```jsonl
{"op":"create","node":{"type":"person","summary":"小明","attrs":{"age":26}}}
{"op":"create","node":{"type":"tree","summary":"文档根目录","attrs":{"name":"文档"}}}
{"op":"query","query":{"predicate":{"op":"range","field":"/age","gte":20},"order_by":{"field":"/age"},"include_data":true}}
{"op":"get","id":1,"links":"summary","link_limit":20}
{"op":"view","type":"tree","id":2}
{"op":"patch","id":1,"patch":{"attrs":{"city":"上海"}}}
{"op":"batch","ops":[{"op":"link","from":1,"relation":"next","to":2},{"op":"delete","id":3}]}
```

## Rust 接入

```toml
[dependencies]
fastnode = { git = "https://github.com/lo2589/FastNode" }
serde_json = { version = "1", features = ["arbitrary_precision"] }
```

```rust
use fastnode::{Link, LinkOptions, NewNode, OrderBy, Predicate, Query, Result, Store};
use serde_json::json;

fn main() -> Result<()> {
    let mut db = Store::open(":memory:")?;
    let a = db.create(NewNode::new("person", "小明", json!({"age":26,"jobs":[{"company":"星云科技"}]})))?;
    let b = db.create(NewNode::new("person", "小红", json!({"age":31,"jobs":[{"company":"星云科技"}]})))?;
    db.link(&Link { from: a, relation: "knows".into(), to: b })?;

    let mut query = Query::new(Predicate::Any { value: json!("星云科技") });
    query.order_by = Some(OrderBy::desc("/age"));
    assert_eq!(db.query(&query)?.ids, vec![b, a]);

    let node = db.get_with(a, &LinkOptions::default())?.unwrap();
    assert_eq!(node.links.unwrap().out[0].summary.as_deref(), Some("小红"));

    db.write(|w| {
        let found = w.select(&Predicate::Any { value: json!("星云科技") })?;   // 能读到本事务里还没提交的写入
        for id in &found {
            w.patch(id, json!({"attrs":{"checked":true}}))?;
        }
        Ok(())
    })?;
    Ok(())
}
```

| 类型 | 主要方法 |
|---|---|
| `Store` | `open` `write` `create` `create_many` `get` `get_with` `neighbors` `replace` `patch` `delete` `link` `unlink` `links` `query` `select` `import` `stats` `checkpoint` `define_index` `set_policy` `policy` |
| `Write`（事务内） | `get` `get_with` `neighbors` `create` `replace` `patch` `delete` `link` `unlink` `select` `query` `now` |

`select` 返回 `NodeSet`，也就是 `roaring::RoaringBitmap`，可以继续做交集、并集、差集。

## Python 接入

用 maturin 从源码构建 wheel 并安装：

```bash
pip install maturin
maturin build --release                      # 产物在 target/wheels/
pip install target/wheels/fastnode-*.whl
```

```python
import fastnode

db = fastnode.Store("data/demo.db")          # ":memory:" 用内存库
a = db.create({"type": "person", "summary": "小明", "attrs": {"age": 26}})
b = db.create({"type": "person", "summary": "小红", "attrs": {"age": 31}})
db.link(a, "knows", b)

db.query({"predicate": {"op": "range", "field": "/age", "gte": 20},
          "order_by": {"field": "/age", "direction": "desc"}, "include_data": True})
db.find("小红")                               # 任意路径含这个值的 id 列表
db.get(a, links="summary")                    # 带引用读出
```

方法与核心 API 一一对应：`create` `create_many` `get` `query` `find` `patch` `delete` `link` `unlink` `import` `stats`。参数和返回值都是原生 dict / list，查询协议与命令行相同；进程内直接调 Rust 库，没有子进程开销。

## 存储

```text
nodes            id → version, type, summary, attrs（type、summary 排在 attrs 前面）
postings         (路径, 值, 分块) → 位图：字符串、@type、存在标记、多个 Node 共享的数字
counts           (路径, 值) → 持有的 Node 数；另有 (值, 路径) 索引，供 any 使用
numbers          (路径, 精确排序键, id)，另有 (id, 路径, 键) 索引：所有数字
intervals        (路径, start, end, id)，另有按 id 查的索引、没有结束时间的区间的部分索引
interval_fields  每个路径写入过的最长有限区间长度（只增不减）
links            (source, relation, target) + (target, relation, source)
index_defs / composites   复合索引定义与条目 (名字, 分组键, 有序键, id)
policies         按 type 的写入策略
clock            事务时间戳的最后取值
```

- **持久化**：SQLite `WAL + synchronous=FULL`。写入串行，读取可以并发；一次查询及其内容读取在同一个快照内完成。
- **单一持有者数字**：只写有序行；第二个 Node 取到同一个值时补建位图，持有者减回一个时撤掉。
- **版本**：当前是 schema v5；v4 的库打开时原地升级，更早的版本会拒绝打开，需要重新导入。

## 代码结构

```text
src/model/          Node、链接、谓词、查询参数
src/index/          JSON → 位图键、数字排序键、区间行、路径规则
src/store/          schema、读取与引用、写入、索引维护、数字与区间行
src/query/          谓词求值、等值、范围、区间、图遍历、排序与游标
src/types/          Schema、Span 与派生类型 tree / event / state / code / relationship
src/bin/fastnode/   命令行与 rpc
tests/              crud storage attrs sparse order interval graph types scan composite policy write_reads cli
examples/           demo、types、million（百万基准）、jsonl_million.py
```

每个代码文件不超过 300 行。

## 测试与复现

```bash
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked --examples --bins
./target/release/examples/million data/another-million.db 1000000
python3 examples/jsonl_million.py 1000000 --database data/another-jsonl.db --input data/another.jsonl --report reports/another-jsonl.json
```

`tests/scan.rs` 和 `tests/sparse.rs` 把过滤、三种排序路径、四种区间关系、单一持有者数字的结果，与独立扫描逐项对照。基准要求使用新的数据库路径。

设计参考：[SQLite WAL](https://www.sqlite.org/wal.html)、[rusqlite](https://docs.rs/rusqlite/latest/rusqlite/)、[RoaringBitmap](https://docs.rs/roaring/latest/roaring/bitmap/struct.RoaringBitmap.html)。
