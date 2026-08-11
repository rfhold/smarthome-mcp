use schemars::JsonSchema;
use serde::Deserialize;

use super::valid_entity_id;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CameraSnapshotInput {
    /// Camera entity ID explicitly exposed to Assist.
    pub entity_id: String,
}

pub(crate) struct CameraSnapshotQuery {
    pub(crate) entity_id: String,
}

impl CameraSnapshotInput {
    pub(crate) fn validate(self) -> Result<CameraSnapshotQuery, ()> {
        if !valid_entity_id(&self.entity_id) || !self.entity_id.starts_with("camera.") {
            return Err(());
        }
        Ok(CameraSnapshotQuery {
            entity_id: self.entity_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exactly_one_camera_entity() {
        assert!(
            CameraSnapshotInput {
                entity_id: "camera.front_door".to_owned(),
            }
            .validate()
            .is_ok()
        );
        for entity_id in [
            "sensor.front_door",
            "camera",
            "camera.",
            "camera.Front_door",
            "camera.front-door",
        ] {
            assert!(
                CameraSnapshotInput {
                    entity_id: entity_id.to_owned(),
                }
                .validate()
                .is_err(),
                "accepted {entity_id}"
            );
        }
    }
}
