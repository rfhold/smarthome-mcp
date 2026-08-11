use schemars::JsonSchema;
use serde::Deserialize;

const MAX_IDENTIFIER_BYTES: usize = 255;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadEmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverRoutersInput {
    /// Discovery duration in seconds. Defaults to 3.
    #[schemars(range(min = 1, max = 10))]
    pub duration_seconds: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPreferredDatasetInput {
    /// Stored Thread dataset identifier.
    pub dataset_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPreferredRouterInput {
    /// Stored Thread dataset identifier.
    pub dataset_id: String,
    /// Border agent identifier, or null when unavailable.
    pub border_agent_id: Option<String>,
    /// Thread router extended address.
    pub extended_address: String,
}

pub(crate) struct RouterDiscoveryQuery {
    pub(crate) duration_seconds: u8,
}

pub(crate) struct PreferredDatasetCommand {
    pub(crate) dataset_id: String,
}

pub(crate) struct PreferredRouterCommand {
    pub(crate) dataset_id: String,
    pub(crate) border_agent_id: Option<String>,
    pub(crate) extended_address: String,
}

impl DiscoverRoutersInput {
    pub(crate) fn validate(self) -> Result<RouterDiscoveryQuery, ()> {
        let duration_seconds = self.duration_seconds.unwrap_or(3);
        if !(1..=10).contains(&duration_seconds) {
            return Err(());
        }
        Ok(RouterDiscoveryQuery { duration_seconds })
    }
}

impl SetPreferredDatasetInput {
    pub(crate) fn validate(self) -> Result<PreferredDatasetCommand, ()> {
        if !valid_identifier(&self.dataset_id) {
            return Err(());
        }
        Ok(PreferredDatasetCommand {
            dataset_id: self.dataset_id,
        })
    }
}

impl SetPreferredRouterInput {
    pub(crate) fn validate(self) -> Result<PreferredRouterCommand, ()> {
        if !valid_identifier(&self.dataset_id)
            || !valid_identifier(&self.extended_address)
            || self
                .border_agent_id
                .as_deref()
                .is_some_and(|value| !valid_identifier(value))
        {
            return Err(());
        }
        Ok(PreferredRouterCommand {
            dataset_id: self.dataset_id,
            border_agent_id: self.border_agent_id,
            extended_address: self.extended_address,
        })
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_duration_is_bounded() {
        assert_eq!(
            DiscoverRoutersInput {
                duration_seconds: None
            }
            .validate()
            .unwrap()
            .duration_seconds,
            3
        );
        for duration_seconds in [0, 11] {
            assert!(
                DiscoverRoutersInput {
                    duration_seconds: Some(duration_seconds)
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn identifiers_are_tightly_validated() {
        assert!(
            SetPreferredRouterInput {
                dataset_id: "dataset-1".to_owned(),
                border_agent_id: None,
                extended_address: "001122aabb".to_owned(),
            }
            .validate()
            .is_ok()
        );
        for value in ["", "has space", "slash/value", &"x".repeat(256)] {
            assert!(
                SetPreferredDatasetInput {
                    dataset_id: value.to_owned()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn schemas_are_closed_and_bounded() {
        let schema = serde_json::to_value(schemars::schema_for!(DiscoverRoutersInput)).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["duration_seconds"]["minimum"], 1);
        assert_eq!(schema["properties"]["duration_seconds"]["maximum"], 10);
        assert!(
            serde_json::from_value::<SetPreferredDatasetInput>(
                serde_json::json!({"dataset_id":"one","unknown":true})
            )
            .is_err()
        );
    }
}
