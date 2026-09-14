//! Per-type write policies. History Nodes are append-only: never deleted or
//! replaced, and only a few declared attrs keys may be patched in place.

use super::{Store, Write};
use crate::{Node, index};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WritePolicy {
    #[serde(default = "allowed")]
    pub delete: bool,
    #[serde(default = "allowed")]
    pub replace: bool,
    /// attrs JSON Pointers that patch may change (with everything below
    /// them); None allows any patch, including type and summary.
    #[serde(default)]
    pub patch: Option<Vec<String>>,
}

fn allowed() -> bool {
    true
}

impl Default for WritePolicy {
    fn default() -> Self {
        Self {
            delete: true,
            replace: true,
            patch: None,
        }
    }
}

impl WritePolicy {
    /// No delete, no replace; patch only at or below these attrs pointers.
    pub fn append_only(patchable: &[&str]) -> Self {
        Self {
            delete: false,
            replace: false,
            patch: Some(patchable.iter().map(|p| p.to_string()).collect()),
        }
    }
}

pub(crate) fn load(conn: &Connection, kind: &str) -> Result<WritePolicy> {
    let row: Option<(bool, bool, Option<String>)> = conn
        .prepare_cached("SELECT can_delete,can_replace,patchable FROM policies WHERE type=?1")?
        .query_row([kind], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(match row {
        None => WritePolicy::default(),
        Some((delete, replace, patch)) => WritePolicy {
            delete,
            replace,
            patch: patch.map(|p| serde_json::from_str(&p)).transpose()?,
        },
    })
}

impl Store {
    pub fn set_policy(&mut self, kind: &str, policy: &WritePolicy) -> Result<()> {
        ensure!(!kind.is_empty(), "policy type is required");
        for pointer in policy.patch.iter().flatten() {
            ensure!(
                pointer.starts_with('/'),
                "patchable keys are attrs pointers such as /recorded/end"
            );
            index::field_valid(pointer)?;
        }
        let patch = policy
            .patch
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.write(|w| {
            w.tx.execute(
                "INSERT INTO policies(type,can_delete,can_replace,patchable) VALUES(?1,?2,?3,?4) ON CONFLICT(type) DO UPDATE SET can_delete=excluded.can_delete,can_replace=excluded.can_replace,patchable=excluded.patchable",
                params![kind, policy.delete, policy.replace, patch],
            )?;
            w.policies.clear();
            Ok(())
        })
    }

    pub fn policy(&self, kind: &str) -> Result<WritePolicy> {
        load(&self.conn, kind)
    }
}

impl Write<'_> {
    fn policy_for(&mut self, kind: &str) -> Result<WritePolicy> {
        if !self.policies.contains_key(kind) {
            let policy = load(&self.tx, kind)?;
            self.policies.insert(kind.to_owned(), policy);
        }
        Ok(self.policies[kind].clone())
    }

    pub(super) fn check_delete(&mut self, node: &Node) -> Result<()> {
        ensure!(
            self.policy_for(&node.kind)?.delete,
            "{} nodes cannot be deleted (write policy)",
            node.kind
        );
        Ok(())
    }

    pub(super) fn check_replace(&mut self, node: &Node) -> Result<()> {
        ensure!(
            self.policy_for(&node.kind)?.replace,
            "{} nodes cannot be replaced (write policy)",
            node.kind
        );
        Ok(())
    }

    pub(super) fn check_patch(&mut self, node: &Node, patch: &Value) -> Result<()> {
        let Some(allowed) = self.policy_for(&node.kind)?.patch else {
            return Ok(());
        };
        let mut changed = Vec::new();
        for (key, value) in patch.as_object().into_iter().flatten() {
            ensure!(
                key == "attrs",
                "{} nodes only allow patching attrs at {allowed:?} (write policy)",
                node.kind
            );
            leaves(value, String::new(), &mut changed);
        }
        for pointer in changed {
            let permitted = allowed
                .iter()
                .any(|p| pointer == *p || pointer.starts_with(&format!("{p}/")));
            ensure!(
                permitted,
                "patching {pointer} is not allowed for {} nodes (write policy)",
                node.kind
            );
        }
        Ok(())
    }
}

// Every pointer a merge patch sets or removes.
fn leaves(value: &Value, path: String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                leaves(child, format!("{path}/{}", index::escape(key)), out);
            }
        }
        _ => out.push(path),
    }
}
