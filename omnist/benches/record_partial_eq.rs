use omnist::ops::{equivalent, normalize};
use omnist::schema::{Field, FieldType, INTEGER, Record, Ref, Schema};
use std::time::{Duration, Instant};

fn make_record_schema(num_fields: usize, reverse: bool) -> (Record, Schema) {
    let mut fields = Vec::with_capacity(num_fields);
    for i in 0..num_fields {
        let label = format!("field_{:04}", i);
        fields.push(Field::new(label, FieldType::Scalar(INTEGER), 1, Some(1)).unwrap());
    }
    if reverse {
        fields.reverse();
    }
    let record = Record::new(fields).unwrap();
    let mut env = indexmap::IndexMap::new();
    env.insert("Root".to_string(), record.clone());
    let schema = Schema::new(Ref::new("Root"), env).unwrap();
    (record, schema)
}

fn bench_op<F: Fn()>(name: &str, iters: usize, f: F) -> Duration {
    // Warmup
    for _ in 0..(iters / 10).max(5) {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let elapsed = start.elapsed();
    let per_iter = elapsed / (iters as u32);
    println!("{:<38} {:>10.3?} / iter", name, per_iter);
    per_iter
}

fn main() {
    println!("=== Benchmark: Record equality and schema operations ===");

    for &field_count in &[50, 200, 500] {
        let (rec_a, schema_a) = make_record_schema(field_count, false);
        let (rec_b, schema_b) = make_record_schema(field_count, true);

        println!("\n--- Field count: {} ---", field_count);
        let iters = match field_count {
            50 => 10_000,
            200 => 2_000,
            500 => 500,
            _ => 1_000,
        };

        bench_op(
            &format!("Record == Record ({} fields)", field_count),
            iters,
            || {
                let eq = rec_a == rec_b;
                std::hint::black_box(eq);
            },
        );

        bench_op(
            &format!("equivalent ({} fields)", field_count),
            iters,
            || {
                let eq = equivalent(&schema_a, &schema_b);
                std::hint::black_box(eq);
            },
        );

        bench_op(
            &format!("normalize ({} fields)", field_count),
            iters / 2,
            || {
                let norm = normalize(&schema_a);
                std::hint::black_box(norm);
            },
        );
    }
}
