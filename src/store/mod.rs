mod maintain;
mod numbers;
mod read;
mod schema;
mod write;

pub(crate) use read::{MAX_LINK_LIMIT, get_node, link_refs, load_node};
pub use write::Write;

use crate::{Direction, Link, LinkOptions, LinkRef, NewNode, Node, NodeId, Stats};
use anyhow::{Result, ensure};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::Value;
use std::{path::Path, time::Duration};

pub struct Store {
    pub(crate) conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(30))?;
        schema::init(&conn)?;
        conn.set_prepared_statement_cache_capacity(128);
        Ok(Self { conn })
    }

    /// Runs `action` in one write transaction; an error rolls everything back.
    pub fn write<T>(&mut self, action: impl FnOnce(&mut Write<'_>) -> Result<T>) -> Result<T> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut writer = Write::new(tx);
        let result = action(&mut writer)?;
        writer.commit()?;
        Ok(result)
    }

    pub fn create(&mut self, node: NewNode) -> Result<NodeId> {
        self.write(|w| w.create(node))
    }
    pub fn create_many(&mut self, nodes: Vec<NewNode>) -> Result<Vec<NodeId>> {
        self.write(|w| nodes.into_iter().map(|n| w.create(n)).collect())
    }
    /// The Node without its links.
    pub fn get(&self, id: NodeId) -> Result<Option<Node>> {
        get_node(&self.conn, id)
    }
    /// The Node plus its links in both directions, read in one snapshot.
    pub fn get_with(&mut self, id: NodeId, options: &LinkOptions) -> Result<Option<Node>> {
        check_limit(options)?;
        let tx = self.conn.transaction()?;
        let node = load_node(&tx, id, Some(options))?;
        tx.commit()?;
        Ok(node)
    }
    /// Links of one relation in one direction, ordered by the far Node's id.
    pub fn neighbors(
        &self,
        id: NodeId,
        relation: &str,
        direction: Direction,
        options: &LinkOptions,
    ) -> Result<(Vec<LinkRef>, bool)> {
        check_limit(options)?;
        link_refs(&self.conn, id, direction, Some(relation), options)
    }
    pub fn replace(&mut self, id: NodeId, node: NewNode) -> Result<()> {
        self.write(|w| w.replace(id, node))
    }
    pub fn patch(&mut self, id: NodeId, patch: Value) -> Result<()> {
        self.write(|w| w.patch(id, patch))
    }
    pub fn delete(&mut self, id: NodeId) -> Result<bool> {
        self.write(|w| w.delete(id))
    }
    pub fn link(&mut self, link: &Link) -> Result<bool> {
        self.write(|w| w.link(link))
    }
    pub fn unlink(&mut self, link: &Link) -> Result<bool> {
        self.write(|w| w.unlink(link))
    }
    pub fn links(&self, id: NodeId) -> Result<Vec<Link>> {
        let mut stmt = self.conn.prepare_cached("SELECT source,relation,target FROM links WHERE source=?1 UNION SELECT source,relation,target FROM links WHERE target=?1 ORDER BY 1,2,3")?;
        Ok(stmt
            .query_map([id], |r| {
                Ok(Link {
                    from: r.get(0)?,
                    relation: r.get(1)?,
                    to: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn stats(&mut self) -> Result<Stats> {
        let tx = self.conn.transaction()?;
        let count = |sql: &str| tx.query_row(sql, [], |r| r.get::<_, i64>(0).map(|v| v as u64));
        let nodes =
            count("SELECT COALESCE((SELECT n FROM counts WHERE field='' AND token='u:'),0)")?;
        let links = count("SELECT count(*) FROM links")?;
        let intervals = count("SELECT count(*) FROM intervals")?;
        let bitmap_blocks = count("SELECT count(*) FROM postings")?;
        let bitmap_bytes = count("SELECT COALESCE(sum(length(bitmap)),0) FROM postings")?;
        let indexed_fields = count("SELECT count(DISTINCT field) FROM counts WHERE field<>''")?;
        tx.commit()?;
        Ok(Stats {
            nodes,
            links,
            intervals,
            bitmap_blocks,
            bitmap_bytes,
            indexed_fields,
        })
    }
    pub fn checkpoint(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }
}

fn check_limit(options: &LinkOptions) -> Result<()> {
    ensure!(
        options.limit <= MAX_LINK_LIMIT,
        "link limit must be <= {MAX_LINK_LIMIT}"
    );
    Ok(())
}
