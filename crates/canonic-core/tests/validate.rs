// Port of canonic/packages/canonic/test/validate.test.ts (the validateSpec half. The compiler
// tests cover the compileSpec half). Fixtures are vendored from canonic/packages/canonic/test/fixtures.

use canonic_core::spec::{ArrayRule, Path, Rule, Source, Step, TransformRef, parse_path};
use canonic_core::{Issue, Spec, Status, parse_spec, validate_spec};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(p: PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(&p).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn schema() -> jsonschema::Validator {
    let s = read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec.schema.json"));
    jsonschema::validator_for(&s).unwrap()
}

fn ours(spec: &Value) -> bool {
    validate_spec(spec).is_empty()
}

fn constructs() -> Vec<Value> {
    let mut files: Vec<_> = fs::read_dir(fixtures().join("constructs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files.into_iter().map(|f| read(f)["spec"].clone()).collect()
}

fn invalid(kind: &str) -> Vec<(String, Value)> {
    let doc = read(fixtures().join("invalid.json"));
    doc[kind]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["name"].as_str().unwrap().to_string(), c["spec"].clone()))
        .collect()
}

fn valid() -> Vec<Value> {
    let mut v = constructs();
    v.extend(invalid("semantic").into_iter().map(|(_, s)| s));
    v
}

fn issues(spec: &Value) -> Vec<String> {
    validate_spec(spec)
        .iter()
        .map(|i| format!("{}: {}", i.at, i.message))
        .collect()
}

fn base(fields: Value) -> Value {
    json!({ "provider": "a", "endpoint": "e", "version": 1, "status": "draft", "fields": fields })
}

#[test]
fn valid_grammar() {
    let ajv = schema();
    let specs = valid();
    assert_eq!(specs.len(), 19 + 21);
    for (i, spec) in specs.iter().enumerate() {
        assert!(ajv.is_valid(spec), "schema rejects valid #{i}");
        assert_eq!(validate_spec(spec), vec![], "valid #{i}");
    }
}

#[test]
fn own_proto_key_is_rejected_by_both() {
    let spec: Value = serde_json::from_str(
        r#"{"provider":"a","endpoint":"e","version":1,"status":"draft","fields":{"email":{"__proto__":{"from":"x"}}}}"#,
    )
    .unwrap();
    assert_eq!((schema().is_valid(&spec), ours(&spec)), (false, false));
}

#[test]
fn rejects_invalid_grammar() {
    let ajv = schema();
    let cases = invalid("grammar");
    assert_eq!(cases.len(), 50);
    for (name, spec) in cases {
        assert!(!ajv.is_valid(&spec), "schema accepts: {name}");
        assert!(!ours(&spec), "we accept: {name}");
    }
}

#[test]
fn agrees_with_json_schema() {
    let ajv = schema();
    let mut specs = constructs();
    for kind in ["grammar", "semantic"] {
        specs.extend(invalid(kind).into_iter().map(|(_, s)| s));
    }
    for spec in specs {
        assert_eq!(ours(&spec), ajv.is_valid(&spec), "{spec}");
    }
}

// xorshift64: deterministic, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn string(&mut self) -> String {
        const CHARS: &[char] = &[
            'a', 'b', 'Z', '_', '$', '-', '.', '[', ']', '"', '\\', ' ', '0', '1', '\n', 'é',
        ];
        (0..self.below(7))
            .map(|_| CHARS[self.below(CHARS.len())])
            .collect()
    }
    fn json(&mut self, depth: u32) -> Value {
        match self.below(if depth == 0 { 6 } else { 8 }) {
            0 => Value::Null,
            1 => json!(self.below(2) == 0),
            2 => json!(self.below(5) as i64 - 1),
            3 => json!(self.below(100) as f64 / 7.0),
            4 | 5 => json!(self.string()),
            6 => Value::Array((0..self.below(3)).map(|_| self.json(depth - 1)).collect()),
            _ => Value::Object(
                (0..self.below(3))
                    .map(|_| (self.string(), self.json(depth - 1)))
                    .collect(),
            ),
        }
    }
    fn junk(&mut self) -> Value {
        let fixed = [
            json!("a..b"),
            json!("[0]"),
            json!("[\"x\"]"),
            json!("trim"),
            json!(["a", "b"]),
            json!({"cast": "number"}),
            json!({}),
        ];
        if self.below(2) == 0 {
            fixed[self.below(fixed.len())].clone()
        } else {
            self.json(2)
        }
    }
}

/// Replaces, deletes or adds one property somewhere in the tree (JSON pointers to every container).
fn mutate(root: &mut Value, pick: usize, key: String, value: Value) {
    fn walk(v: &Value, at: String, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                out.push(at.clone());
                for (k, c) in m {
                    walk(
                        c,
                        format!("{at}/{}", k.replace('~', "~0").replace('/', "~1")),
                        out,
                    );
                }
            }
            Value::Array(a) => {
                out.push(at.clone());
                for (i, c) in a.iter().enumerate() {
                    walk(c, format!("{at}/{i}"), out);
                }
            }
            _ => {}
        }
    }
    let mut nodes = vec![];
    walk(root, String::new(), &mut nodes);
    let node = root.pointer_mut(&nodes[pick % nodes.len()]).unwrap();
    match node {
        Value::Object(m) => {
            let target = if !m.is_empty() && !pick.is_multiple_of(3) {
                m.keys().nth(pick % m.len()).unwrap().clone()
            } else {
                key
            };
            if pick.is_multiple_of(5) {
                m.shift_remove(&target);
            } else {
                m.insert(target, value);
            }
        }
        // A non-index key on a JS array is invisible to both validators, so only indices matter here.
        Value::Array(a) if !a.is_empty() && !pick.is_multiple_of(3) => {
            let i = pick % a.len();
            if pick.is_multiple_of(5) {
                a.remove(i);
            } else {
                a[i] = value;
            }
        }
        _ => {}
    }
}

#[test]
fn agree_on_5000_mutated_specs() {
    let ajv = schema();
    let specs = valid();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for run in 0..5000 {
        let mut s = specs[rng.below(specs.len())].clone();
        for _ in 0..1 + rng.below(3) {
            let (pick, key, value) = (rng.below(1 << 20), rng.string(), rng.junk());
            mutate(&mut s, pick, key, value);
        }
        assert_eq!(ours(&s), ajv.is_valid(&s), "run {run}: {s}");
    }
}

#[test]
fn reports_at_and_message_like_the_docs() {
    let spec = json!({
        "provider": "a", "endpoint": "e", "version": 1, "status": "live",
        "fields": { "email": {}, "skills[]": { "from": "s", "item": { "name": "n" }, "pluck": "n" } }
    });
    assert_eq!(
        issues(&spec).join("; "),
        r#"spec.status: must be one of draft, shadow, promoted, deprecated; spec.fields["email"]: needs exactly one of from, coalesce; spec.fields["skills[]"]: item and pluck can't be combined"#
    );
}

#[test]
fn header_messages() {
    assert_eq!(issues(&json!([])), ["spec: must be an object"]);
    let spec = json!({ "$schema": 1, "provider": "", "version": 1.5, "status": "draft", "extra": true, "anchors": [], "fields": {} });
    assert_eq!(
        issues(&spec),
        [
            "spec.extra: unknown property",
            "spec.$schema: must be a string",
            "spec.provider: must be a non-empty string",
            "spec.endpoint: must be a non-empty string",
            "spec.version: must be an integer >= 1",
            "spec.anchors: must be an object",
            "spec.fields: must be a non-empty object",
        ]
    );
    let spec = json!({ "provider": "a", "endpoint": "e", "version": 2.0, "status": "shadow", "anchors": { "email": "" }, "fields": { "a": "a" } });
    assert_eq!(
        issues(&spec),
        [r#"spec.anchors["email"]: must be a non-empty string"#]
    );
}

#[test]
fn rule_messages() {
    let spec = base(json!({
        "a-b": "x",
        "n": 3,
        "c": { "coalesce": ["x"], "default": null, "bogus": 1 },
        "d": { "from": ["x"], "transform": [] },
        "e": { "from": "x", "transform": ["ok", "bad-name", { "a": 1, "b": 2 }, 7] },
        "l[]": "x",
        "m[]": { "from": "x", "item": { "k": "v" }, "transform": "t" },
        "o[]": { "from": "x", "pluck": "a..b", "default": 1 }
    }));
    assert_eq!(
        issues(&spec),
        [
            r#"spec.fields["a-b"]: canonical field must be dotted identifiers, optionally ending in []"#,
            r#"spec.fields["n"]: must be a path or a rule object"#,
            r#"spec.fields["c"].bogus: unknown property"#,
            r#"spec.fields["c"].coalesce: must be an array of at least 2 paths"#,
            r#"spec.fields["c"].default: must be a string, number or boolean"#,
            r#"spec.fields["d"].from: must be an array of at least 2 paths"#,
            r#"spec.fields["d"].transform: must not be empty"#,
            r#"spec.fields["e"].transform[1]: invalid transform name"#,
            r#"spec.fields["e"].transform[2]: must be a transform name or { name: argument }"#,
            r#"spec.fields["e"].transform[3]: must be a transform name or { name: argument }"#,
            r#"spec.fields["l[]"]: array fields need a rule object with from"#,
            r#"spec.fields["m[]"].transform: not allowed with item"#,
            r#"spec.fields["o[]"].default: unknown property"#,
            r#"spec.fields["o[]"].pluck: must be a path like a.b[0]["c d"]"#,
        ]
    );
}

#[test]
fn path_grammar() {
    let k = |s: &str| Step::Key(s.into());
    let ok: &[(&str, Path)] = &[
        ("a", vec![k("a")]),
        ("org.website", vec![k("org"), k("website")]),
        ("data.job-title", vec![k("data"), k("job-title")]),
        ("a[0][12]", vec![k("a"), Step::Idx(0), Step::Idx(12)]),
        ("[3].x", vec![Step::Idx(3), k("x")]),
        (r#"["first name"]["a.b"]"#, vec![k("first name"), k("a.b")]),
        (r#"["we\"ird"]"#, vec![k("we\"ird")]),
        (r#"["é\n\/"]"#, vec![k("é\n/")]),
        (r#"[""]"#, vec![k("")]),
        ("$", vec![]),
        ("$.data", vec![k("data")]),
        ("$[0].name", vec![Step::Idx(0), k("name")]),
        (r#"["$"]"#, vec![k("$")]),
        ("$$", vec![k("$$")]),
        ("$a.$", vec![k("$a"), k("$")]),
        ("a.$", vec![k("a"), k("$")]),
        ("_.b_", vec![k("_"), k("b_")]),
    ];
    for (p, want) in ok {
        assert_eq!(parse_path(p).as_ref(), Some(want), "{p}");
    }
    let bad = [
        "",
        "a..b",
        "a.",
        ".a",
        "[01]",
        "[x]",
        "a.[0]",
        "a b",
        "['x']",
        "[\"a\nb\"]",
        "[\"a\\x\"]",
        "[\"a\\u12\"]",
        "[\"a\"",
        "[\"a\"]x",
        "1a",
        "-a",
        "a[]",
        "é",
    ];
    for p in bad {
        assert_eq!(parse_path(p), None, "{p:?}");
    }
    // Same verdict as the schema pattern.
    let ajv = schema();
    for p in ok.iter().map(|(p, _)| *p).chain(bad) {
        assert_eq!(
            ours(&base(json!({ "a": p }))),
            ajv.is_valid(&base(json!({ "a": p }))),
            "{p:?}"
        );
    }
}

#[test]
fn lone_surrogate_in_a_quoted_key_becomes_replacement_char() {
    assert_eq!(
        parse_path(r#"["a\ud800"]"#),
        Some(vec![Step::Key("a\u{fffd}".into())])
    );
    assert_eq!(parse_path(r#"["😀"]"#), Some(vec![Step::Key("😀".into())]));
}

#[test]
fn parse_spec_returns_the_typed_spec() {
    let spec = json!({
        "provider": "acme", "endpoint": "person.enrich", "version": 3, "status": "promoted",
        "anchors": { "email": "\"mail\":" },
        "fields": {
            "email": "contact.mail",
            "name": { "from": ["first", "last"], "transform": { "join": " " } },
            "phone": { "coalesce": ["p1", "p2"], "default": "none", "transform": ["trim", "lower"] },
            "jobs[]": { "from": "$", "item": { "title": "role" } },
            "tags[]": { "from": "t", "pluck": "name", "transform": "trim" }
        }
    });
    let s: Spec = parse_spec(&spec).unwrap();
    assert_eq!(
        (
            s.provider.as_str(),
            s.endpoint.as_str(),
            s.version,
            s.status
        ),
        ("acme", "person.enrich", 3, Status::Promoted)
    );
    assert_eq!(
        s.anchors.get("email").map(String::as_str),
        Some("\"mail\":")
    );
    let key = |s: &str| Step::Key(s.into());
    let names: Vec<_> = s.fields.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, ["email", "name", "phone", "jobs[]", "tags[]"]);
    let Rule::Scalar(email) = &s.fields[0].1 else {
        panic!()
    };
    assert_eq!(
        email.source,
        Source::From(vec![key("contact"), key("mail")])
    );
    assert!(email.default.is_none() && email.transform.is_empty());
    let Rule::Scalar(name) = &s.fields[1].1 else {
        panic!()
    };
    assert_eq!(
        name.source,
        Source::FromMany(vec![vec![key("first")], vec![key("last")]])
    );
    assert_eq!(
        name.transform,
        [TransformRef {
            name: "join".into(),
            arg: Some(json!(" "))
        }]
    );
    let Rule::Scalar(phone) = &s.fields[2].1 else {
        panic!()
    };
    assert_eq!(
        phone.source,
        Source::Coalesce(vec![vec![key("p1")], vec![key("p2")]])
    );
    assert_eq!(phone.default, Some(json!("none")));
    assert_eq!(
        phone
            .transform
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["trim", "lower"]
    );
    let Rule::Array(ArrayRule {
        from,
        coalesce,
        item: Some(item),
        item_ref: None,
        pluck: None,
        transform,
        missing: None,
        empty: None,
        wrap: false,
        flatten,
        filter: None,
        unique: None,
        sort: None,
        limit: None,
        group: None,
    }) = &s.fields[3].1
    else {
        panic!()
    };
    assert!(from.is_empty() && coalesce.is_empty() && flatten.is_empty() && transform.is_empty());
    assert_eq!(item[0].0, "title");
    let Rule::Array(tags) = &s.fields[4].1 else {
        panic!()
    };
    assert_eq!(
        (tags.pluck.clone(), tags.transform.len()),
        (Some(vec![key("name")]), 1)
    );

    let err: Vec<Issue> = parse_spec(&json!({})).unwrap_err();
    assert_eq!(
        err[0].to_string(),
        "spec.provider: must be a non-empty string"
    );
}
