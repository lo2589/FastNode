//! Predicate evaluation over the persisted indexes. Every predicate yields a
//! NodeSet; ordering and paging happen once the full set is known.

mod estimate;
mod eval;
mod exact;
mod graph;
mod interval;
mod order;
mod order_key;
mod postings;
mod range;
mod seek;
mod validate;

use crate::store::load_node;
use crate::{LinkOptions, NodeId, NodeSet, Predicate, Query, QueryResult, Store, Write};
use anyhow::Result;
use rusqlite::Connection;
use std::ops::Bound::{Excluded, Unbounded};

pub(crate) struct Engine<'a> {
    pub(crate) conn: &'a Connection,
}

impl Store {
    pub fn query(&mut self, query: &Query) -> Result<QueryResult> {
        validate::query(query)?;
        let tx = self.conn.transaction()?;
        let result = run(&tx, query)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn select(&mut self, predicate: &Predicate) -> Result<NodeSet> {
        validate::predicate(predicate, 0)?;
        let tx = self.conn.transaction()?;
        let set = Engine { conn: &tx }.eval(predicate, None)?;
        tx.commit()?;
        Ok(set)
    }
}

impl Write<'_> {
    /// Evaluates inside this transaction, seeing its uncommitted writes.
    pub fn select(&mut self, predicate: &Predicate) -> Result<NodeSet> {
        validate::predicate(predicate, 0)?;
        self.flush()?;
        Engine { conn: &self.tx }.eval(predicate, None)
    }

    /// `Store::query` inside this transaction, seeing its uncommitted writes.
    pub fn query(&mut self, query: &Query) -> Result<QueryResult> {
        validate::query(query)?;
        self.flush()?;
        run(&self.tx, query)
    }
}

fn run(conn: &Connection, query: &Query) -> Result<QueryResult> {
    let engine = Engine { conn };
    let set = engine.eval(&query.predicate, None)?;
    let (ids, next_after, next_cursor) = match &query.order_by {
        Some(order) => {
            let page = engine.order(&set, order, query.cursor.as_deref(), query.limit)?;
            (page.ids, None, page.next_cursor)
        }
        None => id_page(&set, query.after, query.limit),
    };
    let nodes = if query.include_data {
        let links = LinkOptions {
            mode: query.links,
            limit: query.link_limit,
        };
        Some(
            ids.iter()
                .map(|id| {
                    load_node(conn, *id, Some(&links))?
                        .ok_or_else(|| anyhow::anyhow!("index references missing node {id}"))
                })
                .collect::<Result<_>>()?,
        )
    } else {
        None
    };
    Ok(QueryResult {
        total: set.len(),
        ids,
        nodes,
        next_after,
        next_cursor,
    })
}

type Page = (Vec<NodeId>, Option<NodeId>, Option<String>);

fn id_page(set: &NodeSet, after: NodeId, limit: usize) -> Page {
    let mut ids: Vec<NodeId> = set
        .range((Excluded(after), Unbounded))
        .take(limit.saturating_add(1))
        .collect();
    let more = ids.len() > limit;
    ids.truncate(limit);
    let next = if more { ids.last().copied() } else { None };
    (ids, next, None)
}
