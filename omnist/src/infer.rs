//! Schema inference: draft a `record` [`Schema`] that accepts a set of
//! sample [`Doc`]uments.
//!
//! Ported from `~/dev/omnist/omnist/infer.py` (issue #14). Given one or
//! more sample Documents, [`infer`] drafts a `record` schema that accepts
//! them:
//!
//! * a label present in every sample with count 1 becomes a required field
//!   (`[1,1]`); absent in some samples -> `[0,1]`; seen more than once ->
//!   an array (`[min, None]`, permissive on length);
//! * scalar children become one [`crate::schema::Scalar`] (nullable if any
//!   sample was `null`). Samples disagreeing on scalar shape raise, except
//!   `integer`/`number` mixing, which collapses to `number` (the one
//!   subset relation between scalars -- see `docs/design/model.md`);
//! * object children become a nested, named `record` (recursively).
//!
//! Since the model has no inline records, nested records are given
//! generated names derived from their label.
//!
//! [`infer`] deliberately does **not** auto-normalize: the raw result keeps
//! a 1:1 correspondence between sample labels and generated record names,
//! which may therefore contain structurally-identical duplicate records.
//! Call [`crate::ops::normalize`] on the result where a canonical minimal
//! schema is wanted (issue #12, itself resolving issues #143/#151 in the
//! Python reference).
//!
//! ## `allow_any` and `AnyFallback`
//!
//! [`infer`] keeps its original two-argument signature and always infers
//! with `allow_any: false` (matching its long-standing behavior). Two
//! scenarios can't be resolved to one precise type from the samples alone:
//!
//! 1. a label whose samples mix objects and scalars;
//! 2. a label whose scalar samples disagree on kind in a way that isn't the
//!    integer/number subset relation (e.g. `string` and `boolean` seen
//!    under the same label).
//!
//! With `allow_any: false` (via [`infer`], or [`infer_with_report`] called
//! that way), both scenarios are a [`SchemaError`]. [`infer_with_report`]
//! additionally accepts `allow_any: true`, matching Python's
//! `infer_with_report`/`AnyFallback`: instead of erroring, the field is
//! opened as [`crate::schema::FieldType::Any`] and one [`AnyFallback`] is
//! recorded (`location` is `RecordName.label`; `reason` says why).
//!
//! ## No native temporal input
//!
//! [`crate::document::Scalar`] has no `date`/`time`/`datetime` variant (see
//! `document.rs`'s module doc), so unlike the Python reference (which can
//! receive real `datetime.date` sample values), every string sample here
//! infers as `string` -- never `date`/`time`/`datetime` -- regardless of
//! its shape. A schema wanting a temporal field has to be authored (or
//! edited in after inference), not inferred from string-shaped samples.
//! This is a deliberate architecture consequence of issue #4's Value model,
//! not a bug.

use indexmap::{IndexMap, IndexSet};

use crate::document::{Doc, Scalar as DocScalar};
use crate::error::SchemaError;
use crate::schema::{Field, FieldType, Record, Ref, Scalar, ScalarKind, Schema};

/// A single field [`infer_with_report`] opened as `any` under
/// `allow_any: true`. `location` reads `RecordName.label`; `reason` says why
/// the field could not be given a single precise type. Mirrors Python's
/// `AnyFallback` dataclass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnyFallback {
    pub location: String,
    pub reason: String,
}

/// Infers a `record` [`Schema`] (rooted at `root_name`) that accepts every
/// sample in `samples`. Every sample's root must be an object (a record
/// shape) -- an empty `samples` list, or any sample whose root is a bare
/// scalar, is a [`SchemaError`]. Always infers with `allow_any: false` --
/// see [`infer_with_report`] for the `allow_any: true` variant.
pub fn infer(samples: &[Doc], root_name: &str) -> Result<Schema, SchemaError> {
    infer_with_report(samples, root_name, false).map(|(schema, _)| schema)
}

/// Like [`infer`], but also takes `allow_any` and returns every
/// [`AnyFallback`] recorded along the way (empty when `allow_any` is
/// `false`, since every ambiguous field is a hard error in that mode
/// instead). Mirrors Python's `infer_with_report`.
pub fn infer_with_report(
    samples: &[Doc],
    root_name: &str,
    allow_any: bool,
) -> Result<(Schema, Vec<AnyFallback>), SchemaError> {
    if samples.is_empty() {
        return Err(SchemaError::new("cannot infer a schema from zero samples"));
    }
    for s in samples {
        if s.root().is_leaf() {
            return Err(SchemaError::new(
                "infer expects object (record) samples at the root",
            ));
        }
    }
    let mut env: IndexMap<String, Record> = IndexMap::new();
    let mut used: IndexSet<String> = IndexSet::new();
    let mut fallbacks: Vec<AnyFallback> = Vec::new();
    let roots: Vec<_> = samples.iter().map(Doc::root).collect();
    infer_record(
        &roots,
        root_name,
        &mut env,
        &mut used,
        allow_any,
        &mut fallbacks,
    )?;
    let schema = Schema::new(Ref::new(root_name), env)?;
    Ok((schema, fallbacks))
}

// Note: unlike the Python reference's `_infer_record`, there is no explicit
// `depth > MAX_DEPTH` guard here. Every `node` this function ever sees
// comes from a `Doc` (via `Doc::root()`/`Cursor::edges()`), and `Doc`
// construction (`crate::document::build_node`/`check_write_depth`) already
// rejects anything past `MAX_DEPTH` before a `Doc` can exist at all -- so
// recursing one level per nested record can never itself exceed a bound
// the input was already forced under. Mirrors `document.rs`'s decision to
// drop Python's `_check_int_digits` guard rather than carry forward
// permanently-dead code: a depth check here would have no reachable
// failing branch to test (confirmed by trying to construct a
// deeper-than-`MAX_DEPTH` `Doc` sample in `infer::tests` -- `Doc::of` itself
// errors first, every time).
fn infer_record(
    nodes: &[crate::document::Cursor<'_>],
    name: &str,
    env: &mut IndexMap<String, Record>,
    used: &mut IndexSet<String>,
    allow_any: bool,
    fallbacks: &mut Vec<AnyFallback>,
) -> Result<(), SchemaError> {
    used.insert(name.to_string());

    // Pass 1: every label that appears at all, in first-seen order across
    // samples (not just within one sample) -- this keeps the result
    // independent of sample order, per the Python reference's rationale.
    let mut order: Vec<String> = Vec::new();
    let mut seen_labels: IndexSet<String> = IndexSet::new();
    for node in nodes {
        for label in node.labels() {
            if seen_labels.insert(label.clone()) {
                order.push(label);
            }
        }
    }

    // Pass 2: one count per sample for every label (defaulting to 0), plus
    // the actual child cursors for type inference.
    let mut children: IndexMap<String, Vec<crate::document::Cursor<'_>>> =
        order.iter().map(|l| (l.clone(), Vec::new())).collect();
    let mut per_sample_counts: IndexMap<String, Vec<usize>> =
        order.iter().map(|l| (l.clone(), Vec::new())).collect();
    for node in nodes {
        let edges = node.edges().expect("root already confirmed non-leaf");
        let mut counts_here: IndexMap<&str, usize> = IndexMap::new();
        for (label, child) in &edges {
            *counts_here.entry(label.as_str()).or_insert(0) += 1;
            children.get_mut(label).unwrap().push(child.clone());
        }
        for label in &order {
            let c = counts_here.get(label.as_str()).copied().unwrap_or(0);
            per_sample_counts.get_mut(label).unwrap().push(c);
        }
    }

    let mut fields: Vec<Field> = Vec::with_capacity(order.len());
    for label in &order {
        let counts = &per_sample_counts[label];
        let lo = *counts.iter().min().unwrap();
        let hi = *counts.iter().max().unwrap();
        let (cmin, cmax) = if hi > 1 { (0, None) } else { (lo, Some(1)) };
        let ty = infer_type(
            &children[label],
            label,
            name,
            env,
            used,
            allow_any,
            fallbacks,
        )?;
        fields.push(Field::new(label.clone(), ty, cmin, cmax)?);
    }
    env.insert(name.to_string(), Record::new(fields)?);
    Ok(())
}

fn infer_type(
    child_nodes: &[crate::document::Cursor<'_>],
    label: &str,
    record_name: &str,
    env: &mut IndexMap<String, Record>,
    used: &mut IndexSet<String>,
    allow_any: bool,
    fallbacks: &mut Vec<AnyFallback>,
) -> Result<FieldType, SchemaError> {
    let is_obj: Vec<bool> = child_nodes.iter().map(|c| !c.is_leaf()).collect();
    if is_obj.iter().all(|&b| b) {
        let rec_name = unique_name(label, used);
        infer_record(child_nodes, &rec_name, env, used, allow_any, fallbacks)?;
        return Ok(FieldType::Ref(Ref::new(rec_name)));
    }
    if is_obj.iter().any(|&b| b) {
        if allow_any {
            fallbacks.push(AnyFallback {
                location: format!("{record_name}.{label}"),
                reason: "mixes objects and values".to_string(),
            });
            return Ok(FieldType::Any);
        }
        return Err(SchemaError::new(format!(
            "label {label:?} mixes objects and values; cannot infer one type"
        )));
    }
    // All scalars.
    let mut names: IndexSet<&'static str> = IndexSet::new();
    let mut null = false;
    for c in child_nodes {
        let v = c.value().expect("scalar node confirmed by is_obj check");
        // Inlined rather than routed through a separate "value -> kind
        // name" helper: a helper covering all five `DocScalar` variants
        // would need an unreachable `Null` arm (this loop already peels
        // `Null` off first), which is exactly the kind of permanently-dead
        // branch the porting playbook flags -- matching directly here
        // keeps every arm real and independently tested (see
        // `infer::tests` for one sample of each kind, including a `null`
        // mixed in). A `Str` sample always infers `"string"`, never
        // `date`/`time`/`datetime` -- those only infer from a genuinely
        // Date/Time/Datetime-typed sample (issue #105; before that variant
        // existed, this was structurally impossible for any sample --
        // `docs/limitations.md`'s prior "infer never infers temporal
        // kinds" note is now stale for samples sourced from a format that
        // can produce one, e.g. OML's/TOML's own native temporal grammar).
        match v {
            DocScalar::Null => null = true,
            DocScalar::Bool(_) => {
                names.insert("boolean");
            }
            DocScalar::Int(_) => {
                names.insert("integer");
            }
            DocScalar::Float(_) => {
                names.insert("number");
            }
            DocScalar::Str(_) => {
                names.insert("string");
            }
            DocScalar::Date(_) => {
                names.insert("date");
            }
            DocScalar::Time(_) => {
                names.insert("time");
            }
            DocScalar::Datetime(_) => {
                names.insert("datetime");
            }
        }
    }
    if names.contains("number") {
        names.shift_remove("integer"); // the one subset relation
    }
    if names.is_empty() {
        // No non-null sample observed -- default to (nullable) string.
        return Ok(FieldType::Scalar(Scalar::new(ScalarKind::String, null)));
    }
    if names.len() > 1 {
        let mut sorted: Vec<&str> = names.into_iter().collect();
        sorted.sort_unstable();
        if allow_any {
            fallbacks.push(AnyFallback {
                location: format!("{record_name}.{label}"),
                reason: format!(
                    "values of more than one scalar kind ({})",
                    sorted.join(", ")
                ),
            });
            return Ok(FieldType::Any);
        }
        return Err(SchemaError::new(format!(
            "label {label:?} has values of more than one scalar ({}); cannot infer one \
             scalar type",
            sorted.join(", ")
        )));
    }
    let kind = ScalarKind::parse(names.iter().next().unwrap())
        .expect("names only ever holds known scalar kind names, inserted above");
    Ok(FieldType::Scalar(Scalar::new(kind, null)))
}

/// A fresh, `used`-unique `PascalCase` record name derived from `base`
/// (typically a field label). Mirrors the Python reference's `_unique`.
fn unique_name(base: &str, used: &mut IndexSet<String>) -> String {
    let ident = identifier(base);
    let name = if ident.is_empty() {
        "Rec".to_string()
    } else {
        ident
    };
    // `name` is always non-empty here: either `ident` was non-empty, or the
    // "Rec" fallback above kicked in -- so there's always a first char to
    // uppercase. `.expect()` documents that instead of leaving a `None` arm
    // with no reachable input to test (same "confirmed unreachable, not
    // assumed" bar as document.rs's `mandatory_u32`).
    let mut chars = name.chars();
    let first = chars
        .next()
        .expect("name is never empty: identifier()'s fallback or the \"Rec\" default guarantees a first char");
    let name = first.to_uppercase().collect::<String>() + chars.as_str();
    let mut cand = name.clone();
    let mut i = 2;
    while used.contains(&cand) {
        cand = format!("{name}{i}");
        i += 1;
    }
    used.insert(cand.clone());
    cand
}

/// Substitutes every non-alnum/underscore char with `_`, then strips
/// leading digits/underscores -- falling back to the substituted-but-
/// unstripped string if that would leave nothing. Mirrors the Python
/// reference's `_identifier`.
fn identifier(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = out.trim_start_matches(|c: char| c.is_ascii_digit() || c == '_');
    if trimmed.is_empty() {
        out
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests;
