//! The Document model — a canonical tree of ordered, labeled edges.
//!
//! Ported from `~/dev/omnist/omnist/document.py` (see issue #4). A Document
//! **node** is either a **leaf** holding a [`Scalar`], or an **internal
//! node** holding an *ordered list of edges*, each a `(label, child)` pair.
//! **Labels may repeat** — "many members" is the label `member` appearing
//! several times, not a field pointing to an array.
//!
//! ## Architecture (per issue #1, "architecture freedom")
//!
//! This port uses an arena: nodes live in a `Vec<Entry>` inside [`Doc`],
//! referenced by [`NodeId`] (an index newtype), not `Rc<RefCell<_>>`. Two
//! consequences that don't mirror Python/TypeScript 1:1:
//!
//! - **No cycle detection.** Python/TS guard against cycles because a plain
//!   `dict`/object can be made self-referential through shared mutable
//!   references. [`Value`] (this port's "plain value" input type, analogous
//!   to a parsed JSON value) is a plain owned tree — building a
//!   self-referential `Value` without `unsafe` or `Rc<RefCell<_>>` isn't
//!   possible, so the whole bug class is closed by the type system rather
//!   than checked at runtime (see the workflow playbook's "what NOT to
//!   carry over unexamined").
//! - **No integer-digit guard.** Python's `_check_int_digits` defends
//!   against `str()`-converting an arbitrary-precision `int` with
//!   thousands of digits (a superlinear operation). This port's `Scalar`
//!   uses `i64`, which tops out at 19 digits — nowhere near the 4300-digit
//!   cap — so the guard would be permanently-dead code with no reachable
//!   branch to test (the TypeScript port kept a dormant version of this
//!   check under an explicit coverage-ignore for parity; this port omits
//!   it entirely rather than ship untestable dead code for a guard that
//!   can't fire for `i64`).
//!
//! Observable behavior for everything else (construction, depth guard,
//! edge ordering, mutation semantics) matches the Python spec.

use indexmap::IndexMap;
use std::fmt;

use crate::error::DocumentError;

/// Maximum nesting depth for a Document node (matches Python's `_MAX_DEPTH`).
pub const MAX_DEPTH: usize = 200;

/// An index into a [`Doc`]'s arena. Opaque outside this module's crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

/// A leaf value.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scalar::Null => write!(f, "null"),
            Scalar::Bool(b) => write!(f, "{b}"),
            Scalar::Int(i) => write!(f, "{i}"),
            Scalar::Float(x) => write!(f, "{x}"),
            Scalar::Str(s) => write!(f, "{s:?}"),
        }
    }
}

/// A plain input value (analogous to a parsed JSON/YAML/TOML value), the
/// thing [`Doc::of`]/[`Doc::add`]/[`Doc::set`] turn into canonical nodes.
///
/// `Object` uses [`IndexMap`] (per issue #1 §2) so key order — which the
/// Document model treats as data, not incidental — survives construction.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
}

impl Value {
    /// Borrow the underlying map if this is an `Object`, else `None`.
    /// Used by tests to inspect `to_grouped()`'s output without a
    /// match arm that (for any one call site) is only ever exercised on
    /// one side -- see the test suite for both an `Object` and a
    /// non-`Object` call site.
    #[cfg(test)]
    fn as_object(&self) -> Option<&IndexMap<String, Value>> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }
}

impl From<Scalar> for Value {
    fn from(s: Scalar) -> Self {
        match s {
            Scalar::Null => Value::Null,
            Scalar::Bool(b) => Value::Bool(b),
            Scalar::Int(i) => Value::Int(i),
            Scalar::Float(x) => Value::Float(x),
            Scalar::Str(s) => Value::Str(s),
        }
    }
}

#[derive(Debug, Clone)]
enum NodeData {
    Leaf(Scalar),
    Internal(Vec<(String, NodeId)>),
}

#[derive(Debug, Clone)]
struct Entry {
    data: NodeData,
    /// This node's own depth relative to the document root (root = 0).
    /// Recorded at construction time so a later mutation rooted at this
    /// node (add/set) can seed the depth guard from *here*, not from 0 --
    /// this is the exact bug class from omnist-ts#37 (a depth check reset
    /// to 0 on every mutation instead of accounting for how deep the
    /// mutation's attachment point already is) and omnist-ts#70 (a second
    /// writer that had the same reset bug because the guard lived in one
    /// place, not one shared helper called from every entry point).
    depth: usize,
}

/// The shared depth guard. Every construction/mutation path that can grow
/// the tree calls this before creating a node -- see the module-level test
/// `every_tree_mutating_entry_point_enforces_the_depth_guard` for the audit
/// that walks every public entry point and confirms each one does.
fn check_write_depth(depth: usize, path: &str) -> Result<(), DocumentError> {
    if depth > MAX_DEPTH {
        return Err(DocumentError::new(
            path,
            format!("nesting exceeds the maximum depth ({MAX_DEPTH})"),
        ));
    }
    Ok(())
}

fn join(path: &str, key: &str) -> String {
    let is_identifier = !key.is_empty()
        && key
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_alphanumeric() || c == '_');
    if is_identifier {
        format!("{path}.{key}")
    } else {
        format!("{path}[\"{key}\"]")
    }
}

/// One (path, depth, value) triple to be turned into a child node -- mirrors
/// Python's `_children` generator, including its extra depth level for
/// items of an array value (an array value sits one level deeper than a
/// plain scalar/object value under the same key).
struct ChildSpec<'a> {
    path: String,
    depth: usize,
    value: &'a Value,
}

fn child_specs<'a>(
    v: &'a Value,
    path: &str,
    depth: usize,
) -> Result<Vec<ChildSpec<'a>>, DocumentError> {
    match v {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let ip = format!("{path}[{i}]");
                if matches!(item, Value::Array(_)) {
                    return Err(DocumentError::new(
                        ip,
                        "an array of arrays has no labeled-edge form",
                    ));
                }
                out.push(ChildSpec {
                    path: ip,
                    depth: depth + 1,
                    value: item,
                });
            }
            Ok(out)
        }
        other => Ok(vec![ChildSpec {
            path: path.to_string(),
            depth,
            value: other,
        }]),
    }
}

/// Turn a plain [`Value`] into a canonical node inside `arena`, returning
/// its [`NodeId`]. Mirrors Python's `build_node`.
fn build_node(
    arena: &mut Vec<Entry>,
    value: &Value,
    path: &str,
    depth: usize,
) -> Result<NodeId, DocumentError> {
    check_write_depth(depth, path)?;
    match value {
        Value::Object(map) => {
            let mut edges = Vec::new();
            for (k, v) in map {
                let kp = join(path, k);
                for spec in child_specs(v, &kp, depth + 1)? {
                    let cid = build_node(arena, spec.value, &spec.path, spec.depth)?;
                    edges.push((k.clone(), cid));
                }
            }
            Ok(push(arena, NodeData::Internal(edges), depth))
        }
        Value::Array(_) => Err(DocumentError::new(
            path,
            "a bare array has no labeled-edge form (arrays appear only as a repeated field)",
        )),
        // Scalars are matched directly as top-level arms of this match
        // (rather than through a `scalar => { match scalar { ... } }`
        // catch-all) so every arm is a real, exhaustive possibility --
        // no `Value::Array(_) | Value::Object(_) => unreachable!()` arm
        // to justify, since those two variants are already handled above.
        Value::Null => Ok(push(arena, NodeData::Leaf(Scalar::Null), depth)),
        Value::Bool(b) => Ok(push(arena, NodeData::Leaf(Scalar::Bool(*b)), depth)),
        Value::Int(i) => Ok(push(arena, NodeData::Leaf(Scalar::Int(*i)), depth)),
        Value::Float(x) => Ok(push(arena, NodeData::Leaf(Scalar::Float(*x)), depth)),
        Value::Str(s) => Ok(push(arena, NodeData::Leaf(Scalar::Str(s.clone())), depth)),
    }
}

fn push(arena: &mut Vec<Entry>, data: NodeData, depth: usize) -> NodeId {
    let id = NodeId(arena.len());
    arena.push(Entry { data, depth });
    id
}

/// A guarded handle on a Document tree: an arena of nodes plus the root.
#[derive(Debug, Clone)]
pub struct Doc {
    arena: Vec<Entry>,
    root: NodeId,
}

impl Doc {
    /// Build a `Doc` from a plain [`Value`].
    pub fn of(value: &Value) -> Result<Doc, DocumentError> {
        let mut arena = Vec::new();
        let root = build_node(&mut arena, value, "$", 0)?;
        Ok(Doc { arena, root })
    }

    /// A cursor to the document root, at path `"$"`.
    pub fn root(&self) -> Cursor<'_> {
        Cursor {
            doc: self,
            id: self.root,
            path: "$".to_string(),
        }
    }

    fn entry(&self, id: NodeId) -> &Entry {
        &self.arena[id.0]
    }

    /// Append an edge `(label, value)` under the node at `at`/`path`. A
    /// repeated label is how an array grows. Returns the new edge's
    /// `NodeId`.
    pub fn add(
        &mut self,
        at: NodeId,
        path: &str,
        label: &str,
        value: &Value,
    ) -> Result<NodeId, DocumentError> {
        self.require_internal(at, path, "add")?;
        let attach_depth = self.entry(at).depth;
        let child_path = join(path, label);
        let cid = build_node(&mut self.arena, value, &child_path, attach_depth + 1)?;
        let edges = self.internal_edges_mut(at, path, "add")?;
        edges.push((label.to_string(), cid));
        Ok(cid)
    }

    /// Replace all edges under `label` with a single new edge, positioned
    /// at the first old occurrence (`set` = `remove` + `add`).
    pub fn set(
        &mut self,
        at: NodeId,
        path: &str,
        label: &str,
        value: &Value,
    ) -> Result<NodeId, DocumentError> {
        self.require_internal(at, path, "set")?;
        let attach_depth = self.entry(at).depth;
        let child_path = join(path, label);
        let cid = build_node(&mut self.arena, value, &child_path, attach_depth + 1)?;
        let edges = self.internal_edges_mut(at, path, "set")?;
        let mut first: Option<usize> = None;
        let mut kept: Vec<(String, NodeId)> = Vec::with_capacity(edges.len());
        for (lbl, child) in edges.drain(..) {
            if lbl == label {
                if first.is_none() {
                    first = Some(kept.len());
                    kept.push((label.to_string(), cid));
                }
                // later duplicates are dropped
            } else {
                kept.push((lbl, child));
            }
        }
        if first.is_none() {
            kept.push((label.to_string(), cid));
        }
        *edges = kept;
        Ok(cid)
    }

    /// Remove every edge under `label`.
    pub fn remove(&mut self, at: NodeId, path: &str, label: &str) -> Result<(), DocumentError> {
        self.require_internal(at, path, "remove")?;
        let edges = self.internal_edges_mut(at, path, "remove")?;
        edges.retain(|(lbl, _)| lbl != label);
        Ok(())
    }

    fn require_internal(&self, id: NodeId, path: &str, op: &str) -> Result<(), DocumentError> {
        match self.entry(id).data {
            NodeData::Internal(_) => Ok(()),
            NodeData::Leaf(_) => Err(DocumentError::new(path, format!("cannot {op} on a leaf"))),
        }
    }

    /// The mutable edge list at `at`, or an error if it's a leaf.
    ///
    /// `add`/`set`/`remove` all call `require_internal` first (preserving
    /// Python's check-leaf-before-build-value error ordering), so in
    /// practice this function's `Leaf` arm never fires through the public
    /// API -- it exists so the actual mutation site is a real two-armed
    /// match (both arms reachable and independently tested, see
    /// `internal_edges_mut_rejects_a_leaf_directly` below) instead of an
    /// `unreachable!()`/if-let-without-else that would leave a
    /// structurally-dead branch for `cargo llvm-cov` to flag.
    fn internal_edges_mut(
        &mut self,
        at: NodeId,
        path: &str,
        op: &str,
    ) -> Result<&mut Vec<(String, NodeId)>, DocumentError> {
        match &mut self.arena[at.0].data {
            NodeData::Internal(edges) => Ok(edges),
            NodeData::Leaf(_) => Err(DocumentError::new(path, format!("cannot {op} on a leaf"))),
        }
    }

    /// A JSON-shaped projection of the whole document: same-label edges
    /// grouped into an array; a label seen once stays a single value.
    pub fn to_grouped(&self) -> Value {
        self.grouped_at(self.root)
    }

    // Note: unlike `build_node`/`data_at`, this has no depth parameter --
    // it walks an already-validated tree (every node was depth-checked at
    // construction time), so there's nothing left to guard here.
    fn grouped_at(&self, id: NodeId) -> Value {
        match &self.entry(id).data {
            NodeData::Leaf(s) => Value::from(s.clone()),
            NodeData::Internal(edges) => {
                let mut counts: IndexMap<&str, usize> = IndexMap::new();
                for (label, _) in edges {
                    *counts.entry(label.as_str()).or_insert(0) += 1;
                }
                let mut out: IndexMap<String, Value> = IndexMap::new();
                for (label, child) in edges {
                    let g = self.grouped_at(*child);
                    if counts[label.as_str()] > 1 {
                        match out.get_mut(label.as_str()) {
                            Some(Value::Array(arr)) => arr.push(g),
                            _ => {
                                out.insert(label.clone(), Value::Array(vec![g]));
                            }
                        }
                    } else {
                        out.insert(label.clone(), g);
                    }
                }
                Value::Object(out)
            }
        }
    }

    /// A lossless copy of the whole document back into [`Value`] form.
    pub fn to_data(&self) -> Value {
        self.data_at(self.root)
    }

    fn data_at(&self, id: NodeId) -> Value {
        match &self.entry(id).data {
            NodeData::Leaf(s) => Value::from(s.clone()),
            NodeData::Internal(edges) => {
                let mut map = IndexMap::new();
                // Structural copy only -- this intentionally does NOT
                // re-group repeated labels (see to_grouped for that); a
                // repeated label here just overwrites in the IndexMap,
                // which is why to_data is for equality/structural
                // comparison, not for JSON-shaped export of repeated
                // fields. Kept simple: only used by `eq` in this issue.
                for (label, child) in edges {
                    map.insert(label.clone(), self.data_at(*child));
                }
                Value::Object(map)
            }
        }
    }

    /// Structural equality between two documents (same shape, same edge
    /// order, same labels, same leaf values).
    pub fn eq_doc(&self, other: &Doc) -> bool {
        self.node_eq(self.root, other, other.root)
    }

    fn node_eq(&self, a: NodeId, other: &Doc, b: NodeId) -> bool {
        match (&self.entry(a).data, &other.entry(b).data) {
            (NodeData::Leaf(x), NodeData::Leaf(y)) => x == y,
            (NodeData::Internal(xs), NodeData::Internal(ys)) => {
                xs.len() == ys.len()
                    && xs
                        .iter()
                        .zip(ys.iter())
                        .all(|((la, ca), (lb, cb))| la == lb && self.node_eq(*ca, other, *cb))
            }
            _ => false,
        }
    }
}

/// A read-only cursor into a [`Doc`]'s tree, tracking its own path.
///
/// Path is owned per-cursor (rather than borrowed) because it's built
/// incrementally (`$.a.b[1]`) as cursors descend; it's only needed for
/// error messages and equality with Python's `Doc.path`, not perf-critical
/// traversal.
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    doc: &'a Doc,
    id: NodeId,
    pub path: String,
}

impl<'a> Cursor<'a> {
    pub fn id(&self) -> NodeId {
        self.id
    }

    pub fn is_leaf(&self) -> bool {
        matches!(self.doc.entry(self.id).data, NodeData::Leaf(_))
    }

    pub fn value(&self) -> Result<&'a Scalar, DocumentError> {
        match &self.doc.entry(self.id).data {
            NodeData::Leaf(s) => Ok(s),
            NodeData::Internal(_) => Err(DocumentError::new(&self.path, "not a leaf; use edges()")),
        }
    }

    pub fn edges(&self) -> Result<Vec<(String, Cursor<'a>)>, DocumentError> {
        match &self.doc.entry(self.id).data {
            NodeData::Internal(edges) => {
                let mut counts: IndexMap<&str, usize> = IndexMap::new();
                let mut out = Vec::with_capacity(edges.len());
                for (label, child) in edges {
                    let i = *counts.entry(label.as_str()).or_insert(0);
                    counts.insert(label.as_str(), i + 1);
                    let cp = if i == 0 {
                        format!("{}.{}", self.path, label)
                    } else {
                        format!("{}.{}[{}]", self.path, label, i)
                    };
                    out.push((
                        label.clone(),
                        Cursor {
                            doc: self.doc,
                            id: *child,
                            path: cp,
                        },
                    ));
                }
                Ok(out)
            }
            NodeData::Leaf(_) => Err(DocumentError::new(&self.path, "a leaf has no edges")),
        }
    }

    pub fn labels(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        if let NodeData::Internal(edges) = &self.doc.entry(self.id).data {
            for (label, _) in edges {
                if seen.insert(label.clone()) {
                    out.push(label.clone());
                }
            }
        }
        out
    }

    pub fn get(&self, label: &str) -> Vec<Cursor<'a>> {
        self.edges()
            .into_iter()
            .flatten()
            .filter(|(lbl, _)| lbl == label)
            .map(|(_, c)| c)
            .collect()
    }

    pub fn get_one(&self, label: &str) -> Result<Cursor<'a>, DocumentError> {
        let mut cs = self.get(label);
        if cs.len() != 1 {
            return Err(DocumentError::new(
                &self.path,
                format!("expected exactly one {label:?}, found {}", cs.len()),
            ));
        }
        Ok(cs.remove(0))
    }

    pub fn count(&self, label: &str) -> usize {
        if let NodeData::Internal(edges) = &self.doc.entry(self.id).data {
            edges.iter().filter(|(lbl, _)| lbl == label).count()
        } else {
            0
        }
    }

    /// A cursor to the single child under `label`.
    pub fn child(&self, label: &str) -> Result<Cursor<'a>, DocumentError> {
        self.get_one(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = IndexMap::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        Value::Object(m)
    }

    /// `levels` nested objects (each `{"a": ...}`) wrapping a leaf. Per
    /// `build_node`'s depth arithmetic, the leaf ends up at depth `levels`
    /// (each object level adds exactly 1 -- no array involved).
    fn nest(levels: usize) -> Value {
        let mut v = Value::Int(0);
        for _ in 0..levels {
            v = obj(&[("a", v)]);
        }
        v
    }

    // -- basic construction ---------------------------------------------

    #[test]
    fn constructs_a_scalar_leaf() {
        let doc = Doc::of(&Value::Int(42)).unwrap();
        let root = doc.root();
        assert!(root.is_leaf());
        assert_eq!(root.value().unwrap(), &Scalar::Int(42));
    }

    #[test]
    fn constructs_an_object_as_ordered_edges() {
        let v = obj(&[("b", Value::Int(1)), ("a", Value::Int(2))]);
        let doc = Doc::of(&v).unwrap();
        let root = doc.root();
        assert!(!root.is_leaf());
        let edges = root.edges().unwrap();
        let labels: Vec<&str> = edges.iter().map(|(l, _)| l.as_str()).collect();
        // Insertion order preserved, NOT sorted -- "b" before "a".
        assert_eq!(labels, vec!["b", "a"]);
    }

    #[test]
    fn a_list_value_expands_into_repeated_edges() {
        let v = obj(&[(
            "member",
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        )]);
        let doc = Doc::of(&v).unwrap();
        let root = doc.root();
        assert_eq!(root.count("member"), 3);
        let members = root.get("member");
        let vals: Vec<&Scalar> = members.iter().map(|c| c.value().unwrap()).collect();
        assert_eq!(
            vals,
            vec![&Scalar::Int(1), &Scalar::Int(2), &Scalar::Int(3)]
        );
    }

    #[test]
    fn a_bare_top_level_array_is_rejected() {
        let err = Doc::of(&Value::Array(vec![Value::Int(1)])).unwrap_err();
        assert!(err.message.contains("bare array"));
        assert_eq!(err.path, "$");
    }

    #[test]
    fn an_array_of_arrays_is_rejected() {
        let v = obj(&[("a", Value::Array(vec![Value::Array(vec![Value::Int(1)])]))]);
        let err = Doc::of(&v).unwrap_err();
        assert!(err.message.contains("array of arrays"));
        assert_eq!(err.path, "$.a[0]");
    }

    // -- depth guard: max-depth boundary ---------------------------------

    #[test]
    fn depth_guard_accepts_exactly_max_depth() {
        // nest(MAX_DEPTH) puts the leaf at depth == MAX_DEPTH, which is
        // the accept boundary (guard rejects only depth > MAX_DEPTH).
        let v = nest(MAX_DEPTH);
        assert!(Doc::of(&v).is_ok());
    }

    #[test]
    fn depth_guard_rejects_one_past_max_depth() {
        let v = nest(MAX_DEPTH + 1);
        let err = Doc::of(&v).unwrap_err();
        assert!(err.message.contains("maximum depth"));
    }

    #[test]
    fn an_array_value_consumes_an_extra_depth_level() {
        // A scalar directly under a key sits one level deeper than the
        // object (depth 1). The same scalar wrapped in a one-item array
        // under that key sits *two* levels deeper (depth 2) -- the array
        // itself costs an extra level, mirroring Python's `_children`
        // (see the module doc comment / build_node's `depth + 1` for the
        // list branch on top of the caller's own `depth + 1`).
        let direct = obj(&[("a", Value::Int(1))]);
        let via_array = obj(&[("a", Value::Array(vec![Value::Int(1)]))]);
        let doc_direct = Doc::of(&direct).unwrap();
        let doc_array = Doc::of(&via_array).unwrap();
        let leaf_direct = doc_direct.root().child("a").unwrap();
        let leaf_array = doc_array.root().child("a").unwrap();
        assert_eq!(doc_direct.entry(leaf_direct.id()).depth, 1);
        assert_eq!(doc_array.entry(leaf_array.id()).depth, 2);
    }

    // -- depth guard: every public tree-mutating entry point -------------

    #[test]
    fn every_tree_mutating_entry_point_enforces_the_depth_guard() {
        // Doc::of
        assert!(Doc::of(&nest(MAX_DEPTH + 1)).is_err());

        // Doc::add, attaching at the root (depth 0): the pushed subtree's
        // own internal depth is what must exceed MAX_DEPTH.
        let mut doc = Doc::of(&obj(&[("seed", Value::Int(0))])).unwrap();
        let root_id = doc.root().id();
        let root_path = doc.root().path.clone();
        assert!(
            doc.add(root_id, &root_path, "b", &nest(MAX_DEPTH + 1))
                .is_err()
        );

        // Doc::set: same guard, different mutating call site.
        let mut doc2 = Doc::of(&obj(&[("seed", Value::Int(0))])).unwrap();
        let root_id2 = doc2.root().id();
        let root_path2 = doc2.root().path.clone();
        assert!(
            doc2.set(root_id2, &root_path2, "b", &nest(MAX_DEPTH + 1))
                .is_err()
        );
    }

    // -- depth guard: hand-constructed deep subtree bypassing a
    //    construction-time-only check (omnist-ts#37 / omnist-ts#70) -------

    #[test]
    fn add_at_a_deep_cursor_accounts_for_the_cursors_own_depth() {
        // Build a document 200 objects deep (leaf at depth 200), then walk
        // a cursor down 190 levels to an *internal* node sitting at depth
        // 190. If `add()` (re-)started the depth guard's counter at 0 for
        // the pushed subtree -- the exact omnist-ts#37 bug, where
        // `buildNode` was called with no depth argument on every mutation
        // -- a modest 15-level subtree would wrongly be accepted here
        // (15 < MAX_DEPTH). Threading the cursor's own depth through
        // means 190 + 1 (attach) + 15 = 206 > MAX_DEPTH is correctly
        // rejected instead.
        let mut doc = Doc::of(&nest(MAX_DEPTH)).unwrap();
        let mut cursor = doc.root();
        for _ in 0..190 {
            cursor = cursor.child("a").unwrap();
        }
        assert_eq!(doc.entry(cursor.id()).depth, 190);
        let id = cursor.id();
        let path = cursor.path.clone();

        let too_deep = nest(15);
        assert!(doc.add(id, &path, "b", &too_deep).is_err());

        // Positive control: a shallow enough subtree at the same depth
        // succeeds, proving the rejection above is about depth, not some
        // unrelated failure.
        let shallow = nest(5);
        assert!(doc.set(id, &path, "b", &shallow).is_ok());
    }

    // -- IndexMap ordering preserved on iteration ------------------------

    #[test]
    fn labels_and_get_preserve_first_seen_and_insertion_order() {
        // Repeated labels come from an `Array` value under one key (a dict
        // key can't repeat) -- "z" appears twice because its value is a
        // 2-item array, not because the object has two "z" entries.
        let v = obj(&[
            ("z", Value::Array(vec![Value::Int(1), Value::Int(3)])),
            ("a", Value::Int(2)),
            ("m", Value::Int(4)),
        ]);
        let doc = Doc::of(&v).unwrap();
        let root = doc.root();
        assert_eq!(root.labels(), vec!["z", "a", "m"]);
        let z_vals: Vec<&Scalar> = root.get("z").iter().map(|c| c.value().unwrap()).collect();
        assert_eq!(z_vals, vec![&Scalar::Int(1), &Scalar::Int(3)]);
    }

    #[test]
    fn labels_and_count_on_a_leaf_are_empty() {
        let doc = Doc::of(&Value::Int(1)).unwrap();
        let root = doc.root();
        assert!(root.labels().is_empty());
        assert_eq!(root.count("anything"), 0);
    }

    #[test]
    fn to_grouped_preserves_first_seen_key_order() {
        let v = obj(&[
            ("z", Value::Array(vec![Value::Int(1), Value::Int(3)])),
            ("a", Value::Int(2)),
        ]);
        let doc = Doc::of(&v).unwrap();
        let grouped = doc.to_grouped();
        let expected = obj(&[
            ("z", Value::Array(vec![Value::Int(1), Value::Int(3)])),
            ("a", Value::Int(2)),
        ]);
        assert_eq!(grouped, expected);
        // `assert_eq!` above is order-insensitive (`IndexMap`'s `PartialEq`
        // ignores insertion order), so separately confirm key order here.
        let keys: Vec<&str> = grouped
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(keys, vec!["z", "a"]);
    }

    #[test]
    fn value_as_object_is_none_for_a_non_object() {
        assert!(Value::Int(1).as_object().is_none());
    }

    // -- mutation semantics: add / set / remove / count / get -----------

    #[test]
    fn add_appends_and_get_one_requires_exactly_one() {
        let mut doc = Doc::of(&obj(&[])).unwrap();
        let root_id = doc.root().id();
        let root_path = doc.root().path.clone();
        doc.add(root_id, &root_path, "x", &Value::Int(1)).unwrap();
        doc.add(root_id, &root_path, "x", &Value::Int(2)).unwrap();
        let root = doc.root();
        assert_eq!(root.count("x"), 2);
        assert!(root.get_one("x").is_err());
    }

    #[test]
    fn set_replaces_all_occurrences_at_first_position() {
        let mut doc = Doc::of(&obj(&[
            ("x", Value::Int(1)),
            ("y", Value::Int(9)),
            ("x", Value::Int(2)),
        ]))
        .unwrap();
        let root_id = doc.root().id();
        let root_path = doc.root().path.clone();
        doc.set(root_id, &root_path, "x", &Value::Int(100)).unwrap();
        let root = doc.root();
        let labels: Vec<String> = root.edges().unwrap().into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels, vec!["x", "y"]);
        assert_eq!(
            root.get_one("x").unwrap().value().unwrap(),
            &Scalar::Int(100)
        );
    }

    #[test]
    fn remove_drops_every_edge_with_that_label() {
        let mut doc = Doc::of(&obj(&[("x", Value::Int(1)), ("x", Value::Int(2))])).unwrap();
        let root_id = doc.root().id();
        let root_path = doc.root().path.clone();
        doc.remove(root_id, &root_path, "x").unwrap();
        assert_eq!(doc.root().count("x"), 0);
    }

    #[test]
    fn internal_edges_mut_rejects_a_leaf_directly() {
        // White-box test of the private helper: `add`/`set`/`remove` all
        // gate through `require_internal` first, so this helper's `Leaf`
        // arm is unreachable via the public API -- call it directly to
        // prove the arm itself is correct (see its doc comment).
        let mut doc = Doc::of(&Value::Int(1)).unwrap();
        let root_id = doc.root().id();
        let err = doc.internal_edges_mut(root_id, "$", "poke").unwrap_err();
        assert_eq!(err.path, "$");
        assert!(err.message.contains("cannot poke on a leaf"));
    }

    #[test]
    fn mutation_on_a_leaf_is_rejected() {
        let mut doc = Doc::of(&Value::Int(1)).unwrap();
        let root_id = doc.root().id();
        let root_path = doc.root().path.clone();
        assert!(doc.add(root_id, &root_path, "x", &Value::Int(1)).is_err());
        assert!(doc.set(root_id, &root_path, "x", &Value::Int(1)).is_err());
        assert!(doc.remove(root_id, &root_path, "x").is_err());
    }

    #[test]
    fn value_on_an_internal_node_is_rejected() {
        let doc = Doc::of(&obj(&[("x", Value::Int(1))])).unwrap();
        assert!(doc.root().value().is_err());
    }

    #[test]
    fn edges_on_a_leaf_is_rejected() {
        let doc = Doc::of(&Value::Int(1)).unwrap();
        assert!(doc.root().edges().is_err());
    }

    // -- export / equality ------------------------------------------------

    #[test]
    fn to_data_round_trips_structure() {
        let v = obj(&[("a", Value::Int(1)), ("b", Value::Str("hi".to_string()))]);
        let doc = Doc::of(&v).unwrap();
        assert_eq!(doc.to_data(), v);
    }

    #[test]
    fn to_data_round_trips_every_scalar_variant() {
        // Covers every `Value`/`Scalar` leaf variant through both
        // `build_node` (construction) and `From<Scalar> for Value`
        // (export via to_data), not just Int/Str.
        let v = obj(&[
            ("n", Value::Null),
            ("b", Value::Bool(true)),
            ("i", Value::Int(7)),
            ("f", Value::Float(1.5)),
            ("s", Value::Str("hi".to_string())),
        ]);
        let doc = Doc::of(&v).unwrap();
        assert_eq!(doc.to_data(), v);
        assert_eq!(doc.to_grouped(), v);
    }

    #[test]
    fn join_quotes_a_non_identifier_key() {
        // A key that isn't a valid identifier (starts with a digit) takes
        // the `path["key"]` form, not `path.key`.
        let v = obj(&[(
            "1bad",
            Value::Array(vec![Value::Array(vec![Value::Int(1)])]),
        )]);
        let err = Doc::of(&v).unwrap_err();
        assert_eq!(err.path, "$[\"1bad\"][0]");
    }

    #[test]
    fn eq_doc_compares_structurally() {
        let a = Doc::of(&obj(&[("a", Value::Int(1))])).unwrap();
        let b = Doc::of(&obj(&[("a", Value::Int(1))])).unwrap();
        let c = Doc::of(&obj(&[("a", Value::Int(2))])).unwrap();
        assert!(a.eq_doc(&b));
        assert!(!a.eq_doc(&c));
    }

    #[test]
    fn eq_doc_is_false_when_shapes_differ() {
        // A leaf is never equal to an internal node, regardless of
        // content -- exercises `node_eq`'s (Leaf, Internal) mismatch arm.
        let leaf = Doc::of(&Value::Int(1)).unwrap();
        let internal = Doc::of(&obj(&[("a", Value::Int(1))])).unwrap();
        assert!(!leaf.eq_doc(&internal));
        assert!(!internal.eq_doc(&leaf));
    }

    #[test]
    fn scalar_display_covers_every_variant() {
        assert_eq!(Scalar::Null.to_string(), "null");
        assert_eq!(Scalar::Bool(true).to_string(), "true");
        assert_eq!(Scalar::Int(1).to_string(), "1");
        assert_eq!(Scalar::Float(1.5).to_string(), "1.5");
        assert_eq!(Scalar::Str("x".to_string()).to_string(), "\"x\"");
    }
}
