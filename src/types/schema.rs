use crate::store::{get_node, link_refs};
use crate::{Direction, Link, LinkOptions, Node, NodeId, Predicate, Store, Write};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};

/// Where a first-level field reads from.
#[derive(Debug, Clone, Copy)]
pub enum Source {
    /// A JSON Pointer into attrs.
    Attr(&'static str),
    /// Links of one relation; `many` keeps all, otherwise the first.
    Link {
        relation: &'static str,
        direction: Direction,
        many: bool,
    },
}

#[derive(Debug)]
pub struct Field {
    pub name: &'static str,
    pub source: Source,
}

/// A derived type: the Node `type` it covers and its first-level fields.
#[derive(Debug)]
pub struct Schema {
    pub kind: &'static str,
    pub fields: &'static [Field],
}

/// A Node read through a Schema: id, type, summary and every field, flat.
#[derive(Debug, Clone)]
pub struct View {
    pub node: Node,
    pub fields: Map<String, Value>,
}

impl Schema {
    pub fn field(&self, name: &str) -> Result<&Field> {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .with_context(|| format!("{} has no field {name}", self.kind))
    }

    /// Every Node of this type.
    pub fn all(&self) -> Predicate {
        Predicate::Eq {
            field: "@type".into(),
            value: json!(self.kind),
        }
    }

    pub fn read(&self, db: &mut Store, id: NodeId, options: &LinkOptions) -> Result<Option<View>> {
        Ok(self.read_many(db, &[id], options)?.pop().flatten())
    }

    /// Resolves every field of every id in one read snapshot.
    pub fn read_many(
        &self,
        db: &mut Store,
        ids: &[NodeId],
        options: &LinkOptions,
    ) -> Result<Vec<Option<View>>> {
        let tx = db.conn.transaction()?;
        let views = ids
            .iter()
            .map(|id| self.resolve(&tx, *id, options))
            .collect::<Result<Vec<_>>>()?;
        tx.commit()?;
        Ok(views)
    }

    fn resolve(
        &self,
        conn: &Connection,
        id: NodeId,
        options: &LinkOptions,
    ) -> Result<Option<View>> {
        let Some(node) = get_node(conn, id)? else {
            return Ok(None);
        };
        ensure!(
            node.kind == self.kind,
            "node {id} is a {}, not a {}",
            node.kind,
            self.kind
        );
        let fields = resolve_fields(
            conn,
            &node,
            self.fields.iter().map(|f| {
                (
                    f.name,
                    match f.source {
                        Source::Attr(p) => Src::Attr(p),
                        Source::Link { relation, direction, many } => Src::Link { relation, direction, many },
                    },
                )
            }),
            options,
        )?;
        Ok(Some(View { node, fields }))
    }

    /// Sets a link field: out fields link from `id`, in fields link to it.
    pub fn link(&self, w: &mut Write, id: NodeId, name: &str, other: NodeId) -> Result<bool> {
        let Source::Link {
            relation,
            direction,
            ..
        } = self.field(name)?.source
        else {
            bail!("{name} is not a link field of {}", self.kind);
        };
        let (from, to) = match direction {
            Direction::Out => (id, other),
            Direction::In => (other, id),
        };
        w.link(&Link {
            from,
            relation: relation.into(),
            to,
        })
    }
}

#[derive(Clone, Copy)]
enum Src<'a> {
    Attr(&'a str),
    Link { relation: &'a str, direction: Direction, many: bool },
}

fn resolve_fields<'a>(
    conn: &Connection,
    node: &Node,
    fields: impl Iterator<Item = (&'a str, Src<'a>)>,
    options: &LinkOptions,
) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for (name, source) in fields {
        let value = match source {
            Src::Attr(pointer) => node.attrs.pointer(pointer).cloned().unwrap_or(Value::Null),
            Src::Link { relation, direction, many } => {
                let limit = if many { options.limit } else { 1 };
                let links = LinkOptions { mode: options.mode, limit };
                let (refs, _) = link_refs(conn, node.id, direction, Some(relation), &links)?;
                match many {
                    true => serde_json::to_value(refs)?,
                    false => refs.first().map_or(Ok(Value::Null), serde_json::to_value)?,
                }
            }
        };
        out.insert(name.into(), value);
    }
    Ok(out)
}

/// A runtime-defined derived type, stored in the database by `define_type`.
/// One field is either an attrs path (`attr`) or a link relation (`link`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeDef {
    pub kind: String,
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    #[serde(default)]
    pub attr: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub many: bool,
}

impl TypeDef {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.kind.is_empty(), "type kind is required");
        let mut names = std::collections::HashSet::new();
        for f in &self.fields {
            ensure!(names.insert(&f.name), "duplicate field {}", f.name);
            match (&f.attr, &f.link) {
                (Some(p), None) => ensure!(p.starts_with('/'), "attr must be a JSON pointer: {p}"),
                (None, Some(r)) => ensure!(!r.is_empty(), "link relation must not be empty"),
                _ => bail!("field {} needs exactly one of attr / link", f.name),
            }
        }
        Ok(())
    }

    /// Reads Node `id` through this runtime definition.
    pub fn read(&self, db: &mut Store, id: NodeId, options: &LinkOptions) -> Result<Option<View>> {
        let tx = db.conn.transaction()?;
        let view = self.resolve(&tx, id, options)?;
        tx.commit()?;
        Ok(view)
    }

    fn resolve(&self, conn: &Connection, id: NodeId, options: &LinkOptions) -> Result<Option<View>> {
        let Some(node) = get_node(conn, id)? else {
            return Ok(None);
        };
        ensure!(
            node.kind == self.kind,
            "node {id} is a {}, not a {}",
            node.kind,
            self.kind
        );
        let fields = resolve_fields(
            conn,
            &node,
            self.fields.iter().map(|f| {
                (
                    f.name.as_str(),
                    match (&f.attr, &f.link) {
                        (Some(p), None) => Src::Attr(p.as_str()),
                        (None, Some(r)) => Src::Link { relation: r.as_str(), direction: f.direction, many: f.many },
                        _ => unreachable!("validated at define_type"),
                    },
                )
            }),
            options,
        )?;
        Ok(Some(View { node, fields }))
    }
}

impl View {
    /// One field deserialized into the consumer's type.
    pub fn get<T: DeserializeOwned>(&self, name: &str) -> Result<T> {
        let value = self
            .fields
            .get(name)
            .with_context(|| format!("no field {name}"))?;
        serde_json::from_value(value.clone()).with_context(|| format!("field {name}"))
    }
}

impl Serialize for View {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut flat = Map::new();
        flat.insert("id".into(), json!(self.node.id));
        flat.insert("version".into(), json!(self.node.version));
        flat.insert("type".into(), json!(self.node.kind));
        flat.insert("summary".into(), json!(self.node.summary));
        flat.extend(self.fields.clone());
        flat.serialize(serializer)
    }
}
