use super::{Direction, NodeId, SortDirection, Step};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Predicate {
    All,
    Ids {
        ids: Vec<NodeId>,
    },
    Eq {
        field: String,
        value: Value,
    },
    In {
        field: String,
        values: Vec<Value>,
    },
    /// A scalar value at any attrs path.
    Any {
        value: Value,
    },
    Exists {
        field: String,
    },
    Prefix {
        field: String,
        prefix: String,
    },
    /// Numbers or strings; every bound of one range has the same type.
    Range {
        field: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gt: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gte: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lt: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lte: Option<Value>,
    },
    /// Intervals at `field` holding the instant `value`.
    At {
        field: String,
        value: Number,
    },
    /// Intervals sharing any instant with [start, end).
    Overlap {
        field: String,
        start: Number,
        end: Number,
    },
    /// Intervals covering all of [start, end).
    Contains {
        field: String,
        start: Number,
        end: Number,
    },
    /// Intervals lying inside [start, end).
    ContainedBy {
        field: String,
        start: Number,
        end: Number,
    },
    /// Entries of a declared composite index: one group, in ordering order,
    /// within optional bounds, nearest first, at most `limit`.
    Seek {
        index: String,
        group: Vec<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gt: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gte: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lt: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lte: Option<Value>,
        #[serde(default)]
        direction: SortDirection,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    HasLink {
        relation: String,
        target: NodeId,
        #[serde(default)]
        direction: Direction,
    },
    And {
        args: Vec<Predicate>,
    },
    Or {
        args: Vec<Predicate>,
    },
    Not {
        arg: Box<Predicate>,
    },
    Traverse {
        from: Box<Predicate>,
        steps: Vec<Step>,
    },
}

// Serde's internally-tagged enum buffer does not preserve every arbitrary-
// precision JSON number. Parse the tag from a serde_json::Value instead.
impl<'de> Deserialize<'de> for Predicate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        parse(value).map_err(serde::de::Error::custom)
    }
}

type Map = serde_json::Map<String, Value>;

fn take<T: serde::de::DeserializeOwned>(map: &mut Map, key: &str) -> anyhow::Result<T> {
    let value = map
        .remove(key)
        .ok_or_else(|| anyhow::anyhow!("missing field {key}"))?;
    Ok(serde_json::from_value(value)?)
}

fn optional<T: serde::de::DeserializeOwned>(map: &mut Map, key: &str) -> anyhow::Result<Option<T>> {
    map.remove(key)
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v).map_err(Into::into))
        .transpose()
}

fn parse(value: Value) -> anyhow::Result<Predicate> {
    let Value::Object(mut map) = value else {
        anyhow::bail!("predicate must be an object");
    };
    let m = &mut map;
    let op: String = take(m, "op")?;
    let p = match op.as_str() {
        "all" => Predicate::All,
        "ids" => Predicate::Ids {
            ids: take(m, "ids")?,
        },
        "eq" => Predicate::Eq {
            field: take(m, "field")?,
            value: take(m, "value")?,
        },
        "in" => Predicate::In {
            field: take(m, "field")?,
            values: take(m, "values")?,
        },
        "any" => Predicate::Any {
            value: take(m, "value")?,
        },
        "exists" => Predicate::Exists {
            field: take(m, "field")?,
        },
        "prefix" => Predicate::Prefix {
            field: take(m, "field")?,
            prefix: take(m, "prefix")?,
        },
        "range" => Predicate::Range {
            field: take(m, "field")?,
            gt: optional(m, "gt")?,
            gte: optional(m, "gte")?,
            lt: optional(m, "lt")?,
            lte: optional(m, "lte")?,
        },
        "at" => Predicate::At {
            field: take(m, "field")?,
            value: take(m, "value")?,
        },
        "overlap" => Predicate::Overlap {
            field: take(m, "field")?,
            start: take(m, "start")?,
            end: take(m, "end")?,
        },
        "contains" => Predicate::Contains {
            field: take(m, "field")?,
            start: take(m, "start")?,
            end: take(m, "end")?,
        },
        "contained_by" => Predicate::ContainedBy {
            field: take(m, "field")?,
            start: take(m, "start")?,
            end: take(m, "end")?,
        },
        "seek" => Predicate::Seek {
            index: take(m, "index")?,
            group: take(m, "group")?,
            gt: optional(m, "gt")?,
            gte: optional(m, "gte")?,
            lt: optional(m, "lt")?,
            lte: optional(m, "lte")?,
            direction: optional(m, "direction")?.unwrap_or_default(),
            limit: optional(m, "limit")?,
        },
        "has_link" => Predicate::HasLink {
            relation: take(m, "relation")?,
            target: take(m, "target")?,
            direction: optional(m, "direction")?.unwrap_or_default(),
        },
        "and" => Predicate::And {
            args: take(m, "args")?,
        },
        "or" => Predicate::Or {
            args: take(m, "args")?,
        },
        "not" => Predicate::Not {
            arg: take(m, "arg")?,
        },
        "traverse" => Predicate::Traverse {
            from: take(m, "from")?,
            steps: take(m, "steps")?,
        },
        _ => anyhow::bail!("unknown predicate op {op}"),
    };
    anyhow::ensure!(
        map.is_empty(),
        "unknown predicate fields: {:?}",
        map.keys().collect::<Vec<_>>()
    );
    Ok(p)
}
