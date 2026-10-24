//! Resolves a grammar-valid spec against the canonical schema into a `Plan`. Every error names the
//! spec field, and resolution stops at the first one. Port of canonic/packages/canonic/src/resolve.ts
//! and the compile entry point of artifact.ts.

use crate::canonical::{Empty, Kind, Node, load_schema};
use crate::plan::{
    CompileError, Field, Format, ListKind, ListOps, Meta, Named, Plan, RoutingField, Scalar, TxRef,
};
use crate::spec::{
    ArrayRule, FieldMap, Rule, ScalarRule, Source, Step, TransformRef, field_key, parse_spec,
};
use crate::transforms::{ArgError, builtin};
use serde_json::Value;
use std::collections::HashMap;

/// `JSON.stringify` of a string.
fn json(s: &str) -> String {
    Value::from(s).to_string()
}

struct Resolver<'c> {
    custom: &'c [String],
    stat_names: Vec<String>,
    names: Vec<String>,
    /// Top-level spec key to its scalar, `None` for an array rule. Anchors look fields up here.
    top: HashMap<String, Option<Scalar>>,
}

impl Resolver<'_> {
    fn intern(&mut self, name: &str) -> usize {
        self.names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| {
                self.names.push(name.to_string());
                self.names.len() - 1
            })
    }

    fn tx(&self, refs: &[TransformRef], at: &str) -> Result<Vec<TxRef>, String> {
        refs.iter()
            .map(|t| {
                // An injected transform replaces a default of the same name, and its argument is its own business.
                if let Some(index) = self.custom.iter().position(|c| *c == t.name) {
                    let (name, arg) = (t.name.clone(), t.arg.clone());
                    return Ok(TxRef::Custom { index, name, arg });
                }
                builtin(&t.name, t.arg.as_ref())
                    .map(TxRef::Builtin)
                    .map_err(|e| match e {
                        ArgError::UnknownTransform => {
                            format!("{at}: unknown transform {}", json(&t.name))
                        }
                        ArgError::InvalidArgument => {
                            format!("{at}: invalid argument for {}", t.name)
                        }
                    })
            })
            .collect()
    }

    fn scalar(&self, r: &ScalarRule, node: &Node, stat: usize, at: &str) -> Result<Scalar, String> {
        let (semantic, routing) = match &node.kind {
            Kind::Scalar {
                semantic, routing, ..
            } => (*semantic, *routing),
            // An open map is copied as it is, like a scalar.
            Kind::Object { open: true, .. } => (None, false),
            Kind::Array { .. } => return Err(format!("{at}: canonical field is an array; add []")),
            Kind::Object { .. } => return Err(format!("{at}: canonical field is an object")),
        };
        let tx = self.tx(&r.transform, at)?;
        // The validator step every routing field ends with.
        let validator = if routing {
            let paths = match &r.source {
                Source::From(p) => std::slice::from_ref(p),
                Source::FromMany(ps) | Source::Coalesce(ps) => ps.as_slice(),
            };
            if paths.iter().any(Vec::is_empty) {
                return Err(format!(
                    "{at}: routing fields can't read the whole response ($)"
                ));
            }
            // A default skips the validator and turns every response into a hit.
            if r.default.is_some() {
                return Err(format!("{at}: routing fields can't have a default"));
            }
            Some(semantic.ok_or_else(|| {
                format!(
                    "{at}: routing field needs a semantic with a validator (email, phone, domain, linkedin_url, url, iso_date, money)"
                )
            })?)
        } else {
            None
        };
        let (source, default) = (r.source.clone(), r.default.clone());
        Ok(Scalar {
            source,
            tx,
            validator,
            default,
            stat,
        })
    }

    fn array(
        &mut self,
        r: &ArrayRule,
        node: &Node,
        stat: usize,
        at: &str,
    ) -> Result<Field, String> {
        let Kind::Array { items, nested } = &node.kind else {
            return Err(format!("{at}: canonical field is not an array"));
        };
        if *nested && (r.item.is_some() || r.pluck.is_some()) {
            return Err(format!("{at}: arrays of arrays can only be copied whole"));
        }
        let object = match items.as_deref() {
            Some(Node {
                kind:
                    Kind::Object {
                        children,
                        open: false,
                    },
                ..
            }) => Some(children),
            _ => None,
        };
        let kind = if let Some(item) = &r.item {
            let Some(children) = object else {
                return Err(format!("{at}: array of scalars; use pluck, or from alone"));
            };
            let prefix = format!("{}.", self.stat_names[stat]);
            ListKind::Item(self.object(children, item, Some(&prefix), &format!("{at}.item"))?)
        } else if object.is_some() {
            return Err(format!("{at}: array of objects; use item"));
        } else {
            let tx = self.tx(&r.transform, at)?;
            match &r.pluck {
                Some(p) => ListKind::Pluck(p.clone(), tx),
                None => ListKind::Plain(tx),
            }
        };
        // The spec's option first, then the schema's `x-empty`, then language 1 behavior.
        let ops = ListOps {
            missing: r.missing.or(node.empty).unwrap_or(Empty::Null),
            empty: r.empty.unwrap_or(Empty::List),
        };
        Ok(Field::List {
            from: r.from.clone(),
            kind,
            stat,
            ops,
        })
    }

    /// Resolves `map` against an object's children. Inside an item, `within` prefixes stat names.
    fn object(
        &mut self,
        children: &[(String, Node)],
        map: &FieldMap,
        within: Option<&str>,
        at: &str,
    ) -> Result<Vec<Named<Field>>, String> {
        let mut mapped = HashMap::new();
        for (key, rule) in map {
            let kat = format!("{at}[{}]", json(key));
            let path = field_key(key).expect("the grammar checked the key").0;
            let target = lookup(children, &path).ok_or_else(|| {
                let name = key.strip_suffix("[]").unwrap_or(key);
                format!("{kat}: canonical schema has no field {}", json(name))
            })?;
            let stat = self.stat_names.len();
            self.stat_names
                .push(format!("{}{key}", within.unwrap_or("")));
            // The grammar puts an array rule exactly on the keys ending in [].
            let field = match rule {
                Rule::Const(_)
                    if key.ends_with("[]") && !matches!(target.kind, Kind::Array { .. }) =>
                {
                    return Err(format!("{kat}: canonical field is not an array"));
                }
                Rule::Const(v) => Field::Const(v.clone(), stat),
                Rule::Array(r) => self.array(r, target, stat, &kat)?,
                Rule::Scalar(r) => Field::Scalar(self.scalar(r, target, stat, &kat)?),
            };
            if within.is_none() {
                let scalar = match &field {
                    Field::Scalar(s) => Some(s.clone()),
                    _ => None,
                };
                self.top.insert(key.clone(), scalar);
            }
            // After the rule, so a language 1 pair keeps the TypeScript message of its failing rule.
            if mapped.contains_key(&path) {
                return Err(format!("{kat}: canonical field is mapped twice"));
            }
            mapped.insert(path, field);
        }
        let built = self.build(children, &[], &mut mapped);
        // A field left over sits inside a const object, which `build` does not walk into.
        if let Some((key, _)) = map
            .iter()
            .find(|(k, _)| field_key(k).is_some_and(|(p, _)| mapped.contains_key(&p)))
        {
            return Err(format!("{at}[{}]: inside a const field", json(key)));
        }
        Ok(built)
    }

    /// Walks the canonical tree in schema order so every canonical field appears, null when unmapped.
    fn build(
        &mut self,
        children: &[(String, Node)],
        prefix: &[String],
        mapped: &mut HashMap<Vec<String>, Field>,
    ) -> Vec<Named<Field>> {
        children
            .iter()
            .map(|(name, child)| {
                let path = [prefix, std::slice::from_ref(name)].concat();
                let value = match (mapped.remove(&path), &child.kind) {
                    (Some(f), _) => f,
                    (
                        None,
                        Kind::Object {
                            children: c,
                            open: false,
                        },
                    ) => Field::Object(self.build(c, &path, mapped)),
                    (None, _) => Field::Null,
                };
                // A mapped array handles `x-empty: "missing"` itself (`ListOps::missing`).
                let value = match (child.empty, value) {
                    (Some(Empty::Missing), v) if !matches!(v, Field::List { .. }) => {
                        Field::OmitNull(Box::new(v))
                    }
                    (_, v) => v,
                };
                Named {
                    name: name.clone(),
                    name_idx: self.intern(name),
                    value,
                }
            })
            .collect()
    }
}

/// The node at a canonical path, through objects that are not open maps only.
fn lookup<'a>(mut children: &'a [(String, Node)], path: &[String]) -> Option<&'a Node> {
    let mut node: Option<&Node> = None;
    for part in path {
        if let Some(n) = node {
            let Kind::Object {
                children: c,
                open: false,
            } = &n.kind
            else {
                return None;
            };
            children = c;
        }
        node = Some(&children.iter().find(|(k, _)| k == part)?.1);
    }
    node
}

/// Routing fields in schema order, keyed by spec field. Routing fields are never inside arrays.
fn routing(fields: &[Named<Field>], stat_names: &[String], out: &mut Vec<RoutingField>) {
    for f in fields {
        // Every variant by name, so a new wrapper cannot hide a routing field.
        match f.value.inner() {
            Field::Object(c) => routing(c, stat_names, out),
            Field::Scalar(s) if s.validator.is_some() => {
                let (name, scalar) = (stat_names[s.stat].clone(), s.clone());
                out.push(RoutingField {
                    name,
                    scalar,
                    anchor: None,
                });
            }
            Field::Scalar(_) | Field::Null | Field::List { .. } | Field::Const(..) => {}
            Field::OmitNull(_) => unreachable!("inner() unwraps it"),
        }
    }
}

/// An anchor names the raw key token of a routing field read from one path, for example `"workEmail":`.
fn check_anchor(field: Option<&Option<Scalar>>, key: &str, token: &str) -> Result<(), String> {
    let at = format!("anchors[{}]", json(key));
    let Some(field) = field else {
        return Err(format!("{at}: not a field in this spec"));
    };
    let Some(Scalar {
        source,
        validator: Some(_),
        ..
    }) = field
    else {
        return Err(format!("{at}: only routing fields take anchors"));
    };
    let Source::From(p) = source else {
        return Err(format!(
            "{at}: anchored fields need a single from path ending in a key"
        ));
    };
    let Some(Step::Key(last)) = p.last() else {
        return Err(format!(
            "{at}: anchored fields need a single from path ending in a key"
        ));
    };
    let want = format!("{}:", json(last));
    if token != want {
        return Err(format!("{at}: token must be {}", json(&want)));
    }
    Ok(())
}

/// Validates the spec, loads the canonical schema and resolves one against the other.
/// `custom` names the deployment's own transforms. `TxRef::Custom::index` points into it.
pub fn compile(spec: &Value, schema: &Value, custom: &[String]) -> Result<Plan, CompileError> {
    let fail = |message| CompileError { message };
    let s = parse_spec(spec).map_err(|issues| {
        let all: Vec<String> = issues.iter().map(ToString::to_string).collect();
        fail(format!("invalid spec: {}", all.join("; ")))
    })?;
    let canonical = load_schema(schema, s.language).map_err(|i| fail(i.to_string()))?;
    let mut r = Resolver {
        custom,
        stat_names: Vec::new(),
        names: Vec::new(),
        top: HashMap::new(),
    };
    let resolved = (|| {
        let fields = r.object(&canonical.root, &s.fields, None, "fields")?;
        let mut routed = Vec::new();
        routing(&fields, &r.stat_names, &mut routed);
        let mut order = Vec::new();
        // In the spec's own key order, so the first error is the one canonic reports.
        let anchors = spec
            .get("anchors")
            .and_then(Value::as_object)
            .into_iter()
            .flatten();
        for (key, token) in anchors {
            let token = token.as_str().unwrap_or_default();
            check_anchor(r.top.get(key), key, token)?;
            if let Some(i) = routed.iter().position(|f| f.name == *key) {
                routed[i].anchor = Some(token.to_string());
                order.push(i);
            }
        }
        Ok::<_, String>((fields, routed, order))
    })();
    let (fields, routing, anchors) =
        resolved.map_err(|m| fail(format!("{} {} v{}: {m}", s.provider, s.endpoint, s.version)))?;
    let mut plan = Plan {
        format: Format(1),
        meta: Meta {
            provider: s.provider,
            endpoint: s.endpoint,
            version: s.version,
            status: s.status,
        },
        fields,
        routing,
        anchors,
        stat_names: r.stat_names,
        custom_names: custom.to_vec(),
        names: r.names,
    };
    if plan.needs_v2() {
        plan.format = Format(2);
    }
    Ok(plan)
}
