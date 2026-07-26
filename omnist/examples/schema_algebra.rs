//! One schema-algebra operation: pruning unreachable/unsatisfiable parts of
//! a schema. `cargo run --example schema_algebra`.
use omnist::ops::prune;
use omnist::osd::{parse_schema, to_osd};

fn main() {
    // `Dead` is unreachable from the root, and `Broken` can never be
    // satisfied (its only mandatory field requires itself).
    let schema = parse_schema(
        r#"
        record Broken {
            "self": Broken,
        }
        record Dead {
            "x": string,
        }
        record Root {
            "name": string,
        }
        root Root
        "#,
    )
    .unwrap();

    let pruned = prune(&schema);

    // `Dead` and `Broken` are gone; `Root` (satisfiable, reachable) remains.
    assert!(pruned.env().contains_key("Root"));
    assert!(!pruned.env().contains_key("Dead"));
    assert!(!pruned.env().contains_key("Broken"));

    println!("{}", to_osd(&pruned, Some(2)));
}
