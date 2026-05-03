//! Stable SHA-256 fingerprint helpers.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Returns the SHA-256 fingerprint for a string.
pub fn fingerprint_str(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Returns the SHA-256 fingerprint for a serializable value using canonical JSON.
pub fn fingerprint_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let canonical = canonical_json(&serde_json::to_value(value)?);
    Ok(fingerprint_str(&canonical))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_string(value).expect("primitive JSON serialization cannot fail")
        }
        Value::Array(values) => {
            let mut rendered = String::from("[");
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    rendered.push(',');
                }
                rendered.push_str(&canonical_json(item));
            }
            rendered.push(']');
            rendered
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);

            let mut rendered = String::from("{");
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    rendered.push(',');
                }
                rendered.push_str(
                    &serde_json::to_string(key).expect("object keys are always valid strings"),
                );
                rendered.push(':');
                rendered.push_str(&canonical_json(item));
            }
            rendered.push('}');
            rendered
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{fingerprint_json, fingerprint_str};

    #[test]
    fn fingerprints_strings_with_sha256() {
        assert_eq!(
            fingerprint_str("wonder-of-u"),
            "9332d026a1f4f3ba29c93875a89561b13ad5ce948defb6247ef8322377e4cd90"
        );
    }

    #[test]
    fn fingerprints_objects_independently_of_key_order() {
        let left = json!({ "b": 2, "a": 1 });
        let right = json!({ "a": 1, "b": 2 });

        assert_eq!(
            fingerprint_json(&left).unwrap(),
            fingerprint_json(&right).unwrap()
        );
    }
}
