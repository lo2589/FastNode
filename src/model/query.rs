use super::{Direction, LinkMode, Node, NodeId, Predicate, default_link_limit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub relation: String,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub filter: Option<Predicate>,
    /// Repeat this relation: keep Nodes whose shortest hop count is in
    /// min..=max. Omitting both means exactly one hop; max absent means
    /// until no new Node is reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
}

impl Step {
    pub fn new(relation: impl Into<String>, direction: Direction) -> Self {
        Self {
            relation: relation.into(),
            direction,
            filter: None,
            min: None,
            max: None,
        }
    }

    pub(crate) fn repeat(&self) -> Option<(u32, Option<u32>)> {
        (self.min.is_some() || self.max.is_some()).then(|| (self.min.unwrap_or(1), self.max))
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

/// Sort by one single-valued path. Numbers precede strings when ascending;
/// Nodes without a value come last in either direction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderBy {
    pub field: String,
    #[serde(default)]
    pub direction: SortDirection,
}

impl OrderBy {
    pub fn asc(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            direction: SortDirection::Asc,
        }
    }
    pub fn desc(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            direction: SortDirection::Desc,
        }
    }
}

fn default_limit() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub predicate: Predicate,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Id cursor for results in id order.
    #[serde(default)]
    pub after: NodeId,
    #[serde(default)]
    pub include_data: bool,
    /// Applies to returned Nodes when include_data is true.
    #[serde(default)]
    pub links: LinkMode,
    #[serde(default = "default_link_limit")]
    pub link_limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_by: Option<OrderBy>,
    /// next_cursor from the previous page of an ordered query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl Query {
    pub fn new(predicate: Predicate) -> Self {
        Self {
            predicate,
            limit: default_limit(),
            after: 0,
            include_data: false,
            links: LinkMode::default(),
            link_limit: default_link_limit(),
            order_by: None,
            cursor: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueryResult {
    pub total: u64,
    pub ids: Vec<NodeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<Node>>,
    pub next_after: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
