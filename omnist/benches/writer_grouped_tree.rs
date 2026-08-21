use omnist::document::{Doc, RawNode, Scalar};
use omnist::formats::json::write_json;
use omnist::formats::yaml::write_yaml;
use std::time::Instant;

fn make_unique_keys_doc(n: usize) -> Doc {
    let edges = (0..n)
        .map(|i| {
            (
                format!("key_{i}"),
                RawNode::Leaf(Scalar::Int((i as i64).into())),
            )
        })
        .collect();
    Doc::from_raw(RawNode::Edges(edges)).unwrap()
}

fn make_repeated_keys_doc(keys: usize, reps: usize) -> Doc {
    let mut edges = Vec::with_capacity(keys * reps);
    for i in 0..reps {
        for k in 0..keys {
            edges.push((
                format!("key_{k}"),
                RawNode::Leaf(Scalar::Int((i as i64).into())),
            ));
        }
    }
    Doc::from_raw(RawNode::Edges(edges)).unwrap()
}

fn main() {
    println!("=== Benchmark: JSON & YAML writer grouped-tree ===");

    let unique_doc = make_unique_keys_doc(5000);
    let repeated_doc = make_repeated_keys_doc(10, 500);

    // Warmup
    let _ = write_json(&unique_doc, None, false, None).unwrap();
    let _ = write_json(&repeated_doc, None, false, None).unwrap();
    let _ = write_yaml(&unique_doc, false, None).unwrap();
    let _ = write_yaml(&repeated_doc, false, None).unwrap();

    let iters = 50;

    // 1. JSON - Unique Keys (5,000 keys)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = write_json(&unique_doc, None, false, None).unwrap();
    }
    let json_unique_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    // 2. JSON - Repeated Keys (10 keys x 500 reps = 5,000 edges)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = write_json(&repeated_doc, None, false, None).unwrap();
    }
    let json_repeated_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    // 3. YAML - Unique Keys (5,000 keys)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = write_yaml(&unique_doc, false, None).unwrap();
    }
    let yaml_unique_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    // 4. YAML - Repeated Keys (10 keys x 500 reps = 5,000 edges)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = write_yaml(&repeated_doc, false, None).unwrap();
    }
    let yaml_repeated_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    println!(
        "JSON unique keys (5,000):     {:>8.3} ms / iter",
        json_unique_ms
    );
    println!(
        "JSON repeated keys (5,000):   {:>8.3} ms / iter",
        json_repeated_ms
    );
    println!(
        "YAML unique keys (5,000):     {:>8.3} ms / iter",
        yaml_unique_ms
    );
    println!(
        "YAML repeated keys (5,000):   {:>8.3} ms / iter",
        yaml_repeated_ms
    );
}
