mod camera_snapshot;
mod get_history;
mod get_states;
mod list_devices;
mod list_entities;

pub use camera_snapshot::CameraSnapshotInput;
pub use get_history::GetHistoryInput;
pub use get_states::GetStatesInput;
pub use list_devices::ListDevicesInput;
pub use list_entities::ListEntitiesInput;

pub(crate) use camera_snapshot::CameraSnapshotQuery;
pub(crate) use get_history::HistoryQuery;
pub(crate) use get_states::StatesQuery;
pub(crate) use list_devices::DevicesQuery;
pub(crate) use list_entities::EntitiesQuery;

pub(crate) fn valid_entity_id(value: &str) -> bool {
    let Some((domain, object_id)) = value.split_once('.') else {
        return false;
    };
    valid_name(domain) && valid_name(object_id)
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ids_are_strict() {
        assert!(valid_entity_id("sensor.living_room_temperature"));
        for value in [
            "sensor",
            ".name",
            "sensor.",
            "Sensor.name",
            "sensor.bad-name",
        ] {
            assert!(!valid_entity_id(value));
        }
    }
}
