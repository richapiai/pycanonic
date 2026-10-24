// `holds` over a parsed document: `present`, `$input` paths and element-relative paths.

use canonic_core::cond::{Cond, holds};
use canonic_core::doc::{KeyPath, KeyStep};
use canonic_core::{Filter, Key, parse};

fn path(keys: &[&str]) -> KeyPath {
    (keys.iter().enumerate())
        .map(|(i, k)| KeyStep::Key(Key::new(k.to_string(), i as u64)))
        .collect()
}

fn present(keys: &[&str], want: bool) -> Cond<KeyPath> {
    Cond::Present(path(keys), want)
}

#[test]
fn present_reads_the_element_then_the_root_and_input_reads_ctx() {
    let all = Filter::all();
    let root = parse(br#"{"a": {"b": 1}, "n": null}"#, &all).unwrap();
    let element = parse(br#"{"c": 2}"#, &all).unwrap();
    let ctx = parse(br#"{"page": 3}"#, &all).unwrap();
    let (root, element, ctx) = (Some(&&root), Some(&&element), Some(&&ctx));
    assert!(holds(&present(&["a", "b"], true), root, None, None));
    assert!(holds(&present(&["n"], false), root, None, None));
    assert!(holds(&present(&["x"], false), root, None, None));
    // Inside an item map, a path reads the element, not the root.
    assert!(holds(&present(&["c"], true), root, element, None));
    assert!(holds(&present(&["a"], false), root, element, None));
    assert!(holds(
        &present(&["$input", "page"], true),
        root,
        element,
        ctx
    ));
    assert!(holds(
        &present(&["$input", "page"], false),
        root,
        None,
        None
    ));
    let all_of = Cond::All(vec![present(&["a"], true), present(&["x"], true)]);
    assert!(!holds(&all_of, root, None, None));
    assert!(holds(&Cond::All(vec![]), root, None, None));
}
