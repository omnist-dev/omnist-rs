use std::time::Instant;

fn generate_yaml(n: usize) -> String {
    let mut s = String::with_capacity(n * 40);
    s.push_str("base: &base\n");
    for i in 0..n {
        s.push_str(&format!("  k{i}: {i}\n"));
    }
    s.push_str("child:\n  <<: *base\n  override: 1\n");
    s
}

fn main() {
    println!("=== YAML Merge Key Benchmark ===");
    for &n in &[2000, 4000, 8000] {
        let yaml = generate_yaml(n);
        // Warm up
        let _ = omnist::formats::yaml::read_yaml(&yaml).unwrap();

        // Measure iterations
        let iters = 5;
        let start = Instant::now();
        for _ in 0..iters {
            let _ = omnist::formats::yaml::read_yaml(&yaml).unwrap();
        }
        let elapsed = start.elapsed();
        let avg_us = elapsed.as_micros() as f64 / iters as f64;
        let avg_ms = avg_us / 1000.0;
        println!("N = {:>5}: {:>8.3} ms (per iteration)", n, avg_ms);
    }
}
