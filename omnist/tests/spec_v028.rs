//! omnist-spec v0.28.0-beta programmatic-schema rules (DIV-5: no conformance
//! vector can pin these, so these tests are the only pin).
//!
//! * S-8: a bad record name or `Ref` target name, reached by direct
//!   construction, is `schema.invalid-name` at `$` (name in the message only).
//! * S-24 / OSD-16: `to_osd` fails with `write.unsupported-value` at the
//!   record path `R` on a `max = 0` field. `[0,0]` stays representable.
//! * S-5/E-13 interplay and `prune`: never emits `max = 0` from a rebuilt
//!   record.
//! * S-22 (`schema.invalid-label`) and S-23 (`schema.unknown-record`) have no
//!   Rust surface: see `docs/limitations.md`.

use indexmap::IndexMap;
use omnist::ops::{normalize, prune};
use omnist::osd::to_osd;
use omnist::schema::{Field, FieldType, Record, Ref, STRING, Schema};

fn env(pairs: Vec<(&str, Record)>) -> IndexMap<String, Record> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn rec(fields: Vec<Field>) -> Record {
    Record::new(fields).unwrap()
}

const BAD_NAMES: &[&str] = &[
    "",
    "bad name",
    "9lives",
    "a-b",
    "a.b",
    "caf\u{e9}",
    "a\nb",
    "$",
    "R.a",
];

const GOOD_NAMES: &[&str] = &["R", "_", "_x9", "a1B2", "any_", "String"];

#[test]
fn bad_record_name_is_invalid_name_at_dollar() {
    for bad in BAD_NAMES {
        let e = Schema::new(
            Ref::new(*bad),
            env(vec![(
                bad,
                rec(vec![Field::required("a", STRING).unwrap()]),
            )]),
        )
        .unwrap_err();
        assert_eq!(e.code, "schema.invalid-name", "name {bad:?}");
        assert_eq!(e.path, "$", "name {bad:?}");
        assert!(e.message.contains(&format!("{bad:?}")), "msg {}", e.message);
    }
}

#[test]
fn bad_ref_target_name_is_invalid_name_at_dollar() {
    for bad in BAD_NAMES {
        // As a field's Ref target.
        let e = Schema::new(
            Ref::new("R"),
            env(vec![(
                "R",
                rec(vec![Field::required("a", Ref::new(*bad)).unwrap()]),
            )]),
        )
        .unwrap_err();
        assert_eq!(e.code, "schema.invalid-name", "ref {bad:?}");
        assert_eq!(e.path, "$", "ref {bad:?}");
        assert!(e.message.contains(&format!("{bad:?}")));
        // As the root reference.
        let e = Schema::new(
            Ref::new(*bad),
            env(vec![(
                "R",
                rec(vec![Field::required("a", STRING).unwrap()]),
            )]),
        )
        .unwrap_err();
        assert_eq!(e.code, "schema.invalid-name", "root {bad:?}");
        assert_eq!(e.path, "$", "root {bad:?}");
    }
}

#[test]
fn invalid_name_path_never_contains_the_label() {
    // A label that S-22 might also reject must not leak into the path.
    let e = Schema::new(
        Ref::new("R"),
        env(vec![(
            "R",
            rec(vec![
                Field::required("secret label", Ref::new("no good")).unwrap(),
            ]),
        )]),
    )
    .unwrap_err();
    assert_eq!(e.path, "$");
    assert!(!e.path.contains("secret"));
}

#[test]
fn valid_names_still_accepted() {
    for good in GOOD_NAMES {
        let s = Schema::new(
            Ref::new(*good),
            env(vec![(
                good,
                rec(vec![Field::required("a", STRING).unwrap()]),
            )]),
        );
        // `String` is a valid name (scalar keywords are lowercase).
        assert!(s.is_ok(), "name {good:?}: {s:?}");
    }
}

#[test]
fn reserved_names_keep_their_own_code() {
    let e = Schema::new(
        Ref::new("any"),
        env(vec![(
            "any",
            rec(vec![Field::required("a", STRING).unwrap()]),
        )]),
    )
    .unwrap_err();
    assert_eq!(e.code, "schema.reserved-name");
}

fn zero_max_schema(extra_first: bool) -> Schema {
    let mut pairs = vec![];
    if extra_first {
        pairs.push(("A", rec(vec![Field::required("ok", STRING).unwrap()])));
    }
    pairs.push((
        "R",
        rec(vec![
            Field::required("keep", STRING).unwrap(),
            Field::new("never", STRING, 0, Some(0)).unwrap(),
        ]),
    ));
    Schema::new(Ref::new("R"), env(pairs)).unwrap()
}

#[test]
fn zero_zero_is_representable_but_to_osd_fails_at_record_path() {
    for extra_first in [false, true] {
        let s = zero_max_schema(extra_first);
        for indent in [None, Some(2)] {
            let e = to_osd(&s, indent).unwrap_err();
            assert_eq!(e.code.as_deref(), Some("write.unsupported-value"));
            assert_eq!(
                e.path.as_deref(),
                Some("R"),
                "path is the record, not R.never"
            );
            assert!(!e.message.contains("R.never"));
        }
    }
}

#[test]
fn to_osd_reports_first_offending_record_in_declaration_order() {
    let s = Schema::new(
        Ref::new("A"),
        env(vec![
            ("A", rec(vec![Field::required("ok", STRING).unwrap()])),
            ("B", rec(vec![Field::new("z", STRING, 0, Some(0)).unwrap()])),
            ("C", rec(vec![Field::new("z", STRING, 0, Some(0)).unwrap()])),
        ]),
    )
    .unwrap();
    assert_eq!(to_osd(&s, None).unwrap_err().path.as_deref(), Some("B"));
}

#[test]
fn nonzero_max_and_unbounded_still_write() {
    let s = Schema::new(
        Ref::new("R"),
        env(vec![(
            "R",
            rec(vec![
                Field::new("a", STRING, 0, Some(1)).unwrap(),
                Field::new("b", STRING, 0, None).unwrap(),
            ]),
        )]),
    )
    .unwrap();
    assert!(to_osd(&s, None).is_ok());
}

#[test]
fn prune_removes_zero_max_from_rebuilt_records() {
    let s = zero_max_schema(true);
    let p = prune(&s);
    for r in p.env().values() {
        assert!(r.fields().iter().all(|f| f.max != Some(0)));
    }
    assert!(to_osd(&p, None).is_ok(), "prune-then-write succeeds");
}

#[test]
fn prune_keeps_unsatisfiable_root_intact_so_callers_prune_before_writing() {
    // Root has a mandatory field typed to an unsatisfiable record and a
    // max = 0 field: the root is kept as-is (spec 6.5), max = 0 included.
    let s = Schema::new(
        Ref::new("R"),
        env(vec![(
            "R",
            rec(vec![
                Field::required("loop", Ref::new("R")).unwrap(),
                Field::new("never", STRING, 0, Some(0)).unwrap(),
            ]),
        )]),
    )
    .unwrap();
    let p = prune(&s);
    let root = &p.env()["R"];
    assert!(root.fields().iter().any(|f| f.max == Some(0)));
    assert_eq!(
        to_osd(&p, None).unwrap_err().code.as_deref(),
        Some("write.unsupported-value")
    );
}

#[test]
fn normalize_never_introduces_zero_max() {
    let s = Schema::new(
        Ref::new("R"),
        env(vec![(
            "R",
            rec(vec![
                Field::required("keep", STRING).unwrap(),
                Field::new("opt", FieldType::Any, 0, Some(3)).unwrap(),
            ]),
        )]),
    )
    .unwrap();
    for r in normalize(&s).env().values() {
        assert!(r.fields().iter().all(|f| f.max != Some(0)));
    }
}
