use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::valid_entity_id;

const MIN_TEMPERATURE: f64 = -273.15;
const MAX_TEMPERATURE: f64 = 1000.0;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityControlInput {
    /// Domain-matching Home Assistant entity ID explicitly exposed to Assist.
    pub entity_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LightTurnOnInput {
    /// Light entity ID explicitly exposed to Assist.
    pub entity_id: String,
    /// Optional brightness percentage from 0 through 100.
    #[schemars(range(min = 0, max = 100))]
    pub brightness_pct: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FanPercentageInput {
    /// Fan entity ID explicitly exposed to Assist.
    pub entity_id: String,
    /// Fan percentage from 0 through 100.
    #[schemars(range(min = 0, max = 100))]
    pub percentage: u8,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoverPositionInput {
    /// Cover entity ID explicitly exposed to Assist.
    pub entity_id: String,
    /// Cover position from 0 through 100.
    #[schemars(range(min = 0, max = 100))]
    pub position: u8,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClimateTemperatureInput {
    /// Climate entity ID explicitly exposed to Assist.
    pub entity_id: String,
    /// Finite target temperature from absolute zero through 1000 degrees.
    #[schemars(range(min = -273.15, max = 1000.0))]
    pub temperature: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaPlayerVolumeInput {
    /// Media player entity ID explicitly exposed to Assist.
    pub entity_id: String,
    /// Volume level from 0.0 through 1.0.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub volume_level: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControlAction {
    SceneActivate,
    LightTurnOn,
    LightTurnOff,
    SwitchTurnOn,
    SwitchTurnOff,
    FanTurnOn,
    FanTurnOff,
    FanSetPercentage,
    CoverOpen,
    CoverClose,
    CoverStop,
    CoverSetPosition,
    ClimateTurnOn,
    ClimateTurnOff,
    ClimateSetTemperature,
    MediaPlayerTurnOn,
    MediaPlayerTurnOff,
    MediaPlayerPlay,
    MediaPlayerPause,
    MediaPlayerStop,
    MediaPlayerVolumeSet,
    LockLock,
    LockUnlock,
}

#[derive(Debug)]
pub(crate) struct Control {
    action: ControlAction,
    entity_id: String,
    parameter: Option<ControlParameter>,
}

#[derive(Debug)]
enum ControlParameter {
    Brightness(u8),
    Percentage(u8),
    Position(u8),
    Temperature(f64),
    Volume(f64),
}

impl ControlAction {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::SceneActivate => "scene.activate",
            Self::LightTurnOn => "light.turn_on",
            Self::LightTurnOff => "light.turn_off",
            Self::SwitchTurnOn => "switch.turn_on",
            Self::SwitchTurnOff => "switch.turn_off",
            Self::FanTurnOn => "fan.turn_on",
            Self::FanTurnOff => "fan.turn_off",
            Self::FanSetPercentage => "fan.set_percentage",
            Self::CoverOpen => "cover.open",
            Self::CoverClose => "cover.close",
            Self::CoverStop => "cover.stop",
            Self::CoverSetPosition => "cover.set_position",
            Self::ClimateTurnOn => "climate.turn_on",
            Self::ClimateTurnOff => "climate.turn_off",
            Self::ClimateSetTemperature => "climate.set_temperature",
            Self::MediaPlayerTurnOn => "media_player.turn_on",
            Self::MediaPlayerTurnOff => "media_player.turn_off",
            Self::MediaPlayerPlay => "media_player.play",
            Self::MediaPlayerPause => "media_player.pause",
            Self::MediaPlayerStop => "media_player.stop",
            Self::MediaPlayerVolumeSet => "media_player.volume_set",
            Self::LockLock => "lock.lock",
            Self::LockUnlock => "lock.unlock",
        }
    }

    pub(crate) fn service(self) -> (&'static str, &'static str) {
        match self {
            Self::SceneActivate => ("scene", "turn_on"),
            Self::LightTurnOn => ("light", "turn_on"),
            Self::LightTurnOff => ("light", "turn_off"),
            Self::SwitchTurnOn => ("switch", "turn_on"),
            Self::SwitchTurnOff => ("switch", "turn_off"),
            Self::FanTurnOn => ("fan", "turn_on"),
            Self::FanTurnOff => ("fan", "turn_off"),
            Self::FanSetPercentage => ("fan", "set_percentage"),
            Self::CoverOpen => ("cover", "open_cover"),
            Self::CoverClose => ("cover", "close_cover"),
            Self::CoverStop => ("cover", "stop_cover"),
            Self::CoverSetPosition => ("cover", "set_cover_position"),
            Self::ClimateTurnOn => ("climate", "turn_on"),
            Self::ClimateTurnOff => ("climate", "turn_off"),
            Self::ClimateSetTemperature => ("climate", "set_temperature"),
            Self::MediaPlayerTurnOn => ("media_player", "turn_on"),
            Self::MediaPlayerTurnOff => ("media_player", "turn_off"),
            Self::MediaPlayerPlay => ("media_player", "media_play"),
            Self::MediaPlayerPause => ("media_player", "media_pause"),
            Self::MediaPlayerStop => ("media_player", "media_stop"),
            Self::MediaPlayerVolumeSet => ("media_player", "volume_set"),
            Self::LockLock => ("lock", "lock"),
            Self::LockUnlock => ("lock", "unlock"),
        }
    }
}

impl Control {
    fn new(
        action: ControlAction,
        entity_id: String,
        parameter: Option<ControlParameter>,
    ) -> Result<Self, ()> {
        let (domain, _) = action.service();
        if !valid_entity_id(&entity_id) || !entity_id.starts_with(&format!("{domain}.")) {
            return Err(());
        }
        Ok(Self {
            action,
            entity_id,
            parameter,
        })
    }

    pub(crate) fn action(&self) -> &'static str {
        self.action.name()
    }

    pub(crate) fn service(&self) -> (&'static str, &'static str) {
        self.action.service()
    }

    pub(crate) fn entity_id(&self) -> &str {
        &self.entity_id
    }

    pub(crate) fn service_data(&self) -> Value {
        let mut data = json!({"entity_id": self.entity_id});
        let (name, value) = match self.parameter {
            None => return data,
            Some(ControlParameter::Brightness(value)) => ("brightness_pct", json!(value)),
            Some(ControlParameter::Percentage(value)) => ("percentage", json!(value)),
            Some(ControlParameter::Position(value)) => ("position", json!(value)),
            Some(ControlParameter::Temperature(value)) => ("temperature", json!(value)),
            Some(ControlParameter::Volume(value)) => ("volume_level", json!(value)),
        };
        data.as_object_mut().unwrap().insert(name.to_owned(), value);
        data
    }
}

impl EntityControlInput {
    pub(crate) fn validate(self, action: ControlAction) -> Result<Control, ()> {
        Control::new(action, self.entity_id, None)
    }
}

impl LightTurnOnInput {
    pub(crate) fn validate(self) -> Result<Control, ()> {
        if self.brightness_pct.is_some_and(|value| value > 100) {
            return Err(());
        }
        Control::new(
            ControlAction::LightTurnOn,
            self.entity_id,
            self.brightness_pct.map(ControlParameter::Brightness),
        )
    }
}

impl FanPercentageInput {
    pub(crate) fn validate(self) -> Result<Control, ()> {
        if self.percentage > 100 {
            return Err(());
        }
        Control::new(
            ControlAction::FanSetPercentage,
            self.entity_id,
            Some(ControlParameter::Percentage(self.percentage)),
        )
    }
}

impl CoverPositionInput {
    pub(crate) fn validate(self) -> Result<Control, ()> {
        if self.position > 100 {
            return Err(());
        }
        Control::new(
            ControlAction::CoverSetPosition,
            self.entity_id,
            Some(ControlParameter::Position(self.position)),
        )
    }
}

impl ClimateTemperatureInput {
    pub(crate) fn validate(self) -> Result<Control, ()> {
        if !self.temperature.is_finite()
            || !(MIN_TEMPERATURE..=MAX_TEMPERATURE).contains(&self.temperature)
        {
            return Err(());
        }
        Control::new(
            ControlAction::ClimateSetTemperature,
            self.entity_id,
            Some(ControlParameter::Temperature(self.temperature)),
        )
    }
}

impl MediaPlayerVolumeInput {
    pub(crate) fn validate(self) -> Result<Control, ()> {
        if !self.volume_level.is_finite() || !(0.0..=1.0).contains(&self.volume_level) {
            return Err(());
        }
        Control::new(
            ControlAction::MediaPlayerVolumeSet,
            self.entity_id,
            Some(ControlParameter::Volume(self.volume_level)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIONS: [ControlAction; 23] = [
        ControlAction::SceneActivate,
        ControlAction::LightTurnOn,
        ControlAction::LightTurnOff,
        ControlAction::SwitchTurnOn,
        ControlAction::SwitchTurnOff,
        ControlAction::FanTurnOn,
        ControlAction::FanTurnOff,
        ControlAction::FanSetPercentage,
        ControlAction::CoverOpen,
        ControlAction::CoverClose,
        ControlAction::CoverStop,
        ControlAction::CoverSetPosition,
        ControlAction::ClimateTurnOn,
        ControlAction::ClimateTurnOff,
        ControlAction::ClimateSetTemperature,
        ControlAction::MediaPlayerTurnOn,
        ControlAction::MediaPlayerTurnOff,
        ControlAction::MediaPlayerPlay,
        ControlAction::MediaPlayerPause,
        ControlAction::MediaPlayerStop,
        ControlAction::MediaPlayerVolumeSet,
        ControlAction::LockLock,
        ControlAction::LockUnlock,
    ];

    #[test]
    fn action_catalog_has_exact_fixed_names_and_services() {
        let actual = ACTIONS
            .map(|action| (action.name(), action.service()))
            .to_vec();
        assert_eq!(
            actual,
            vec![
                ("scene.activate", ("scene", "turn_on")),
                ("light.turn_on", ("light", "turn_on")),
                ("light.turn_off", ("light", "turn_off")),
                ("switch.turn_on", ("switch", "turn_on")),
                ("switch.turn_off", ("switch", "turn_off")),
                ("fan.turn_on", ("fan", "turn_on")),
                ("fan.turn_off", ("fan", "turn_off")),
                ("fan.set_percentage", ("fan", "set_percentage")),
                ("cover.open", ("cover", "open_cover")),
                ("cover.close", ("cover", "close_cover")),
                ("cover.stop", ("cover", "stop_cover")),
                ("cover.set_position", ("cover", "set_cover_position")),
                ("climate.turn_on", ("climate", "turn_on")),
                ("climate.turn_off", ("climate", "turn_off")),
                ("climate.set_temperature", ("climate", "set_temperature")),
                ("media_player.turn_on", ("media_player", "turn_on")),
                ("media_player.turn_off", ("media_player", "turn_off")),
                ("media_player.play", ("media_player", "media_play")),
                ("media_player.pause", ("media_player", "media_pause")),
                ("media_player.stop", ("media_player", "media_stop")),
                ("media_player.volume_set", ("media_player", "volume_set")),
                ("lock.lock", ("lock", "lock")),
                ("lock.unlock", ("lock", "unlock")),
            ]
        );
    }

    #[test]
    fn entity_controls_require_the_exact_action_domain() {
        assert!(
            EntityControlInput {
                entity_id: "lock.front_door".to_owned()
            }
            .validate(ControlAction::LockUnlock)
            .is_ok()
        );
        for entity_id in ["switch.front_door", "lock", "lock.Front_door"] {
            assert!(
                EntityControlInput {
                    entity_id: entity_id.to_owned()
                }
                .validate(ControlAction::LockUnlock)
                .is_err()
            );
        }
    }

    #[test]
    fn numeric_controls_accept_boundaries_and_reject_values_outside_them() {
        for brightness_pct in [0, 100] {
            let control = LightTurnOnInput {
                entity_id: "light.kitchen".to_owned(),
                brightness_pct: Some(brightness_pct),
            }
            .validate()
            .unwrap();
            assert_eq!(control.service_data()["brightness_pct"], brightness_pct);
        }
        assert!(
            LightTurnOnInput {
                entity_id: "light.kitchen".to_owned(),
                brightness_pct: Some(101),
            }
            .validate()
            .is_err()
        );
        assert!(
            serde_json::from_value::<LightTurnOnInput>(
                json!({"entity_id":"light.kitchen","brightness_pct":-1})
            )
            .is_err()
        );

        for percentage in [0, 100] {
            let control = FanPercentageInput {
                entity_id: "fan.office".to_owned(),
                percentage,
            }
            .validate()
            .unwrap();
            assert_eq!(control.service_data()["percentage"], percentage);
        }
        assert!(
            FanPercentageInput {
                entity_id: "fan.office".to_owned(),
                percentage: 101,
            }
            .validate()
            .is_err()
        );
        assert!(
            serde_json::from_value::<FanPercentageInput>(
                json!({"entity_id":"fan.office","percentage":-1})
            )
            .is_err()
        );

        for position in [0, 100] {
            let control = CoverPositionInput {
                entity_id: "cover.office".to_owned(),
                position,
            }
            .validate()
            .unwrap();
            assert_eq!(control.service_data()["position"], position);
        }
        assert!(
            CoverPositionInput {
                entity_id: "cover.office".to_owned(),
                position: 101,
            }
            .validate()
            .is_err()
        );
        assert!(
            serde_json::from_value::<CoverPositionInput>(
                json!({"entity_id":"cover.office","position":-1})
            )
            .is_err()
        );

        for temperature in [MIN_TEMPERATURE, MAX_TEMPERATURE] {
            let control = ClimateTemperatureInput {
                entity_id: "climate.office".to_owned(),
                temperature,
            }
            .validate()
            .unwrap();
            assert_eq!(control.service_data()["temperature"], temperature);
        }
        for temperature in [f64::NAN, f64::INFINITY, -273.16, 1000.01] {
            assert!(
                ClimateTemperatureInput {
                    entity_id: "climate.office".to_owned(),
                    temperature
                }
                .validate()
                .is_err()
            );
        }

        for volume_level in [0.0, 1.0] {
            let control = MediaPlayerVolumeInput {
                entity_id: "media_player.lounge".to_owned(),
                volume_level,
            }
            .validate()
            .unwrap();
            assert_eq!(control.service_data()["volume_level"], volume_level);
        }
        for volume_level in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
            assert!(
                MediaPlayerVolumeInput {
                    entity_id: "media_player.lounge".to_owned(),
                    volume_level
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn generated_schemas_reject_unknown_fields_and_bound_numbers() {
        for schema in [
            schemars::schema_for!(EntityControlInput),
            schemars::schema_for!(LightTurnOnInput),
            schemars::schema_for!(FanPercentageInput),
            schemars::schema_for!(CoverPositionInput),
            schemars::schema_for!(ClimateTemperatureInput),
            schemars::schema_for!(MediaPlayerVolumeInput),
        ] {
            let schema = serde_json::to_value(schema).unwrap();
            assert_eq!(schema["additionalProperties"], false);
        }
        let temperature =
            serde_json::to_value(schemars::schema_for!(ClimateTemperatureInput)).unwrap();
        assert_eq!(temperature["properties"]["temperature"]["minimum"], -273.15);
        assert_eq!(temperature["properties"]["temperature"]["maximum"], 1000.0);
    }
}
