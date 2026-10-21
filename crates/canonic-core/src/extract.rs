//! Reads the routing fields out of raw JSON text without building a document. Port of
//! canonic/packages/canonic/src/{extract.ts,trie.ts}.
//!
//! One resumable state machine serves `extract(raw)` and `stream()` pushes. It walks a trie of the
//! wanted paths, skips unwanted subtrees, and stops as soon as every path is resolved. It trusts
//! its input: malformed JSON resolves the remaining paths as missing and never panics. Duplicate
//! keys: the first occurrence wins.
//!
//! `extract` tries anchors first (a `memmem` search for the key token, skipping a hit preceded by
//! an odd run of backslashes), then scans for whatever they missed. `stream` always scans. Skips
//! walk bytes, never past the last wanted value: a stage-1 index of the whole input would read the
//! whole body, however early the fields sit.
//!
//! Captured values are decoded like the parser does (`unescape`, the `Node::Num` rule) and go
//! through the field's transforms and validator with the executor's own functions, so `extract`
//! equals the routing fields of `map`.

use crate::exec::{transform, transform_many};
use crate::json::{ALL, Filter, Node, parse, unescape};
use crate::plan::{Plan, Scalar, TxRef};
use crate::spec::{Source, Step};
use crate::transforms::Transform;
use crate::value::Value;
use memchr::memmem::Finder;
use memchr::{memchr, memchr2};
use std::borrow::Cow;
use std::sync::Arc;

/// A trie node. Children are by key (decoded bytes) and by array index; ids are path slots.
#[derive(Default)]
struct TNode {
    keys: Vec<(Box<[u8]>, usize)>,
    idx: Vec<(usize, usize)>,
    /// Paths that end here.
    leaves: Vec<usize>,
    /// Paths at or below here.
    paths: Vec<usize>,
    depth: usize,
    /// When a captured value also has wanted paths below it, they're read by scanning the captured
    /// text, so duplicate keys resolve first-wins there too.
    sub: Option<Sub>,
}

struct Sub {
    trie: Trie,
    /// The outer slot of each of `trie`'s paths.
    ids: Vec<usize>,
}

/// Node 0 is the root.
struct Trie {
    nodes: Vec<TNode>,
    len: usize,
}

impl Trie {
    fn new(paths: &[&[Step]]) -> Trie {
        let mut nodes = vec![TNode::default()];
        for (id, path) in paths.iter().enumerate() {
            let mut n = 0;
            nodes[0].paths.push(id);
            for step in *path {
                n = child(&mut nodes, n, step);
                nodes[n].paths.push(id);
            }
            nodes[n].leaves.push(id);
        }
        for n in &mut nodes {
            if !n.leaves.is_empty() && n.paths.len() > n.leaves.len() {
                let below = n.paths.iter().copied().filter(|id| !n.leaves.contains(id));
                let ids: Vec<usize> = below.collect();
                let rest: Vec<&[Step]> = ids.iter().map(|&id| &paths[id][n.depth..]).collect();
                n.sub = Some(Sub {
                    trie: Trie::new(&rest),
                    ids,
                });
            }
        }
        Trie {
            nodes,
            len: paths.len(),
        }
    }
}

fn child(nodes: &mut Vec<TNode>, n: usize, step: &Step) -> usize {
    let found = match step {
        Step::Key(k) => nodes[n]
            .keys
            .iter()
            .find(|(x, _)| **x == *k.as_bytes())
            .map(|x| x.1),
        Step::Idx(i) => nodes[n].idx.iter().find(|(x, _)| x == i).map(|x| x.1),
    };
    if let Some(c) = found {
        return c;
    }
    let c = nodes.len();
    let depth = nodes[n].depth + 1;
    nodes.push(TNode {
        depth,
        ..TNode::default()
    });
    match step {
        Step::Key(k) => nodes[n].keys.push((k.as_bytes().into(), c)),
        Step::Idx(i) => nodes[n].idx.push((*i, c)),
    }
    c
}

/// What `Artifact` builds once from the plan: the trie, the path slots of each routing field, the
/// anchor finders, and what the routing values go through.
pub(crate) struct Extractor {
    trie: Trie,
    routing: Vec<Scalar>,
    /// Per routing field, the slot of each source path.
    slots: Vec<Vec<usize>>,
    /// Slot and finder of each anchored field, in the spec's `anchors` order.
    anchors: Vec<(usize, Finder<'static>)>,
    custom: Vec<Arc<dyn Transform>>,
}

impl Extractor {
    pub(crate) fn new<'p>(plan: &'p Plan, custom: &[Arc<dyn Transform>]) -> Extractor {
        let mut paths: Vec<&[Step]> = Vec::new();
        let mut slot = |p: &'p [Step]| match paths.iter().position(|q| *q == p) {
            Some(i) => i,
            None => {
                paths.push(p);
                paths.len() - 1
            }
        };
        let slots: Vec<Vec<usize>> = (plan.routing.iter())
            .map(|r| match &r.scalar.source {
                Source::From(p) => vec![slot(p)],
                Source::FromMany(ps) | Source::Coalesce(ps) => ps.iter().map(|p| slot(p)).collect(),
            })
            .collect();
        let anchors = (plan.anchors.iter())
            .filter_map(|&i| {
                let token = plan.routing.get(i)?.anchor.as_ref()?;
                Some((slots[i][0], Finder::new(token).into_owned()))
            })
            .collect();
        Extractor {
            trie: Trie::new(&paths),
            routing: plan.routing.iter().map(|r| r.scalar.clone()).collect(),
            slots,
            anchors,
            custom: custom.to_vec(),
        }
    }

    /// Anchors first, then one scan for what they missed.
    pub(crate) fn extract(&self, raw: &[u8]) -> Vec<Option<Value<'static>>> {
        let mut s = State::new(self.trie.len);
        let mut r = Run {
            s: &mut s,
            t: &self.trie,
            b: raw,
        };
        r.s.fin = true;
        let mut from = 0;
        for (id, finder) in &self.anchors {
            from = r.anchor(*id, finder, from);
        }
        r.s.pos = 0;
        r.s.mode = Mode::None;
        r.end();
        self.results(|i| match s.slots[i] {
            Slot::Found(a, b) => Some(&raw[a..b]),
            _ => None,
        })
    }

    /// A routing field's value: `None` when absent (every source path missing or null), `Null`
    /// when its transforms or validator reject what was found.
    /// `text(slot)` is the raw text captured for a path slot.
    fn results<'b>(&self, text: impl Fn(usize) -> Option<&'b [u8]>) -> Vec<Option<Value<'static>>> {
        let fields = self.routing.iter().zip(&self.slots);
        fields
            .map(|(s, slots)| {
                // A custom transform first in line gets a container whole; nothing else reads one.
                let full = matches!(s.tx.first(), Some(TxRef::Custom { .. }));
                let found = |&i: &usize| {
                    let n = text(i).and_then(|v| node(v, full));
                    n.filter(|n| !matches!(n, Node::Null))
                };
                let v = match s.source {
                    Source::FromMany(_) => {
                        let docs: Vec<Option<Node>> = slots.iter().map(found).collect();
                        if docs.iter().all(Option::is_none) {
                            return None;
                        }
                        let refs: Vec<Option<&Node>> = docs.iter().map(Option::as_ref).collect();
                        transform_many(&self.custom, &s.tx, s.validator, &refs).into_owned()
                    }
                    // `coalesce` takes the first present value and does not retry after a rejection.
                    _ => {
                        let d = slots.iter().find_map(found)?;
                        transform(&self.custom, &s.tx, s.validator, &&d).into_owned()
                    }
                };
                Some(v)
            })
            .collect()
    }
}

/// The captured text of one value, decoded as the parser would; `None` for a scalar that isn't
/// JSON (`12x`). A container is parsed only when `full`; otherwise it is just a container.
fn node(v: &[u8], full: bool) -> Option<Node<'_>> {
    Some(match v {
        [b'"', body @ .., b'"'] => Node::Str(unescape(body)),
        [b'{' | b'[', ..] if full => parse(v, &ALL).unwrap_or(Node::Skipped),
        [b'{' | b'[', ..] => Node::Skipped,
        b"true" => Node::Bool(true),
        b"false" => Node::Bool(false),
        b"null" => Node::Null,
        _ => match parse(v, &Filter::default()) {
            Ok(n @ Node::Num(t)) if t.len() == v.len() => n,
            _ => return None,
        },
    })
}

/// `extract` fed in chunks, always in scanner mode. `result` ends the input and returns what a
/// scanning `extract` gives on the bytes pushed. Holds the bytes from the start of the value being
/// read, not the whole body.
pub struct ExtractStream {
    ex: Arc<Extractor>,
    buf: Vec<u8>,
    s: State,
    /// Captured text, copied out of `buf` before it is compacted (`Slot::Kept`).
    kept: Vec<Option<Vec<u8>>>,
}

impl ExtractStream {
    pub(crate) fn new(ex: Arc<Extractor>) -> ExtractStream {
        let s = State::new(ex.trie.len);
        ExtractStream {
            kept: vec![None; ex.trie.len],
            ex,
            buf: Vec::new(),
            s,
        }
    }

    /// Feeds the next chunk. True once every routing field has a value or is known absent;
    /// chunks pushed after that are ignored.
    pub fn push(&mut self, chunk: &[u8]) -> bool {
        if self.s.st == St::Done {
            return true;
        }
        self.buf.extend_from_slice(chunk);
        self.run().run();
        self.compact();
        self.s.st == St::Done
    }

    /// Ends the input; unresolved fields are `None`.
    pub fn result(mut self) -> Vec<Option<Value<'static>>> {
        self.run().end();
        let (s, buf, kept) = (&self.s, &self.buf, &self.kept);
        self.ex.results(|i| match s.slots[i] {
            Slot::Found(a, b) => Some(&buf[a..b]),
            Slot::Kept => kept[i].as_deref(),
            Slot::Pending | Slot::Missing => None,
        })
    }

    fn run(&mut self) -> Run<'_> {
        Run {
            s: &mut self.s,
            t: &self.ex.trie,
            b: &self.buf,
        }
    }

    /// Drops consumed input, keeping a captured value's start and any partial token.
    fn compact(&mut self) {
        let s = &mut self.s;
        for (slot, kept) in s.slots.iter_mut().zip(&mut self.kept) {
            if let Slot::Found(a, b) = *slot {
                *kept = Some(self.buf[a..b].to_vec());
                *slot = Slot::Kept;
            }
        }
        if s.st == St::Done {
            self.buf = Vec::new();
            return;
        }
        let keep = if s.mode == Mode::Capture {
            s.cap_start
        } else {
            s.pos
        };
        self.buf.drain(..keep);
        s.pos -= keep;
        s.cap_start = s.cap_start.saturating_sub(keep);
        s.key_scan = s.key_scan.saturating_sub(keep);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum St {
    Value,
    KeyFirst,
    Key,
    AfterKey,
    AfterValue,
    ElemFirst,
    Done,
}

/// Reading a value we skip, or one we capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    None,
    Skip,
    Capture,
}

struct Frame {
    node: usize,
    arr: bool,
    idx: usize,
}

/// A path's value as the scanner knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Pending,
    Missing,
    /// The value's text is `start..end` of the input.
    Found(usize, usize),
    /// Found, and copied out of a stream's buffer.
    Kept,
}

/// The scanner state that survives between pushes. Positions index the buffer it runs over.
struct State {
    slots: Vec<Slot>,
    remaining: usize,
    pos: usize,
    /// No more input will come.
    fin: bool,
    st: St,
    target: Option<usize>,
    stack: Vec<Frame>,
    mode: Mode,
    v_depth: usize,
    v_str: bool,
    v_scalar: bool,
    cap_start: usize,
    cap_node: usize,
    /// Where the closing-quote search of an unfinished key resumes.
    key_scan: usize,
}

impl State {
    fn new(len: usize) -> State {
        State {
            slots: vec![Slot::Pending; len],
            remaining: len,
            pos: 0,
            fin: false,
            st: if len == 0 { St::Done } else { St::Value },
            target: Some(0),
            stack: Vec::new(),
            mode: Mode::None,
            v_depth: 0,
            v_str: false,
            v_scalar: false,
            cap_start: 0,
            cap_node: 0,
            key_scan: 0,
        }
    }
}

/// One run of the scanner over the input so far.
struct Run<'a> {
    s: &'a mut State,
    t: &'a Trie,
    b: &'a [u8],
}

const BOM: &[u8] = b"\xEF\xBB\xBF";

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\r' | b'\t')
}

fn is_delim(c: u8) -> bool {
    matches!(c, b',' | b'}' | b']') || is_ws(c)
}

/// Offset just past the closing quote of a string whose body starts at `i`; `Err` with where to
/// resume when the input ends first (never inside an escape, so the next chunk pairs it right).
fn string_end(b: &[u8], mut i: usize) -> Result<usize, usize> {
    loop {
        let Some(k) = memchr2(b'"', b'\\', b.get(i..).unwrap_or_default()) else {
            return Err(b.len().max(i));
        };
        let j = i + k;
        if b[j] == b'"' {
            return Ok(j + 1);
        }
        if j + 1 >= b.len() {
            return Err(j);
        }
        i = j + 2;
    }
}

/// True when `b[i]` is preceded by an odd run of backslashes, i.e. escaped.
fn odd_backslashes(b: &[u8], i: usize) -> bool {
    b[..i].iter().rev().take_while(|&&c| c == b'\\').count() % 2 == 1
}

impl Run<'_> {
    fn run(&mut self) {
        if !self.bom() {
            return;
        }
        while self.s.st != St::Done {
            if self.s.mode != Mode::None {
                if !self.value() {
                    return;
                }
                continue;
            }
            if self.s.remaining == 0 {
                return self.finish();
            }
            let Some(c) = self.next() else {
                if self.s.fin {
                    self.finish();
                }
                return;
            };
            if !self.step(c) {
                return;
            }
        }
    }

    /// Ends the input; anything unresolved is missing.
    fn end(&mut self) {
        if self.s.st != St::Done {
            self.s.fin = true;
            self.run();
            self.finish();
        }
    }

    /// Steps over a byte order mark before the root value. False while the input so far could
    /// still be the start of one.
    fn bom(&mut self) -> bool {
        let s = &mut *self.s;
        if s.pos != 0 || s.st != St::Value || !s.stack.is_empty() || s.mode != Mode::None {
            return true;
        }
        if self.b.starts_with(BOM) {
            s.pos = BOM.len();
            return true;
        }
        s.fin || self.b.is_empty() || !BOM.starts_with(self.b)
    }

    fn step(&mut self, c: u8) -> bool {
        match self.s.st {
            St::Value => self.begin(c),
            St::KeyFirst if c == b'}' => self.close(),
            St::KeyFirst | St::Key => self.key(c),
            St::AfterKey => {
                if c != b':' {
                    return self.bail();
                }
                self.s.pos += 1;
                self.s.st = St::Value;
                true
            }
            St::ElemFirst if c == b']' => self.close(),
            St::ElemFirst => {
                self.s.target = self.elem(0);
                self.s.st = St::Value;
                true
            }
            St::AfterValue => self.after(c),
            St::Done => false,
        }
    }

    /// The byte at `pos` after whitespace, `None` at the end of the input so far.
    fn next(&mut self) -> Option<u8> {
        while self.b.get(self.s.pos).is_some_and(|&c| is_ws(c)) {
            self.s.pos += 1;
        }
        self.b.get(self.s.pos).copied()
    }

    fn begin(&mut self, c: u8) -> bool {
        let t = self.t;
        if let Some(n) = self.s.target
            && self.pending(n)
        {
            let node = &t.nodes[n];
            if !node.leaves.is_empty() {
                self.s.cap_node = n;
                return self.start(c, Mode::Capture) || self.bail();
            }
            if c == b'{' && !node.keys.is_empty() {
                return self.open(n, false);
            }
            if c == b'[' && !node.idx.is_empty() {
                return self.open(n, true);
            }
            self.miss(n);
        }
        self.start(c, Mode::Skip) || self.bail()
    }

    fn start(&mut self, c: u8, mode: Mode) -> bool {
        if !matches!(
            c,
            b'"' | b'{' | b'[' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
        ) {
            return false;
        }
        let s = &mut *self.s;
        s.mode = mode;
        s.cap_start = s.pos;
        s.v_scalar = !matches!(c, b'"' | b'{' | b'[');
        s.v_str = c == b'"';
        s.v_depth = usize::from(matches!(c, b'{' | b'['));
        if !s.v_scalar {
            s.pos += 1;
        }
        true
    }

    /// Continues the value in progress. False when it needs more input.
    fn value(&mut self) -> bool {
        let done = if self.s.v_scalar {
            self.scalar_end()
        } else {
            self.skip_body()
        };
        if !done {
            return false;
        }
        let mode = std::mem::replace(&mut self.s.mode, Mode::None);
        if mode == Mode::Capture {
            self.captured();
        }
        self.after_value();
        true
    }

    /// A number or literal is complete at the next delimiter, or at the end of the input.
    fn scalar_end(&mut self) -> bool {
        while self.b.get(self.s.pos).is_some_and(|&c| !is_delim(c)) {
            self.s.pos += 1;
        }
        self.s.pos < self.b.len() || self.s.fin
    }

    /// Skips a string or container body.
    fn skip_body(&mut self) -> bool {
        loop {
            if self.s.v_str {
                match string_end(self.b, self.s.pos) {
                    Ok(end) => {
                        self.s.pos = end;
                        self.s.v_str = false;
                    }
                    Err(resume) => {
                        self.s.pos = resume;
                        return false;
                    }
                }
            }
            if self.s.v_depth == 0 {
                return true;
            }
            if !self.skip_bytes() {
                return false;
            }
        }
    }

    /// Scans to the container's end (true) or the end of the input (false), resumably.
    fn skip_bytes(&mut self) -> bool {
        let (b, s) = (self.b, &mut *self.s);
        let mut pos = s.pos;
        let mut depth = s.v_depth;
        while let Some(&c) = b.get(pos) {
            pos += 1;
            match c {
                b'"' => match string_end(b, pos) {
                    Ok(end) => pos = end,
                    Err(resume) => {
                        s.pos = resume;
                        s.v_depth = depth;
                        s.v_str = true;
                        return false;
                    }
                },
                b'{' | b'[' => depth += 1,
                b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        s.pos = pos;
        s.v_depth = depth;
        depth == 0
    }

    fn captured(&mut self) {
        let (t, b) = (self.t, self.b);
        let n = &t.nodes[self.s.cap_node];
        let (start, end) = (self.s.cap_start, self.s.pos);
        for &id in &n.leaves {
            self.set(id, Slot::Found(start, end));
        }
        let Some(sub) = &n.sub else { return };
        if !self.pending(self.s.cap_node) {
            return;
        }
        let mut inner = State::new(sub.trie.len);
        Run {
            s: &mut inner,
            t: &sub.trie,
            b: &b[start..end],
        }
        .end();
        for (&id, v) in sub.ids.iter().zip(inner.slots) {
            let v = match v {
                Slot::Found(a, z) => Slot::Found(start + a, start + z),
                _ => Slot::Missing,
            };
            self.set(id, v);
        }
    }

    fn key(&mut self, c: u8) -> bool {
        if c != b'"' {
            return self.bail();
        }
        let from = self.s.key_scan.max(self.s.pos + 1);
        match string_end(self.b, from) {
            Err(resume) => {
                if self.s.fin {
                    return self.bail();
                }
                self.s.key_scan = resume;
                false
            }
            Ok(end) => {
                self.s.key_scan = 0;
                let Some(top) = self.s.stack.last().map(|f| f.node) else {
                    return self.bail();
                };
                self.s.target = self.matching(top, &self.b[self.s.pos + 1..end - 1]);
                self.s.pos = end;
                self.s.st = St::AfterKey;
                true
            }
        }
    }

    /// The child of `n` for the raw key text `raw`, decoded only if it has an escape.
    fn matching(&self, n: usize, raw: &[u8]) -> Option<usize> {
        let key = match memchr(b'\\', raw) {
            Some(_) => Cow::Owned(unescape(raw).into_owned().into_bytes()),
            None => Cow::Borrowed(raw),
        };
        let keys = &self.t.nodes[n].keys;
        keys.iter().find(|(k, _)| **k == *key).map(|&(_, c)| c)
    }

    fn after(&mut self, c: u8) -> bool {
        let Some(top) = self.s.stack.last() else {
            return self.bail();
        };
        let (node, arr) = (top.node, top.arr);
        if c == if arr { b']' } else { b'}' } {
            return self.close();
        }
        if c != b',' {
            return self.bail();
        }
        self.s.pos += 1;
        if !self.pending(node) {
            // Nothing left to find in this container: skip the rest of it in one go.
            let s = &mut *self.s;
            s.stack.pop();
            s.mode = Mode::Skip;
            s.v_scalar = false;
            s.v_str = false;
            s.v_depth = 1;
            return true;
        }
        if arr {
            let i = self.s.stack.last_mut().map_or(0, |f| {
                f.idx += 1;
                f.idx
            });
            self.s.target = self.elem(i);
            self.s.st = St::Value;
        } else {
            self.s.st = St::Key;
        }
        true
    }

    fn elem(&self, i: usize) -> Option<usize> {
        let n = self.s.stack.last()?.node;
        let idx = &self.t.nodes[n].idx;
        idx.iter().find(|&&(x, _)| x == i).map(|&(_, c)| c)
    }

    fn open(&mut self, node: usize, arr: bool) -> bool {
        self.s.stack.push(Frame { node, arr, idx: 0 });
        self.s.pos += 1;
        self.s.st = if arr { St::ElemFirst } else { St::KeyFirst };
        true
    }

    fn close(&mut self) -> bool {
        self.s.pos += 1;
        if let Some(f) = self.s.stack.pop() {
            self.miss(f.node);
        }
        self.after_value();
        true
    }

    fn after_value(&mut self) {
        if self.s.stack.is_empty() {
            self.finish();
        } else {
            self.s.st = St::AfterValue;
        }
    }

    fn pending(&self, n: usize) -> bool {
        let slots = &self.s.slots;
        self.t.nodes[n]
            .paths
            .iter()
            .any(|&id| slots[id] == Slot::Pending)
    }

    /// Resolves path `id` to `v` (`Found` or `Missing`) unless it already is.
    fn set(&mut self, id: usize, v: Slot) {
        let s = &mut *self.s;
        if s.slots[id] != Slot::Pending {
            return;
        }
        s.slots[id] = v;
        s.remaining -= 1;
    }

    fn miss(&mut self, n: usize) {
        let t = self.t;
        for &id in &t.nodes[n].paths {
            self.set(id, Slot::Missing);
        }
    }

    fn bail(&mut self) -> bool {
        self.finish();
        false
    }

    fn finish(&mut self) {
        self.miss(0);
        self.s.st = St::Done;
        self.s.mode = Mode::None;
    }

    /// Where the anchor token occurs at or after `from`, not escaped; `None` if nowhere.
    fn find(&self, finder: &Finder, from: usize) -> Option<usize> {
        let mut at = from;
        while let Some(k) = finder.find(self.b.get(at..)?) {
            let q = at + k;
            if !odd_backslashes(self.b, q) {
                return Some(q);
            }
            at = q + 1;
        }
        None
    }

    /// Reads one anchored value; returns where the next anchor's search starts. Each search
    /// starts where the previous one matched, so a provider that keeps its key order costs one
    /// pass; a miss retries from the start.
    fn anchor(&mut self, id: usize, finder: &Finder, from: usize) -> usize {
        let hit = self.find(finder, from);
        let hit = hit.or_else(|| (from > 0).then(|| self.find(finder, 0)).flatten());
        let Some(q) = hit else { return from };
        self.s.pos = q + finder.needle().len();
        let Some(c) = self.next() else { return from };
        if !self.start(c, Mode::Capture) {
            return from;
        }
        let done = if self.s.v_scalar {
            self.scalar_end()
        } else {
            self.skip_body()
        };
        if done {
            self.set(id, Slot::Found(self.s.cap_start, self.s.pos));
        }
        self.s.pos
    }
}
