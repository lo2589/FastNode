use anyhow::{Result, ensure};
use rusqlite::Connection;

pub(crate) const VERSION: u32 = 5;

// type and summary precede attrs so reading them never touches the overflow
// pages of a large attrs document. Postings and counts hold every string and
// every number shared by two or more Nodes; numbers holds every number.
const TABLES: &str = "
CREATE TABLE IF NOT EXISTS nodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK(id BETWEEN 1 AND 4294967295),
    version INTEGER NOT NULL DEFAULT 1,
    type TEXT NOT NULL CHECK(length(type)>0),
    summary TEXT NOT NULL CHECK(length(summary)>0),
    attrs TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS postings (
    field TEXT NOT NULL, token TEXT NOT NULL, shard INTEGER NOT NULL,
    cardinality INTEGER NOT NULL, bitmap BLOB NOT NULL,
    PRIMARY KEY(field, token, shard)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS counts (
    field TEXT NOT NULL, token TEXT NOT NULL, n INTEGER NOT NULL,
    PRIMARY KEY(field, token)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS counts_token ON counts(token, field);
CREATE TABLE IF NOT EXISTS numbers (
    field TEXT NOT NULL, value BLOB NOT NULL,
    node_id INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    PRIMARY KEY(field, value, node_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS numbers_node ON numbers(node_id, field, value);
CREATE TABLE IF NOT EXISTS intervals (
    field TEXT NOT NULL, lo BLOB NOT NULL, hi BLOB NOT NULL,
    node_id INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    PRIMARY KEY(field, lo, hi, node_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS intervals_node ON intervals(node_id, field, lo, hi);
CREATE INDEX IF NOT EXISTS intervals_open ON intervals(field, lo) WHERE hi=X'03';
CREATE TABLE IF NOT EXISTS interval_fields (
    field TEXT PRIMARY KEY, max_len REAL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS links (
    source INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    relation TEXT NOT NULL CHECK(length(relation)>0),
    target INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    PRIMARY KEY(source, relation, target)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS links_reverse ON links(target, relation, source);
CREATE TABLE IF NOT EXISTS index_defs (
    name TEXT PRIMARY KEY, type TEXT NOT NULL, grouping TEXT NOT NULL, ordering TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS typedefs (
    kind TEXT PRIMARY KEY, def TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS composites (
    name TEXT NOT NULL, grp BLOB NOT NULL, ord BLOB NOT NULL,
    node_id INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    PRIMARY KEY(name, grp, ord, node_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS composites_node ON composites(node_id, name, grp, ord);
CREATE TABLE IF NOT EXISTS policies (
    type TEXT PRIMARY KEY, can_delete INTEGER NOT NULL, can_replace INTEGER NOT NULL, patchable TEXT
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS clock (id INTEGER PRIMARY KEY CHECK(id = 1), last INTEGER NOT NULL);
";

pub(super) fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA cache_size=-65536; PRAGMA temp_store=MEMORY;")?;
    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        matches!(version, 0 | 4 | VERSION),
        "database schema v{version} is not supported (this FastNode uses v{VERSION} and upgrades v4 in place); import the data into a new database"
    );
    conn.execute_batch(&format!(
        "BEGIN IMMEDIATE;{TABLES}PRAGMA user_version={VERSION};COMMIT;"
    ))?;
    Ok(())
}
