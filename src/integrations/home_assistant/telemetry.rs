use std::{sync::OnceLock, time::Instant};

use opentelemetry::{KeyValue, global};

use super::Error;

struct HomeAssistantMetrics {
    requests: opentelemetry::metrics::Counter<u64>,
    duration: opentelemetry::metrics::Histogram<f64>,
    in_flight: opentelemetry::metrics::UpDownCounter<i64>,
}

fn metrics() -> &'static HomeAssistantMetrics {
    static METRICS: OnceLock<HomeAssistantMetrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        let meter = global::meter("smarthome_mcp.home_assistant");
        HomeAssistantMetrics {
            requests: meter
                .u64_counter("smarthome_mcp.home_assistant.requests")
                .with_description("Completed bounded Home Assistant operations")
                .build(),
            duration: meter
                .f64_histogram("smarthome_mcp.home_assistant.duration")
                .with_unit("s")
                .with_description("Home Assistant operation duration")
                .build(),
            in_flight: meter
                .i64_up_down_counter("smarthome_mcp.home_assistant.in_flight")
                .with_description("Active Home Assistant operations")
                .build(),
        }
    })
}

pub(super) struct MetricsGuard {
    pub(super) action: &'static str,
    started: Instant,
    finished: bool,
}

impl MetricsGuard {
    pub(super) fn new(action: &'static str) -> Self {
        let guard = Self {
            action: metric_action(action),
            started: Instant::now(),
            finished: false,
        };
        metrics()
            .in_flight
            .add(1, &[KeyValue::new("action", guard.action)]);
        guard
    }

    pub(super) fn finish(&mut self, outcome: &'static str) {
        if self.finished {
            return;
        }
        let base = [KeyValue::new("action", self.action)];
        let completed = [
            KeyValue::new("action", self.action),
            KeyValue::new("outcome", metric_outcome(outcome)),
        ];
        let metrics = metrics();
        metrics.in_flight.add(-1, &base);
        metrics.requests.add(1, &completed);
        metrics
            .duration
            .record(self.started.elapsed().as_secs_f64(), &completed);
        self.finished = true;
    }
}

impl Drop for MetricsGuard {
    fn drop(&mut self) {
        self.finish("cancelled");
    }
}

fn metric_action(value: &'static str) -> &'static str {
    match value {
        "entity.list" => "entity.list",
        "device.list" => "device.list",
        "state.get" => "state.get",
        "history.get" => "history.get",
        "camera.snapshot" => "camera.snapshot",
        "network.list" => "network.list",
        "router.discover" => "router.discover",
        "thread.readiness.get" => "thread.readiness.get",
        "network.set_preferred" => "network.set_preferred",
        "router.set_preferred" => "router.set_preferred",
        "matter.readiness.get" => "matter.readiness.get",
        "matter.device.list" => "matter.device.list",
        "device.diagnostics" => "device.diagnostics",
        "device.ping" => "device.ping",
        "device.interview" => "device.interview",
        "scene.activate" => "scene.activate",
        "scene.upsert" => "scene.upsert",
        "automation.upsert" => "automation.upsert",
        "automation.validate" => "automation.validate",
        "automation.traces" => "automation.traces",
        "light.turn_on" => "light.turn_on",
        "light.turn_off" => "light.turn_off",
        "switch.turn_on" => "switch.turn_on",
        "switch.turn_off" => "switch.turn_off",
        "fan.turn_on" => "fan.turn_on",
        "fan.turn_off" => "fan.turn_off",
        "fan.set_percentage" => "fan.set_percentage",
        "cover.open" => "cover.open",
        "cover.close" => "cover.close",
        "cover.stop" => "cover.stop",
        "cover.set_position" => "cover.set_position",
        "climate.turn_on" => "climate.turn_on",
        "climate.turn_off" => "climate.turn_off",
        "climate.set_temperature" => "climate.set_temperature",
        "media_player.turn_on" => "media_player.turn_on",
        "media_player.turn_off" => "media_player.turn_off",
        "media_player.play" => "media_player.play",
        "media_player.pause" => "media_player.pause",
        "media_player.stop" => "media_player.stop",
        "media_player.volume_set" => "media_player.volume_set",
        "lock.lock" => "lock.lock",
        "lock.unlock" => "lock.unlock",
        _ => "unknown",
    }
}

fn metric_outcome(value: &'static str) -> &'static str {
    match value {
        "success"
        | "capacity_exhausted"
        | "timeout"
        | "unauthorized"
        | "not_allowed"
        | "not_matter_device"
        | "not_found"
        | "request_rejected"
        | "upstream_unavailable"
        | "response_too_large"
        | "invalid_response"
        | "cancelled" => value,
        _ => "upstream_unavailable",
    }
}

pub(super) fn request_outcome<T>(result: &Result<T, Error>) -> &'static str {
    match result {
        Ok(_) => "success",
        Err(Error::InvalidArguments) => "invalid_arguments",
        Err(Error::CapacityExhausted) => "capacity_exhausted",
        Err(Error::Timeout) => "timeout",
        Err(Error::Unauthorized) => "unauthorized",
        Err(Error::NotAllowed) => "not_allowed",
        Err(Error::NotMatterDevice) => "not_matter_device",
        Err(Error::NotFound) => "not_found",
        Err(Error::RequestRejected) => "request_rejected",
        Err(Error::UpstreamUnavailable) => "upstream_unavailable",
        Err(Error::ResponseTooLarge) => "response_too_large",
        Err(Error::InvalidResponse) => "invalid_response",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_labels_are_allowlisted() {
        for action in [
            "entity.list",
            "device.list",
            "state.get",
            "history.get",
            "camera.snapshot",
            "network.list",
            "router.discover",
            "thread.readiness.get",
            "network.set_preferred",
            "router.set_preferred",
            "matter.readiness.get",
            "matter.device.list",
            "device.diagnostics",
            "device.ping",
            "device.interview",
            "scene.activate",
            "scene.upsert",
            "automation.upsert",
            "automation.validate",
            "automation.traces",
            "light.turn_on",
            "light.turn_off",
            "switch.turn_on",
            "switch.turn_off",
            "fan.turn_on",
            "fan.turn_off",
            "fan.set_percentage",
            "cover.open",
            "cover.close",
            "cover.stop",
            "cover.set_position",
            "climate.turn_on",
            "climate.turn_off",
            "climate.set_temperature",
            "media_player.turn_on",
            "media_player.turn_off",
            "media_player.play",
            "media_player.pause",
            "media_player.stop",
            "media_player.volume_set",
            "lock.lock",
            "lock.unlock",
        ] {
            let mut guard = MetricsGuard::new(action);
            assert_eq!(guard.action, action);
            guard.finish("success");
        }
        let mut guard = MetricsGuard::new("attacker-controlled");
        assert_eq!(guard.action, "unknown");
        guard.finish("attacker-controlled");
        assert_eq!(metric_outcome("not_matter_device"), "not_matter_device");
    }
}
