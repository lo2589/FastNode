use super::NodeLinks;
use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type NodeId = u32;
pub type NodeSet = RoaringBitmap;

/// What callers write: type and summary are required, attrs is any JSON object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NewNode {
    #[serde(rename = "type")]
    pub kind: String,
    pub summary: String,
    #[serde(default = "empty_object")]
    pub attrs: Value,
}

impl NewNode {
    pub fn new(kind: impl Into<String>, summary: impl Into<String>, attrs: Value) -> Self {
        Self {
            kind: kind.into(),
            summary: summary.into(),
            attrs,
        }
    }
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: NodeId,
    pub version: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub summary: String,
    pub attrs: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<NodeLinks>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Link {
    pub from: NodeId,
    pub relation: String,
    pub to: NodeId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Stats {
    pub nodes: u64,
    pub links: u64,
    pub intervals: u64,
    pub bitmap_blocks: u64,
    pub bitmap_bytes: u64,
    pub indexed_fields: u64,
}
