use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;

use super::valid_entity_id;

const MAX_ENTITIES: usize = 10;
const MAX_RANGE_SECONDS: i64 = 24 * 60 * 60;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetHistoryInput {
    /// Home Assistant entity IDs. Every entity must be explicitly exposed to Assist.
    pub entity_ids: Vec<String>,
    /// Inclusive RFC3339 range start.
    pub start: String,
    /// Inclusive RFC3339 range end. Defaults to the current time.
    pub end: Option<String>,
}

pub(crate) struct HistoryQuery {
    pub(crate) entity_ids: Vec<String>,
    pub(crate) start: DateTime<Utc>,
    pub(crate) end: DateTime<Utc>,
}

impl GetHistoryInput {
    pub(crate) fn validate(self) -> Result<HistoryQuery, ()> {
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
        let start = DateTime::parse_from_rfc3339(&self.start)
            .map_err(|_| ())?
            .with_timezone(&Utc);
        let end = self
            .end
            .map(|value| DateTime::parse_from_rfc3339(&value).map(|time| time.with_timezone(&Utc)))
            .transpose()
            .map_err(|_| ())?
            .unwrap_or_else(Utc::now);
        let duration = end.signed_duration_since(start).num_seconds();
        if duration <= 0 || duration > MAX_RANGE_SECONDS {
            return Err(());
        }
        Ok(HistoryQuery {
            entity_ids,
            start,
            end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_24_hour_range() {
        assert!(
            GetHistoryInput {
                entity_ids: vec!["sensor.one".to_owned()],
                start: "2026-08-08T00:00:00Z".to_owned(),
                end: Some("2026-08-09T00:00:00Z".to_owned()),
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn rejects_long_or_reversed_ranges() {
        for end in ["2026-08-10T00:00:01Z", "2026-08-07T00:00:00Z"] {
            assert!(
                GetHistoryInput {
                    entity_ids: vec!["sensor.one".to_owned()],
                    start: "2026-08-08T00:00:00Z".to_owned(),
                    end: Some(end.to_owned()),
                }
                .validate()
                .is_err()
            );
        }
    }
}
