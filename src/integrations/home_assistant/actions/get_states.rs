use schemars::JsonSchema;
use serde::Deserialize;

use super::valid_entity_id;

const MAX_ENTITIES: usize = 25;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetStatesInput {
    /// Home Assistant entity IDs. Every entity must be explicitly exposed to Assist.
    pub entity_ids: Vec<String>,
}

pub(crate) struct StatesQuery {
    pub(crate) entity_ids: Vec<String>,
}

impl GetStatesInput {
    pub(crate) fn validate(self) -> Result<StatesQuery, ()> {
        let mut entity_ids = self.entity_ids;
        if entity_ids.is_empty()
            || entity_ids.len() > MAX_ENTITIES
            || entity_ids.iter().any(|value| !valid_entity_id(value))
        {
            return Err(());
        }
        entity_ids.sort();
        if entity_ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(());
        }
        Ok(StatesQuery { entity_ids })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicates_and_empty_lists() {
        assert!(GetStatesInput { entity_ids: vec![] }.validate().is_err());
        assert!(
            GetStatesInput {
                entity_ids: vec!["sensor.one".to_owned(), "sensor.one".to_owned()],
            }
            .validate()
            .is_err()
        );
    }
}
