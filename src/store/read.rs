use super::Write;
use crate::{Direction, LinkMode, LinkOptions, LinkRef, Node, NodeId, NodeLinks};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::OnceLock;

pub(crate) const MAX_LINK_LIMIT: usize = 10_000;

impl Write<'_> {
    /// A Node with its links, as seen inside this transaction.
    pub fn get_with(&self, id: NodeId, options: &LinkOptions) -> Result<Option<Node>> {
        super::check_limit(options)?;
        load_node(&self.tx, id, Some(options))
    }

    /// Links of one relation in one direction, as seen inside this transaction.
    pub fn neighbors(
        &self,
        id: NodeId,
        relation: &str,
        direction: Direction,
        options: &LinkOptions,
    ) -> Result<(Vec<LinkRef>, bool)> {
        super::check_limit(options)?;
        link_refs(&self.tx, id, direction, Some(relation), options)
    }
}

pub(crate) fn get_node(conn: &Connection, id: NodeId) -> Result<Option<Node>> {
    let row: Option<(u64, String, String, String)> = conn
        .prepare_cached("SELECT version,type,summary,attrs FROM nodes WHERE id=?1")?
        .query_row([id], |r| {
            Ok((r.get::<_, i64>(0)? as u64, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    row.map(|(version, kind, summary, attrs)| {
        Ok(Node {
            id,
            version,
            kind,
            summary,
            attrs: serde_json::from_str(&attrs)?,
            links: None,
        })
    })
    .transpose()
}

pub(crate) fn load_node(
    conn: &Connection,
    id: NodeId,
    links: Option<&LinkOptions>,
) -> Result<Option<Node>> {
    let Some(mut node) = get_node(conn, id)? else {
        return Ok(None);
    };
    if let Some(options) = links {
        let (out, out_more) = link_refs(conn, id, Direction::Out, None, options)?;
        let (incoming, in_more) = link_refs(conn, id, Direction::In, None, options)?;
        node.links = Some(NodeLinks {
            out,
            incoming,
            out_more,
            in_more,
        });
    }
    Ok(Some(node))
}

/// One index-ordered scan per direction, optionally of one relation. The far
/// Node's columns come from a primary-key join; summary mode never parses attrs.
pub(crate) fn link_refs(
    conn: &Connection,
    id: NodeId,
    direction: Direction,
    relation: Option<&str>,
    options: &LinkOptions,
) -> Result<(Vec<LinkRef>, bool)> {
    let mut stmt = conn.prepare_cached(link_sql(direction, options.mode, relation.is_some()))?;
    let limit = options.limit as i64 + 1;
    let mut rows = match relation {
        Some(relation) => stmt.query(params![id, limit, relation])?,
        None => stmt.query(params![id, limit])?,
    };
    let mut refs = Vec::new();
    let mut more = false;
    while let Some(row) = rows.next()? {
        if refs.len() == options.limit {
            more = true;
            break;
        }
        let mut link = LinkRef {
            relation: row.get(0)?,
            id: row.get(1)?,
            kind: None,
            summary: None,
            attrs: None,
        };
        if options.mode != LinkMode::None {
            link.kind = Some(row.get(2)?);
            link.summary = Some(row.get(3)?);
        }
        if options.mode == LinkMode::Full {
            link.attrs = Some(serde_json::from_str(&row.get::<_, String>(4)?)?);
        }
        refs.push(link);
    }
    Ok((refs, more))
}

// Built once: direction × mode × relation filter.
fn link_sql(direction: Direction, mode: LinkMode, by_relation: bool) -> &'static str {
    static SQL: OnceLock<Vec<String>> = OnceLock::new();
    let all = SQL.get_or_init(|| {
        let mut all = Vec::new();
        for direction in [Direction::Out, Direction::In] {
            for mode in [LinkMode::None, LinkMode::Summary, LinkMode::Full] {
                for by_relation in [false, true] {
                    all.push(build_sql(direction, mode, by_relation));
                }
            }
        }
        all
    });
    &all[direction as usize * 6 + mode as usize * 2 + usize::from(by_relation)]
}

fn build_sql(direction: Direction, mode: LinkMode, by_relation: bool) -> String {
    let (near, far) = match direction {
        Direction::Out => ("source", "target"),
        Direction::In => ("target", "source"),
    };
    let (columns, join) = match mode {
        LinkMode::None => ("", String::new()),
        LinkMode::Summary => (
            ",n.type,n.summary",
            format!(" JOIN nodes n ON n.id=l.{far}"),
        ),
        LinkMode::Full => (
            ",n.type,n.summary,n.attrs",
            format!(" JOIN nodes n ON n.id=l.{far}"),
        ),
    };
    let filter = if by_relation {
        " AND l.relation=?3"
    } else {
        ""
    };
    format!(
        "SELECT l.relation,l.{far}{columns} FROM links l{join} WHERE l.{near}=?1{filter} ORDER BY l.relation,l.{far} LIMIT ?2"
    )
}
