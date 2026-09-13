use anyhow::{Context, Result, bail, ensure};
use fastnode::{Link, NewNode, Write};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// One write inside `batch`, or a single-operation rpc request.
#[derive(Deserialize)]
#[serde(try_from = "Value")]
pub enum Mutation {
    Create { node: NewNode },
    Replace { id: u32, node: NewNode },
    Patch { id: u32, patch: Value },
    Delete { id: u32 },
    Link(Link),
    Unlink(Link),
}

fn take<T: serde::de::DeserializeOwned>(map: &mut Map<String, Value>, key: &str) -> Result<T> {
    let value = map
        .remove(key)
        .with_context(|| format!("missing field {key}"))?;
    Ok(serde_json::from_value(value)?)
}

fn link(map: &mut Map<String, Value>) -> Result<Link> {
    Ok(Link {
        from: take(map, "from")?,
        relation: take(map, "relation")?,
        to: take(map, "to")?,
    })
}

impl TryFrom<Value> for Mutation {
    type Error = anyhow::Error;
    fn try_from(value: Value) -> Result<Self> {
        let Value::Object(mut map) = value else {
            bail!("operation must be an object");
        };
        let m = &mut map;
        let op: String = take(m, "op")?;
        let result = match op.as_str() {
            "create" => Self::Create {
                node: take(m, "node")?,
            },
            "replace" => Self::Replace {
                id: take(m, "id")?,
                node: take(m, "node")?,
            },
            "patch" => Self::Patch {
                id: take(m, "id")?,
                patch: take(m, "patch")?,
            },
            "delete" => Self::Delete { id: take(m, "id")? },
            "link" => Self::Link(link(m)?),
            "unlink" => Self::Unlink(link(m)?),
            _ => bail!("unknown mutation {op}"),
        };
        ensure!(
            map.is_empty(),
            "unknown mutation fields: {:?}",
            map.keys().collect::<Vec<_>>()
        );
        Ok(result)
    }
}

impl Mutation {
    pub fn run(self, writer: &mut Write<'_>) -> Result<Value> {
        Ok(match self {
            Self::Create { node } => json!({"id":writer.create(node)?}),
            Self::Replace { id, node } => {
                writer.replace(id, node)?;
                json!({"updated":true})
            }
            Self::Patch { id, patch } => {
                writer.patch(id, patch)?;
                json!({"updated":true})
            }
            Self::Delete { id } => json!({"deleted":writer.delete(id)?}),
            Self::Link(link) => json!({"created":writer.link(&link)?}),
            Self::Unlink(link) => json!({"deleted":writer.unlink(&link)?}),
        })
    }
}
