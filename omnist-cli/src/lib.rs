pub fn version_line() -> String {
    format!("omnist {}", omnist::VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_includes_crate_version() {
        assert_eq!(version_line(), format!("omnist {}", omnist::VERSION));
    }
}
