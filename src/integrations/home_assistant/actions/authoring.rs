use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};
use unicode_casefold::UnicodeCaseFold;

const MAX_NATIVE_CONFIG_BYTES: usize = 256 * 1024;
const MAX_NATIVE_CONFIG_DEPTH: usize = 32;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigUpsertInput {
    /// Stable Home Assistant config key using lowercase letters, digits, and underscores.
    pub config_key: String,
    /// Complete native Home Assistant scene or automation configuration object.
    pub config: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigListInput {
    /// Optional case-insensitive match against config key, entity ID, or name.
    pub query: Option<String>,
    /// Maximum entries to return. Defaults to 50 and cannot exceed 100.
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigGetInput {
    /// Stable Home Assistant config key using lowercase letters, digits, and underscores.
    pub config_key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomationValidateInput {
    /// Native Home Assistant singular trigger section.
    pub trigger: Option<Value>,
    /// Native Home Assistant plural triggers section. Do not combine with `trigger`.
    pub triggers: Option<Value>,
    /// Native Home Assistant singular condition section.
    pub condition: Option<Value>,
    /// Native Home Assistant plural conditions section. Do not combine with `condition`.
    pub conditions: Option<Value>,
    /// Native Home Assistant singular action section.
    pub action: Option<Value>,
    /// Native Home Assistant plural actions section. Do not combine with `action`.
    pub actions: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomationTracesInput {
    /// Home Assistant automation config key whose recent traces should be summarized.
    pub item_id: String,
    /// Maximum traces to return. Defaults to 10 and cannot exceed 50.
    #[schemars(range(min = 1, max = 50))]
    pub limit: Option<u8>,
}

pub(crate) struct ConfigUpsert {
    pub(crate) config_key: String,
    pub(crate) config: Value,
}

pub(crate) struct ConfigListQuery {
    pub(crate) query: Option<String>,
    pub(crate) limit: usize,
}

pub(crate) struct ConfigGetQuery {
    pub(crate) config_key: String,
}

pub(crate) struct AutomationValidation {
    pub(crate) sections: Map<String, Value>,
}

pub(crate) struct AutomationTracesQuery {
    pub(crate) item_id: String,
    pub(crate) limit: usize,
}

impl ConfigUpsertInput {
    pub(crate) fn validate(self) -> Result<ConfigUpsert, ()> {
        let Some(config) = self.config.as_object() else {
            return Err(());
        };
        if !valid_config_key(&self.config_key)
            || config
                .get("id")
                .is_some_and(|id| id.as_str() != Some(&self.config_key))
        {
            return Err(());
        }
        validate_native_json(&self.config)?;
        Ok(ConfigUpsert {
            config_key: self.config_key,
            config: self.config,
        })
    }
}

impl ConfigListInput {
    pub(crate) fn validate(self) -> Result<ConfigListQuery, ()> {
        let limit = self.limit.unwrap_or(50);
        if !(1..=100).contains(&limit)
            || self
                .query
                .as_ref()
                .is_some_and(|query| query.len() > 256 || query.chars().any(char::is_control))
        {
            return Err(());
        }
        Ok(ConfigListQuery {
            query: self.query.map(|query| query.case_fold().collect()),
            limit: usize::from(limit),
        })
    }
}

impl ConfigGetInput {
    pub(crate) fn validate(self) -> Result<ConfigGetQuery, ()> {
        if !valid_config_key(&self.config_key) {
            return Err(());
        }
        Ok(ConfigGetQuery {
            config_key: self.config_key,
        })
    }
}

impl AutomationValidateInput {
    pub(crate) fn validate(self) -> Result<AutomationValidation, ()> {
        let mut sections = Map::new();
        normalize_section(&mut sections, "triggers", self.trigger, self.triggers)?;
        normalize_section(&mut sections, "conditions", self.condition, self.conditions)?;
        normalize_section(&mut sections, "actions", self.action, self.actions)?;
        if sections.is_empty() {
            return Err(());
        }
        Ok(AutomationValidation { sections })
    }
}

impl AutomationTracesInput {
    pub(crate) fn validate(self) -> Result<AutomationTracesQuery, ()> {
        let limit = self.limit.unwrap_or(10);
        if !valid_config_key(&self.item_id) || !(1..=50).contains(&limit) {
            return Err(());
        }
        Ok(AutomationTracesQuery {
            item_id: self.item_id,
            limit: usize::from(limit),
        })
    }
}

fn normalize_section(
    sections: &mut Map<String, Value>,
    name: &str,
    singular: Option<Value>,
    plural: Option<Value>,
) -> Result<(), ()> {
    let value = match (singular, plural) {
        (Some(_), Some(_)) => return Err(()),
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => return Ok(()),
    };
    if !value.is_object() && !value.is_array() {
        return Err(());
    }
    validate_native_json(&value)?;
    sections.insert(name.to_owned(), value);
    Ok(())
}

pub(crate) fn validate_native_json(value: &Value) -> Result<(), ()> {
    if serde_json::to_vec(value).map_err(|_| ())?.len() > MAX_NATIVE_CONFIG_BYTES
        || json_depth(value) > MAX_NATIVE_CONFIG_DEPTH
    {
        Err(())
    } else {
        Ok(())
    }
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
        _ => 0,
    }
}

pub(crate) fn valid_config_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upserts_require_a_safe_key_and_bounded_object() {
        assert!(
            ConfigUpsertInput {
                config_key: "evening_scene".to_owned(),
                config: json!({"name":"Evening","entities":{}}),
            }
            .validate()
            .is_ok()
        );
        for key in ["", "Bad", "bad/key", "bad-key", &"x".repeat(65)] {
            assert!(
                ConfigUpsertInput {
                    config_key: key.to_owned(),
                    config: json!({}),
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            ConfigUpsertInput {
                config_key: "key".to_owned(),
                config: json!([]),
            }
            .validate()
            .is_err()
        );
        assert!(
            ConfigUpsertInput {
                config_key: "key".to_owned(),
                config: json!({"payload":"x".repeat(MAX_NATIVE_CONFIG_BYTES)}),
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn upsert_config_id_must_match_the_stable_key() {
        for config in [json!({"alias":"Absent"}), json!({"id":"stable_key"})] {
            assert!(
                ConfigUpsertInput {
                    config_key: "stable_key".to_owned(),
                    config,
                }
                .validate()
                .is_ok()
            );
        }
        for config in [
            json!({"id":"different_key"}),
            json!({"id":7}),
            json!({"id":null}),
        ] {
            assert!(
                ConfigUpsertInput {
                    config_key: "stable_key".to_owned(),
                    config,
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn validation_aliases_normalize_without_duplicates() {
        let validation = AutomationValidateInput {
            trigger: Some(json!({"platform":"state"})),
            triggers: None,
            condition: None,
            conditions: Some(json!([])),
            action: Some(json!([{"service":"light.turn_on"}])),
            actions: None,
        }
        .validate()
        .unwrap();
        assert!(validation.sections.contains_key("triggers"));
        assert!(validation.sections.contains_key("conditions"));
        assert!(validation.sections.contains_key("actions"));

        assert!(
            AutomationValidateInput {
                trigger: Some(json!({})),
                triggers: Some(json!([])),
                condition: None,
                conditions: None,
                action: None,
                actions: None,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn nesting_and_trace_limits_are_enforced() {
        let mut nested = json!(true);
        for _ in 0..=MAX_NATIVE_CONFIG_DEPTH {
            nested = json!([nested]);
        }
        assert!(validate_native_json(&nested).is_err());
        for limit in [0, 51] {
            assert!(
                AutomationTracesInput {
                    item_id: "automation_key".to_owned(),
                    limit: Some(limit),
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn schemas_are_closed() {
        for schema in [
            serde_json::to_value(schemars::schema_for!(ConfigUpsertInput)).unwrap(),
            serde_json::to_value(schemars::schema_for!(ConfigListInput)).unwrap(),
            serde_json::to_value(schemars::schema_for!(ConfigGetInput)).unwrap(),
            serde_json::to_value(schemars::schema_for!(AutomationValidateInput)).unwrap(),
            serde_json::to_value(schemars::schema_for!(AutomationTracesInput)).unwrap(),
        ] {
            assert_eq!(schema["additionalProperties"], false);
        }
    }

    #[test]
    fn config_read_inputs_are_bounded_and_normalized() {
        let list = ConfigListInput {
            query: Some("Evening Scene".to_owned()),
            limit: None,
        }
        .validate()
        .unwrap();
        assert_eq!(list.query.as_deref(), Some("evening scene"));
        assert_eq!(list.limit, 50);
        for limit in [0, 101] {
            assert!(
                ConfigListInput {
                    query: None,
                    limit: Some(limit)
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            ConfigListInput {
                query: Some("bad\nquery".to_owned()),
                limit: None
            }
            .validate()
            .is_err()
        );
        assert!(
            ConfigGetInput {
                config_key: "safe_key".to_owned()
            }
            .validate()
            .is_ok()
        );
        assert!(
            ConfigGetInput {
                config_key: "bad/key".to_owned()
            }
            .validate()
            .is_err()
        );
    }
}
