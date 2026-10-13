//! A multi-megabyte hex/octal/binary integer literal is refused with
//! `document.limit.int-digits` in linear time: the digit cap is decided from
//! the bit length, not by rendering the integer in decimal (a superlinear
//! conversion that took about 9 s for a 4 MB hex literal before 0.11.1-alpha).

use omnist::document::Doc;
use std::time::{Duration, Instant};

#[test]
fn huge_radix_literals_are_refused_quickly() {
    let mb4 = 4usize << 20;
    for (fmt, text) in [
        ("toml", format!("a = 0x{}", "f".repeat(mb4))),
        ("toml", format!("a = 0o{}", "7".repeat(mb4))),
        ("toml", format!("a = 0b{}", "1".repeat(mb4))),
        ("yaml", format!("a: 0x{}", "f".repeat(mb4))),
    ] {
        let t = Instant::now();
        let err = Doc::from_format(fmt, &text).unwrap_err().to_string();
        assert!(err.starts_with("$.a: integer literal exceeds"), "{err}");
        assert!(
            t.elapsed() < Duration::from_secs(4),
            "{fmt} took {:?}",
            t.elapsed()
        );
    }
}
