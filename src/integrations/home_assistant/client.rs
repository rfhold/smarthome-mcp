use std::{collections::HashSet, sync::Arc, time::Duration};

use futures_util::{SinkExt as _, StreamExt as _};
use reqwest::{Client, Response, StatusCode, redirect::Policy};
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_tungstenite::{
    WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use tracing::Instrument as _;
use url::Url;

use crate::{config::Secret, http_client};

use super::{
    Error,
    actions::{EntitiesQuery, HistoryQuery, StatesQuery, valid_entity_id},
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

    async fn exposed_entities(&self) -> Result<HashSet<String>, Error> {
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
        socket
            .send(Message::Text(
                json!({"id":1,"type":"homeassistant/expose_entity/list"})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|_| Error::UpstreamUnavailable)?;
        let response = websocket_json(&mut socket).await?;
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
        let _ = socket.close(None).await;
        Ok(exposed)
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
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{
        Json, Router,
        body::{Body, Bytes},
        extract::{OriginalUri, Path, State, WebSocketUpgrade, ws},
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::get,
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
    use tokio::{net::TcpListener, task::JoinHandle};
    use tracing::instrument::WithSubscriber as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;

    #[derive(Clone)]
    struct MockHomeAssistant {
        exposure: Value,
        states: Value,
        state_override: Option<Value>,
        history: Value,
        states_status: StatusCode,
        states_delay: Option<Duration>,
        websocket_calls: Arc<AtomicUsize>,
        state_calls: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<String>>>,
        traceparents: Arc<Mutex<Vec<String>>>,
    }

    impl MockHomeAssistant {
        fn new(exposure: Value, states: Value, history: Value) -> Self {
            Self {
                exposure,
                states,
                state_override: None,
                history,
                states_status: StatusCode::OK,
                states_delay: None,
                websocket_calls: Arc::new(AtomicUsize::new(0)),
                state_calls: Arc::new(AtomicUsize::new(0)),
                requests: Arc::new(Mutex::new(Vec::new())),
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

        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        let query_spans = spans
            .iter()
            .filter(|span| span.name == "home_assistant.query")
            .collect::<Vec<_>>();
        let client_spans = spans
            .iter()
            .filter(|span| span.name == "http.client.request")
            .collect::<Vec<_>>();
        assert_eq!(query_spans.len(), 4);
        assert_eq!(client_spans.len(), 4);

        for traceparent in [
            success_traceparent,
            status_error_traceparent,
            redirect_traceparent,
            cancelled_traceparent,
        ] {
            assert_traceparent_matches_client_span(&traceparent, &client_spans);
        }

        for client_span in &client_spans {
            assert_eq!(client_span.span_kind, SpanKind::Client);
            assert!(query_spans.iter().any(|query_span| {
                client_span.parent_span_id == query_span.span_context.span_id()
                    && client_span.span_context.trace_id() == query_span.span_context.trace_id()
            }));
            assert_eq!(
                span_attribute(client_span, "http.request.method").as_deref(),
                Some("GET")
            );
            assert!(span_attribute(client_span, "outcome").is_some_and(|value| !value.is_empty()));
            assert_span_is_privacy_bounded(client_span);
        }

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

    async fn serve(mock: MockHomeAssistant) -> (Url, JoinHandle<()>) {
        let app = Router::new()
            .route("/api/websocket", get(websocket))
            .route("/api/states", get(all_states))
            .route("/api/states/{entity_id}", get(one_state))
            .route("/api/history/period/{start}", get(history))
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
        let command = receive_json(&mut socket).await;
        mock.requests.lock().unwrap().push(format!(
            "ws-command:{}:{}",
            command
                .get("id")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            command.get("type").and_then(Value::as_str).unwrap_or("")
        ));
        socket
            .send(ws::Message::Text(mock.exposure.to_string().into()))
            .await
            .unwrap();
    }

    async fn receive_json(socket: &mut ws::WebSocket) -> Value {
        let message = socket.recv().await.unwrap().unwrap();
        match message {
            ws::Message::Text(text) => serde_json::from_str(text.as_ref()).unwrap(),
            _ => panic!("expected text WebSocket message"),
        }
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
