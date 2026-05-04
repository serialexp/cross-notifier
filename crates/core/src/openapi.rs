//! Embeds the OpenAPI spec at compile time and exposes it as both YAML
//! (source of truth) and JSON (converted once at module init).

use std::sync::OnceLock;

pub const YAML: &str = include_str!("openapi.yaml");

static JSON: OnceLock<String> = OnceLock::new();

/// Returns the spec as JSON, converting from YAML on first call. Panics
/// if the embedded YAML is invalid — caught by the library's own tests.
pub fn json() -> &'static str {
    JSON.get_or_init(|| {
        let doc: serde_yaml::Value =
            serde_yaml::from_str(YAML).expect("embedded openapi.yaml is invalid");
        serde_json::to_string_pretty(&doc).expect("openapi yaml → json conversion failed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_parses() {
        let _: serde_yaml::Value = serde_yaml::from_str(YAML).unwrap();
    }

    #[test]
    fn json_is_valid() {
        let v: serde_json::Value = serde_json::from_str(json()).unwrap();
        assert!(v.get("openapi").is_some());
        assert!(v.get("paths").is_some());
    }
}
