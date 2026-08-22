use omnist::document::{Doc, RawNode, Scalar};
use omnist::formats::toml::{check_toml, write_toml};
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
    println!("=== Benchmark: TOML writer and checker ===");

    let unique_doc = make_unique_keys_doc(5000);
    let repeated_doc = make_repeated_keys_doc(10, 500);

    // Warmup
    let _ = write_toml(&unique_doc, false, None).unwrap();
    let _ = write_toml(&repeated_doc, false, None).unwrap();
    let _ = check_toml(&unique_doc);
    let _ = check_toml(&repeated_doc);

    let iters = 50;

    // 1. TOML Write - Unique Keys (5,000 keys)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = write_toml(&unique_doc, false, None).unwrap();
    }
    let toml_write_unique_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    // 2. TOML Write - Repeated Keys (10 keys x 500 reps = 5,000 edges)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = write_toml(&repeated_doc, false, None).unwrap();
    }
    let toml_write_repeated_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    // 3. TOML Check - Unique Keys (5,000 keys)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = check_toml(&unique_doc);
    }
    let toml_check_unique_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    // 4. TOML Check - Repeated Keys (10 keys x 500 reps = 5,000 edges)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = check_toml(&repeated_doc);
    }
    let toml_check_repeated_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    println!(
        "TOML write unique keys (5,000):     {:>8.3} ms / iter",
        toml_write_unique_ms
    );
    println!(
        "TOML write repeated keys (5,000):   {:>8.3} ms / iter",
        toml_write_repeated_ms
    );
    println!(
        "TOML check unique keys (5,000):     {:>8.3} ms / iter",
        toml_check_unique_ms
    );
    println!(
        "TOML check repeated keys (5,000):   {:>8.3} ms / iter",
        toml_check_repeated_ms
    );
}
