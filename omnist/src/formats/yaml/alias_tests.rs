//! Tests for the YAML alias expansion limit (spec §2.4.1, D-18/D-19/D-20):
//! the worked examples of the spec, boundaries, cycles, merge handling,
//! option validation, the bomb shapes, and the false-positive guards.

use std::time::{Duration, Instant};

use super::*;
use crate::document::{RawNode, Scalar};

/// The number of edges under the root's `label` child.
fn edges_under(doc: &Doc, label: &str) -> usize {
    match doc.to_raw() {
        RawNode::Edges(es) => match es.into_iter().find(|(l, _)| l == label) {
            Some((_, RawNode::Edges(inner))) => inner.len(),
            other => panic!("{label} is not a mapping: {other:?}"),
        },
        other => panic!("root is not a mapping: {other:?}"),
    }
}

/// `read_yaml_with` at an explicit maximum.
fn read_at(text: &str, max: u32) -> Result<Doc, OmnistError> {
    let opts = YamlReadOptions {
        max_alias_expansion: max,
    };
    read_yaml_with(text, &opts)
}

/// Whether `err` is the D-18/D-20 rejection: code and path pinned.
fn is_alias_rejection(err: &OmnistError) -> bool {
    matches!(err, OmnistError::Document(e)
        if e.code.as_deref() == Some("document.limit.alias-expansion") && e.path == "$")
}

fn assert_rejected(text: &str, max: u32) {
    let err = read_at(text, max).expect_err("expected an alias expansion rejection");
    assert!(is_alias_rejection(&err), "got {err:?}");
}

fn assert_accepted(text: &str, max: u32) {
    if let Err(e) = read_at(text, max) {
        panic!("expected acceptance at max {max}, got {e:?}");
    }
}

/// The smallest maximum at which `text` is accepted: `ceil(max E)` over every
/// candidate node. Pins the worked examples' arithmetic exactly.
fn smallest_accepting_max(text: &str) -> u32 {
    (1..=10_000)
        .find(|&m| read_at(text, m).is_ok())
        .expect("accepted at the ceiling")
}

/// A flow mapping of `n` integer keys `k1..kn`.
fn flow_map(n: usize) -> String {
    let keys: Vec<String> = (1..=n).map(|i| format!("k{i}: {i}")).collect();
    format!("{{{}}}", keys.join(", "))
}

// ------------------------------------------------------- spec worked examples

#[test]
fn spec_worked_example_unanchored_merge_of_four_aliases_is_6_50() {
    // W(b) = 4, W(t) = 1 + 4 * (4 - 1) = 13, S(t) = 2: E(t) = 6.50.
    let text = "b: &b {k1: 1, k2: 2, k3: 3}\nt: {<<: [*b, *b, *b, *b]}\n";
    assert_rejected(text, 6);
    assert_accepted(text, 7);
    assert_eq!(smallest_accepting_max(text), 7);
}

#[test]
fn spec_worked_example_inline_merge_source_is_0_75() {
    // t: {<<: {a: 1}, z: 1}: W = 3, S = 4 (t, the << slot, a, z): E = 0.75.
    let text = "t: {<<: {a: 1}, z: 1}\n";
    assert_accepted(text, 1);
    assert_eq!(smallest_accepting_max(text), 1);
}

#[test]
fn spec_worked_example_merge_key_with_a_sequence_is_1_33() {
    // z: W = 1 + (2 - 1) + (2 - 1) + 1 = 4, S = 3: E = 1.33, so 2 is the
    // smallest accepting integer maximum and 1 rejects it.
    let text = "p: &p {k: 1}\nq: &q {j: 2}\nz: &z {<<: [*p, *q], m: 3}\n";
    assert_rejected(text, 1);
    assert_eq!(smallest_accepting_max(text), 2);
    // The materialized value is the flattened four-slot mapping.
    let doc = read_at(text, 2).unwrap();
    assert_eq!(edges_under(&doc, "z"), 3);
}

#[test]
fn spec_worked_example_nested_chain_r_is_2_00() {
    // p: W = S = 2. q: W = 1 + (2 - 1) + 1 = 3, S = 3. r: W = 1 + 3 = 4,
    // S = 2: E(r) = 2.00.
    let text = "p: &p {k: 1}\nq: &q {<<: *p, m: 2}\nr: &r {n: *q}\n";
    assert_rejected(text, 1);
    assert_eq!(smallest_accepting_max(text), 2);
}

#[test]
fn spec_worked_example_key_override_overestimates_and_is_still_conformant() {
    // q: {<<: *p, k: 9} computes W = 3 although only {k: 9} is materialized:
    // the bound must NOT be refined by collision resolution (D-19). At max 1
    // the structural E = 3 / 3 = 1.00 is accepted either way; the point is the
    // value still reads correctly.
    let text = "p: &p {k: 1}\nq: &q {<<: *p, k: 9}\n";
    let doc = read_at(text, 1).unwrap();
    assert_eq!(edges_under(&doc, "q"), 1);
}

#[test]
fn anchored_scalar_is_one_to_one_and_never_checked() {
    let text = "c: &c 5\nrefs: [*c, *c, *c, *c, *c, *c, *c, *c]\n";
    assert_eq!(smallest_accepting_max(text), 1);
}

// ----------------------------------------------------------------- boundaries

#[test]
fn boundary_accepts_at_the_limit_and_rejects_one_past() {
    // x: W = S = 5. y: W = 1 + 5 = 6, S = 2: E = 3. top is a plain alias.
    let text = "x: &x\n  a: 1\n  b: 2\n  c: 3\n  d: 4\ny: &y\n  inner: *x\ntop: *y\n";
    assert_accepted(text, 3);
    assert_rejected(text, 2);
}

#[test]
fn boundary_default_is_50_and_the_comparison_is_strictly_greater() {
    // A mapping that merges an n-key anchor and writes one key of its own:
    // W = 1 + n + 1, S = 3, so E = (n + 2) / 3. n = 148 is exactly 50.00
    // (accepted: only E > max rejects); n = 149 is 50.33.
    let at = |n: usize| format!("base: &base {}\nt: {{<<: *base, own: 1}}\n", flow_map(n));
    assert!(read_yaml(&at(148)).is_ok());
    let err = read_yaml(&at(149)).unwrap_err();
    assert!(is_alias_rejection(&err), "got {err:?}");
    // The README's example: 150 keys is E = 50.67 and is rejected.
    assert!(is_alias_rejection(&read_yaml(&at(150)).unwrap_err()));
    // ... and raising the option accepts it.
    assert_accepted(&at(150), 51);
}

// ------------------------------------------------------------ option checking

#[test]
fn option_default_is_50_and_zero_selects_it() {
    assert_eq!(YamlReadOptions::default().max_alias_expansion, 50);
    assert_eq!(DEFAULT_MAX_ALIAS_EXPANSION, 50);
    assert_eq!(
        YamlReadOptions::default().effective_max_alias_expansion(),
        50
    );
    let zero = YamlReadOptions {
        max_alias_expansion: 0,
    };
    assert_eq!(zero.effective_max_alias_expansion(), 50);
    let text = format!("base: &base {}\nt: {{<<: *base, own: 1}}\n", flow_map(149));
    // Zero never widens the limit: it behaves exactly like the default.
    assert!(is_alias_rejection(&read_at(&text, 0).unwrap_err()));
    let text148 = format!("base: &base {}\nt: {{<<: *base, own: 1}}\n", flow_map(148));
    assert_accepted(&text148, 0);
}

#[test]
fn option_ceiling_is_10000_and_larger_is_refused_not_clamped() {
    assert_eq!(MAX_ALIAS_EXPANSION_CEILING, 10_000);
    assert_accepted("a: 1\n", 10_000);
    let mut opts = YamlReadOptions {
        max_alias_expansion: 10_001,
    };
    let e = opts.validate().unwrap_err();
    assert_eq!(e.code, None);
    assert!(e.message.contains("10001") && e.message.contains("10000"));
    // read_yaml_with refuses before reading anything, error unchanged.
    let err = read_yaml_with("a: 1\n", &opts).unwrap_err();
    assert!(matches!(&err, OmnistError::Document(d) if d.code.is_none() && d.message == e.message));
    assert!(YamlReadOptions::default().validate().is_ok());
    opts.max_alias_expansion = 10_000;
    assert!(opts.validate().is_ok());
    opts.max_alias_expansion = u32::MAX;
    assert!(opts.validate().is_err());
}

#[test]
fn a_raised_maximum_accepts_what_the_default_rejects_and_a_lowered_one_rejects_more() {
    let text = format!("base: &base {}\nt: {{<<: *base, own: 1}}\n", flow_map(300));
    assert!(is_alias_rejection(&read_yaml(&text).unwrap_err()));
    assert_accepted(&text, 101);
    assert_rejected(&text, 100);
}

// --------------------------------------------------------------------- cycles

#[test]
fn self_referential_anchors_are_rejected_under_the_same_code() {
    for text in [
        "a: &a\n  b: *a\n",
        "a: &a {<<: *a, k: 1}\n",
        "a: &a [*a]\n",
        "a: &a {<<: [*a], k: 1}\n",
        // Through another definition: c names the still-open a.
        "a: &a\n  b: &b\n    c: *a\n",
        "a: &a\n  b: &b\n    <<: *a\n",
        // Inside a sequence that is itself the anchored node.
        "a: &a\n  - 1\n  - *a\n",
    ] {
        for max in [1, 50, 10_000] {
            assert_rejected(text, max);
        }
    }
}

#[test]
fn a_cycle_is_rejected_before_it_is_ever_materialized() {
    // If a cyclic or self-merging structure were built, the old behaviour
    // silently accepted `{k: 1}`; D-20 forbids that.
    let err = read_yaml("a: &a {<<: *a, k: 1}\n").unwrap_err();
    assert!(is_alias_rejection(&err), "got {err:?}");
}

// ------------------------------------------------------------- merge handling

#[test]
fn a_merge_sequence_holds_no_slot_and_the_merge_entry_is_one_slot() {
    // <<: [*p, *q] adds one S slot ("<<") and no carrier slot: z has S = 3.
    // With 4 aliases of a 3-key anchor E(t) is 6.50 (spec example), so the
    // carrier sequence neither adds a W slot nor an S slot.
    let carrier = "b: &b {k1: 1, k2: 2, k3: 3}\nt: {<<: [*b, *b, *b, *b]}\n";
    assert_eq!(smallest_accepting_max(carrier), 7);
    // The single-alias form: W = 1 + 3 = 4, S = 2: E = 2.
    let single = "b: &b {k1: 1, k2: 2, k3: 3}\nt: {<<: *b}\n";
    assert_eq!(smallest_accepting_max(single), 2);
}

#[test]
fn the_merged_result_is_unchanged_by_the_check() {
    let text = "base: &b\n  x: 1\n  y: 2\nchild:\n  <<: *b\n  y: 20\n  z: 3\n";
    let doc = read_yaml(text).unwrap();
    let child = doc.root().get_one("child").unwrap();
    assert_eq!(
        *child.get_one("y").unwrap().value().unwrap(),
        Scalar::Int(20.into())
    );
    assert_eq!(
        *child.get_one("x").unwrap().value().unwrap(),
        Scalar::Int(1.into())
    );
}

#[test]
fn an_inline_merge_source_with_aliases_can_exceed_the_maximum_on_its_own() {
    // The inline mapping {x1: *b, x2: *b} is a candidate: W = 1 + 2 * 4 = 9,
    // S = 3: E = 3 (vector inline-merge-source-at-declared-limit). The outer t
    // flattens it: W(t) = 1 + (9 - 1) + 1 = 10, S(t) = 1 + 1 + 2 + 1 = 5: E = 2.
    let text = "base: &b {k1: 1, k2: 2, k3: 3}\nt: {<<: {x1: *b, x2: *b}, z: 1}\n";
    assert_accepted(text, 3);
    assert_rejected(text, 2);
    // The plain inline form with no aliases is exactly 1.00.
    assert_eq!(smallest_accepting_max("t: {<<: {a: 1, b: 2}, z: 1}\n"), 1);
}

#[test]
fn a_merge_sequence_may_mix_inline_mappings_and_aliases() {
    let text = "p: &p {k: 1}\nt: {<<: [{a: 1}, *p], z: 1}\n";
    assert_accepted(text, 2);
    let doc = read_at(text, 2).unwrap();
    assert_eq!(edges_under(&doc, "t"), 3);
}

#[test]
fn an_anchored_merge_sequence_is_an_ordinary_flattened_value() {
    // The documented reading of the spec's edge: an anchored literal merge
    // sequence is not the syntactic carrier. It must still read and merge.
    let text = "p: &p {k: 1}\nq: &q {j: 2}\nt: {<<: &s [*p, *q], m: 3}\n";
    let doc = read_at(text, 50).unwrap();
    assert_eq!(edges_under(&doc, "t"), 3);
}

#[test]
fn a_merge_of_a_scalar_is_still_a_clean_parse_error_not_an_alias_rejection() {
    let err = read_yaml("a: &a 1\nb: {<<: *a}\n").unwrap_err();
    assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
}

#[test]
fn a_quoted_or_tagged_merge_key_is_an_ordinary_key_for_the_count() {
    // "<<" quoted is a literal key: a plain alias value, W(b) in full.
    let text = "b: &b {k1: 1, k2: 2, k3: 3}\nt: {\"<<\": *b}\n";
    // W(t) = 1 + 4 = 5, S(t) = 2: E = 2.5.
    assert_eq!(smallest_accepting_max(text), 3);
}

#[test]
fn an_unanchored_sequence_that_holds_aliases_is_a_candidate() {
    let text = "b: &b {k1: 1, k2: 2, k3: 3, k4: 4, k5: 5, k6: 6, k7: 7, k8: 8}\n\
                t: [*b, *b, *b, *b, *b, *b, *b, *b, *b, *b]\n";
    // W(t) = 1 + 10 * 9 = 91, S(t) = 11: E = 8.27.
    assert_rejected(text, 8);
    assert_accepted(text, 9);
}

#[test]
fn the_document_root_is_a_candidate() {
    // Every aliasing entry is small on its own; only the root as a whole
    // exceeds. b: W = S = 11. Root: W = 1 + 11 + 14 * 11, S = 1 + 11 + 14.
    let mut text = String::from(
        "base: &b {k1: 1, k2: 2, k3: 3, k4: 4, k5: 5, k6: 6, k7: 7, k8: 8, k9: 9, k10: 10}\n",
    );
    for i in 1..=14 {
        text.push_str(&format!("r{i}: *b\n"));
    }
    // E(root) = 166 / 26 = 6.38.
    assert_rejected(&text, 6);
    assert_accepted(&text, 7);
}

#[test]
fn a_nested_container_is_checked_on_its_own_not_diluted_by_a_large_document() {
    // 200 plain entries dilute the root to ~1.0, but the one amplifying
    // mapping is still rejected where it sits.
    let mut text = String::new();
    for i in 0..200 {
        text.push_str(&format!("p{i}: {i}\n"));
    }
    text.push_str("b: &b [1,2,3,4,5,6,7,8,9,10]\nt: {k: *b}\n");
    // E(t) = (1 + 11) / 2 = 6.
    assert_rejected(&text, 5);
    assert_accepted(&text, 6);
}

// ------------------------------------------------------------- check ordering

#[test]
fn a_scan_error_wins_over_an_earlier_alias_rejection() {
    // As every earlier self-reference rejection did: the stream is scanned to
    // the end first.
    let err = read_yaml("a: &a {<<: *a}\nb: [1, 2\n").unwrap_err();
    assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
}

/// 20 scalars; 100 of them; 100 of those: 210101 nodes at E = 2080, so only a
/// raised maximum lets it past D-18, and the node cap then refuses it.
fn node_cap_text() -> String {
    let leaves: Vec<&str> = vec!["x"; 20];
    let mut text = format!("a0: &a0 [{}]\n", leaves.join(", "));
    text.push_str(&format!("a1: &a1 [{}]\n", vec!["*a0"; 100].join(", ")));
    text.push_str(&format!("a2: &a2 [{}]\n", vec!["*a1"; 100].join(", ")));
    text
}

#[test]
fn the_materialized_node_cap_still_applies_below_the_expansion_limit() {
    let text = node_cap_text();
    let err = read_at(&text, 10_000).unwrap_err();
    assert!(
        matches!(&err, OmnistError::Parse(e) if e.code == "document.limit.nodes"),
        "got {err:?}"
    );
    // At the default it is the expansion limit, and the code says so.
    assert!(is_alias_rejection(&read_yaml(&text).unwrap_err()));
}

#[test]
fn deep_nesting_far_past_the_cap_still_reports_depth_not_a_stack_overflow() {
    // One more indent per line: block collections have no parser-side limit.
    let deep: String = (0..1000)
        .map(|i| format!("{}a:\n", " ".repeat(i)))
        .collect();
    let err = read_yaml(&deep).unwrap_err();
    assert!(
        matches!(&err, OmnistError::Document(e) if e.code.as_deref() == Some("document.limit.depth")),
        "got {err:?}"
    );
}

// ---------------------------------------------------------------------- bombs

/// Billion-laughs generations: `a0` holds two scalars, every later anchor
/// holds `branching` aliases of the previous one.
fn bomb(branching: usize, levels: usize) -> String {
    let mut out = String::from("a0: &a0 [x, x]\n");
    for i in 1..levels {
        let refs = vec![format!("*a{}", i - 1); branching].join(", ");
        out.push_str(&format!("a{i}: &a{i} [{refs}]\n"));
    }
    out
}

fn timed_rejection(text: &str) -> Duration {
    let start = Instant::now();
    let err = read_yaml(text).unwrap_err();
    let elapsed = start.elapsed();
    assert!(is_alias_rejection(&err), "got {err:?}");
    elapsed
}

#[test]
fn bombs_are_rejected_fast_with_the_alias_code() {
    let limit = Duration::from_secs(2);
    for (b, l) in [(4, 10), (2, 30), (50, 4)] {
        let took = timed_rejection(&bomb(b, l));
        assert!(took < limit, "branching {b} x {l} took {took:?}");
    }
}

#[test]
fn bomb_unanchored_merge_fan_in_of_2000_by_2000() {
    let mut text = format!("base: &b {}\n", flow_map(2000));
    text.push_str(&format!("t: {{<<: [{}]}}\n", vec!["*b"; 2000].join(", ")));
    let took = timed_rejection(&text);
    assert!(took < Duration::from_secs(5), "took {took:?}");
}

#[test]
fn bomb_root_list_of_aliasing_mappings_100000_by_1000() {
    let scalars: Vec<String> = (0..1000).map(|i| i.to_string()).collect();
    let mut text = format!("base: &b [{}]\nlist:\n", scalars.join(", "));
    for _ in 0..100_000 {
        text.push_str("  - {k: *b}\n");
    }
    // The input itself is about 1.3 MB: parse (scan) cost is linear in it and is
    // paid in full; the check refuses at the first aliasing mapping and no
    // expansion is built.
    let took = timed_rejection(&text);
    assert!(took < Duration::from_secs(20), "took {took:?}");
}

// ------------------------------------------------------------ false positives

#[test]
fn compose_style_100_services_merging_a_20_key_defaults_block_is_accepted() {
    let keys: Vec<String> = (1..=20).map(|i| format!("  d{i}: {i}\n")).collect();
    let mut text = format!("x-defaults: &d\n{}services:\n", keys.concat());
    for i in 0..100 {
        text.push_str(&format!("  svc{i}:\n    <<: *d\n    image: img{i}\n"));
    }
    assert!(read_yaml(&text).is_ok());
}

#[test]
fn a_100_key_block_aliased_60_times_at_the_root_is_accepted_and_100_times_rejected() {
    let build = |n: usize| {
        let keys: String = (1..=100).map(|i| format!("  k{i}: {i}\n")).collect();
        let mut text = format!("base: &b\n{keys}");
        for i in 0..n {
            text.push_str(&format!("r{i}: *b\n"));
        }
        text
    };
    assert!(read_yaml(&build(60)).is_ok());
    assert!(is_alias_rejection(&read_yaml(&build(100)).unwrap_err()));
}

#[test]
fn a_scalar_aliased_500_times_is_accepted() {
    let mut text = String::from("c: &c hello\n");
    for i in 0..500 {
        text.push_str(&format!("r{i}: *c\n"));
    }
    assert!(read_yaml(&text).is_ok());
}

#[test]
fn a_depth_3_anchor_chain_is_accepted() {
    let text = "a: &a {x: 1, y: 2}\nb: &b {p: *a, q: *a}\nc: &c {m: *b, n: *b}\nd: {z: *c}\n";
    assert!(read_yaml(text).is_ok());
}

#[test]
fn an_ordinary_document_with_no_aliases_reads_at_the_smallest_maximum() {
    let text = "a: 1\nb: [1, 2, {c: 3}]\nd:\n  e: x\n";
    assert_eq!(smallest_accepting_max(text), 1);
}

#[test]
fn anchors_redefined_later_are_independent_definitions() {
    let text = "a: &x {k: 1}\nb: *x\nc: &x {k: 1, j: 2}\nd: *x\n";
    assert!(read_yaml(text).is_ok());
}

// ---------------------------------------------------- every entry point checks

#[test]
fn the_registry_codec_and_every_reader_path_apply_the_limit() {
    let text = bomb(4, 10);
    let fmt = crate::registry::get_format("yaml").unwrap();
    assert!(is_alias_rejection(&(fmt.read)(&text).unwrap_err()));
    assert!(is_alias_rejection(&read_yaml(&text).unwrap_err()));
    assert!(is_alias_rejection(
        &read_yaml_with(&text, &YamlReadOptions::default()).unwrap_err()
    ));
    let wide = YamlReadOptions {
        max_alias_expansion: 10_000,
    };
    // Even the widest maximum refuses this bomb as an expansion (E ~ 100000).
    assert!(is_alias_rejection(
        &read_yaml_with(&text, &wide).unwrap_err()
    ));
    // The node cap is reached only below the ceiling.
    assert!(matches!(
        read_yaml_with(&node_cap_text(), &wide).unwrap_err(),
        OmnistError::Parse(e) if e.code == "document.limit.nodes"
    ));
}

#[test]
fn a_leading_bom_and_a_following_syntax_error_keep_their_codes() {
    assert!(read_yaml("\u{feff}a: 1\n").is_ok());
    assert!(matches!(
        read_yaml("\u{feff}\u{feff}a: 1\n").unwrap_err(),
        OmnistError::Parse(_)
    ));
}

/// The example in `docs/formats/yaml.md` ("The maximum is configurable").
#[test]
fn doc_example_raise_the_maximum() {
    let options = YamlReadOptions::default().with_max_alias_expansion(200);
    let doc = read_yaml_with("a: &x [1, 2]\nb: *x\n", &options).unwrap();
    assert_eq!(doc.root().get("b").len(), 2);
}

#[test]
fn inline_merge_sources_add_their_slots_minus_the_container_to_s() {
    // p: W = S = 9. Each inline {a: *p}: W = 10, S = 2 (E = 5). In r the
    // carrier adds 3 * (10 - 1) to W and 3 * (2 - 1) to S: W = 28, S = 5,
    // E = 5.6, so 6 is the smallest accepting maximum. Counting the inline
    // container's slot too (s instead of s - 1) would give S = 8 and 5.
    let keys: Vec<String> = (0..8).map(|i| format!("k{i}: 1")).collect();
    let text = format!(
        "p: &p {{{}}}\nr: {{<<: [{{a: *p}}, {{a: *p}}, {{a: *p}}]}}\n",
        keys.join(", ")
    );
    assert_accepted(&text, 6);
    assert_rejected(&text, 5);
}
