//! YAML parsing helpers.

/// Parses a YAML string into a generic `serde_yaml::Value`.
pub fn parse_yaml_value(input: &str) -> Result<serde_yaml::Value, serde_yaml::Error> {
    serde_yaml::from_str(input)
}

#[cfg(test)]
mod tests {
    use serde_yaml::Value;

    use super::parse_yaml_value;

    #[test]
    fn parses_yaml_documents() {
        let value = parse_yaml_value("name: wonder\nitems:\n  - one\n").unwrap();

        assert_eq!(value["name"], Value::String("wonder".to_owned()));
        assert_eq!(value["items"][0], Value::String("one".to_owned()));
    }
}
