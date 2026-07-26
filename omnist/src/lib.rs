pub mod document;
pub mod error;
pub mod oml;
pub mod osd;
pub mod schema;

pub use error::{DocumentError, OmnistError, ParseError, SchemaError, WriteError};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_cargo_toml() {
        assert_eq!(VERSION, "0.0.1-alpha");
    }
}
