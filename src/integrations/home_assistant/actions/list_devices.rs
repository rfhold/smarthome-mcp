use schemars::JsonSchema;
use serde::Deserialize;

const DEFAULT_LIMIT: u16 = 100;
const MAX_LIMIT: u16 = 100;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesInput {
    /// Maximum exposed current-state entities to group. Defaults to 100.
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u16>,
}

pub(crate) struct DevicesQuery {
    pub(crate) limit: usize,
}

impl ListDevicesInput {
    pub(crate) fn validate(self) -> Result<DevicesQuery, ()> {
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(());
        }
        Ok(DevicesQuery {
            limit: usize::from(limit),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_default_minimum_and_maximum_limits() {
        assert_eq!(
            ListDevicesInput { limit: None }.validate().unwrap().limit,
            100
        );
        assert_eq!(
            ListDevicesInput { limit: Some(1) }
                .validate()
                .unwrap()
                .limit,
            1
        );
        assert_eq!(
            ListDevicesInput { limit: Some(100) }
                .validate()
                .unwrap()
                .limit,
            100
        );
    }

    #[test]
    fn rejects_limits_outside_the_range() {
        for limit in [0, 101] {
            assert!(ListDevicesInput { limit: Some(limit) }.validate().is_err());
        }
    }

    #[test]
    fn schema_bounds_limit_and_rejects_unknown_fields() {
        let schema = serde_json::to_value(schemars::schema_for!(ListDevicesInput)).unwrap();
        assert_eq!(schema["properties"]["limit"]["minimum"], 1);
        assert_eq!(schema["properties"]["limit"]["maximum"], 100);
        assert_eq!(schema["additionalProperties"], false);
        assert!(
            serde_json::from_value::<ListDevicesInput>(
                serde_json::json!({"limit":1,"unexpected":true})
            )
            .is_err()
        );
    }
}
