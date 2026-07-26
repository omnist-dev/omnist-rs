//! Name-keyed format dispatch through the registry (issue #31):
//! `Doc::from_format`/`to_format`, `formats()`, and registering a plugin
//! format at runtime. `cargo run --example format_registry`.
use omnist::document::Doc;
use omnist::{Format, formats, register_format};

fn main() {
    // The five builtins are always registered.
    assert_eq!(
        formats(),
        vec!["json", "oml", "toml", "xml", "yaml"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
    );

    let d = Doc::from_format("json", r#"{"a": 1}"#).unwrap();
    assert_eq!(d.to_format("yaml").unwrap(), "a: 1");

    // Register a plugin format: a trivial "csv"-like single-record reader.
    register_format(Format::new(
        "kv",
        |text| {
            let edges = text
                .split(',')
                .map(|pair| {
                    let (k, v) = pair.split_once('=').unwrap();
                    (
                        k.to_string(),
                        omnist::document::RawNode::Leaf(omnist::document::Scalar::Str(
                            v.to_string(),
                        )),
                    )
                })
                .collect();
            Doc::from_raw(omnist::document::RawNode::Edges(edges)).map_err(Into::into)
        },
        |doc| {
            let omnist::document::RawNode::Edges(edges) = doc.to_raw() else {
                return Ok(String::new());
            };
            Ok(edges
                .iter()
                .map(|(k, v)| {
                    let omnist::document::RawNode::Leaf(omnist::document::Scalar::Str(s)) = v
                    else {
                        unreachable!()
                    };
                    format!("{k}={s}")
                })
                .collect::<Vec<_>>()
                .join(","))
        },
    ));
    assert!(formats().contains(&"kv".to_string()));
    let kv = Doc::from_format("kv", "a=1,b=2").unwrap();
    assert_eq!(kv.to_format("kv").unwrap(), "a=1,b=2");

    println!("{}", formats().join(", "));
}
