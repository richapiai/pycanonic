//! Resolves a grammar-valid spec against the canonical schema into a `Plan`. Every error names the
//! spec field, and resolution stops at the first one. Port of canonic/packages/canonic/src/resolve.ts
//! and the compile entry point of artifact.ts.

use crate::canonical::{Empty, Kind, Node, load_schema};
use crate::plan::{
    self, CompileError, Field, Format, ListKind, ListOps, Meta, Named, Plan, Reject, RoutingField,
    Scalar, TxRef,
};
use crate::spec::{
    ArrayRule, FieldMap, Rule, ScalarRule, Source, SourceItem, Step, TransformRef, field_key,
    is_group, parse_spec,
};
use crate::transforms::{ArgError, Builtin, builtin, language_2_only};
use serde_json::Value;
use std::collections::HashMap;

/// `JSON.stringify` of a string.
fn json(s: &str) -> String {
    Value::from(s).to_string()
}

struct Resolver<'c> {
    custom: &'c [String],
    /// Language 2: the spec's definitions, inlined where an item map names one.
    definitions: &'c [(String, FieldMap)],
    /// The spec language. Only language 2 reserves `$group` and `$input`.
    language: u8,
    /// A scalar reads `$input` (language 2).
    context: bool,
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

    /// `sources` is how many values the first transform sees (`source_count`).
    fn tx(&self, refs: &[TransformRef], at: &str, sources: usize) -> Result<Vec<TxRef>, String> {
        let tx: Vec<TxRef> = refs
            .iter()
            .map(|t| {
                // An injected transform replaces a default of the same name, and its argument is its own business.
                if let Some(index) = self.custom.iter().position(|c| *c == t.name) {
                    let (name, arg) = (t.name.clone(), t.arg.clone());
                    return Ok(TxRef::Custom { index, name, arg });
                }
                // Language 1 does not have the language 2 transforms, nor parse_date's formats.
                let found = match builtin(&t.name, t.arg.as_ref()) {
                    _ if self.language == 1 && language_2_only(&t.name) => {
                        Err(ArgError::UnknownTransform)
                    }
                    Ok(b) if self.language == 1 && b.is_language_2() => {
                        Err(ArgError::InvalidArgument)
                    }
                    found => found,
                };
                found.map(TxRef::Builtin).map_err(|e| match e {
                    ArgError::UnknownTransform => {
                        format!("{at}: unknown transform {}", json(&t.name))
                    }
                    ArgError::InvalidArgument if arg_example(t, self.language).is_some() => {
                        let got = t.arg.as_ref().map_or("none".into(), |a| a.to_string());
                        let want = arg_example(t, self.language).unwrap_or_default();
                        format!(
                            "{at}: invalid argument for {}: got {got}, expected {want}",
                            t.name
                        )
                    }
                    ArgError::InvalidArgument => {
                        format!("{at}: invalid argument for {}", t.name)
                    }
                })
            })
            .collect::<Result<_, _>>()?;
        source_args(&tx, sources, at)?;
        Ok(tx)
    }

    /// The plan's source for `source`. A computed entry is a scalar rule with its own source count
    /// and its transforms resolved in place. `at` is the location of the `from`.
    fn plan_source(&self, source: &Source, at: &str) -> Result<plan::Source, String> {
        Ok(match source {
            Source::From(p) => plan::Source::From(p.clone()),
            Source::FromMany(ps) => plan::Source::FromMany(ps.clone()),
            Source::Coalesce(ps) => plan::Source::Coalesce(ps.clone()),
            Source::CoalesceWhen(es) => plan::Source::CoalesceWhen(es.clone()),
            Source::Computed(items) => plan::Source::Computed(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| match item {
                        SourceItem::Path(p) => Ok(plan::Item::Path(p.clone())),
                        SourceItem::Rule(r) => {
                            let at = format!("{at}[{i}]");
                            Ok(plan::Item::Rule(Box::new(plan::Computed {
                                tx: self.tx(&r.transform, &at, source_count(&r.source))?,
                                from: self.plan_source(&r.source, &format!("{at}.from"))?,
                            })))
                        }
                    })
                    .collect::<Result<_, String>>()?,
            ),
        })
    }

    fn scalar(
        &mut self,
        r: &ScalarRule,
        node: &Node,
        stat: usize,
        at: &str,
    ) -> Result<Scalar, String> {
        let (semantic, routing) = match &node.kind {
            Kind::Scalar {
                semantic, routing, ..
            } => (*semantic, *routing),
            // An open map is copied as it is, like a scalar.
            Kind::Object { open: true, .. } => (None, false),
            Kind::Array { .. } => return Err(format!("{at}: canonical field is an array; add []")),
            Kind::Object { .. } => return Err(format!("{at}: canonical field is an object")),
        };
        let tx = self.tx(&r.transform, at, source_count(&r.source))?;
        let plan_source = self.plan_source(&r.source, &format!("{at}.from"))?;
        // The validator step every routing field ends with.
        let validator = if routing {
            if matches!(r.source, Source::Computed(_)) {
                return Err(format!("{at}: routing fields can't read a computed source"));
            }
            // `extract` has no conditions.
            if matches!(&r.source, Source::CoalesceWhen(es) if es.iter().any(|e| e.1.is_some())) {
                return Err(format!("{at}: routing fields can't have when"));
            }
            let paths = r.source.paths();
            if paths.iter().any(|p| p.is_empty()) {
                return Err(format!(
                    "{at}: routing fields can't read the whole response ($)"
                ));
            }
            // `extract` has no context.
            let input = Step::Key("$input".into());
            if self.language == 2 && paths.iter().any(|p| p.first() == Some(&input)) {
                return Err(format!("{at}: routing fields can't read $input"));
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
        let input = Step::Key("$input".into());
        self.context |=
            self.language == 2 && r.source.paths().iter().any(|p| p.first() == Some(&input));
        let default = r.default.clone();
        Ok(Scalar {
            source: plan_source,
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
        let defs = self.definitions;
        let item = r.item.as_ref().or_else(|| {
            let name = r.item_ref.as_ref()?;
            defs.iter().find(|(n, _)| n == name).map(|(_, m)| m)
        });
        let kind = if let Some(item) = item {
            let Some(children) = object else {
                return Err(format!("{at}: array of scalars; use pluck, or from alone"));
            };
            let prefix = format!("{}.", self.stat_names[stat]);
            ListKind::Item(self.object(children, item, Some(&prefix), &format!("{at}.item"))?)
        } else if object.is_some() {
            return Err(format!("{at}: array of objects; use item"));
        } else {
            let tx = self.tx(&r.transform, at, 1)?;
            match &r.pluck {
                Some(p) => ListKind::Pluck(p.clone(), tx),
                None => ListKind::Plain(tx),
            }
        };
        // The spec's option first, then the schema's `x-empty`, then language 1 behavior.
        let ops = ListOps {
            missing: r.missing.or(node.empty).unwrap_or(Empty::Null),
            empty: r.empty.unwrap_or(Empty::List),
            wrap: r.wrap,
            flatten: r.flatten.clone(),
            filter: r.filter.clone(),
            unique: r.unique.clone(),
            sort: r.sort.clone(),
            limit: r.limit,
            group: r.group.clone(),
            from_group: self.language == 2 && is_group(&r.from),
        };
        let input = Step::Key("$input".into());
        let conds = r.coalesce.iter().filter_map(|e| e.1.as_ref());
        self.context |= conds
            .flat_map(|c| c.paths())
            .any(|p| p.first() == Some(&input));
        Ok(Field::List {
            from: r.from.clone(),
            kind,
            stat,
            ops,
            coalesce: r.coalesce.clone(),
        })
    }

    /// Language 2: `{"from": <path>, "transform": <one custom transform>}` on an array or
    /// object node hands the transform the whole value. `None` for any other rule.
    fn container(
        &mut self,
        rule: &Rule,
        node: &Node,
        stat: usize,
        at: &str,
    ) -> Result<Option<Field>, String> {
        if self.language != 2 || matches!(node.kind, Kind::Scalar { .. }) {
            return Ok(None);
        }
        let (from, tx, ops) = match rule {
            Rule::Scalar(ScalarRule {
                source: Source::From(p),
                default: None,
                transform,
            }) => (p, transform, false),
            Rule::Array(r) if r.item.is_none() && r.item_ref.is_none() && r.pluck.is_none() => {
                let ops = !r.coalesce.is_empty()
                    || r.missing.is_some()
                    || r.empty.is_some()
                    || r.wrap
                    || !r.flatten.is_empty()
                    || r.filter.is_some()
                    || r.unique.is_some()
                    || r.sort.is_some()
                    || r.limit.is_some()
                    || r.group.is_some();
                (&r.from, &r.transform, ops)
            }
            _ => return Ok(None),
        };
        let [t] = tx.as_slice() else {
            return Ok(None);
        };
        let tx = match self.tx(std::slice::from_ref(t), at, 1)?.remove(0) {
            tx @ TxRef::Custom { .. } => tx,
            TxRef::Builtin(_) => return Ok(None),
        };
        if ops {
            return Err(format!(
                "{at}: a container transform takes only from and transform"
            ));
        }
        let input = Step::Key("$input".into());
        self.context |= from.first() == Some(&input);
        let (from, node) = (from.clone(), node.clone());
        Ok(Some(Field::Container {
            from,
            tx,
            node,
            stat,
        }))
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
            let (rule, when) = match rule {
                Rule::When { when, omit, rule } => (&**rule, Some((when, *omit))),
                rule => (rule, None),
            };
            let path = field_key(key).expect("the grammar checked the key").0;
            // `extract` reads a routing field below an object whatever its `when`.
            if when.is_some() && matches!(rule, Rule::Object { .. }) {
                let routed = map.iter().any(|(k, _)| {
                    let p = field_key(k).map(|(p, _)| p).unwrap_or_default();
                    let node = (p.len() > path.len() && p.starts_with(&path))
                        .then(|| lookup(children, &p))
                        .flatten();
                    matches!(
                        node.map(|n| &n.kind),
                        Some(Kind::Scalar { routing: true, .. })
                    )
                });
                if routed {
                    return Err(format!("{kat}: routing fields can't have when"));
                }
            }
            if path.is_empty() {
                return Err(format!("{kat}: only an array root has the field \"[]\""));
            }
            let target = lookup(children, &path).ok_or_else(|| {
                let name = key.strip_suffix("[]").unwrap_or(key);
                format!("{kat}: canonical schema has no field {}", json(name))
            })?;
            let stat = self.stat_names.len();
            // An object rule reads no value of its own, so it has no stat slot.
            if !matches!(rule, Rule::Object { .. }) {
                // No dot before a quoted key: `["@graph"][]["@id"]`, as the path grammar writes it.
                let within = within.unwrap_or("");
                let within = match key.starts_with('[') {
                    true => within.strip_suffix('.').unwrap_or(within),
                    false => within,
                };
                self.stat_names.push(format!("{within}{key}"));
            }
            // The grammar puts an array rule exactly on the keys ending in [].
            let field = match rule {
                Rule::Const(_)
                    if key.ends_with("[]") && !matches!(target.kind, Kind::Array { .. }) =>
                {
                    return Err(format!("{kat}: canonical field is not an array"));
                }
                Rule::Const(v) => Field::Const(v.clone(), stat),
                Rule::Object {
                    present,
                    if_empty_null,
                } => {
                    let open = !matches!(target.kind, Kind::Object { open: false, .. });
                    if key.ends_with("[]") || open {
                        let rule = match (present.is_empty(), if_empty_null) {
                            (false, _) => "present",
                            (true, true) => "if_empty",
                            (true, false) => "when",
                        };
                        return Err(format!("{kat}: {rule} needs an object node"));
                    }
                    // `build` fills the members.
                    Field::Object {
                        members: Vec::new(),
                        present: present.clone(),
                        if_empty: if_empty_null.then_some(Empty::Null),
                    }
                }
                Rule::Array(r) => match self.container(rule, target, stat, &kat)? {
                    Some(f) => f,
                    None => self.array(r, target, stat, &kat)?,
                },
                Rule::Aggregate(a) => match &target.kind {
                    Kind::Scalar { routing: true, .. } => {
                        return Err(format!("{kat}: routing fields can't be aggregates"));
                    }
                    Kind::Scalar { .. } | Kind::Object { open: true, .. } => {
                        Field::Aggregate(a.clone(), stat)
                    }
                    Kind::Array { .. } => {
                        return Err(format!("{kat}: canonical field is an array; add []"));
                    }
                    Kind::Object { .. } => {
                        return Err(format!("{kat}: canonical field is an object"));
                    }
                },
                Rule::Scalar(r) => match self.container(rule, target, stat, &kat)? {
                    Some(f) => f,
                    None => Field::Scalar(self.scalar(r, target, stat, &kat)?),
                },
                Rule::When { .. } => unreachable!("spec::rule wraps a rule once"),
            };
            if within.is_none() {
                let scalar = match &field {
                    Field::Scalar(s) => Some(s.clone()),
                    _ => None,
                };
                self.top.insert(key.clone(), scalar);
            }
            // `extract` reads a routing field whatever the condition.
            if when.is_some() && matches!(&field, Field::Scalar(s) if s.validator.is_some()) {
                return Err(format!("{kat}: routing fields can't have when"));
            }
            let field = match when {
                Some((when, omit)) => Field::When {
                    when: when.clone(),
                    omit,
                    field: Box::new(field),
                },
                None => field,
            };
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

    /// Language 2: the one field `"[]"` of an array root, an array rule over the whole output.
    fn array_root(&mut self, node: &Node, map: &FieldMap) -> Result<Vec<Named<Field>>, String> {
        let rule = match map.as_slice() {
            [(key, rule)] if key == "[]" => rule,
            _ => return Err(r#"fields: an array root needs exactly the field "[]""#.into()),
        };
        let at = r#"fields["[]"]"#;
        let Rule::Array(a) = rule else {
            return Err(format!(
                "{at}: an array root needs an array rule without when"
            ));
        };
        let stat = self.stat_names.len();
        self.stat_names.push("[]".into());
        let value = match self.container(rule, node, stat, at)? {
            Some(f) => f,
            None => self.array(a, node, stat, at)?,
        };
        let name_idx = self.intern("[]");
        Ok(vec![Named {
            name: "[]".into(),
            name_idx,
            value,
        }])
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
                // an object that writes nothing but null follows its `x-empty`. The
                // spec's `if_empty` wins. `missing` is the `OmitNull` below.
                let x_empty = child
                    .empty
                    .filter(|e| matches!(e, Empty::Null | Empty::Object));
                // A `when` wraps the field once it is built.
                let (field, when) = match mapped.remove(&path) {
                    Some(Field::When { when, omit, field }) => (Some(*field), Some((when, omit))),
                    f => (f, None),
                };
                let value = match (field, &child.kind) {
                    (
                        Some(Field::Object {
                            present, if_empty, ..
                        }),
                        Kind::Object { children: c, .. },
                    ) => Field::Object {
                        members: self.build(c, &path, mapped),
                        present,
                        if_empty: if_empty.or(x_empty),
                    },
                    (Some(f), _) => f,
                    // a node with `x-empty` that nothing maps follows it, as a missing
                    // source does. Without `x-empty`, an object keeps its null leaves.
                    (
                        None,
                        Kind::Object {
                            children: c,
                            open: false,
                        },
                    ) if child.empty.is_none() || mapped.keys().any(|k| k.starts_with(&path)) => {
                        Field::Object {
                            members: self.build(c, &path, mapped),
                            present: Vec::new(),
                            if_empty: x_empty,
                        }
                    }
                    (None, _) => match child.empty {
                        Some(e @ (Empty::List | Empty::Object)) => Field::Empty(e),
                        _ => Field::Null,
                    },
                };
                let value = match when {
                    Some((when, omit)) => Field::When {
                        when,
                        omit,
                        field: Box::new(value),
                    },
                    None => value,
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
/// How many values a scalar rule's source gives its first transform. A `coalesce` gives one.
pub(crate) fn source_count(source: &Source) -> usize {
    match source {
        Source::From(_) | Source::Coalesce(_) | Source::CoalesceWhen(_) => 1,
        Source::FromMany(ps) => ps.len(),
        Source::Computed(items) => items.len(),
    }
}

/// R51 and R53: a number, compare or `format` transform sees the `from` list when it is first in
/// line. Every other step gets one value. This is the only place that words these errors.
fn source_args(tx: &[TxRef], sources: usize, at: &str) -> Result<(), String> {
    let fail = |name: &str, got: String, want: String| {
        Err(format!(
            "{at}: invalid argument for {name}: got {got}, expected {want}"
        ))
    };
    for (i, t) in tx.iter().enumerate() {
        let on_list = i == 0 && sources > 1;
        let TxRef::Builtin(b) = t else {
            continue;
        };
        if let Builtin::Format(tpl) = b {
            let seen = if on_list { sources } else { 1 };
            if (on_list && tpl.uses_next()) || tpl.max_slot().is_some_and(|n| n >= seen) {
                let got = json(&tpl.text());
                let want = if on_list {
                    let slots: Vec<String> = (0..sources).map(|n| format!("{{{n}}}")).collect();
                    format!("{{\"format\": \"{}\"}}", slots.join("/"))
                } else {
                    "{\"format\": \"item:{}\"}".into()
                };
                return fail("format", got, want);
            }
            continue;
        }
        let Builtin::Num(op, k) = b else {
            continue;
        };
        let name = op.name();
        match (on_list, k) {
            (true, Some(k)) => {
                return fail(
                    name,
                    k.to_string(),
                    "no argument on a from list, or one source in from".into(),
                );
            }
            (false, None) if sources > 1 => {
                return fail(
                    name,
                    "none".into(),
                    format!("a number, as in {{\"{name}\": 10}}"),
                );
            }
            (false, None) => {
                return fail(
                    name,
                    "none".into(),
                    format!("two or more sources in from, or {{\"{name}\": 10}}"),
                );
            }
            _ => {}
        }
    }
    Ok(())
}

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
            Field::Object { members, .. } => routing(members, stat_names, out),
            Field::Scalar(s) if s.validator.is_some() => {
                let (name, scalar) = (stat_names[s.stat].clone(), s.clone());
                out.push(RoutingField {
                    name,
                    scalar,
                    anchor: None,
                });
            }
            Field::Scalar(_)
            | Field::Null
            | Field::List { .. }
            | Field::Const(..)
            | Field::Aggregate(..)
            | Field::Container { .. }
            | Field::Empty(_) => {}
            Field::OmitNull(_) | Field::When { .. } => unreachable!("inner() unwraps it"),
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
    let plan::Source::From(p) = source else {
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

/// The definition names that `map` refers to, in nested item maps too, not through a reference.
fn refs<'m>(map: &'m FieldMap, out: &mut Vec<&'m str>) {
    for (_, r) in map {
        let r = match r {
            Rule::When { rule, .. } => &**rule,
            r => r,
        };
        if let Rule::Array(a) = r {
            out.extend(a.item_ref.as_deref());
            if let Some(m) = &a.item {
                refs(m, out);
            }
        }
    }
}

/// Language 2: every name a spec refers to is a definition, and no definition refers to
/// itself through others.
fn check_definitions(fields: &FieldMap, defs: &[(String, FieldMap)]) -> Result<(), String> {
    let get = |n: &str| defs.iter().find(|(k, _)| k == n).map(|(_, m)| m);
    let mut all = Vec::new();
    refs(fields, &mut all);
    defs.iter().for_each(|(_, m)| refs(m, &mut all));
    if let Some(n) = all.iter().find(|n| get(n).is_none()) {
        return Err(format!("definitions: no definition {}", json(n)));
    }
    // A depth-first walk. `trail` is the chain of names to here, `done` the names with no cycle.
    fn walk<'m>(
        name: &'m str,
        get: &impl Fn(&str) -> Option<&'m FieldMap>,
        trail: &mut Vec<&'m str>,
        done: &mut Vec<&'m str>,
    ) -> Result<(), String> {
        if let Some(i) = trail.iter().position(|t| *t == name) {
            let chain: Vec<String> = trail[i..].iter().chain([&name]).map(|n| json(n)).collect();
            return Err(format!("definitions: cycle through {}", chain.join(" -> ")));
        }
        if done.contains(&name) {
            return Ok(());
        }
        trail.push(name);
        let mut next = Vec::new();
        refs(get(name).expect("checked above"), &mut next);
        for n in next {
            walk(n, get, trail, done)?;
        }
        trail.pop();
        done.push(name);
        Ok(())
    }
    let mut done = Vec::new();
    for (name, _) in defs {
        walk(name, &get, &mut Vec::new(), &mut done)?;
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
        definitions: &s.definitions,
        language: s.language,
        context: false,
        stat_names: Vec::new(),
        names: Vec::new(),
        top: HashMap::new(),
    };
    let resolved = (|| {
        check_definitions(&s.fields, &s.definitions)?;
        let fields = match &canonical.array {
            Some(node) => r.array_root(node, &s.fields)?,
            None => r.object(&canonical.root, &s.fields, None, "fields")?,
        };
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
        context: r.context,
        reject: s
            .reject
            .into_iter()
            .map(|(cond, reason)| Reject { cond, reason })
            .collect(),
        case_insensitive: s.case_insensitive,
        array: canonical.array.is_some(),
    };
    if plan.needs_v2() {
        plan.format = Format(2);
    }
    Ok(plan)
}

/// The valid example for a transform whose language 2 argument error says what was given (R61).
/// `split_name` only gets it for an object argument, so the string form keeps its old text.
fn arg_example(t: &TransformRef, language: u8) -> Option<&'static str> {
    match t.name.as_str() {
        "format" => Some(r#"{"format": "urn:example:item:{}"}"#),
        "url_part" => Some(r#"{"url_part": {"query": "ref"}}"#),
        "split_name" if language == 2 && t.arg.as_ref().is_some_and(|a| a.is_object()) => {
            Some(r#"{"split_name": {"part": "last", "mode": "first_word"}}"#)
        }
        _ => None,
    }
}
