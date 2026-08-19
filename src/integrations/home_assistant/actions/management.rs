use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use unicode_casefold::UnicodeCaseFold;

use super::{valid_config_key, validate_native_json};

const MAX_YAML_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlueprintListInput {
    /// Optional case-insensitive match against path, name, or description.
    pub search: Option<String>,
    /// Maximum entries to return. Defaults to 50 and cannot exceed 100.
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlueprintGetInput {
    /// Relative automation blueprint path ending in `.yaml`.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlueprintSaveInput {
    /// Relative automation blueprint path ending in `.yaml`.
    pub path: String,
    /// Complete semantic automation blueprint YAML, at most 256 KiB.
    pub yaml: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomationFromBlueprintInput {
    /// Stable Home Assistant automation key.
    pub config_key: String,
    /// Relative automation blueprint path ending in `.yaml`.
    pub path: String,
    /// Bounded native blueprint inputs.
    pub input: Value,
    /// Optional automation alias.
    pub alias: Option<String>,
    /// Optional automation description.
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmInput {
    /// Must be exactly true.
    #[schemars(schema_with = "true_schema")]
    pub confirm: bool,
}

pub(crate) struct BlueprintListQuery {
    pub(crate) search: Option<String>,
    pub(crate) limit: usize,
}
pub(crate) struct BlueprintPath(pub(crate) String);
pub(crate) struct BlueprintSave {
    pub(crate) path: String,
    pub(crate) yaml: String,
}
pub(crate) struct AutomationFromBlueprint {
    pub(crate) config_key: String,
    pub(crate) path: String,
    pub(crate) input: Value,
    pub(crate) alias: Option<String>,
    pub(crate) description: Option<String>,
}
impl BlueprintListInput {
    pub(crate) fn validate(self) -> Result<BlueprintListQuery, ()> {
        let limit = self.limit.unwrap_or(50);
        validate_search(self.search.as_deref())?;
        if !(1..=100).contains(&limit) {
            return Err(());
        }
        Ok(BlueprintListQuery {
            search: self.search.map(|v| v.case_fold().collect()),
            limit: limit.into(),
        })
    }
}

impl BlueprintGetInput {
    pub(crate) fn validate(self) -> Result<BlueprintPath, ()> {
        validate_blueprint_path(&self.path)?;
        Ok(BlueprintPath(self.path))
    }
}

impl BlueprintSaveInput {
    pub(crate) fn validate(self) -> Result<BlueprintSave, ()> {
        validate_blueprint_path(&self.path)?;
        if self.yaml.is_empty() || self.yaml.len() > MAX_YAML_BYTES || self.yaml.contains('\0') {
            return Err(());
        }
        Ok(BlueprintSave {
            path: self.path,
            yaml: self.yaml,
        })
    }
}

impl AutomationFromBlueprintInput {
    pub(crate) fn validate(self) -> Result<AutomationFromBlueprint, ()> {
        if !valid_config_key(&self.config_key) {
            return Err(());
        }
        validate_blueprint_path(&self.path)?;
        if !self.input.is_object() {
            return Err(());
        }
        validate_native_json(&self.input)?;
        validate_optional_text(self.alias.as_deref(), 256)?;
        validate_optional_text(self.description.as_deref(), 1024)?;
        Ok(AutomationFromBlueprint {
            config_key: self.config_key,
            path: self.path,
            input: self.input,
            alias: self.alias,
            description: self.description,
        })
    }
}

impl ConfirmInput {
    pub(crate) fn validate(self) -> Result<(), ()> {
        self.confirm.then_some(()).ok_or(())
    }
}

fn validate_blueprint_path(value: &str) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > 255
        || !value.ends_with(".yaml")
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(());
    }
    let mut count = 0;
    for segment in value.split('/') {
        count += 1;
        if segment.is_empty()
            || matches!(segment, "." | "..")
            || segment.len() > 128
            || !segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        {
            return Err(());
        }
    }
    if count > 8 { Err(()) } else { Ok(()) }
}

fn validate_search(value: Option<&str>) -> Result<(), ()> {
    validate_optional_text(value, 256)
}
fn validate_optional_text(value: Option<&str>, max: usize) -> Result<(), ()> {
    if value.is_some_and(|v| v.is_empty() || v.len() > max || v.chars().any(char::is_control)) {
        Err(())
    } else {
        Ok(())
    }
}

fn true_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    serde_json::from_value(serde_json::json!({"type":"boolean","const":true}))
        .expect("valid schema")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_and_confirmations_are_strict() {
        assert!(
            BlueprintGetInput {
                path: "vendor/motion.yaml".into()
            }
            .validate()
            .is_ok()
        );
        for path in ["/tmp/a.yaml", "../a.yaml", "a.yml", "a\\b.yaml", "a\n.yaml"] {
            assert!(BlueprintGetInput { path: path.into() }.validate().is_err());
        }
        assert!(ConfirmInput { confirm: false }.validate().is_err());
    }

    #[test]
    fn schemas_are_closed_and_confirmation_is_const_true() {
        let schema = serde_json::to_value(schemars::schema_for!(ConfirmInput)).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["confirm"]["const"], true);
        for schema in [
            serde_json::to_value(schemars::schema_for!(BlueprintSaveInput)).unwrap(),
            serde_json::to_value(schemars::schema_for!(AutomationFromBlueprintInput)).unwrap(),
        ] {
            assert_eq!(schema["additionalProperties"], false, "{schema}");
        }
        assert_eq!(
            serde_json::to_value(schemars::schema_for!(EmptyInput)).unwrap()["additionalProperties"],
            false
        );
        assert!(
            BlueprintSaveInput {
                path: "a.yaml".into(),
                yaml: "x".repeat(MAX_YAML_BYTES + 1)
            }
            .validate()
            .is_err()
        );
        assert!(
            AutomationFromBlueprintInput {
                config_key: "safe".into(),
                path: "a.yaml".into(),
                input: json!([]),
                alias: None,
                description: None
            }
            .validate()
            .is_err()
        );
    }
}
