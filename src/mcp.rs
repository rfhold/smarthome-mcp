#![allow(clippy::useless_vec)]

use std::sync::Arc;

use axum::Router;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use mcp::{
    McpProtectedResourceMetadata, McpToolResult, OAuthAuthorizationServer,
    server::{
        ServerContext, ServerError, ServerResult, StreamableHttpAuthorization,
        StreamableHttpOptions, streamable_http_router_with_options,
    },
};
use serde_json::json;

use crate::{
    config::OAuthConfig,
    integrations::home_assistant::{
        Error as HomeAssistantError,
        actions::{
            CameraSnapshotInput, GetHistoryInput, GetStatesInput, ListDevicesInput,
            ListEntitiesInput,
        },
    },
    services::Services,
};

#[cfg(test)]
const TOOL_NAME: &str = "home_assistant_query";

#[derive(Clone)]
pub struct SmarthomeMcp {
    services: Arc<Services>,
}

pub fn router(
    config: &OAuthConfig,
    services: Arc<Services>,
    oauth: &OAuthAuthorizationServer,
) -> Result<Router, String> {
    let handler = Arc::new(SmarthomeMcp { services });
    let required_scope = config.required_scope.clone();
    let metadata =
        McpProtectedResourceMetadata::new(config.resource.clone(), [config.issuer.clone()])
            .with_scopes([required_scope.clone()])
            .with_resource_name("Smarthome MCP");
    let hosted = oauth.clone();
    let authorization = StreamableHttpAuthorization::hosted(metadata, move |token, context| {
        hosted.authorize_token(token, context)
    })
    .map_err(|_| "invalid MCP authorization configuration".to_owned())?
    .with_required_scopes([required_scope]);
    let options = StreamableHttpOptions::default()
        .without_root_protected_resource_metadata()
        .with_authorization(authorization);
    Ok(streamable_http_router_with_options(handler, options))
}

#[mcp::progressive_server(
    name = "smarthome-mcp",
    version = "0.1.0",
    description = "Authenticated, policy-bounded smart-home tools.",
    tool(
        name = "home_assistant_query",
        description = "Read explicitly Assist-exposed Home Assistant entities, history, and camera frames.",
        annotations = json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": true
        }),
        namespace(entity, description = "Query Home Assistant entities."),
        namespace(device, description = "Query Home Assistant devices."),
        namespace(state, description = "Query Home Assistant current states."),
        namespace(history, description = "Query Home Assistant state history."),
        namespace(camera, description = "Read Home Assistant camera frames.")
    )
)]
impl SmarthomeMcp {
    /// List current normalized states grouped by Home Assistant device and
    /// effective area. Only entities currently exposed to Assist are included.
    #[action(tool = "home_assistant_query", name = "device.list")]
    async fn list_devices(
        &self,
        input: ListDevicesInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(_) => {
                return Ok(tool_error(
                    "list devices",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.list_devices(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(query_result("devices", output)),
            Err(error) => Ok(tool_error("list devices", error)),
        }
    }

    /// List current states for entities explicitly exposed to Home Assistant's
    /// conversation assistant. Results may be searched, filtered by domain,
    /// and are deterministically limited.
    #[action(tool = "home_assistant_query", name = "entity.list")]
    async fn list_entities(
        &self,
        input: ListEntitiesInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(_) => {
                return Ok(tool_error(
                    "list entities",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.list_entities(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(query_result("entities", output)),
            Err(error) => Ok(tool_error("list entities", error)),
        }
    }

    /// Get normalized current states for up to 25 explicit entity IDs. Every
    /// requested entity must currently be explicitly exposed to Assist.
    #[action(tool = "home_assistant_query", name = "state.get")]
    async fn get_states(
        &self,
        input: GetStatesInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(_) => {
                return Ok(tool_error(
                    "get states",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.get_states(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(query_result("states", output)),
            Err(error) => Ok(tool_error("get states", error)),
        }
    }

    /// Get minimal significant state history for up to 10 explicit entity IDs
    /// over no more than 24 hours. Arbitrary attributes are never returned.
    #[action(tool = "home_assistant_query", name = "history.get")]
    async fn get_history(
        &self,
        input: GetHistoryInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(_) => {
                return Ok(tool_error(
                    "get history",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.get_history(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(query_result("history", output)),
            Err(error) => Ok(tool_error("get history", error)),
        }
    }

    /// Get the current frame for one camera explicitly exposed to Assist.
    #[action(tool = "home_assistant_query", name = "camera.snapshot")]
    async fn camera_snapshot(
        &self,
        input: CameraSnapshotInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(_) => {
                return Ok(tool_error(
                    "get camera snapshot",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.camera_snapshot_with(&query, |snapshot| async move {
                let encoded = STANDARD.encode(snapshot.data);
                Ok(McpToolResult::new(json!({
                    "content": [
                        {"type":"text","text":"Returned the current camera frame."},
                        {"type":"image","data":encoded,"mimeType":snapshot.mime_type}
                    ],
                    "structuredContent": {
                        "action": "camera.snapshot",
                        "entity_id": snapshot.entity_id,
                        "mime_type": snapshot.mime_type,
                    }
                })))
            }) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(output),
            Err(error) => Ok(tool_error("get camera snapshot", error)),
        }
    }
}

fn query_result(noun: &str, output: serde_json::Value) -> McpToolResult {
    let count = output
        .get("entities")
        .or_else(|| output.get("devices"))
        .or_else(|| output.get("history"))
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    McpToolResult::new(json!({
        "content": [{"type":"text","text":format!("Returned {count} {noun} item(s).")}],
        "structuredContent": output
    }))
}

fn tool_error(action_name: &str, error: HomeAssistantError) -> McpToolResult {
    error.into_tool_error(action_name).into_mcp_result()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{Json, body::Bytes, extract::WebSocketUpgrade, response::Response, routing::get};
    use mcp::protocol::MCP_PROTOCOL_VERSION;
    use reqwest::{Client, StatusCode};
    use serde_json::Value;
    use tokio::{net::TcpListener, task::JoinHandle};

    use crate::{config::Secret, integrations::home_assistant::HomeAssistantClient};

    use super::*;

    async fn serve(router: Router) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (origin, task)
    }

    async fn endpoint() -> (String, JoinHandle<()>) {
        endpoint_for(url::Url::parse("http://127.0.0.1:1/").unwrap()).await
    }

    async fn endpoint_for(home_assistant_origin: url::Url) -> (String, JoinHandle<()>) {
        endpoint_for_with_timeout(home_assistant_origin, Duration::from_millis(100)).await
    }

    async fn endpoint_for_with_timeout(
        home_assistant_origin: url::Url,
        timeout: Duration,
    ) -> (String, JoinHandle<()>) {
        let client = HomeAssistantClient::for_test(
            home_assistant_origin,
            Secret("test-token".to_owned()),
            timeout,
        );
        let handler = Arc::new(SmarthomeMcp {
            services: Arc::new(Services::new(client)),
        });
        let (origin, task) = serve(mcp::server::streamable_http_router(handler)).await;
        (format!("{origin}/mcp"), task)
    }

    async fn home_assistant() -> (url::Url, JoinHandle<()>) {
        home_assistant_with_camera(b"\x89PNG\r\n\x1a\nframe".to_vec()).await
    }

    async fn home_assistant_with_camera(camera: Vec<u8>) -> (url::Url, JoinHandle<()>) {
        let camera = Bytes::from(camera);
        let router = Router::new()
            .route("/api/websocket", get(mock_websocket))
            .route(
                "/api/camera_proxy/camera.front_door",
                get(move || {
                    let camera = camera.clone();
                    async move { ([(reqwest::header::CONTENT_TYPE, "image/png")], camera) }
                }),
            )
            .route(
                "/api/states",
                get(|| async {
                    Json(json!([{
                        "entity_id":"sensor.allowed",
                        "state":"1",
                        "attributes":{},
                        "last_changed":"2026-08-10T00:00:00Z",
                        "last_updated":"2026-08-10T00:00:00Z"
                    }]))
                }),
            );
        let (origin, task) = serve(router).await;
        (url::Url::parse(&origin).unwrap(), task)
    }

    async fn mock_websocket(upgrade: WebSocketUpgrade) -> Response {
        upgrade.on_upgrade(|mut socket| async move {
            use axum::extract::ws::Message;
            use futures_util::StreamExt as _;

            socket
                .send(Message::Text(
                    json!({"type":"auth_required"}).to_string().into(),
                ))
                .await
                .unwrap();
            socket.next().await.unwrap().unwrap();
            socket
                .send(Message::Text(json!({"type":"auth_ok"}).to_string().into()))
                .await
                .unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let command: Value = serde_json::from_str(text.as_ref()).unwrap();
                let id = command["id"].as_u64().unwrap();
                let result = match command["type"].as_str().unwrap() {
                    "homeassistant/expose_entity/list" => json!({
                        "exposed_entities":{
                            "sensor.allowed":{"conversation":true},
                            "camera.front_door":{"conversation":true}
                        }
                    }),
                    "config/entity_registry/get_entries" => json!({"sensor.allowed":null}),
                    "config/device_registry/list" | "config/area_registry/list" => json!([]),
                    _ => break,
                };
                socket
                    .send(Message::Text(
                        json!({"id":id,"type":"result","success":true,"result":result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
        })
    }

    fn request(method: &str, id: &str, params: Value) -> Value {
        let mut body = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
        body["params"]["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name":"smarthome-tests","version":"1"}
        });
        body
    }

    async fn post(endpoint: &str, body: Value) -> (StatusCode, Value) {
        let (status, payload, _) = post_with_wire_size(endpoint, body).await;
        (status, payload)
    }

    async fn post_with_wire_size(endpoint: &str, body: Value) -> (StatusCode, Value, usize) {
        let method = body["method"].as_str().unwrap();
        let mut request = Client::new()
            .post(endpoint)
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("mcp-protocol-version", MCP_PROTOCOL_VERSION)
            .header("mcp-method", method);
        if let Some(name) = body["params"]["name"].as_str() {
            request = request.header("mcp-name", name);
        }
        let response = request.json(&body).send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        let wire_size = text.len();
        let payload = text
            .lines()
            .rev()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap_or(&text);
        (status, serde_json::from_str(payload).unwrap(), wire_size)
    }

    #[tokio::test]
    async fn discovery_lists_only_the_home_assistant_tool() {
        let (endpoint, task) = endpoint().await;
        let (_, response) = post(&endpoint, request("tools/list", "list", json!({}))).await;
        assert_eq!(response["result"]["tools"][0]["name"], TOOL_NAME);
        assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 1);
        assert_eq!(
            response["result"]["tools"][0]["annotations"]["readOnlyHint"],
            true
        );
        task.abort();
    }

    #[tokio::test]
    async fn semantic_validation_returns_a_safe_tool_error_without_upstream_contact() {
        let (endpoint, task) = endpoint().await;
        let (_, response) = post(
            &endpoint,
            request(
                "tools/call",
                "call",
                json!({
                    "name": TOOL_NAME,
                    "arguments": {"action":"state.get", "input":{"entity_ids":[]}}
                }),
            ),
        )
        .await;
        assert_eq!(
            response["result"]["structuredContent"]["error"]["code"],
            "invalid_arguments"
        );
        assert_eq!(response["result"]["isError"], true);
        task.abort();
    }

    #[tokio::test]
    async fn generated_help_and_schema_use_only_dotted_actions() {
        let (endpoint, task) = endpoint().await;
        let (_, response) = post(
            &endpoint,
            request(
                "tools/call",
                "help",
                json!({"name": TOOL_NAME, "arguments":{"action":"help"}}),
            ),
        )
        .await;
        let serialized = serde_json::to_string(&response).unwrap();
        for namespace_help in [
            "help.entity",
            "help.device",
            "help.state",
            "help.history",
            "help.camera",
        ] {
            assert!(serialized.contains(namespace_help));
        }
        for legacy in ["list_entities", "list_devices", "get_states", "get_history"] {
            assert!(!serialized.contains(legacy));
        }
        for (namespace_help, action) in [
            ("help.entity", "entity.list"),
            ("help.device", "device.list"),
            ("help.state", "state.get"),
            ("help.history", "history.get"),
            ("help.camera", "camera.snapshot"),
        ] {
            let (_, help) = post(
                &endpoint,
                request(
                    "tools/call",
                    namespace_help,
                    json!({"name": TOOL_NAME, "arguments":{"action":namespace_help}}),
                ),
            )
            .await;
            assert_eq!(
                help["result"]["structuredContent"]["actions"][0]["action"],
                action
            );
        }

        let (_, discovery) = post(&endpoint, request("tools/list", "list", json!({}))).await;
        let schema =
            serde_json::to_string(&discovery["result"]["tools"][0]["inputSchema"]).unwrap();
        for action in [
            "entity.list",
            "device.list",
            "state.get",
            "history.get",
            "camera.snapshot",
        ] {
            assert!(schema.contains(action));
        }
        for legacy in ["list_entities", "list_devices", "get_states", "get_history"] {
            assert!(!schema.contains(legacy));
        }
        task.abort();
    }

    #[tokio::test]
    async fn legacy_action_names_are_rejected() {
        let (endpoint, task) = endpoint().await;
        for legacy in [
            "list_entities",
            "list_devices",
            "device_list",
            "get_states",
            "get_history",
        ] {
            let (_, response) = post(
                &endpoint,
                request(
                    "tools/call",
                    legacy,
                    json!({"name": TOOL_NAME, "arguments":{"action":legacy,"input":{}}}),
                ),
            )
            .await;
            assert!(
                response.get("error").is_some(),
                "accepted legacy action {legacy}"
            );
        }
        task.abort();
    }

    #[tokio::test]
    async fn device_list_dispatches_filters_and_rejects_unknown_input_fields() {
        let (home_assistant_origin, home_assistant_task) = home_assistant().await;
        let (endpoint, task) = endpoint_for(home_assistant_origin).await;
        let (_, dispatched) = post(
            &endpoint,
            request(
                "tools/call",
                "device",
                json!({
                    "name": TOOL_NAME,
                    "arguments":{"action":"device.list","input":{"limit":1}}
                }),
            ),
        )
        .await;
        assert_eq!(
            dispatched["result"]["structuredContent"]["action"],
            "device.list"
        );
        assert_eq!(
            dispatched["result"]["structuredContent"]["devices"],
            json!([{
                "entities":[{
                    "entity_id":"sensor.allowed",
                    "domain":"sensor",
                    "state":"1",
                    "last_changed":"2026-08-10T00:00:00Z",
                    "last_updated":"2026-08-10T00:00:00Z"
                }]
            }])
        );
        assert_eq!(
            dispatched["result"]["structuredContent"]["truncated"],
            false
        );
        assert_eq!(
            dispatched["result"]["content"][0]["text"],
            "Returned 1 devices item(s)."
        );

        let (_, filtered) = post(
            &endpoint,
            request(
                "tools/call",
                "device-filter",
                json!({
                    "name": TOOL_NAME,
                    "arguments":{
                        "action":"device.list",
                        "input":{"limit":1},
                        "filter":".devices[0].entities[0].entity_id"
                    }
                }),
            ),
        )
        .await;
        assert_eq!(filtered["result"]["structuredContent"], "sensor.allowed");
        assert_eq!(
            filtered["result"]["content"][0]["text"],
            "\"sensor.allowed\""
        );

        let (_, rejected) = post(
            &endpoint,
            request(
                "tools/call",
                "unknown-field",
                json!({
                    "name": TOOL_NAME,
                    "arguments":{
                        "action":"device.list",
                        "input":{"limit":1,"unexpected":true}
                    }
                }),
            ),
        )
        .await;
        assert!(rejected.get("error").is_some());
        task.abort();
        home_assistant_task.abort();
    }

    #[tokio::test]
    async fn camera_snapshot_dispatches_as_standard_base64_image_and_filters_metadata_only() {
        let (home_assistant_origin, home_assistant_task) = home_assistant().await;
        let (endpoint, task) = endpoint_for(home_assistant_origin).await;
        let (_, dispatched) = post(
            &endpoint,
            request(
                "tools/call",
                "camera",
                json!({
                    "name": TOOL_NAME,
                    "arguments":{
                        "action":"camera.snapshot",
                        "input":{"entity_id":"camera.front_door"}
                    }
                }),
            ),
        )
        .await;
        assert_eq!(dispatched["result"]["content"][0]["type"], "text");
        assert_eq!(dispatched["result"]["content"][1]["type"], "image");
        assert_eq!(dispatched["result"]["content"][1]["mimeType"], "image/png");
        assert_eq!(
            STANDARD
                .decode(dispatched["result"]["content"][1]["data"].as_str().unwrap())
                .unwrap(),
            b"\x89PNG\r\n\x1a\nframe"
        );
        assert_eq!(
            dispatched["result"]["structuredContent"],
            json!({
                "action":"camera.snapshot",
                "entity_id":"camera.front_door",
                "mime_type":"image/png"
            })
        );
        assert!(
            !serde_json::to_string(&dispatched["result"]["structuredContent"])
                .unwrap()
                .contains(dispatched["result"]["content"][1]["data"].as_str().unwrap())
        );

        let (_, filtered) = post(
            &endpoint,
            request(
                "tools/call",
                "camera-filter",
                json!({
                    "name": TOOL_NAME,
                    "arguments":{
                        "action":"camera.snapshot",
                        "input":{"entity_id":"camera.front_door"},
                        "filter":".mime_type"
                    }
                }),
            ),
        )
        .await;
        assert_eq!(filtered["result"]["structuredContent"], "image/png");
        assert_eq!(filtered["result"]["content"][1]["type"], "image");

        for input in [
            json!({"entity_id":"sensor.allowed"}),
            json!({"entity_id":"camera.front_door","unexpected":true}),
        ] {
            let (_, rejected) = post(
                &endpoint,
                request(
                    "tools/call",
                    "camera-rejected",
                    json!({
                        "name": TOOL_NAME,
                        "arguments":{"action":"camera.snapshot","input":input}
                    }),
                ),
            )
            .await;
            assert!(
                rejected.get("error").is_some()
                    || rejected["result"]["structuredContent"]["error"]["code"]
                        == "invalid_arguments"
            );
        }
        task.abort();
        home_assistant_task.abort();
    }

    #[tokio::test]
    async fn maximum_camera_snapshot_builds_and_transports_below_the_agent_cap() {
        const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
        const AGENT_TRANSPORT_BYTES: usize = 8 * 1024 * 1024;

        let mut image = vec![0; MAX_IMAGE_BYTES];
        image[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        let (home_assistant_origin, home_assistant_task) =
            home_assistant_with_camera(image.clone()).await;
        let (endpoint, task) =
            endpoint_for_with_timeout(home_assistant_origin, Duration::from_secs(2)).await;
        let (status, response, wire_size) = post_with_wire_size(
            &endpoint,
            request(
                "tools/call",
                "maximum-camera",
                json!({
                    "name": TOOL_NAME,
                    "arguments":{
                        "action":"camera.snapshot",
                        "input":{"entity_id":"camera.front_door"}
                    }
                }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let data = response["result"]["content"][1]["data"].as_str().unwrap();
        let decoded = STANDARD.decode(data).unwrap();
        assert_eq!(decoded.len(), MAX_IMAGE_BYTES);
        assert_eq!(decoded, image);
        assert_eq!(STANDARD.encode(&decoded), data);
        assert_eq!(response["result"]["content"][1]["mimeType"], "image/png");
        assert_eq!(
            response["result"]["structuredContent"],
            json!({
                "action":"camera.snapshot",
                "entity_id":"camera.front_door",
                "mime_type":"image/png"
            })
        );
        assert!(
            serde_json::to_vec(&response["result"]["structuredContent"])
                .unwrap()
                .len()
                < 256
        );
        assert!(
            wire_size < AGENT_TRANSPORT_BYTES,
            "wire size was {wire_size}"
        );

        task.abort();
        home_assistant_task.abort();
    }
}
