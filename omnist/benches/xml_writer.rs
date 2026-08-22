use omnist::document::{Doc, RawNode, Scalar};
use omnist::formats::xml::{check_xml, write_xml};
use std::time::Instant;

fn make_nested_xml_doc(depth: usize, branching: usize) -> Doc {
    fn build_node(d: usize, max_d: usize, b: usize) -> RawNode {
        if d >= max_d {
            RawNode::Leaf(Scalar::Str(format!("val_{d}")))
        } else {
            let edges = (0..b)
                .map(|i| (format!("elem_{i}"), build_node(d + 1, max_d, b)))
                .collect();
            RawNode::Edges(edges)
        }
    }

    let root_content = build_node(1, depth, branching);
    Doc::from_raw(RawNode::Edges(vec![("root".to_string(), root_content)])).unwrap()
}

fn main() {
    println!("=== Benchmark: XML writer and checker ===");

    // Nested tree: depth 10, branching 2 (~1,024 elements)
    let nested_doc = make_nested_xml_doc(10, 2);
    // Flat wide tree: depth 2, branching 2000 (2,000 child elements under root)
    let wide_doc = make_nested_xml_doc(2, 2000);

    // Warmup
    let _ = write_xml(&nested_doc, false, None).unwrap();
    let _ = write_xml(&wide_doc, false, None).unwrap();
    let _ = check_xml(&nested_doc);
    let _ = check_xml(&wide_doc);

    let iters = 50;

    // 1. XML Write - Nested Doc (depth 10, branching 2)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = write_xml(&nested_doc, false, None).unwrap();
    }
    let write_nested_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    // 2. XML Write - Wide Doc (2,000 elements)
    let start = Instant::now();
    for _ in 0..iters {
        let _ = write_xml(&wide_doc, false, None).unwrap();
    }
    let write_wide_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    // 3. XML Check - Nested Doc
    let start = Instant::now();
    for _ in 0..iters {
        let _ = check_xml(&nested_doc);
    }
    let check_nested_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    // 4. XML Check - Wide Doc
    let start = Instant::now();
    for _ in 0..iters {
        let _ = check_xml(&wide_doc);
    }
    let check_wide_ms = (start.elapsed().as_micros() as f64 / iters as f64) / 1000.0;

    println!(
        "XML write nested (10 levels, b=2):   {:>8.3} ms / iter",
        write_nested_ms
    );
    println!(
        "XML write wide (2,000 elements):     {:>8.3} ms / iter",
        write_wide_ms
    );
    println!(
        "XML check nested (10 levels, b=2):   {:>8.3} ms / iter",
        check_nested_ms
    );
    println!(
        "XML check wide (2,000 elements):     {:>8.3} ms / iter",
        check_wide_ms
    );
}
