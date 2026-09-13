use super::NodeId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Out,
    In,
}

/// How much of the Node on the other end of each link is pulled.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinkMode {
    /// relation and id only; reads the link index alone.
    None,
    /// plus type and summary; one primary-key lookup per link, attrs untouched.
    #[default]
    Summary,
    /// plus attrs.
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkOptions {
    pub mode: LinkMode,
    /// Maximum links returned per direction (or per relation).
    pub limit: usize,
}

impl Default for LinkOptions {
    fn default() -> Self {
        Self {
            mode: LinkMode::Summary,
            limit: default_link_limit(),
        }
    }
}

pub(crate) fn default_link_limit() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkRef {
    pub relation: String,
    pub id: NodeId,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NodeLinks {
    pub out: Vec<LinkRef>,
    #[serde(rename = "in")]
    pub incoming: Vec<LinkRef>,
    /// More links exist in this direction than the limit returned.
    pub out_more: bool,
    pub in_more: bool,
}
