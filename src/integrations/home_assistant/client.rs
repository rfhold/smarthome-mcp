use std::{
    collections::{BTreeMap, HashMap, HashSet},
    future::Future,
    sync::Arc,
    time::Duration,
};

use futures_util::{SinkExt as _, StreamExt as _};
use reqwest::{Client, Response, StatusCode, redirect::Policy};
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use tracing::Instrument as _;
use url::Url;

use crate::{config::Secret, http_client};

use super::{
    Error,
    actions::{
        CameraSnapshotQuery, Control, DevicesQuery, EntitiesQuery, HistoryQuery, StatesQuery,
        valid_entity_id,
    },
    telemetry::{MetricsGuard, request_outcome},
};

const MAX_CONCURRENT_QUERIES: usize = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_WS_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_HISTORY_POINTS: usize = 2_000;
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_STATE_BYTES: usize = 256;
const MAX_FRIENDLY_NAME_BYTES: usize = 256;
const MAX_DEVICE_CLASS_BYTES: usize = 128;
const MAX_UNIT_BYTES: usize = 64;
const MAX_REGISTRY_ID_BYTES: usize = 255;
const MAX_REGISTRY_NAME_BYTES: usize = 256;

type HomeAssistantSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone)]
pub struct HomeAssistantClient {
    origin: Url,
    token: Secret,
    http: ClientWithMiddleware,
    concurrency: Arc<Semaphore>,
    timeout: Duration,
}

#[derive(Debug, Deserialize)]
struct RawState {
    entity_id: String,
    state: String,
    #[serde(default)]
    attributes: Value,
    last_changed: String,
    last_updated: String,
}

#[derive(Debug, Serialize)]
struct EntityState {
    entity_id: String,
    domain: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    friendly_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit_of_measurement: Option<String>,
    last_changed: String,
    last_updated: String,
}

#[derive(Debug, Deserialize)]
struct RawEntityRegistryEntry {
    entity_id: String,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    area_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDeviceRegistryEntry {
    id: String,
    #[serde(default)]
    area_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    name_by_user: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAreaRegistryEntry {
    area_id: String,
    name: String,
}

struct EntityRegistryEntry {
    device_id: Option<String>,
    area_id: Option<String>,
}

struct DeviceRegistryEntry {
    area_id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct DeviceGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    area: Option<String>,
    entities: Vec<EntityState>,
}

#[derive(Debug, Deserialize)]
struct RawHistoryState {
    #[serde(default)]
    entity_id: Option<String>,
    state: String,
    last_changed: String,
}

#[derive(Debug, Serialize)]
struct HistoryState {
    state: String,
    last_changed: String,
}

#[derive(Debug, Serialize)]
struct EntityHistory {
    entity_id: String,
    states: Vec<HistoryState>,
}

pub(crate) struct CameraSnapshot {
    pub(crate) entity_id: String,
    pub(crate) mime_type: &'static str,
    pub(crate) data: Vec<u8>,
}

impl HomeAssistantClient {
    pub fn production(origin: Url, token: Secret) -> Result<Self, String> {
        let http = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| "failed to construct Home Assistant HTTP client".to_owned())?;
        Ok(Self {
            origin,
            token,
            http: http_client::with_tracing(http),
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_QUERIES)),
            timeout: REQUEST_TIMEOUT,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(origin: Url, token: Secret, timeout: Duration) -> Self {
        Self {
            origin,
            token,
            http: http_client::with_tracing(
                Client::builder()
                    .redirect(Policy::none())
                    .no_proxy()
                    .build()
                    .unwrap(),
            ),
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_QUERIES)),
            timeout,
        }
    }

    pub(crate) async fn list_entities(&self, query: &EntitiesQuery) -> Result<Value, Error> {
        let _permit = self.admit()?;
        let mut metrics = MetricsGuard::new("entity.list");
        let span = tracing::info_span!(
            target: "smarthome_mcp::home_assistant",
            "home_assistant.query",
            action = "entity.list",
            outcome = tracing::field::Empty,
        );
        let result = tokio::time::timeout(self.timeout, async {
            let exposed = self.exposed_entities().await?;
            let url = self.endpoint(&["api", "states"])?;
            let states: Vec<RawState> = self.get_json(url).await?;
            let mut entities = states
                .into_iter()
                .filter(|state| exposed.contains(&state.entity_id))
                .map(normalize_state)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|state| {
                    query.domains.is_empty() || query.domains.binary_search(&state.domain).is_ok()
                })
                .filter(|state| {
                    query.query.as_ref().is_none_or(|needle| {
                        state.entity_id.to_lowercase().contains(needle)
                            || state
                                .friendly_name
                                .as_ref()
                                .is_some_and(|name| name.to_lowercase().contains(needle))
                    })
                })
                .collect::<Vec<_>>();
            entities.sort_by(|left, right| left.entity_id.cmp(&right.entity_id));
            let truncated = entities.len() > query.limit;
            entities.truncate(query.limit);
            bounded_output(json!({
                "action": "entity.list",
                "entities": entities,
                "truncated": truncated,
            }))
        })
        .instrument(span.clone())
        .await
        .unwrap_or(Err(Error::Timeout));
        let outcome = request_outcome(&result);
        span.record("outcome", outcome);
        metrics.finish(outcome);
        result
    }

    pub(crate) async fn list_devices(&self, query: &DevicesQuery) -> Result<Value, Error> {
        let _permit = self.admit()?;
        let mut metrics = MetricsGuard::new("device.list");
        let span = tracing::info_span!(
            target: "smarthome_mcp::home_assistant",
            "home_assistant.query",
            action = "device.list",
            outcome = tracing::field::Empty,
        );
        let result = tokio::time::timeout(self.timeout, async {
            let mut socket = self.open_websocket().await?;
            let exposed = self.exposed_entities_on(&mut socket).await?;
            let url = self.endpoint(&["api", "states"])?;
            let states: Vec<RawState> = self.get_json(url).await?;
            let mut entities = states
                .into_iter()
                .filter(|state| exposed.contains(&state.entity_id))
                .map(normalize_state)
                .collect::<Result<Vec<_>, _>>()?;
            entities.sort_by(|left, right| left.entity_id.cmp(&right.entity_id));
            if entities
                .windows(2)
                .any(|pair| pair[0].entity_id == pair[1].entity_id)
            {
                return Err(Error::InvalidResponse);
            }
            let truncated = entities.len() > query.limit;
            entities.truncate(query.limit);

            if entities.is_empty() {
                let _ = socket.close(None).await;
                return bounded_output(json!({
                    "action": "device.list",
                    "devices": [],
                    "truncated": truncated,
                }));
            }

            let entity_ids = entities
                .iter()
                .map(|state| state.entity_id.clone())
                .collect::<Vec<_>>();
            let entity_registry = self.entity_registry(&mut socket, &entity_ids).await?;
            let referenced_device_ids = entity_registry
                .values()
                .filter_map(|entry| entry.as_ref()?.device_id.clone())
                .collect::<HashSet<_>>();
            let devices = self
                .device_registry(&mut socket, &referenced_device_ids)
                .await?;
            let referenced_area_ids = entity_registry
                .values()
                .filter_map(|entry| entry.as_ref()?.area_id.clone())
                .chain(devices.values().filter_map(|entry| entry.area_id.clone()))
                .collect::<HashSet<_>>();
            let areas = self
                .area_registry(&mut socket, &referenced_area_ids)
                .await?;
            let _ = socket.close(None).await;

            let groups = group_devices(entities, entity_registry, &devices, &areas)?;
            bounded_output(json!({
                "action": "device.list",
                "devices": groups,
                "truncated": truncated,
            }))
        })
        .instrument(span.clone())
        .await
        .unwrap_or(Err(Error::Timeout));
        let outcome = request_outcome(&result);
        span.record("outcome", outcome);
        metrics.finish(outcome);
        result
    }

    pub(crate) async fn get_states(&self, query: &StatesQuery) -> Result<Value, Error> {
        let _permit = self.admit()?;
        let mut metrics = MetricsGuard::new("state.get");
        let span = tracing::info_span!(
            target: "smarthome_mcp::home_assistant",
            "home_assistant.query",
            action = "state.get",
            outcome = tracing::field::Empty,
        );
        let result = tokio::time::timeout(self.timeout, async {
            let exposed = self.exposed_entities().await?;
            self.authorize(&query.entity_ids, &exposed)?;
            let mut states = Vec::with_capacity(query.entity_ids.len());
            for entity_id in &query.entity_ids {
                let url = self.endpoint(&["api", "states", entity_id])?;
                let state: RawState = self.get_json(url).await?;
                let state = normalize_state(state)?;
                if state.entity_id != *entity_id {
                    return Err(Error::InvalidResponse);
                }
                states.push(state);
            }
            bounded_output(json!({"action": "state.get", "entities": states}))
        })
        .instrument(span.clone())
        .await
        .unwrap_or(Err(Error::Timeout));
        let outcome = request_outcome(&result);
        span.record("outcome", outcome);
        metrics.finish(outcome);
        result
    }

    pub(crate) async fn get_history(&self, query: &HistoryQuery) -> Result<Value, Error> {
        let _permit = self.admit()?;
        let mut metrics = MetricsGuard::new("history.get");
        let span = tracing::info_span!(
            target: "smarthome_mcp::home_assistant",
            "home_assistant.query",
            action = "history.get",
            outcome = tracing::field::Empty,
        );
        let result = tokio::time::timeout(self.timeout, async {
            let exposed = self.exposed_entities().await?;
            self.authorize(&query.entity_ids, &exposed)?;
            let start = query.start.to_rfc3339();
            let mut url = self.endpoint(&["api", "history", "period", &start])?;
            {
                let mut pairs = url.query_pairs_mut();
                pairs.append_pair("filter_entity_id", &query.entity_ids.join(","));
                pairs.append_pair("end_time", &query.end.to_rfc3339());
                pairs.append_pair("minimal_response", "");
                pairs.append_pair("no_attributes", "");
                pairs.append_pair("significant_changes_only", "");
            }
            self.check_url(&url)?;
            let groups: Vec<Vec<RawHistoryState>> = self.get_json(url).await?;
            let total_points = groups.iter().try_fold(0usize, |total, group| {
                total.checked_add(group.len()).ok_or(Error::InvalidResponse)
            })?;
            let mut remaining = MAX_HISTORY_POINTS;
            let truncated = total_points > MAX_HISTORY_POINTS;
            let mut history = Vec::with_capacity(groups.len());
            let mut seen = HashSet::new();
            for group in groups {
                let Some(entity_id) = group.first().and_then(|state| state.entity_id.clone())
                else {
                    if group.is_empty() {
                        continue;
                    }
                    return Err(Error::InvalidResponse);
                };
                if !valid_entity_id(&entity_id)
                    || !query.entity_ids.contains(&entity_id)
                    || !seen.insert(entity_id.clone())
                    || group
                        .iter()
                        .any(|state| state.entity_id.as_ref().is_some_and(|id| id != &entity_id))
                {
                    return Err(Error::InvalidResponse);
                }
                let take = remaining.min(group.len());
                let mut states = Vec::with_capacity(take);
                for state in group {
                    let state = normalize_history_state(state)?;
                    if states.len() < take {
                        states.push(state);
                    }
                }
                remaining -= take;
                if !states.is_empty() {
                    history.push(EntityHistory { entity_id, states });
                }
            }
            bounded_output(json!({
                "action": "history.get",
                "start": query.start.to_rfc3339(),
                "end": query.end.to_rfc3339(),
                "history": history,
                "truncated": truncated,
            }))
        })
        .instrument(span.clone())
        .await
        .unwrap_or(Err(Error::Timeout));
        let outcome = request_outcome(&result);
        span.record("outcome", outcome);
        metrics.finish(outcome);
        result
    }

    #[cfg(test)]
    pub(crate) async fn camera_snapshot(
        &self,
        query: &CameraSnapshotQuery,
    ) -> Result<CameraSnapshot, Error> {
        self.camera_snapshot_with(query, |snapshot| async move { Ok(snapshot) })
            .await
    }

    pub(crate) async fn camera_snapshot_with<T, F, Fut>(
        &self,
        query: &CameraSnapshotQuery,
        complete: F,
    ) -> Result<T, Error>
    where
        F: FnOnce(CameraSnapshot) -> Fut,
        Fut: Future<Output = Result<T, Error>>,
    {
        let _permit = self.admit()?;
        let mut metrics = MetricsGuard::new("camera.snapshot");
        let span = tracing::info_span!(
            target: "smarthome_mcp::home_assistant",
            "home_assistant.query",
            action = "camera.snapshot",
            outcome = tracing::field::Empty,
        );
        let deadline = tokio::time::Instant::now() + self.timeout;
        let result = tokio::time::timeout_at(deadline, async {
            let exposed = self.exposed_entities().await?;
            self.authorize(std::slice::from_ref(&query.entity_id), &exposed)?;
            let url = self.endpoint(&["api", "camera_proxy", &query.entity_id])?;
            let (mime_type, data) = self.get_image(url).await?;
            let snapshot = CameraSnapshot {
                entity_id: query.entity_id.clone(),
                mime_type,
                data,
            };
            let completed = complete(snapshot).await;
            // Give timeout and cancellation observers a chance to reject synchronous completion.
            tokio::task::yield_now().await;
            if tokio::time::Instant::now() >= deadline {
                Err(Error::Timeout)
            } else {
                completed
            }
        })
        .instrument(span.clone())
        .await
        .unwrap_or(Err(Error::Timeout));
        let outcome = request_outcome(&result);
        span.record("outcome", outcome);
        metrics.finish(outcome);
        result
    }

    pub(crate) async fn execute_control(&self, control: &Control) -> Result<Value, Error> {
        let _permit = self.admit()?;
        let action = control.action();
        let mut metrics = MetricsGuard::new(action);
        let span = tracing::info_span!(
            target: "smarthome_mcp::home_assistant",
            "home_assistant.exec",
            action,
            outcome = tracing::field::Empty,
        );
        let result = tokio::time::timeout(self.timeout, async {
            let exposed = self.exposed_entities().await?;
            let entity_id = control.entity_id().to_owned();
            self.authorize(std::slice::from_ref(&entity_id), &exposed)?;
            let (domain, service) = control.service();
            let url = self.endpoint(&["api", "services", domain, service])?;
            self.post_json(url, &control.service_data()).await?;
            Ok(json!({
                "action": control.action(),
                "entity_id": control.entity_id(),
                "success": true,
            }))
        })
        .instrument(span.clone())
        .await
        .unwrap_or(Err(Error::Timeout));
        let outcome = request_outcome(&result);
        span.record("outcome", outcome);
        metrics.finish(outcome);
        result
    }

    fn admit(&self) -> Result<OwnedSemaphorePermit, Error> {
        self.concurrency
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::CapacityExhausted)
    }

    fn authorize(&self, entity_ids: &[String], exposed: &HashSet<String>) -> Result<(), Error> {
        if entity_ids
            .iter()
            .all(|entity_id| exposed.contains(entity_id))
        {
            Ok(())
        } else {
            Err(Error::NotAllowed)
        }
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, Error> {
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .map_err(|_| Error::InvalidArguments)?
            .extend(segments);
        self.check_url(&url)?;
        Ok(url)
    }

    fn check_url(&self, url: &Url) -> Result<(), Error> {
        if url.as_str().len() > MAX_URL_BYTES {
            Err(Error::InvalidArguments)
        } else {
            Ok(())
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, url: Url) -> Result<T, Error> {
        let mut request_span = http_client::RequestSpan::new(&reqwest::Method::GET);
        let response = self
            .http
            .get(url)
            .bearer_auth(self.token.expose())
            .header(reqwest::header::ACCEPT, "application/json")
            .with_extension(request_span.extension())
            .send()
            .instrument(request_span.span())
            .await
            .map_err(|_| {
                request_span.transport_error();
                Error::UpstreamUnavailable
            })?;
        let body_span = request_span.span();
        let body = read_body(response, &mut request_span)
            .instrument(body_span)
            .await?;
        serde_json::from_slice(&body).map_err(|_| Error::InvalidResponse)
    }

    async fn get_image(&self, url: Url) -> Result<(&'static str, Vec<u8>), Error> {
        let mut request_span = http_client::RequestSpan::new(&reqwest::Method::GET);
        let response = self
            .http
            .get(url)
            .bearer_auth(self.token.expose())
            .header(reqwest::header::ACCEPT, "image/jpeg, image/png, image/webp")
            .with_extension(request_span.extension())
            .send()
            .instrument(request_span.span())
            .await
            .map_err(|_| {
                request_span.transport_error();
                Error::UpstreamUnavailable
            })?;
        let mime_type = match response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
        {
            Some("image/jpeg") => Some("image/jpeg"),
            Some("image/png") => Some("image/png"),
            Some("image/webp") => Some("image/webp"),
            _ => None,
        };
        let body_span = request_span.span();
        let body = read_body(response, &mut request_span)
            .instrument(body_span)
            .await?;
        let mime_type = match mime_type {
            Some("image/jpeg") if body.starts_with(&[0xff, 0xd8, 0xff]) => "image/jpeg",
            Some("image/png") if body.starts_with(b"\x89PNG\r\n\x1a\n") => "image/png",
            Some("image/webp")
                if body.len() >= 12 && &body[..4] == b"RIFF" && &body[8..12] == b"WEBP" =>
            {
                "image/webp"
            }
            _ => return Err(Error::InvalidResponse),
        };
        Ok((mime_type, body))
    }

    async fn post_json(&self, url: Url, body: &Value) -> Result<(), Error> {
        let mut request_span = http_client::RequestSpan::new(&reqwest::Method::POST);
        let body = serde_json::to_vec(body).map_err(|_| Error::InvalidArguments)?;
        let response = self
            .http
            .post(url)
            .bearer_auth(self.token.expose())
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .with_extension(request_span.extension())
            .send()
            .instrument(request_span.span())
            .await
            .map_err(|_| {
                request_span.transport_error();
                Error::UpstreamUnavailable
            })?;
        let body_span = request_span.span();
        let _ = read_body(response, &mut request_span)
            .instrument(body_span)
            .await?;
        Ok(())
    }

    async fn open_websocket(&self) -> Result<HomeAssistantSocket, Error> {
        let mut url = self.origin.clone();
        let scheme = match url.scheme() {
            "https" => "wss",
            "http" => "ws",
            _ => return Err(Error::InvalidArguments),
        };
        url.set_scheme(scheme)
            .map_err(|_| Error::InvalidArguments)?;
        url.set_path("/api/websocket");
        self.check_url(&url)?;

        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_WS_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_WS_MESSAGE_BYTES));
        let (mut socket, _) = connect_async_with_config(url, Some(config), false)
            .await
            .map_err(|_| Error::UpstreamUnavailable)?;
        let required = websocket_json(&mut socket).await?;
        if required.get("type").and_then(Value::as_str) != Some("auth_required") {
            return Err(Error::InvalidResponse);
        }
        socket
            .send(Message::Text(
                json!({"type":"auth", "access_token":self.token.expose()})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|_| Error::UpstreamUnavailable)?;
        let auth = websocket_json(&mut socket).await?;
        match auth.get("type").and_then(Value::as_str) {
            Some("auth_ok") => {}
            Some("auth_invalid") => return Err(Error::Unauthorized),
            _ => return Err(Error::InvalidResponse),
        }
        Ok(socket)
    }

    async fn exposed_entities(&self) -> Result<HashSet<String>, Error> {
        let mut socket = self.open_websocket().await?;
        let exposed = self.exposed_entities_on(&mut socket).await?;
        let _ = socket.close(None).await;
        Ok(exposed)
    }

    async fn exposed_entities_on(
        &self,
        socket: &mut HomeAssistantSocket,
    ) -> Result<HashSet<String>, Error> {
        socket
            .send(Message::Text(
                json!({"id":1,"type":"homeassistant/expose_entity/list"})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|_| Error::UpstreamUnavailable)?;
        let response = websocket_json(socket).await?;
        if response.get("id").and_then(Value::as_u64) != Some(1)
            || response.get("type").and_then(Value::as_str) != Some("result")
            || response.get("success").and_then(Value::as_bool) != Some(true)
        {
            return Err(Error::InvalidResponse);
        }
        let entities = response
            .pointer("/result/exposed_entities")
            .and_then(Value::as_object)
            .ok_or(Error::InvalidResponse)?;
        let exposed = entities
            .iter()
            .filter(|(_, assistants)| {
                assistants.get("conversation").and_then(Value::as_bool) == Some(true)
            })
            .map(|(entity_id, _)| entity_id.clone())
            .collect();
        Ok(exposed)
    }

    async fn entity_registry(
        &self,
        socket: &mut HomeAssistantSocket,
        entity_ids: &[String],
    ) -> Result<HashMap<String, Option<EntityRegistryEntry>>, Error> {
        let result = self
            .websocket_command(
                socket,
                2,
                json!({
                    "id": 2,
                    "type": "config/entity_registry/get_entries",
                    "entity_ids": entity_ids,
                }),
            )
            .await?;
        let object = result.as_object().ok_or(Error::InvalidResponse)?;
        if object.len() != entity_ids.len()
            || object
                .keys()
                .any(|entity_id| !entity_ids.contains(entity_id))
        {
            return Err(Error::InvalidResponse);
        }
        let mut entries = HashMap::with_capacity(object.len());
        for entity_id in entity_ids {
            let value = object.get(entity_id).ok_or(Error::InvalidResponse)?;
            if value.is_null() {
                entries.insert(entity_id.clone(), None);
                continue;
            }
            let raw: RawEntityRegistryEntry =
                serde_json::from_value(value.clone()).map_err(|_| Error::InvalidResponse)?;
            if raw.entity_id != *entity_id {
                return Err(Error::InvalidResponse);
            }
            validate_registry_reference(&raw.entity_id)?;
            validate_optional_registry_reference(&raw.device_id)?;
            validate_optional_registry_reference(&raw.area_id)?;
            entries.insert(
                entity_id.clone(),
                Some(EntityRegistryEntry {
                    device_id: raw.device_id,
                    area_id: raw.area_id,
                }),
            );
        }
        Ok(entries)
    }

    async fn device_registry(
        &self,
        socket: &mut HomeAssistantSocket,
        referenced_ids: &HashSet<String>,
    ) -> Result<HashMap<String, DeviceRegistryEntry>, Error> {
        let result = self
            .websocket_command(
                socket,
                3,
                json!({"id":3,"type":"config/device_registry/list"}),
            )
            .await?;
        let raw = result.as_array().ok_or(Error::InvalidResponse)?;
        let mut entries = HashMap::with_capacity(referenced_ids.len());
        for value in raw {
            let Some(id) = value.get("id").and_then(Value::as_str) else {
                continue;
            };
            if !referenced_ids.contains(id) {
                continue;
            }
            let entry: RawDeviceRegistryEntry =
                serde_json::from_value(value.clone()).map_err(|_| Error::InvalidResponse)?;
            validate_registry_reference(&entry.id)?;
            validate_optional_registry_reference(&entry.area_id)?;
            validate_optional_name(&entry.name)?;
            validate_optional_name(&entry.name_by_user)?;
            let name = entry
                .name_by_user
                .filter(|value| !value.is_empty())
                .or_else(|| entry.name.filter(|value| !value.is_empty()));
            if entries
                .insert(
                    entry.id,
                    DeviceRegistryEntry {
                        area_id: entry.area_id,
                        name,
                    },
                )
                .is_some()
            {
                return Err(Error::InvalidResponse);
            }
        }
        Ok(entries)
    }

    async fn area_registry(
        &self,
        socket: &mut HomeAssistantSocket,
        referenced_ids: &HashSet<String>,
    ) -> Result<HashMap<String, String>, Error> {
        let result = self
            .websocket_command(
                socket,
                4,
                json!({"id":4,"type":"config/area_registry/list"}),
            )
            .await?;
        let raw = result.as_array().ok_or(Error::InvalidResponse)?;
        let mut entries = HashMap::with_capacity(referenced_ids.len());
        for value in raw {
            let Some(area_id) = value.get("area_id").and_then(Value::as_str) else {
                continue;
            };
            if !referenced_ids.contains(area_id) {
                continue;
            }
            let entry: RawAreaRegistryEntry =
                serde_json::from_value(value.clone()).map_err(|_| Error::InvalidResponse)?;
            validate_registry_reference(&entry.area_id)?;
            if entry.name.is_empty() || entry.name.len() > MAX_REGISTRY_NAME_BYTES {
                return Err(Error::InvalidResponse);
            }
            if entries.insert(entry.area_id, entry.name).is_some() {
                return Err(Error::InvalidResponse);
            }
        }
        Ok(entries)
    }

    async fn websocket_command(
        &self,
        socket: &mut HomeAssistantSocket,
        id: u64,
        command: Value,
    ) -> Result<Value, Error> {
        socket
            .send(Message::Text(command.to_string().into()))
            .await
            .map_err(|_| Error::UpstreamUnavailable)?;
        let response = websocket_json(socket).await?;
        if response.get("id").and_then(Value::as_u64) != Some(id)
            || response.get("type").and_then(Value::as_str) != Some("result")
            || response.get("success").and_then(Value::as_bool) != Some(true)
        {
            return Err(Error::InvalidResponse);
        }
        response
            .get("result")
            .cloned()
            .ok_or(Error::InvalidResponse)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum DeviceGroupKey {
    Device(String, Option<String>),
    Standalone(String),
}

fn group_devices(
    entities: Vec<EntityState>,
    mut entity_registry: HashMap<String, Option<EntityRegistryEntry>>,
    devices: &HashMap<String, DeviceRegistryEntry>,
    areas: &HashMap<String, String>,
) -> Result<Vec<DeviceGroup>, Error> {
    let mut groups = BTreeMap::<DeviceGroupKey, DeviceGroup>::new();
    for entity in entities {
        let registry = entity_registry
            .remove(&entity.entity_id)
            .ok_or(Error::InvalidResponse)?;
        let device = registry
            .as_ref()
            .and_then(|entry| entry.device_id.as_ref())
            .and_then(|device_id| devices.get(device_id));
        let area_id = registry
            .as_ref()
            .and_then(|entry| entry.area_id.clone())
            .or_else(|| device.and_then(|entry| entry.area_id.clone()));
        let key = registry
            .as_ref()
            .and_then(|entry| entry.device_id.clone())
            .map_or_else(
                || DeviceGroupKey::Standalone(entity.entity_id.clone()),
                |device_id| DeviceGroupKey::Device(device_id, area_id.clone()),
            );
        let group = groups.entry(key).or_insert_with(|| DeviceGroup {
            name: device.and_then(|entry| entry.name.clone()),
            area: area_id
                .as_ref()
                .and_then(|area_id| areas.get(area_id).cloned()),
            entities: Vec::new(),
        });
        group.entities.push(entity);
    }
    if !entity_registry.is_empty() {
        return Err(Error::InvalidResponse);
    }
    let mut groups = groups.into_values().collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        left.area
            .cmp(&right.area)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.entities[0].entity_id.cmp(&right.entities[0].entity_id))
    });
    Ok(groups)
}

fn validate_registry_reference(value: &str) -> Result<(), Error> {
    if value.is_empty() || value.len() > MAX_REGISTRY_ID_BYTES {
        Err(Error::InvalidResponse)
    } else {
        Ok(())
    }
}

fn validate_optional_registry_reference(value: &Option<String>) -> Result<(), Error> {
    value
        .as_deref()
        .map(validate_registry_reference)
        .transpose()
        .map(|_| ())
}

fn validate_optional_name(value: &Option<String>) -> Result<(), Error> {
    if value
        .as_ref()
        .is_some_and(|value| value.len() > MAX_REGISTRY_NAME_BYTES)
    {
        Err(Error::InvalidResponse)
    } else {
        Ok(())
    }
}

fn normalize_state(state: RawState) -> Result<EntityState, Error> {
    if !valid_entity_id(&state.entity_id)
        || state.state.len() > MAX_STATE_BYTES
        || chrono::DateTime::parse_from_rfc3339(&state.last_changed).is_err()
        || chrono::DateTime::parse_from_rfc3339(&state.last_updated).is_err()
        || !state.attributes.is_object()
    {
        return Err(Error::InvalidResponse);
    }
    let domain = state.entity_id.split_once('.').unwrap().0.to_owned();
    Ok(EntityState {
        entity_id: state.entity_id,
        domain,
        state: state.state,
        friendly_name: attribute(&state.attributes, "friendly_name", MAX_FRIENDLY_NAME_BYTES)?,
        device_class: attribute(&state.attributes, "device_class", MAX_DEVICE_CLASS_BYTES)?,
        unit_of_measurement: attribute(&state.attributes, "unit_of_measurement", MAX_UNIT_BYTES)?,
        last_changed: state.last_changed,
        last_updated: state.last_updated,
    })
}

fn normalize_history_state(state: RawHistoryState) -> Result<HistoryState, Error> {
    if state.state.len() > MAX_STATE_BYTES
        || chrono::DateTime::parse_from_rfc3339(&state.last_changed).is_err()
    {
        return Err(Error::InvalidResponse);
    }
    Ok(HistoryState {
        state: state.state,
        last_changed: state.last_changed,
    })
}

fn attribute(attributes: &Value, name: &str, max_bytes: usize) -> Result<Option<String>, Error> {
    match attributes.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.len() <= max_bytes => Ok(Some(value.clone())),
        Some(_) => Err(Error::InvalidResponse),
    }
}

fn bounded_output(value: Value) -> Result<Value, Error> {
    if serde_json::to_vec(&value)
        .map_err(|_| Error::InvalidResponse)?
        .len()
        > MAX_OUTPUT_BYTES
    {
        Err(Error::ResponseTooLarge)
    } else {
        Ok(value)
    }
}

async fn read_body(
    response: Response,
    request_span: &mut http_client::RequestSpan,
) -> Result<Vec<u8>, Error> {
    let status = response.status();
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            request_span.http_error(status);
            return Err(Error::Unauthorized);
        }
        StatusCode::NOT_FOUND => {
            request_span.http_error(status);
            return Err(Error::NotFound);
        }
        StatusCode::TOO_MANY_REQUESTS => {
            request_span.http_error(status);
            return Err(Error::CapacityExhausted);
        }
        status if status.is_client_error() => {
            request_span.http_error(status);
            return Err(Error::RequestRejected);
        }
        status if !status.is_success() => {
            request_span.http_error(status);
            return Err(Error::UpstreamUnavailable);
        }
        _ => {}
    }
    request_span.record_status(status);
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        request_span.response_error();
        return Err(Error::ResponseTooLarge);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            request_span.transport_error();
            Error::UpstreamUnavailable
        })?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            request_span.response_error();
            return Err(Error::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    request_span.success();
    Ok(body)
}

async fn websocket_json<S>(socket: &mut WebSocketStream<S>) -> Result<Value, Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(text.as_ref()).map_err(|_| Error::InvalidResponse);
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(_)) => return Err(Error::InvalidResponse),
            Some(Err(_)) => return Err(Error::UpstreamUnavailable),
            None => return Err(Error::UpstreamUnavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        process::Command,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use axum::{
        Json, Router,
        body::{Body, Bytes},
        extract::{OriginalUri, Path, State, WebSocketUpgrade, ws},
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use chrono::{DateTime, Utc};
    use opentelemetry::{
        global,
        trace::{SpanKind, Status, TracerProvider as _},
    };
    use opentelemetry_sdk::{
        propagation::TraceContextPropagator,
        trace::{Sampler, SdkTracerProvider, SpanData, in_memory_exporter::InMemorySpanExporter},
    };
    use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
    use tracing::instrument::WithSubscriber as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    use crate::integrations::home_assistant::actions::{
        ControlAction, EntityControlInput, LightTurnOnInput,
    };

    use super::*;

    struct CompletionGuard(Arc<AtomicBool>);

    impl Drop for CompletionGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[derive(Clone)]
    struct MockHomeAssistant {
        exposure: Value,
        entity_registry: Value,
        device_registry: Value,
        area_registry: Value,
        registry_response_overrides: HashMap<&'static str, Value>,
        states: Value,
        state_override: Option<Value>,
        history: Value,
        states_status: StatusCode,
        states_delay: Option<Duration>,
        camera_body: Vec<u8>,
        camera_content_type: Option<String>,
        camera_status: StatusCode,
        camera_delay: Option<Duration>,
        camera_declared_length: Option<u64>,
        camera_streamed: bool,
        service_status: StatusCode,
        service_body: Vec<u8>,
        service_delay: Option<Duration>,
        websocket_calls: Arc<AtomicUsize>,
        state_calls: Arc<AtomicUsize>,
        camera_calls: Arc<AtomicUsize>,
        service_calls: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<String>>>,
        commands: Arc<Mutex<Vec<Value>>>,
        traceparents: Arc<Mutex<Vec<String>>>,
    }

    impl MockHomeAssistant {
        fn new(exposure: Value, states: Value, history: Value) -> Self {
            Self {
                exposure,
                entity_registry: json!({}),
                device_registry: json!([]),
                area_registry: json!([]),
                registry_response_overrides: HashMap::new(),
                states,
                state_override: None,
                history,
                states_status: StatusCode::OK,
                states_delay: None,
                camera_body: vec![0xff, 0xd8, 0xff, 0xd9],
                camera_content_type: Some("image/jpeg".to_owned()),
                camera_status: StatusCode::OK,
                camera_delay: None,
                camera_declared_length: None,
                camera_streamed: false,
                service_status: StatusCode::OK,
                service_body: b"[{\"must_not_leak\":true}]".to_vec(),
                service_delay: None,
                websocket_calls: Arc::new(AtomicUsize::new(0)),
                state_calls: Arc::new(AtomicUsize::new(0)),
                camera_calls: Arc::new(AtomicUsize::new(0)),
                service_calls: Arc::new(AtomicUsize::new(0)),
                requests: Arc::new(Mutex::new(Vec::new())),
                commands: Arc::new(Mutex::new(Vec::new())),
                traceparents: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_state_override(mut self, state: Value) -> Self {
            self.state_override = Some(state);
            self
        }

        fn with_states_status(mut self, status: StatusCode) -> Self {
            self.states_status = status;
            self
        }

        fn with_states_delay(mut self, delay: Duration) -> Self {
            self.states_delay = Some(delay);
            self
        }

        fn with_camera(mut self, content_type: Option<&str>, body: Vec<u8>) -> Self {
            self.camera_content_type = content_type.map(str::to_owned);
            self.camera_body = body;
            self
        }

        fn with_camera_status(mut self, status: StatusCode) -> Self {
            self.camera_status = status;
            self
        }

        fn with_camera_delay(mut self, delay: Duration) -> Self {
            self.camera_delay = Some(delay);
            self
        }

        fn with_camera_declared_length(mut self, length: u64) -> Self {
            self.camera_declared_length = Some(length);
            self
        }

        fn with_streamed_camera(mut self) -> Self {
            self.camera_streamed = true;
            self
        }

        fn with_registries(
            mut self,
            entity_registry: Value,
            device_registry: Value,
            area_registry: Value,
        ) -> Self {
            self.entity_registry = entity_registry;
            self.device_registry = device_registry;
            self.area_registry = area_registry;
            self
        }

        fn with_registry_response(mut self, command: &'static str, response: Value) -> Self {
            self.registry_response_overrides.insert(command, response);
            self
        }

        fn with_service_status(mut self, status: StatusCode) -> Self {
            self.service_status = status;
            self
        }

        fn with_service_delay(mut self, delay: Duration) -> Self {
            self.service_delay = Some(delay);
            self
        }
    }

    #[test]
    fn normalization_only_keeps_approved_attributes() {
        let state = normalize_state(RawState {
            entity_id: "sensor.room".to_owned(),
            state: "20".to_owned(),
            attributes: json!({
                "friendly_name":"Room",
                "device_class":"temperature",
                "unit_of_measurement":"C",
                "latitude":"must-not-leak"
            }),
            last_changed: "2026-08-09T00:00:00Z".to_owned(),
            last_updated: "2026-08-09T00:00:00Z".to_owned(),
        })
        .unwrap();
        let value = serde_json::to_value(state).unwrap();
        assert_eq!(value["friendly_name"], "Room");
        assert!(value.get("latitude").is_none());
    }

    #[test]
    fn normalization_rejects_malformed_or_oversized_fields() {
        for state in [
            RawState {
                entity_id: "invalid".to_owned(),
                state: "on".to_owned(),
                attributes: json!({}),
                last_changed: "2026-08-09T00:00:00Z".to_owned(),
                last_updated: "2026-08-09T00:00:00Z".to_owned(),
            },
            RawState {
                entity_id: "sensor.room".to_owned(),
                state: "x".repeat(MAX_STATE_BYTES + 1),
                attributes: json!({}),
                last_changed: "2026-08-09T00:00:00Z".to_owned(),
                last_updated: "2026-08-09T00:00:00Z".to_owned(),
            },
            RawState {
                entity_id: "sensor.room".to_owned(),
                state: "on".to_owned(),
                attributes: json!({"friendly_name": ["not", "text"]}),
                last_changed: "not-a-time".to_owned(),
                last_updated: "2026-08-09T00:00:00Z".to_owned(),
            },
        ] {
            assert_eq!(normalize_state(state).unwrap_err(), Error::InvalidResponse);
        }
    }

    #[tokio::test]
    async fn list_entities_refreshes_exposure_and_filters_normalized_states() {
        let mock = MockHomeAssistant::new(
            exposure(&[("sensor.allowed", true), ("sensor.hidden", false)]),
            json!([
                raw_state("sensor.hidden", "private", "Hidden"),
                raw_state("sensor.allowed", "21", "Kitchen")
            ]),
            json!([]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let query = EntitiesQuery {
            query: Some("kit".to_owned()),
            domains: vec!["sensor".to_owned()],
            limit: 50,
        };

        for _ in 0..2 {
            let result = client.list_entities(&query).await.unwrap();
            assert_eq!(result["action"], "entity.list");
            assert_eq!(result["entities"].as_array().unwrap().len(), 1);
            assert_eq!(result["entities"][0]["entity_id"], "sensor.allowed");
            assert_eq!(result["entities"][0]["friendly_name"], "Kitchen");
            assert!(result["entities"][0].get("latitude").is_none());
            assert_eq!(result["truncated"], false);
        }

        assert_eq!(mock.websocket_calls.load(Ordering::Relaxed), 2);
        assert_eq!(mock.state_calls.load(Ordering::Relaxed), 2);
        let requests = mock.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|entry| entry.as_str() == "http-auth:Bearer test-token")
                .count(),
            2
        );
        assert_eq!(
            requests
                .iter()
                .filter(|entry| entry.as_str() == "ws-auth:test-token")
                .count(),
            2
        );
        assert_eq!(
            requests
                .iter()
                .filter(|entry| entry.as_str() == "ws-command:1:homeassistant/expose_entity/list")
                .count(),
            2
        );
        server.abort();
    }

    #[tokio::test]
    async fn list_devices_groups_selected_states_and_ignores_invalid_registry_only_data() {
        let mock = MockHomeAssistant::new(
            exposure(&[
                ("sensor.alpha", true),
                ("sensor.beta", true),
                ("sensor.standalone", true),
                ("sensor.unregistered", true),
                ("sensor.missing_refs", true),
                ("sensor.hidden", false),
            ]),
            json!([
                raw_state("sensor.hidden", "private", "Hidden State Name"),
                raw_state("sensor.unregistered", "4", "Unregistered"),
                raw_state("sensor.beta", "2", "Beta"),
                raw_state("sensor.alpha", "1", "Alpha"),
                raw_state("sensor.standalone", "3", "Standalone"),
                raw_state("sensor.missing_refs", "5", "Missing References")
            ]),
            json!([]),
        )
        .with_registries(
            json!({
                "sensor.alpha": {
                    "entity_id":"sensor.alpha", "device_id":"device-one", "area_id":"office",
                    "hidden_by":"must-not-leak", "labels":["must-not-leak"]
                },
                "sensor.beta": {
                    "entity_id":"sensor.beta", "device_id":"device-one", "area_id":null
                },
                "sensor.standalone": {
                    "entity_id":"sensor.standalone", "device_id":null, "area_id":"office"
                },
                "sensor.missing_refs": {
                    "entity_id":"sensor.missing_refs", "device_id":"missing-device",
                    "area_id":"missing-area"
                },
                "sensor.unregistered": null
            }),
            json!([
                {
                    "id":"device-one", "area_id":"kitchen", "name":"Original Name",
                    "name_by_user":"Preferred Name", "manufacturer":"must-not-leak",
                    "model":"must-not-leak", "identifiers":["must-not-leak"]
                },
                {
                    "id":"registry-only-device", "area_id":"private-area",
                    "name":["malformed", "Registry Only Secret"],
                    "name_by_user":"x".repeat(257), "manufacturer":"must-not-leak"
                },
                {"id":"registry-only-device","area_id":12,"name":false},
                {"area_id":"private-area","name":"missing unrelated device id"}
            ]),
            json!([
                {"area_id":"kitchen", "name":"Kitchen", "labels":["must-not-leak"]},
                {"area_id":"office", "name":"Office"},
                {"area_id":"private-area", "name":["malformed", "Private Registry Area"]},
                {"area_id":"private-area", "name":"x".repeat(257)},
                {"name":"missing unrelated area id"}
            ]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );

        let result = client
            .list_devices(&DevicesQuery { limit: 100 })
            .await
            .unwrap();
        assert_eq!(result["action"], "device.list");
        assert_eq!(result["devices"].as_array().unwrap().len(), 5);
        assert_eq!(result["truncated"], false);

        let groups = result["devices"].as_array().unwrap();
        let alpha = group_for(groups, "sensor.alpha");
        assert_eq!(alpha["name"], "Preferred Name");
        assert_eq!(alpha["area"], "Office");
        let beta = group_for(groups, "sensor.beta");
        assert_eq!(beta["name"], "Preferred Name");
        assert_eq!(beta["area"], "Kitchen");
        let standalone = group_for(groups, "sensor.standalone");
        assert!(standalone.get("name").is_none());
        assert_eq!(standalone["area"], "Office");
        let unregistered = group_for(groups, "sensor.unregistered");
        assert!(unregistered.get("name").is_none());
        assert!(unregistered.get("area").is_none());
        let missing = group_for(groups, "sensor.missing_refs");
        assert!(missing.get("name").is_none());
        assert!(missing.get("area").is_none());

        let serialized = serde_json::to_string(&result).unwrap();
        for forbidden in [
            "sensor.hidden",
            "private",
            "Hidden State Name",
            "registry-only-device",
            "Registry Only Secret",
            "Private Registry Area",
            "device-one",
            "missing-device",
            "missing-area",
            "office\"",
            "kitchen\"",
            "manufacturer",
            "model",
            "identifiers",
            "labels",
            "hidden_by",
            "latitude",
            "attributes",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }

        let requests = mock.requests.lock().unwrap();
        assert_eq!(
            requests.as_slice(),
            [
                "ws-auth:test-token",
                "ws-command:1:homeassistant/expose_entity/list",
                "http-auth:Bearer test-token",
                "ws-command:2:config/entity_registry/get_entries",
                "ws-command:3:config/device_registry/list",
                "ws-command:4:config/area_registry/list",
            ]
        );
        let commands = mock.commands.lock().unwrap();
        assert_eq!(
            commands[0],
            json!({"id":1,"type":"homeassistant/expose_entity/list"})
        );
        assert_eq!(
            commands[1],
            json!({
                "id":2,
                "type":"config/entity_registry/get_entries",
                "entity_ids":[
                    "sensor.alpha", "sensor.beta", "sensor.missing_refs", "sensor.standalone",
                    "sensor.unregistered"
                ]
            })
        );
        server.abort();
    }

    #[tokio::test]
    async fn list_devices_selects_before_grouping_and_refreshes_exposure() {
        let mock = MockHomeAssistant::new(
            exposure(&[
                ("sensor.zulu", true),
                ("sensor.alpha", true),
                ("sensor.beta", true),
            ]),
            json!([
                raw_state("sensor.zulu", "3", "Zulu"),
                raw_state("sensor.beta", "2", "Beta"),
                raw_state("sensor.alpha", "1", "Alpha")
            ]),
            json!([]),
        )
        .with_registries(
            json!({
                "sensor.alpha":{"entity_id":"sensor.alpha","device_id":"same","area_id":null},
                "sensor.beta":{"entity_id":"sensor.beta","device_id":"same","area_id":null}
            }),
            json!([{"id":"same","area_id":null,"name":"Same","name_by_user":null}]),
            json!([]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );

        for _ in 0..2 {
            let result = client
                .list_devices(&DevicesQuery { limit: 2 })
                .await
                .unwrap();
            assert_eq!(result["truncated"], true);
            assert_eq!(result["devices"].as_array().unwrap().len(), 1);
            assert_eq!(result["devices"][0]["name"], "Same");
            assert_eq!(
                result["devices"][0]["entities"][0]["entity_id"],
                "sensor.alpha"
            );
            assert_eq!(
                result["devices"][0]["entities"][1]["entity_id"],
                "sensor.beta"
            );
        }
        assert_eq!(mock.websocket_calls.load(Ordering::Relaxed), 2);
        assert_eq!(mock.state_calls.load(Ordering::Relaxed), 2);
        server.abort();
    }

    #[tokio::test]
    async fn list_devices_fails_closed_for_malformed_registry_responses() {
        let base = || {
            MockHomeAssistant::new(
                exposure(&[("sensor.allowed", true)]),
                json!([raw_state("sensor.allowed", "1", "Allowed")]),
                json!([]),
            )
            .with_registries(
                json!({"sensor.allowed":{"entity_id":"sensor.allowed","device_id":"device","area_id":null}}),
                json!([{"id":"device","area_id":null,"name":"Device","name_by_user":null}]),
                json!([]),
            )
        };
        let cases = [
            base().with_registries(json!({"sensor.allowed":{}}), json!([]), json!([])),
            base().with_registries(
                json!({
                    "sensor.allowed":{"entity_id":"sensor.allowed","device_id":"device","area_id":null},
                    "sensor.extra":null
                }),
                json!([]),
                json!([]),
            ),
            base().with_registries(
                json!({"sensor.allowed":{"entity_id":"sensor.allowed","device_id":"device","area_id":null}}),
                json!([
                    {"id":"device","area_id":null,"name":"One","name_by_user":null},
                    {"id":"device","area_id":null,"name":"Two","name_by_user":null}
                ]),
                json!([]),
            ),
            base().with_registries(
                json!({"sensor.allowed":{"entity_id":"sensor.allowed","device_id":"device","area_id":"area"}}),
                json!([{"id":"device","area_id":null,"name":"Device","name_by_user":null}]),
                json!([
                    {"area_id":"area","name":"One"},
                    {"area_id":"area","name":"Two"}
                ]),
            ),
            base().with_registries(
                json!({"sensor.allowed":{"entity_id":"sensor.allowed","device_id":"x".repeat(256),"area_id":null}}),
                json!([]),
                json!([]),
            ),
            base().with_registries(
                json!({"sensor.allowed":{"entity_id":"sensor.allowed","device_id":"device","area_id":null}}),
                json!([{"id":"device","area_id":null,"name":"x".repeat(257),"name_by_user":null}]),
                json!([]),
            ),
            base().with_registries(
                json!({"sensor.allowed":{"entity_id":"sensor.allowed","device_id":"device","area_id":null}}),
                json!([{"id":"device","area_id":null,"name":["malformed"],"name_by_user":null}]),
                json!([]),
            ),
            base().with_registries(
                json!({"sensor.allowed":{"entity_id":"sensor.allowed","device_id":"device","area_id":"area"}}),
                json!([{"id":"device","area_id":null,"name":"Device","name_by_user":null}]),
                json!([{"area_id":"area","name":"x".repeat(257)}]),
            ),
            base().with_registries(
                json!({"sensor.allowed":{"entity_id":"sensor.allowed","device_id":"device","area_id":"area"}}),
                json!([{"id":"device","area_id":null,"name":"Device","name_by_user":null}]),
                json!([{"area_id":"area","name":["malformed"]}]),
            ),
            base().with_registry_response(
                "config/entity_registry/get_entries",
                json!({"id":99,"type":"result","success":true,"result":{}}),
            ),
            base().with_registry_response(
                "config/entity_registry/get_entries",
                json!({"id":2,"type":"event","success":true,"result":{}}),
            ),
            base().with_registry_response(
                "config/entity_registry/get_entries",
                json!({"id":2,"type":"result","success":false,"result":{}}),
            ),
            base().with_registry_response(
                "config/entity_registry/get_entries",
                json!({"id":2,"type":"result","success":true}),
            ),
            base().with_registry_response(
                "config/device_registry/list",
                json!({"id":3,"type":"result","success":false,"result":[]}),
            ),
            base().with_registry_response(
                "config/area_registry/list",
                json!({"id":4,"type":"result","success":false,"result":[]}),
            ),
        ];

        for mock in cases {
            let (origin, server) = serve(mock).await;
            let client = HomeAssistantClient::for_test(
                origin,
                Secret("test-token".to_owned()),
                Duration::from_secs(2),
            );
            assert_eq!(
                client.list_devices(&DevicesQuery { limit: 100 }).await,
                Err(Error::InvalidResponse)
            );
            server.abort();
        }
    }

    #[tokio::test]
    async fn list_devices_never_selects_false_or_absent_exposure() {
        let mock = MockHomeAssistant::new(
            json!({
                "id":1,
                "type":"result",
                "success":true,
                "result":{"exposed_entities":{
                    "sensor.false":{"conversation":false},
                    "sensor.malformed":{"conversation":"true"}
                }}
            }),
            json!([
                raw_state("sensor.false", "private", "False"),
                raw_state("sensor.absent", "private", "Absent"),
                raw_state("sensor.malformed", "private", "Malformed")
            ]),
            json!([]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );

        let result = client
            .list_devices(&DevicesQuery { limit: 100 })
            .await
            .unwrap();
        assert!(result["devices"].as_array().unwrap().is_empty());
        assert!(!serde_json::to_string(&result).unwrap().contains("private"));
        assert_eq!(mock.commands.lock().unwrap().len(), 1);
        assert_eq!(mock.state_calls.load(Ordering::Relaxed), 1);
        server.abort();
    }

    #[tokio::test]
    async fn list_devices_rejects_duplicate_selected_states_and_honors_the_operation_timeout() {
        let duplicate = MockHomeAssistant::new(
            exposure(&[("sensor.allowed", true)]),
            json!([
                raw_state("sensor.allowed", "1", "Allowed"),
                raw_state("sensor.allowed", "2", "Allowed Again")
            ]),
            json!([]),
        );
        let (origin, server) = serve(duplicate).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        assert_eq!(
            client.list_devices(&DevicesQuery { limit: 100 }).await,
            Err(Error::InvalidResponse)
        );
        server.abort();

        let delayed = MockHomeAssistant::new(
            exposure(&[("sensor.allowed", true)]),
            json!([raw_state("sensor.allowed", "1", "Allowed")]),
            json!([]),
        )
        .with_states_delay(Duration::from_secs(1));
        let (origin, server) = serve(delayed).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_millis(50),
        );
        assert_eq!(
            client.list_devices(&DevicesQuery { limit: 100 }).await,
            Err(Error::Timeout)
        );
        server.abort();
    }

    #[tokio::test]
    async fn list_devices_uses_non_waiting_capacity_and_releases_its_permit() {
        let client = HomeAssistantClient::for_test(
            Url::parse("http://127.0.0.1:1/").unwrap(),
            Secret("test-token".to_owned()),
            Duration::from_millis(50),
        );
        let mut permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            client.list_devices(&DevicesQuery { limit: 100 }).await,
            Err(Error::CapacityExhausted)
        );

        permits.pop();
        assert_eq!(
            client.list_devices(&DevicesQuery { limit: 100 }).await,
            Err(Error::UpstreamUnavailable)
        );
        assert!(client.admit().is_ok());
    }

    #[test]
    fn output_size_is_bounded() {
        assert_eq!(
            bounded_output(json!({"value":"x".repeat(MAX_OUTPUT_BYTES)})),
            Err(Error::ResponseTooLarge)
        );
    }

    #[tokio::test]
    async fn camera_snapshot_refreshes_exposure_then_uses_the_fixed_authenticated_get() {
        let mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );

        for _ in 0..2 {
            let snapshot = client.camera_snapshot(&camera_query()).await.unwrap();
            assert_eq!(snapshot.entity_id, "camera.front_door");
            assert_eq!(snapshot.mime_type, "image/jpeg");
            assert_eq!(snapshot.data, [0xff, 0xd8, 0xff, 0xd9]);
        }

        assert_eq!(mock.websocket_calls.load(Ordering::Relaxed), 2);
        assert_eq!(mock.camera_calls.load(Ordering::Relaxed), 2);
        assert_eq!(
            mock.requests.lock().unwrap().as_slice(),
            [
                "ws-auth:test-token",
                "ws-command:1:homeassistant/expose_entity/list",
                "http-auth:Bearer test-token",
                "camera:/api/camera_proxy/camera.front_door",
                "camera-accept:image/jpeg, image/png, image/webp",
                "ws-auth:test-token",
                "ws-command:1:homeassistant/expose_entity/list",
                "http-auth:Bearer test-token",
                "camera:/api/camera_proxy/camera.front_door",
                "camera-accept:image/jpeg, image/png, image/webp",
            ]
        );
        server.abort();
    }

    #[tokio::test]
    async fn control_refreshes_exposure_then_uses_only_the_fixed_authenticated_post() {
        let mock =
            MockHomeAssistant::new(exposure(&[("light.kitchen", true)]), json!([]), json!([]));
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let control = LightTurnOnInput {
            entity_id: "light.kitchen".to_owned(),
            brightness_pct: Some(75),
        }
        .validate()
        .unwrap();

        for _ in 0..2 {
            assert_eq!(
                client.execute_control(&control).await.unwrap(),
                json!({
                    "action":"light.turn_on",
                    "entity_id":"light.kitchen",
                    "success":true
                })
            );
        }

        assert_eq!(mock.websocket_calls.load(Ordering::Relaxed), 2);
        assert_eq!(mock.service_calls.load(Ordering::Relaxed), 2);
        assert_eq!(
            mock.requests.lock().unwrap().as_slice(),
            [
                "ws-auth:test-token",
                "ws-command:1:homeassistant/expose_entity/list",
                "http-auth:Bearer test-token",
                "service:/api/services/light/turn_on",
                "service-content-type:application/json",
                "service-body:{\"brightness_pct\":75,\"entity_id\":\"light.kitchen\"}",
                "ws-auth:test-token",
                "ws-command:1:homeassistant/expose_entity/list",
                "http-auth:Bearer test-token",
                "service:/api/services/light/turn_on",
                "service-content-type:application/json",
                "service-body:{\"brightness_pct\":75,\"entity_id\":\"light.kitchen\"}",
            ]
        );
        server.abort();
    }

    #[tokio::test]
    async fn control_denies_unexposed_entities_before_posting() {
        let mock = MockHomeAssistant::new(
            exposure(&[("lock.front_door", false)]),
            json!([]),
            json!([]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let control = EntityControlInput {
            entity_id: "lock.front_door".to_owned(),
        }
        .validate(ControlAction::LockUnlock)
        .unwrap();

        assert_eq!(
            client.execute_control(&control).await,
            Err(Error::NotAllowed)
        );
        assert_eq!(mock.websocket_calls.load(Ordering::Relaxed), 1);
        assert_eq!(mock.service_calls.load(Ordering::Relaxed), 0);
        server.abort();
    }

    #[tokio::test]
    async fn control_fails_closed_on_malformed_exposure_without_posting() {
        let mock = MockHomeAssistant::new(
            json!({
                "id":2,
                "type":"result",
                "success":true,
                "result":{"exposed_entities":{"lock.front_door":{"conversation":true}}}
            }),
            json!([]),
            json!([]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );

        assert_eq!(
            client.execute_control(&lock_control()).await,
            Err(Error::InvalidResponse)
        );
        assert_eq!(mock.websocket_calls.load(Ordering::Relaxed), 1);
        assert_eq!(mock.service_calls.load(Ordering::Relaxed), 0);
        server.abort();
    }

    #[tokio::test]
    async fn control_preserves_status_redirect_timeout_and_capacity_behavior() {
        for (status, expected) in [
            (StatusCode::UNAUTHORIZED, Error::Unauthorized),
            (StatusCode::FORBIDDEN, Error::Unauthorized),
            (StatusCode::NOT_FOUND, Error::NotFound),
            (StatusCode::TOO_MANY_REQUESTS, Error::CapacityExhausted),
            (StatusCode::BAD_REQUEST, Error::RequestRejected),
            (StatusCode::FOUND, Error::UpstreamUnavailable),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Error::UpstreamUnavailable,
            ),
        ] {
            let mock = MockHomeAssistant::new(
                exposure(&[("lock.front_door", true)]),
                json!([]),
                json!([]),
            )
            .with_service_status(status);
            let (origin, server) = serve(mock.clone()).await;
            let client = HomeAssistantClient::for_test(
                origin,
                Secret("test-token".to_owned()),
                Duration::from_secs(2),
            );
            assert_eq!(client.execute_control(&lock_control()).await, Err(expected));
            assert_eq!(mock.service_calls.load(Ordering::Relaxed), 1);
            if status.is_redirection() {
                assert_eq!(mock.state_calls.load(Ordering::Relaxed), 0);
            }
            server.abort();
        }

        let delayed =
            MockHomeAssistant::new(exposure(&[("lock.front_door", true)]), json!([]), json!([]))
                .with_service_delay(Duration::from_secs(1));
        let (origin, server) = serve(delayed).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_millis(50),
        );
        assert_eq!(
            client.execute_control(&lock_control()).await,
            Err(Error::Timeout)
        );
        let permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(permits.len(), MAX_CONCURRENT_QUERIES);
        drop(permits);
        server.abort();

        let client = HomeAssistantClient::for_test(
            Url::parse("http://127.0.0.1:1/").unwrap(),
            Secret("test-token".to_owned()),
            Duration::from_millis(50),
        );
        let mut permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            client.execute_control(&lock_control()).await,
            Err(Error::CapacityExhausted)
        );
        permits.pop();
        assert_eq!(
            client.execute_control(&lock_control()).await,
            Err(Error::UpstreamUnavailable)
        );
        assert!(client.admit().is_ok());
    }

    #[tokio::test]
    async fn cancelling_control_releases_its_permit() {
        let mock =
            MockHomeAssistant::new(exposure(&[("lock.front_door", true)]), json!([]), json!([]))
                .with_service_delay(Duration::from_secs(5));
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(10),
        );
        let running_client = client.clone();
        let running =
            tokio::spawn(async move { running_client.execute_control(&lock_control()).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while mock.service_calls.load(Ordering::Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        running.abort();
        assert!(matches!(running.await, Err(error) if error.is_cancelled()));

        let permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(permits.len(), MAX_CONCURRENT_QUERIES);
        server.abort();
    }

    #[tokio::test]
    async fn control_discards_success_bodies_but_still_enforces_the_response_limit() {
        let mut mock =
            MockHomeAssistant::new(exposure(&[("switch.office", true)]), json!([]), json!([]));
        mock.service_body = vec![b'x'; MAX_RESPONSE_BYTES + 1];
        let (origin, server) = serve(mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let control = EntityControlInput {
            entity_id: "switch.office".to_owned(),
        }
        .validate(ControlAction::SwitchTurnOn)
        .unwrap();

        assert_eq!(
            client.execute_control(&control).await,
            Err(Error::ResponseTooLarge)
        );
        server.abort();
    }

    #[tokio::test]
    async fn camera_snapshot_denies_unexposed_entities_before_the_image_get() {
        let mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", false)]),
            json!([]),
            json!([]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );

        assert_eq!(
            client.camera_snapshot(&camera_query()).await.err(),
            Some(Error::NotAllowed)
        );
        assert_eq!(mock.websocket_calls.load(Ordering::Relaxed), 1);
        assert_eq!(mock.camera_calls.load(Ordering::Relaxed), 0);
        server.abort();
    }

    #[tokio::test]
    async fn camera_snapshot_accepts_only_exact_matching_image_types() {
        let accepted = [
            ("image/jpeg", vec![0xff, 0xd8, 0xff, 0xd9]),
            ("image/png", b"\x89PNG\r\n\x1a\nrest".to_vec()),
            ("image/webp", b"RIFF\x04\0\0\0WEBPdata".to_vec()),
        ];
        for (mime_type, body) in accepted {
            let mock = MockHomeAssistant::new(
                exposure(&[("camera.front_door", true)]),
                json!([]),
                json!([]),
            )
            .with_camera(Some(mime_type), body.clone());
            let (origin, server) = serve(mock).await;
            let client = HomeAssistantClient::for_test(
                origin,
                Secret("test-token".to_owned()),
                Duration::from_secs(2),
            );
            let snapshot = client.camera_snapshot(&camera_query()).await.unwrap();
            assert_eq!(snapshot.mime_type, mime_type);
            assert_eq!(snapshot.data, body);
            server.abort();
        }

        let rejected = [
            (None, vec![0xff, 0xd8, 0xff]),
            (Some("image/jpeg; charset=binary"), vec![0xff, 0xd8, 0xff]),
            (Some("image/svg+xml"), b"<svg/>".to_vec()),
            (Some("text/html"), b"<html>no</html>".to_vec()),
            (Some("application/json"), b"{}".to_vec()),
            (Some("image/jpeg"), b"not an image".to_vec()),
            (Some("image/png"), vec![0xff, 0xd8, 0xff]),
            (Some("image/webp"), b"RIFFbad-data".to_vec()),
        ];
        for (content_type, body) in rejected {
            let mock = MockHomeAssistant::new(
                exposure(&[("camera.front_door", true)]),
                json!([]),
                json!([]),
            )
            .with_camera(content_type, body);
            let (origin, server) = serve(mock).await;
            let client = HomeAssistantClient::for_test(
                origin,
                Secret("test-token".to_owned()),
                Duration::from_secs(2),
            );
            assert_eq!(
                client.camera_snapshot(&camera_query()).await.err(),
                Some(Error::InvalidResponse),
                "accepted {content_type:?}"
            );
            server.abort();
        }
    }

    #[tokio::test]
    async fn camera_snapshot_enforces_declared_and_streamed_image_limits() {
        let mut exact = vec![0; MAX_RESPONSE_BYTES];
        exact[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        let exact_mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        )
        .with_camera(Some("image/png"), exact)
        .with_streamed_camera();
        let (origin, server) = serve(exact_mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        assert_eq!(
            client
                .camera_snapshot(&camera_query())
                .await
                .unwrap()
                .data
                .len(),
            MAX_RESPONSE_BYTES
        );
        server.abort();

        let declared_mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        )
        .with_camera(Some("image/jpeg"), {
            let mut body = vec![0; MAX_RESPONSE_BYTES + 1];
            body[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
            body
        })
        .with_camera_declared_length(MAX_RESPONSE_BYTES as u64 + 1);
        let (origin, server) = serve(declared_mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        assert_eq!(
            client.camera_snapshot(&camera_query()).await.err(),
            Some(Error::ResponseTooLarge)
        );
        server.abort();

        let mut overflow = vec![0; MAX_RESPONSE_BYTES + 1];
        overflow[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
        let streamed_mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        )
        .with_camera(Some("image/jpeg"), overflow)
        .with_streamed_camera();
        let (origin, server) = serve(streamed_mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        assert_eq!(
            client.camera_snapshot(&camera_query()).await.err(),
            Some(Error::ResponseTooLarge)
        );
        server.abort();
    }

    #[tokio::test]
    async fn camera_snapshot_preserves_status_timeout_and_capacity_behavior() {
        for (status, expected) in [
            (StatusCode::UNAUTHORIZED, Error::Unauthorized),
            (StatusCode::FORBIDDEN, Error::Unauthorized),
            (StatusCode::NOT_FOUND, Error::NotFound),
            (StatusCode::TOO_MANY_REQUESTS, Error::CapacityExhausted),
            (StatusCode::BAD_REQUEST, Error::RequestRejected),
            (StatusCode::FOUND, Error::UpstreamUnavailable),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Error::UpstreamUnavailable,
            ),
        ] {
            let mock = MockHomeAssistant::new(
                exposure(&[("camera.front_door", true)]),
                json!([]),
                json!([]),
            )
            .with_camera_status(status);
            let (origin, server) = serve(mock).await;
            let client = HomeAssistantClient::for_test(
                origin,
                Secret("test-token".to_owned()),
                Duration::from_secs(2),
            );
            assert_eq!(
                client.camera_snapshot(&camera_query()).await.err(),
                Some(expected)
            );
            server.abort();
        }

        let delayed = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        )
        .with_camera_delay(Duration::from_secs(1));
        let (origin, server) = serve(delayed).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_millis(50),
        );
        assert_eq!(
            client.camera_snapshot(&camera_query()).await.err(),
            Some(Error::Timeout)
        );
        assert!(client.admit().is_ok());
        server.abort();

        let client = HomeAssistantClient::for_test(
            Url::parse("http://127.0.0.1:1/").unwrap(),
            Secret("test-token".to_owned()),
            Duration::from_millis(50),
        );
        let mut permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            client.camera_snapshot(&camera_query()).await.err(),
            Some(Error::CapacityExhausted)
        );
        permits.pop();
        assert_eq!(
            client.camera_snapshot(&camera_query()).await.err(),
            Some(Error::UpstreamUnavailable)
        );
        assert!(client.admit().is_ok());
    }

    #[tokio::test]
    async fn cancelling_camera_snapshot_releases_its_permit() {
        let mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        )
        .with_camera_delay(Duration::from_secs(5));
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(10),
        );
        let running_client = client.clone();
        let running =
            tokio::spawn(async move { running_client.camera_snapshot(&camera_query()).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while mock.camera_calls.load(Ordering::Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        running.abort();
        assert!(matches!(running.await, Err(error) if error.is_cancelled()));

        let permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(permits.len(), MAX_CONCURRENT_QUERIES);
        server.abort();
    }

    #[tokio::test]
    async fn camera_snapshot_completion_holds_its_permit_until_success() {
        let mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        );
        let (origin, server) = serve(mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let running_client = client.clone();
        let running_started = Arc::clone(&started);
        let running_release = Arc::clone(&release);
        let running = tokio::spawn(async move {
            running_client
                .camera_snapshot_with(&camera_query(), |snapshot| async move {
                    running_started.notify_one();
                    running_release.notified().await;
                    Ok(snapshot.data.len())
                })
                .await
        });
        started.notified().await;

        let permits = (0..MAX_CONCURRENT_QUERIES - 1)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(client.admit(), Err(Error::CapacityExhausted)));
        release.notify_one();
        assert_eq!(running.await.unwrap(), Ok(4));
        drop(permits);
        let permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(permits.len(), MAX_CONCURRENT_QUERIES);
        server.abort();
    }

    #[tokio::test]
    async fn camera_snapshot_completion_timeout_drops_work_and_releases_its_permit() {
        let mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        );
        let (origin, server) = serve(mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_millis(200),
        );
        let started = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let completion_started = Arc::clone(&started);
        let completion_dropped = Arc::clone(&dropped);

        let result = client
            .camera_snapshot_with(&camera_query(), move |_| async move {
                let _guard = CompletionGuard(completion_dropped);
                completion_started.notify_one();
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(())
            })
            .await;
        assert_eq!(result, Err(Error::Timeout));
        assert!(dropped.load(Ordering::Relaxed));
        let permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(permits.len(), MAX_CONCURRENT_QUERIES);
        server.abort();
    }

    #[tokio::test]
    async fn camera_snapshot_synchronous_completion_cannot_emit_late_success() {
        let mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        );
        let (origin, server) = serve(mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_millis(50),
        );

        let result = client
            .camera_snapshot_with(&camera_query(), |_| async move {
                std::thread::sleep(Duration::from_millis(100));
                Ok(())
            })
            .await;
        assert_eq!(result, Err(Error::Timeout));
        let permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(permits.len(), MAX_CONCURRENT_QUERIES);
        server.abort();
    }

    #[tokio::test]
    async fn cancelling_camera_snapshot_during_completion_drops_work_and_releases_its_permit() {
        let mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        );
        let (origin, server) = serve(mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(10),
        );
        let started = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let running_client = client.clone();
        let completion_started = Arc::clone(&started);
        let completion_dropped = Arc::clone(&dropped);
        let running = tokio::spawn(async move {
            running_client
                .camera_snapshot_with(&camera_query(), move |_| async move {
                    let _guard = CompletionGuard(completion_dropped);
                    completion_started.notify_one();
                    std::future::pending::<Result<(), Error>>().await
                })
                .await
        });
        started.notified().await;

        let permits = (0..MAX_CONCURRENT_QUERIES - 1)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(client.admit(), Err(Error::CapacityExhausted)));
        running.abort();
        assert!(matches!(running.await, Err(error) if error.is_cancelled()));
        assert!(dropped.load(Ordering::Relaxed));
        drop(permits);
        let permits = (0..MAX_CONCURRENT_QUERIES)
            .map(|_| client.admit().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(permits.len(), MAX_CONCURRENT_QUERIES);
        server.abort();
    }

    #[tokio::test]
    async fn rest_request_span_is_admitted_propagated_and_privacy_bounded() {
        const CHILD_ENV: &str = "SMARTHOME_MCP_HTTP_SPAN_TEST_CHILD";
        const TEST_NAME: &str = "integrations::home_assistant::client::tests::rest_request_span_is_admitted_propagated_and_privacy_bounded";
        if std::env::var_os(CHILD_ENV).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(CHILD_ENV, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "child test failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        global::set_text_map_propagator(TraceContextPropagator::new());
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = provider.tracer("smarthome-mcp-test");
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .with(crate::observability::trace_filter());
        let dispatch = tracing::Dispatch::new(subscriber);

        let base_mock = || {
            MockHomeAssistant::new(
                exposure(&[("sensor.allowed", true)]),
                json!([raw_state("sensor.allowed", "21", "Kitchen")]),
                json!([]),
            )
        };
        let (success_result, success_traceparent) =
            traced_list(base_mock(), Duration::from_secs(2), &dispatch).await;
        assert!(success_result.is_ok());
        let (status_error_result, status_error_traceparent) = traced_list(
            base_mock().with_states_status(StatusCode::NOT_FOUND),
            Duration::from_secs(2),
            &dispatch,
        )
        .await;
        assert_eq!(status_error_result, Err(Error::NotFound));
        let (redirect_result, redirect_traceparent) = traced_list(
            base_mock().with_states_status(StatusCode::FOUND),
            Duration::from_secs(2),
            &dispatch,
        )
        .await;
        assert_eq!(redirect_result, Err(Error::UpstreamUnavailable));
        let (cancelled_result, cancelled_traceparent) = traced_list(
            base_mock().with_states_delay(Duration::from_secs(5)),
            Duration::from_millis(100),
            &dispatch,
        )
        .await;
        assert_eq!(cancelled_result, Err(Error::Timeout));
        let snapshot_mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        );
        let (snapshot_result, snapshot_traceparent) =
            traced_snapshot(snapshot_mock, Duration::from_secs(2), &dispatch).await;
        assert!(snapshot_result.is_ok());
        let invalid_snapshot_mock = MockHomeAssistant::new(
            exposure(&[("camera.front_door", true)]),
            json!([]),
            json!([]),
        )
        .with_camera(Some("application/json"), b"sensitive-camera-body".to_vec());
        let (invalid_snapshot_result, invalid_snapshot_traceparent) =
            traced_snapshot(invalid_snapshot_mock, Duration::from_secs(2), &dispatch).await;
        assert!(matches!(
            invalid_snapshot_result,
            Err(Error::InvalidResponse)
        ));
        let control_mock = MockHomeAssistant::new(
            exposure(&[("light.private_fixture", true)]),
            json!([]),
            json!([]),
        );
        let (control_result, control_traceparent) =
            traced_control(control_mock, Duration::from_secs(2), &dispatch).await;
        assert!(control_result.is_ok());

        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        let query_spans = spans
            .iter()
            .filter(|span| span.name == "home_assistant.query")
            .collect::<Vec<_>>();
        let exec_spans = spans
            .iter()
            .filter(|span| span.name == "home_assistant.exec")
            .collect::<Vec<_>>();
        let client_spans = spans
            .iter()
            .filter(|span| span.name == "http.client.request")
            .collect::<Vec<_>>();
        assert_eq!(query_spans.len(), 6);
        assert_eq!(exec_spans.len(), 1);
        assert_eq!(client_spans.len(), 7);

        for traceparent in [
            success_traceparent,
            status_error_traceparent,
            redirect_traceparent,
            cancelled_traceparent,
            snapshot_traceparent,
            invalid_snapshot_traceparent,
            control_traceparent,
        ] {
            assert_traceparent_matches_client_span(&traceparent, &client_spans);
        }

        for client_span in &client_spans {
            assert_eq!(client_span.span_kind, SpanKind::Client);
            assert!(query_spans.iter().chain(&exec_spans).any(|operation_span| {
                client_span.parent_span_id == operation_span.span_context.span_id()
                    && client_span.span_context.trace_id() == operation_span.span_context.trace_id()
            }));
            assert!(span_attribute(client_span, "outcome").is_some_and(|value| !value.is_empty()));
            assert_span_is_privacy_bounded(client_span);
        }
        assert_eq!(
            client_spans
                .iter()
                .filter(
                    |span| span_attribute(span, "http.request.method").as_deref() == Some("GET")
                )
                .count(),
            6
        );
        let post_span = client_spans
            .iter()
            .copied()
            .find(|span| span_attribute(span, "http.request.method").as_deref() == Some("POST"))
            .unwrap();
        assert_eq!(
            span_attribute(post_span, "outcome").as_deref(),
            Some("success")
        );
        let exec_span = exec_spans[0];
        assert_eq!(
            span_attribute(exec_span, "action").as_deref(),
            Some("light.turn_on")
        );
        assert_eq!(
            span_attribute(exec_span, "outcome").as_deref(),
            Some("success")
        );
        assert_span_is_privacy_bounded(exec_span);

        let success_span = client_span(&client_spans, "200", "success");
        assert_eq!(success_span.status, Status::Unset);
        let status_error_span = client_span(&client_spans, "404", "http_error");
        assert!(matches!(status_error_span.status, Status::Error { .. }));
        let redirect_span = client_span(&client_spans, "302", "http_error");
        assert_eq!(redirect_span.status, Status::Unset);
        let cancelled_span = client_span(&client_spans, "200", "cancelled");
        assert!(matches!(cancelled_span.status, Status::Error { .. }));

        provider.shutdown().unwrap();
    }

    async fn traced_list(
        mock: MockHomeAssistant,
        timeout: Duration,
        dispatch: &tracing::Dispatch,
    ) -> (Result<Value, Error>, String) {
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("sensitive-test-token".to_owned()),
            timeout,
        );
        let result = client
            .list_entities(&EntitiesQuery {
                query: None,
                domains: Vec::new(),
                limit: 50,
            })
            .with_subscriber(dispatch.clone())
            .await;
        let traceparents = mock.traceparents.lock().unwrap().clone();
        server.abort();
        assert_eq!(traceparents.len(), 1);
        (result, traceparents.into_iter().next().unwrap())
    }

    async fn traced_snapshot(
        mock: MockHomeAssistant,
        timeout: Duration,
        dispatch: &tracing::Dispatch,
    ) -> (Result<CameraSnapshot, Error>, String) {
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("sensitive-camera-token".to_owned()),
            timeout,
        );
        let result = client
            .camera_snapshot(&camera_query())
            .with_subscriber(dispatch.clone())
            .await;
        let traceparents = mock.traceparents.lock().unwrap().clone();
        server.abort();
        assert_eq!(traceparents.len(), 1);
        (result, traceparents.into_iter().next().unwrap())
    }

    async fn traced_control(
        mock: MockHomeAssistant,
        timeout: Duration,
        dispatch: &tracing::Dispatch,
    ) -> (Result<Value, Error>, String) {
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("sensitive-control-token".to_owned()),
            timeout,
        );
        let control = LightTurnOnInput {
            entity_id: "light.private_fixture".to_owned(),
            brightness_pct: Some(75),
        }
        .validate()
        .unwrap();
        let result = client
            .execute_control(&control)
            .with_subscriber(dispatch.clone())
            .await;
        let traceparents = mock.traceparents.lock().unwrap().clone();
        server.abort();
        assert_eq!(traceparents.len(), 1);
        (result, traceparents.into_iter().next().unwrap())
    }

    fn assert_traceparent_matches_client_span(traceparent: &str, client_spans: &[&SpanData]) {
        let parts = traceparent.split('-').collect::<Vec<_>>();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "00");
        assert_eq!(parts[1].len(), 32);
        assert_eq!(parts[2].len(), 16);
        assert_eq!(parts[3].len(), 2);
        assert!(parts[1].bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(parts[2].bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(parts[1], "00000000000000000000000000000000");
        assert_ne!(parts[2], "0000000000000000");
        assert!(client_spans.iter().any(|span| {
            parts[1] == span.span_context.trace_id().to_string()
                && parts[2] == span.span_context.span_id().to_string()
        }));
    }

    fn client_span<'a>(spans: &'a [&SpanData], status: &str, outcome: &str) -> &'a SpanData {
        spans
            .iter()
            .copied()
            .find(|span| {
                span_attribute(span, "http.response.status_code").as_deref() == Some(status)
                    && span_attribute(span, "outcome").as_deref() == Some(outcome)
            })
            .unwrap()
    }

    fn span_attribute(span: &SpanData, key: &str) -> Option<String> {
        span.attributes
            .iter()
            .find(|attribute| attribute.key.as_str() == key)
            .map(|attribute| attribute.value.to_string())
    }

    fn assert_span_is_privacy_bounded(span: &SpanData) {
        for attribute in &span.attributes {
            assert!(!matches!(
                attribute.key.as_str(),
                "server.address"
                    | "server.port"
                    | "url.scheme"
                    | "url.path"
                    | "url.query"
                    | "url.full"
                    | "http.request.header.authorization"
                    | "http.request.body.size"
                    | "user_agent.original"
                    | "error.type"
                    | "error.message"
            ));
        }
        let exported = format!("{span:?}").to_ascii_lowercase();
        for sensitive in [
            "127.0.0.1",
            "/api/states",
            "sensor.allowed",
            "sensitive-test-token",
            "sensitive-camera-token",
            "/api/camera_proxy",
            "camera.front_door",
            "image/jpeg",
            "application/json",
            "sensitive-camera-body",
            "sensitive-control-token",
            "/api/services",
            "light.private_fixture",
            "brightness_pct",
            "must_not_leak",
            "authorization",
            "url.",
            "server.",
            "error.",
        ] {
            assert!(!exported.contains(sensitive));
        }
    }

    #[tokio::test]
    async fn get_states_denies_unexposed_entities_before_rest_reads() {
        let mock =
            MockHomeAssistant::new(exposure(&[("sensor.allowed", true)]), json!([]), json!([]));
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let query = StatesQuery {
            entity_ids: vec!["sensor.hidden".to_owned()],
        };

        assert_eq!(client.get_states(&query).await, Err(Error::NotAllowed));
        assert_eq!(mock.websocket_calls.load(Ordering::Relaxed), 1);
        assert_eq!(mock.state_calls.load(Ordering::Relaxed), 0);
        server.abort();
    }

    #[tokio::test]
    async fn get_states_rejects_a_mismatched_response_entity() {
        let mock =
            MockHomeAssistant::new(exposure(&[("sensor.allowed", true)]), json!([]), json!([]))
                .with_state_override(raw_state("sensor.hidden", "private", "Hidden"));
        let (origin, server) = serve(mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let query = StatesQuery {
            entity_ids: vec!["sensor.allowed".to_owned()],
        };

        assert_eq!(client.get_states(&query).await, Err(Error::InvalidResponse));
        server.abort();
    }

    #[tokio::test]
    async fn get_states_returns_the_public_action_name() {
        let mock = MockHomeAssistant::new(
            exposure(&[("sensor.allowed", true)]),
            json!([raw_state("sensor.allowed", "21", "Kitchen")]),
            json!([]),
        );
        let (origin, server) = serve(mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let result = client
            .get_states(&StatesQuery {
                entity_ids: vec!["sensor.allowed".to_owned()],
            })
            .await
            .unwrap();

        assert_eq!(result["action"], "state.get");
        server.abort();
    }

    #[tokio::test]
    async fn malformed_exposure_response_fails_closed() {
        let mock = MockHomeAssistant::new(
            json!({"id":2,"type":"result","success":true,"result":{"exposed_entities":{}}}),
            json!([]),
            json!([]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let query = EntitiesQuery {
            query: None,
            domains: Vec::new(),
            limit: 50,
        };

        assert_eq!(
            client.list_entities(&query).await,
            Err(Error::InvalidResponse)
        );
        assert_eq!(mock.state_calls.load(Ordering::Relaxed), 0);
        server.abort();
    }

    #[tokio::test]
    async fn history_uses_fixed_minimal_query_and_prunes_response() {
        let mock = MockHomeAssistant::new(
            exposure(&[("sensor.allowed", true)]),
            json!([]),
            json!([[{
                "entity_id":"sensor.allowed",
                "state":"20",
                "last_changed":"2026-08-09T00:30:00Z",
                "attributes":{"latitude":"must-not-leak"}
            }]]),
        );
        let (origin, server) = serve(mock.clone()).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let query = HistoryQuery {
            entity_ids: vec!["sensor.allowed".to_owned()],
            start: timestamp("2026-08-09T00:00:00Z"),
            end: timestamp("2026-08-09T01:00:00Z"),
        };

        let result = client.get_history(&query).await.unwrap();
        assert_eq!(result["action"], "history.get");
        assert_eq!(result["history"][0]["entity_id"], "sensor.allowed");
        assert_eq!(result["history"][0]["states"][0]["state"], "20");
        assert!(
            result["history"][0]["states"][0]
                .get("attributes")
                .is_none()
        );
        let requests = mock.requests.lock().unwrap();
        let history_request = requests
            .iter()
            .find(|entry| entry.starts_with("history:"))
            .unwrap();
        assert!(history_request.contains("filter_entity_id=sensor.allowed"));
        assert!(history_request.contains("end_time=2026-08-09T01%3A00%3A00%2B00%3A00"));
        assert!(history_request.contains("minimal_response="));
        assert!(history_request.contains("no_attributes="));
        assert!(history_request.contains("significant_changes_only="));
        server.abort();
    }

    #[tokio::test]
    async fn history_does_not_report_truncation_at_the_exact_limit() {
        let points = (0..MAX_HISTORY_POINTS)
            .map(|_| {
                json!({
                    "entity_id":"sensor.allowed",
                    "state":"20",
                    "last_changed":"2026-08-09T00:30:00Z"
                })
            })
            .collect::<Vec<_>>();
        let mock = MockHomeAssistant::new(
            exposure(&[("sensor.allowed", true)]),
            json!([]),
            json!([points]),
        );
        let (origin, server) = serve(mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let query = HistoryQuery {
            entity_ids: vec!["sensor.allowed".to_owned()],
            start: timestamp("2026-08-09T00:00:00Z"),
            end: timestamp("2026-08-09T01:00:00Z"),
        };

        let result = client.get_history(&query).await.unwrap();
        assert_eq!(
            result["history"][0]["states"].as_array().unwrap().len(),
            2_000
        );
        assert_eq!(result["truncated"], false);
        server.abort();
    }

    #[tokio::test]
    async fn history_validates_groups_after_the_output_limit() {
        let points = (0..MAX_HISTORY_POINTS)
            .map(|_| {
                json!({
                    "entity_id":"sensor.allowed",
                    "state":"20",
                    "last_changed":"2026-08-09T00:30:00Z"
                })
            })
            .collect::<Vec<_>>();
        let mock = MockHomeAssistant::new(
            exposure(&[("sensor.allowed", true)]),
            json!([]),
            json!([
                points,
                [{
                    "entity_id":"sensor.unrequested",
                    "state":"private",
                    "last_changed":"2026-08-09T00:30:00Z"
                }]
            ]),
        );
        let (origin, server) = serve(mock).await;
        let client = HomeAssistantClient::for_test(
            origin,
            Secret("test-token".to_owned()),
            Duration::from_secs(2),
        );
        let query = HistoryQuery {
            entity_ids: vec!["sensor.allowed".to_owned()],
            start: timestamp("2026-08-09T00:00:00Z"),
            end: timestamp("2026-08-09T01:00:00Z"),
        };

        assert_eq!(
            client.get_history(&query).await,
            Err(Error::InvalidResponse)
        );
        server.abort();
    }

    fn raw_state(entity_id: &str, state: &str, friendly_name: &str) -> Value {
        json!({
            "entity_id": entity_id,
            "state": state,
            "attributes": {
                "friendly_name": friendly_name,
                "device_class": "temperature",
                "unit_of_measurement": "C",
                "latitude": "must-not-leak"
            },
            "last_changed": "2026-08-09T00:00:00Z",
            "last_updated": "2026-08-09T00:00:00Z"
        })
    }

    fn group_for<'a>(groups: &'a [Value], entity_id: &str) -> &'a Value {
        groups
            .iter()
            .find(|group| {
                group["entities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entity| entity["entity_id"] == entity_id)
            })
            .unwrap()
    }

    fn exposure(entries: &[(&str, bool)]) -> Value {
        let entities = entries
            .iter()
            .map(|(entity_id, allowed)| ((*entity_id).to_owned(), json!({"conversation": allowed})))
            .collect::<serde_json::Map<_, _>>();
        json!({
            "id": 1,
            "type": "result",
            "success": true,
            "result": {"exposed_entities": entities}
        })
    }

    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn camera_query() -> CameraSnapshotQuery {
        CameraSnapshotQuery {
            entity_id: "camera.front_door".to_owned(),
        }
    }

    fn lock_control() -> Control {
        EntityControlInput {
            entity_id: "lock.front_door".to_owned(),
        }
        .validate(ControlAction::LockUnlock)
        .unwrap()
    }

    async fn serve(mock: MockHomeAssistant) -> (Url, JoinHandle<()>) {
        let app = Router::new()
            .route("/api/websocket", get(websocket))
            .route("/api/states", get(all_states))
            .route("/api/states/{entity_id}", get(one_state))
            .route("/api/camera_proxy/{entity_id}", get(camera_snapshot))
            .route("/api/history/period/{start}", get(history))
            .route("/api/services/{domain}/{service}", post(service_call))
            .with_state(mock);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (Url::parse(&format!("http://{address}")).unwrap(), server)
    }

    async fn websocket(ws: WebSocketUpgrade, State(mock): State<MockHomeAssistant>) -> Response {
        ws.on_upgrade(move |socket| websocket_session(socket, mock))
    }

    async fn websocket_session(mut socket: ws::WebSocket, mock: MockHomeAssistant) {
        mock.websocket_calls.fetch_add(1, Ordering::Relaxed);
        socket
            .send(ws::Message::Text(
                json!({"type":"auth_required"}).to_string().into(),
            ))
            .await
            .unwrap();
        let auth = receive_json(&mut socket).await;
        mock.requests.lock().unwrap().push(format!(
            "ws-auth:{}",
            auth.get("access_token")
                .and_then(Value::as_str)
                .unwrap_or("")
        ));
        socket
            .send(ws::Message::Text(
                json!({"type":"auth_ok"}).to_string().into(),
            ))
            .await
            .unwrap();
        while let Some(Ok(message)) = socket.recv().await {
            let ws::Message::Text(text) = message else {
                break;
            };
            let command: Value = serde_json::from_str(text.as_ref()).unwrap();
            mock.commands.lock().unwrap().push(command.clone());
            let id = command
                .get("id")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let command_type = command.get("type").and_then(Value::as_str).unwrap_or("");
            mock.requests
                .lock()
                .unwrap()
                .push(format!("ws-command:{id}:{command_type}"));
            let response = match mock.registry_response_overrides.get(command_type) {
                Some(response) => response.clone(),
                None => match command_type {
                    "homeassistant/expose_entity/list" => mock.exposure.clone(),
                    "config/entity_registry/get_entries" => {
                        ws_result(id, mock.entity_registry.clone())
                    }
                    "config/device_registry/list" => ws_result(id, mock.device_registry.clone()),
                    "config/area_registry/list" => ws_result(id, mock.area_registry.clone()),
                    _ => ws_result(id, Value::Null),
                },
            };
            if socket
                .send(ws::Message::Text(response.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    }

    async fn receive_json(socket: &mut ws::WebSocket) -> Value {
        let message = socket.recv().await.unwrap().unwrap();
        match message {
            ws::Message::Text(text) => serde_json::from_str(text.as_ref()).unwrap(),
            _ => panic!("expected text WebSocket message"),
        }
    }

    fn ws_result(id: u64, result: Value) -> Value {
        json!({"id":id,"type":"result","success":true,"result":result})
    }

    async fn all_states(State(mock): State<MockHomeAssistant>, headers: HeaderMap) -> Response {
        record_http_auth(&mock, &headers);
        mock.state_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(delay) = mock.states_delay {
            let body = serde_json::to_vec(&mock.states).unwrap();
            let stream = futures_util::stream::once(async move {
                tokio::time::sleep(delay).await;
                Ok::<_, std::io::Error>(Bytes::from(body))
            });
            return Response::builder()
                .status(mock.states_status)
                .body(Body::from_stream(stream))
                .unwrap();
        }
        (mock.states_status, Json(mock.states)).into_response()
    }

    async fn one_state(
        State(mock): State<MockHomeAssistant>,
        Path(entity_id): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        record_http_auth(&mock, &headers);
        mock.state_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(state) = &mock.state_override {
            return Json(state.clone()).into_response();
        }
        let state = mock
            .states
            .as_array()
            .and_then(|states| states.iter().find(|state| state["entity_id"] == entity_id))
            .cloned();
        match state {
            Some(state) => Json(state).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    async fn history(
        State(mock): State<MockHomeAssistant>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
    ) -> Response {
        record_http_auth(&mock, &headers);
        mock.state_calls.fetch_add(1, Ordering::Relaxed);
        mock.requests.lock().unwrap().push(format!("history:{uri}"));
        Json(mock.history).into_response()
    }

    async fn camera_snapshot(
        State(mock): State<MockHomeAssistant>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
    ) -> Response {
        record_http_auth(&mock, &headers);
        mock.camera_calls.fetch_add(1, Ordering::Relaxed);
        let accept = headers
            .get(reqwest::header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let mut requests = mock.requests.lock().unwrap();
        requests.push(format!("camera:{uri}"));
        requests.push(format!("camera-accept:{accept}"));
        drop(requests);

        let mut response = Response::builder().status(mock.camera_status);
        if let Some(content_type) = &mock.camera_content_type {
            response = response.header(reqwest::header::CONTENT_TYPE, content_type);
        }
        if let Some(length) = mock.camera_declared_length {
            response = response.header(reqwest::header::CONTENT_LENGTH, length);
        }
        let body = if let Some(delay) = mock.camera_delay {
            let body = mock.camera_body.clone();
            Body::from_stream(futures_util::stream::once(async move {
                tokio::time::sleep(delay).await;
                Ok::<_, std::io::Error>(Bytes::from(body))
            }))
        } else if mock.camera_streamed {
            let middle = mock.camera_body.len() / 2;
            let chunks = vec![
                Ok::<_, std::io::Error>(Bytes::copy_from_slice(&mock.camera_body[..middle])),
                Ok(Bytes::copy_from_slice(&mock.camera_body[middle..])),
            ];
            Body::from_stream(futures_util::stream::iter(chunks))
        } else {
            Body::from(mock.camera_body)
        };
        response.body(body).unwrap()
    }

    async fn service_call(
        State(mock): State<MockHomeAssistant>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        record_http_auth(&mock, &headers);
        mock.service_calls.fetch_add(1, Ordering::Relaxed);
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let mut requests = mock.requests.lock().unwrap();
        requests.push(format!("service:{uri}"));
        requests.push(format!("service-content-type:{content_type}"));
        requests.push(format!("service-body:{}", String::from_utf8_lossy(&body)));
        drop(requests);
        let mut response = Response::builder().status(mock.service_status);
        if mock.service_status.is_redirection() {
            response = response.header(reqwest::header::LOCATION, "/api/states");
        }
        let body = if let Some(delay) = mock.service_delay {
            let body = mock.service_body.clone();
            Body::from_stream(futures_util::stream::once(async move {
                tokio::time::sleep(delay).await;
                Ok::<_, std::io::Error>(Bytes::from(body))
            }))
        } else {
            Body::from(mock.service_body)
        };
        response.body(body).unwrap()
    }

    fn record_http_auth(mock: &MockHomeAssistant, headers: &HeaderMap) {
        let authorization = headers
            .get(reqwest::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        mock.requests
            .lock()
            .unwrap()
            .push(format!("http-auth:{authorization}"));
        if let Some(traceparent) = headers
            .get("traceparent")
            .and_then(|value| value.to_str().ok())
        {
            mock.traceparents
                .lock()
                .unwrap()
                .push(traceparent.to_owned());
        }
    }
}
