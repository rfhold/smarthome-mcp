use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatterEmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListMatterDevicesInput {
    /// Maximum Matter devices to return. Defaults to 50.
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatterDeviceInput {
    /// Home Assistant registry device identifier.
    pub device_id: String,
}

pub(crate) struct MatterDevicesQuery {
    pub(crate) limit: usize,
}

pub(crate) struct MatterDeviceQuery {
    pub(crate) device_id: String,
}

impl ListMatterDevicesInput {
    pub(crate) fn validate(self) -> Result<MatterDevicesQuery, ()> {
        let limit = self.limit.unwrap_or(50);
        if !(1..=100).contains(&limit) {
            return Err(());
        }
        Ok(MatterDevicesQuery {
            limit: usize::from(limit),
        })
    }
}

impl MatterDeviceInput {
    pub(crate) fn validate(self) -> Result<MatterDeviceQuery, ()> {
        if self.device_id.is_empty()
            || self.device_id.len() > 255
            || !self.device_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
            })
        {
            return Err(());
        }
        Ok(MatterDeviceQuery {
            device_id: self.device_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_limits_and_device_ids_are_strict() {
        assert_eq!(
            ListMatterDevicesInput { limit: None }
                .validate()
                .unwrap()
                .limit,
            50
        );
        for limit in [0, 101] {
            assert!(
                ListMatterDevicesInput { limit: Some(limit) }
                    .validate()
                    .is_err()
            );
        }
        for device_id in ["", "bad id", "bad/id", &"x".repeat(256)] {
            assert!(
                MatterDeviceInput {
                    device_id: device_id.to_owned()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn schemas_reject_unknown_fields() {
        for schema in [
            schemars::schema_for!(MatterEmptyInput),
            schemars::schema_for!(ListMatterDevicesInput),
            schemars::schema_for!(MatterDeviceInput),
        ] {
            let schema = serde_json::to_value(schema).unwrap();
            assert_eq!(schema["additionalProperties"], false);
        }
        assert!(
            serde_json::from_value::<MatterDeviceInput>(
                serde_json::json!({"device_id":"one","unknown":true})
            )
            .is_err()
        );
    }
}
