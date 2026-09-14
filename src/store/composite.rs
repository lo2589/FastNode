//! Declared composite indexes: Nodes of one type, grouped by equal values at
//! the grouping paths and ordered by the ordering path. `seek` walks one group
//! in order, so "the latest version of this timeline at or before t" is one
//! B-tree descent however long the history is.

use super::{Store, Write};
use crate::NodeId;
use crate::index::{self, Attributes};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexDef {
    pub name: String,
    pub kind: String,
    pub grouping: Vec<String>,
    pub ordering: String,
}

/// (index name, group key, ordering key).
pub(crate) type Entries = BTreeSet<(String, Vec<u8>, Vec<u8>)>;

pub(crate) fn definitions(conn: &Connection) -> Result<Vec<IndexDef>> {
    let mut stmt =
        conn.prepare_cached("SELECT name,type,grouping,ordering FROM index_defs ORDER BY name")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut defs = Vec::new();
    for row in rows {
        let (name, kind, grouping, ordering) = row?;
        defs.push(IndexDef {
            name,
            kind,
            grouping: serde_json::from_str(&grouping)?,
            ordering,
        });
    }
    Ok(defs)
}

pub(crate) fn definition(conn: &Connection, name: &str) -> Result<IndexDef> {
    definitions(conn)?
        .into_iter()
        .find(|d| d.name == name)
        .with_context(|| format!("no composite index named {name}"))
}

/// Group key: the typed token of each grouping value, NUL-separated.
pub(crate) fn group_key(tokens: &[String]) -> Vec<u8> {
    tokens.join("\u{0}").into_bytes()
}

/// The lowest ordering key a string can have; number keys all sort below it.
pub(crate) const STRING_ORDER: u8 = 0x20;

pub(crate) fn order_key(token: &str) -> Result<Vec<u8>> {
    let bytes = index::token_sort_bytes(token)?;
    Ok(if token.starts_with("s:") {
        [&[STRING_ORDER][..], &bytes].concat()
    } else {
        bytes
    })
}

fn scalars<'a>(attrs: &'a Attributes, field: &str) -> Vec<&'a str> {
    attrs
        .postings
        .range((field.to_owned(), String::new())..)
        .take_while(|((f, _), _)| f == field)
        .map(|((_, token), _)| token.as_str())
        .filter(|token| {
            ["s:", "n:", "b:", "z:"]
                .iter()
                .any(|p| token.starts_with(p))
        })
        .collect()
}

/// A Node's entries: only when every grouping path holds exactly one scalar
/// and the ordering path exactly one number or string.
pub(crate) fn entries(defs: &[IndexDef], kind: &str, attrs: &Attributes) -> Result<Entries> {
    let mut out = Entries::new();
    'defs: for def in defs.iter().filter(|d| d.kind == kind) {
        let mut group = Vec::with_capacity(def.grouping.len());
        for path in &def.grouping {
            match scalars(attrs, path).as_slice() {
                [token] => group.push((*token).to_owned()),
                _ => continue 'defs,
            }
        }
        let ordering: Vec<&str> = scalars(attrs, &def.ordering)
            .into_iter()
            .filter(|t| index::is_sortable(t))
            .collect();
        if let [token] = ordering.as_slice() {
            out.insert((def.name.clone(), group_key(&group), order_key(token)?));
        }
    }
    Ok(out)
}

fn insert(w: &Write, entry: &(String, Vec<u8>, Vec<u8>), id: NodeId) -> Result<()> {
    w.tx.prepare_cached("INSERT INTO composites(name,grp,ord,node_id) VALUES(?1,?2,?3,?4)")?
        .execute(params![entry.0, entry.1, entry.2, id])?;
    Ok(())
}

impl Write<'_> {
    fn index_definitions(&mut self) -> Result<Vec<IndexDef>> {
        if self.indexes.is_none() {
            self.indexes = Some(definitions(&self.tx)?);
        }
        Ok(self.indexes.clone().unwrap_or_default())
    }

    /// Keeps a Node's composite entries in step with its old and new attributes.
    pub(super) fn reindex(
        &mut self,
        id: NodeId,
        old: Option<(&str, &Attributes)>,
        new: Option<(&str, &Attributes)>,
    ) -> Result<()> {
        let defs = self.index_definitions()?;
        if defs.is_empty() {
            return Ok(());
        }
        let collect = |side: Option<(&str, &Attributes)>| match side {
            Some((kind, attrs)) => entries(&defs, kind, attrs),
            None => Ok(Entries::new()),
        };
        let (before, after) = (collect(old)?, collect(new)?);
        for (name, group, order) in before.difference(&after) {
            self.tx
                .prepare_cached(
                    "DELETE FROM composites WHERE name=?1 AND grp=?2 AND ord=?3 AND node_id=?4",
                )?
                .execute(params![name, group, order, id])?;
        }
        for entry in after.difference(&before) {
            insert(self, entry, id)?;
        }
        Ok(())
    }
}

impl Store {
    /// Declares a composite index over Nodes of `kind`: equal values at every
    /// `grouping` path, ordered by `ordering`. Existing Nodes are indexed now;
    /// declaring the identical definition again does nothing.
    pub fn define_index(
        &mut self,
        name: &str,
        kind: &str,
        grouping: &[&str],
        ordering: &str,
    ) -> Result<()> {
        ensure!(
            !name.is_empty() && !kind.is_empty(),
            "index name and type are required"
        );
        ensure!(
            !grouping.is_empty(),
            "an index needs at least one grouping path"
        );
        for path in grouping.iter().chain(std::iter::once(&ordering)) {
            ensure!(!path.is_empty(), "index paths must not be empty");
            index::field_valid(path)?;
        }
        let def = IndexDef {
            name: name.into(),
            kind: kind.into(),
            grouping: grouping.iter().map(|p| p.to_string()).collect(),
            ordering: ordering.into(),
        };
        self.write(|w| {
            if let Some(existing) = definitions(&w.tx)?.into_iter().find(|d| d.name == def.name) {
                ensure!(
                    existing == def,
                    "index {} already exists with a different definition",
                    def.name
                );
                return Ok(());
            }
            w.tx.execute(
                "INSERT INTO index_defs(name,type,grouping,ordering) VALUES(?1,?2,?3,?4)",
                params![
                    def.name,
                    def.kind,
                    serde_json::to_string(&def.grouping)?,
                    def.ordering
                ],
            )?;
            w.indexes = None;
            let ids: Vec<NodeId> =
                w.tx.prepare("SELECT id FROM nodes WHERE type=?1")?
                    .query_map([&def.kind], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
            for id in ids {
                let node = w.get(id)?.context("node vanished during backfill")?;
                let attrs = index::attributes(&node.kind, &node.attrs)?;
                for entry in entries(std::slice::from_ref(&def), &node.kind, &attrs)? {
                    insert(w, &entry, id)?;
                }
            }
            Ok(())
        })
    }
}
