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
            AutomationTracesInput, AutomationValidateInput, CameraSnapshotInput,
            ClimateTemperatureInput, ConfigUpsertInput, Control, ControlAction, CoverPositionInput,
            DiscoverRoutersInput, EntityControlInput, FanPercentageInput, GetHistoryInput,
            GetStatesInput, LightTurnOnInput, ListDevicesInput, ListEntitiesInput,
            ListMatterDevicesInput, MatterDeviceInput, MatterEmptyInput, MediaPlayerVolumeInput,
            SetPreferredDatasetInput, SetPreferredRouterInput, ThreadEmptyInput,
        },
    },
    services::Services,
};

#[cfg(test)]
const TOOL_NAME: &str = "home_assistant_query";
#[cfg(test)]
const EXEC_TOOL_NAME: &str = "home_assistant_exec";
#[cfg(test)]
const THREAD_QUERY_TOOL_NAME: &str = "thread_query";
#[cfg(test)]
const THREAD_EXEC_TOOL_NAME: &str = "thread_exec";
#[cfg(test)]
const MATTER_QUERY_TOOL_NAME: &str = "matter_query";
#[cfg(test)]
const MATTER_EXEC_TOOL_NAME: &str = "matter_exec";

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
        description = "Read bounded Home Assistant entity data, validate automation sections, and summarize automation traces.",
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
        namespace(camera, description = "Read Home Assistant camera frames."),
        namespace(automation, description = "Validate automation sections and summarize traces.")
    ),
    tool(
        name = "home_assistant_exec",
        description = "Operate Assist-exposed entities and upsert bounded native scene or automation configs.",
        annotations = json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": true
        }),
        namespace(scene, description = "Activate or upsert Home Assistant scenes."),
        namespace(automation, description = "Upsert Home Assistant automations."),
        namespace(light, description = "Control Home Assistant lights."),
        namespace(switch, description = "Control Home Assistant switches."),
        namespace(fan, description = "Control Home Assistant fans."),
        namespace(cover, description = "Control Home Assistant covers."),
        namespace(climate, description = "Control Home Assistant climate entities."),
        namespace(media_player, description = "Control Home Assistant media players."),
        namespace(lock, description = "Control Home Assistant locks.")
    ),
    tool(
        name = "thread_query",
        description = "Inspect bounded Thread network and border-router status.",
        annotations = json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": true
        }),
        namespace(network, description = "Inspect stored Thread networks."),
        namespace(router, description = "Discover Thread border routers."),
        namespace(readiness, description = "Inspect Thread readiness.")
    ),
    tool(
        name = "thread_exec",
        description = "Select preferred stored Thread networks and border routers.",
        annotations = json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": true,
            "openWorldHint": true
        }),
        namespace(network, description = "Select a preferred Thread network."),
        namespace(router, description = "Select a preferred Thread border router.")
    ),
    tool(
        name = "matter_query",
        description = "Inspect bounded Matter device and readiness information.",
        annotations = json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": true
        }),
        namespace(readiness, description = "Inspect Matter integration readiness."),
        namespace(device, description = "Inspect registered Matter devices.")
    ),
    tool(
        name = "matter_exec",
        description = "Run fixed bounded Matter device maintenance actions.",
        annotations = json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": true
        }),
        namespace(device, description = "Maintain registered Matter devices.")
    )
)]
impl SmarthomeMcp {
    /// Upsert a complete native scene configuration under a stable key. Home
    /// Assistant accepts the change for asynchronous reload; activation is not implied.
    #[action(tool = "home_assistant_exec", name = "scene.upsert")]
    async fn upsert_scene(
        &self,
        input: ConfigUpsertInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let command = match input.validate() {
            Ok(command) => command,
            Err(()) => {
                return Ok(tool_error(
                    "upsert scene",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.upsert_scene(&command) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(accepted_result(output)),
            Err(error) => Ok(tool_error("upsert scene", error)),
        }
    }

    /// Upsert a complete native automation configuration under a stable key.
    /// Acceptance does not guarantee reload completion or future operation.
    #[action(tool = "home_assistant_exec", name = "automation.upsert")]
    async fn upsert_automation(
        &self,
        input: ConfigUpsertInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let command = match input.validate() {
            Ok(command) => command,
            Err(()) => {
                return Ok(tool_error(
                    "upsert automation",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.upsert_automation(&command) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(accepted_result(output)),
            Err(error) => Ok(tool_error("upsert automation", error)),
        }
    }

    /// Validate submitted native automation trigger, condition, and action
    /// sections. A valid result does not guarantee future operation.
    #[action(tool = "home_assistant_query", name = "automation.validate")]
    async fn validate_automation(
        &self,
        input: AutomationValidateInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let validation = match input.validate() {
            Ok(validation) => validation,
            Err(()) => {
                return Ok(tool_error(
                    "validate automation",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.validate_automation(&validation) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("validate automation", error)),
        }
    }

    /// Return a bounded newest-first projection of recent automation traces.
    /// Trace history does not guarantee future operation.
    #[action(tool = "home_assistant_query", name = "automation.traces")]
    async fn automation_traces(
        &self,
        input: AutomationTracesInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "list automation traces",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.automation_traces(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("list automation traces", error)),
        }
    }

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
            Ok(output) => query_result(output),
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
            Ok(output) => query_result(output),
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
            Ok(output) => query_result(output),
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
            Ok(output) => query_result(output),
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

    /// List normalized stored Thread datasets without operational TLVs.
    #[action(tool = "thread_query", name = "network.list")]
    async fn list_thread_networks(
        &self,
        _: ThreadEmptyInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let result = tokio::select! {
            result = self.services.home_assistant.list_thread_networks() => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("list Thread networks", error)),
        }
    }

    /// Discover Thread border routers for one through ten seconds.
    #[action(tool = "thread_query", name = "router.discover")]
    async fn discover_thread_routers(
        &self,
        input: DiscoverRoutersInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "discover Thread routers",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.discover_thread_routers(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("discover Thread routers", error)),
        }
    }

    /// Summarize stored Thread datasets and currently discovered routers.
    #[action(tool = "thread_query", name = "readiness.get")]
    async fn get_thread_readiness(
        &self,
        _: ThreadEmptyInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let result = tokio::select! {
            result = self.services.home_assistant.thread_readiness() => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("get Thread readiness", error)),
        }
    }

    /// Select one stored Thread dataset as preferred.
    #[action(tool = "thread_exec", name = "network.set_preferred")]
    async fn set_preferred_thread_network(
        &self,
        input: SetPreferredDatasetInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let command = match input.validate() {
            Ok(command) => command,
            Err(()) => {
                return Ok(tool_error(
                    "set preferred Thread network",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.set_preferred_thread_dataset(&command) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(control_result(output)),
            Err(error) => Ok(tool_error("set preferred Thread network", error)),
        }
    }

    /// Select one border router for a stored Thread dataset.
    #[action(tool = "thread_exec", name = "router.set_preferred")]
    async fn set_preferred_thread_router(
        &self,
        input: SetPreferredRouterInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let command = match input.validate() {
            Ok(command) => command,
            Err(()) => {
                return Ok(tool_error(
                    "set preferred Thread router",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.set_preferred_thread_router(&command) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(control_result(output)),
            Err(error) => Ok(tool_error("set preferred Thread router", error)),
        }
    }

    /// Report whether the Matter device registry API responds and its known device count.
    #[action(tool = "matter_query", name = "readiness.get")]
    async fn get_matter_readiness(
        &self,
        _: MatterEmptyInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let result = tokio::select! {
            result = self.services.home_assistant.matter_readiness() => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("get Matter readiness", error)),
        }
    }

    /// List devices identified by the Home Assistant registry as Matter devices.
    #[action(tool = "matter_query", name = "device.list")]
    async fn list_matter_devices(
        &self,
        input: ListMatterDevicesInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "list Matter devices",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.list_matter_devices(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("list Matter devices", error)),
        }
    }

    /// Get a strict projection of official Matter node diagnostics.
    #[action(tool = "matter_query", name = "device.diagnostics")]
    async fn get_matter_device_diagnostics(
        &self,
        input: MatterDeviceInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "get Matter device diagnostics",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.matter_device_diagnostics(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("get Matter device diagnostics", error)),
        }
    }

    /// Ping a Matter device's known IP addresses.
    #[action(tool = "matter_query", name = "device.ping")]
    async fn ping_matter_device(
        &self,
        input: MatterDeviceInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "ping Matter device",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.ping_matter_device(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("ping Matter device", error)),
        }
    }

    /// Re-interview one registered Matter device and discard upstream details.
    #[action(tool = "matter_exec", name = "device.interview")]
    async fn interview_matter_device(
        &self,
        input: MatterDeviceInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "interview Matter device",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.interview_matter_device(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(control_result(output)),
            Err(error) => Ok(tool_error("interview Matter device", error)),
        }
    }

    /// Activate one scene explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "scene.activate")]
    async fn activate_scene(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "activate scene",
            input.validate(ControlAction::SceneActivate),
            context,
        )
        .await
    }

    /// Turn on one light, optionally with a brightness percentage.
    #[action(tool = "home_assistant_exec", name = "light.turn_on")]
    async fn turn_on_light(
        &self,
        input: LightTurnOnInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(self, "turn on light", input.validate(), context).await
    }

    /// Turn off one light explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "light.turn_off")]
    async fn turn_off_light(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "turn off light",
            input.validate(ControlAction::LightTurnOff),
            context,
        )
        .await
    }

    /// Turn on one switch explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "switch.turn_on")]
    async fn turn_on_switch(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "turn on switch",
            input.validate(ControlAction::SwitchTurnOn),
            context,
        )
        .await
    }

    /// Turn off one switch explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "switch.turn_off")]
    async fn turn_off_switch(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "turn off switch",
            input.validate(ControlAction::SwitchTurnOff),
            context,
        )
        .await
    }

    /// Turn on one fan explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "fan.turn_on")]
    async fn turn_on_fan(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "turn on fan",
            input.validate(ControlAction::FanTurnOn),
            context,
        )
        .await
    }

    /// Turn off one fan explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "fan.turn_off")]
    async fn turn_off_fan(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "turn off fan",
            input.validate(ControlAction::FanTurnOff),
            context,
        )
        .await
    }

    /// Set one fan's percentage from 0 through 100.
    #[action(tool = "home_assistant_exec", name = "fan.set_percentage")]
    async fn set_fan_percentage(
        &self,
        input: FanPercentageInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(self, "set fan percentage", input.validate(), context).await
    }

    /// Open one cover explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "cover.open")]
    async fn open_cover(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "open cover",
            input.validate(ControlAction::CoverOpen),
            context,
        )
        .await
    }

    /// Close one cover explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "cover.close")]
    async fn close_cover(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "close cover",
            input.validate(ControlAction::CoverClose),
            context,
        )
        .await
    }

    /// Stop one cover explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "cover.stop")]
    async fn stop_cover(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "stop cover",
            input.validate(ControlAction::CoverStop),
            context,
        )
        .await
    }

    /// Set one cover's position from 0 through 100.
    #[action(tool = "home_assistant_exec", name = "cover.set_position")]
    async fn set_cover_position(
        &self,
        input: CoverPositionInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(self, "set cover position", input.validate(), context).await
    }

    /// Turn on one climate entity explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "climate.turn_on")]
    async fn turn_on_climate(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "turn on climate entity",
            input.validate(ControlAction::ClimateTurnOn),
            context,
        )
        .await
    }

    /// Turn off one climate entity explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "climate.turn_off")]
    async fn turn_off_climate(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "turn off climate entity",
            input.validate(ControlAction::ClimateTurnOff),
            context,
        )
        .await
    }

    /// Set one climate entity's finite temperature from -273.15 through 1000.
    #[action(tool = "home_assistant_exec", name = "climate.set_temperature")]
    async fn set_climate_temperature(
        &self,
        input: ClimateTemperatureInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(self, "set climate temperature", input.validate(), context).await
    }

    /// Turn on one media player explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "media_player.turn_on")]
    async fn turn_on_media_player(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "turn on media player",
            input.validate(ControlAction::MediaPlayerTurnOn),
            context,
        )
        .await
    }

    /// Turn off one media player explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "media_player.turn_off")]
    async fn turn_off_media_player(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "turn off media player",
            input.validate(ControlAction::MediaPlayerTurnOff),
            context,
        )
        .await
    }

    /// Start playback on one media player explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "media_player.play")]
    async fn play_media_player(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "play media player",
            input.validate(ControlAction::MediaPlayerPlay),
            context,
        )
        .await
    }

    /// Pause playback on one media player explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "media_player.pause")]
    async fn pause_media_player(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "pause media player",
            input.validate(ControlAction::MediaPlayerPause),
            context,
        )
        .await
    }

    /// Stop playback on one media player explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "media_player.stop")]
    async fn stop_media_player(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "stop media player",
            input.validate(ControlAction::MediaPlayerStop),
            context,
        )
        .await
    }

    /// Set one media player's volume from 0.0 through 1.0.
    #[action(tool = "home_assistant_exec", name = "media_player.volume_set")]
    async fn set_media_player_volume(
        &self,
        input: MediaPlayerVolumeInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(self, "set media player volume", input.validate(), context).await
    }

    /// Lock one lock explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "lock.lock")]
    async fn lock_lock(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "lock entity",
            input.validate(ControlAction::LockLock),
            context,
        )
        .await
    }

    /// Unlock one lock explicitly exposed to Assist.
    #[action(tool = "home_assistant_exec", name = "lock.unlock")]
    async fn unlock_lock(
        &self,
        input: EntityControlInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        execute_control(
            self,
            "unlock entity",
            input.validate(ControlAction::LockUnlock),
            context,
        )
        .await
    }
}

async fn execute_control(
    server: &SmarthomeMcp,
    description: &'static str,
    control: Result<Control, ()>,
    context: ServerContext,
) -> ServerResult<McpToolResult> {
    let control = match control {
        Ok(control) => control,
        Err(()) => {
            return Ok(tool_error(
                description,
                HomeAssistantError::InvalidArguments,
            ));
        }
    };
    let result = tokio::select! {
        result = server.services.home_assistant.execute_control(&control) => result,
        () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
    };
    match result {
        Ok(output) => Ok(control_result(output)),
        Err(error) => Ok(tool_error(description, error)),
    }
}

fn control_result(output: serde_json::Value) -> McpToolResult {
    let action = output["action"].as_str().unwrap_or("control");
    McpToolResult::new(json!({
        "content": [{"type":"text","text":format!("Completed {action}.")}],
        "structuredContent": output
    }))
}

fn accepted_result(output: serde_json::Value) -> McpToolResult {
    McpToolResult::new(json!({
        "content": [{"type":"text","text":"Home Assistant accepted the configuration for asynchronous reload."}],
        "structuredContent": output
    }))
}

fn query_result(output: serde_json::Value) -> ServerResult<McpToolResult> {
    mcp::progressive::tool_result(output, None)
}

fn tool_error(action_name: &str, error: HomeAssistantError) -> McpToolResult {
    error.into_tool_error(action_name).into_mcp_result()
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use axum::{
        Json, Router,
        body::Bytes,
        extract::{State, WebSocketUpgrade},
        response::{IntoResponse as _, Response},
        routing::{get, post as route_post},
    };
    use mcp::protocol::MCP_PROTOCOL_VERSION;
    use reqwest::{Client, StatusCode};
    use serde_json::Value;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
        sync::Notify,
        task::JoinHandle,
    };
    use tracing::instrument::WithSubscriber as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    use crate::{config::Secret, integrations::home_assistant::HomeAssistantClient};

    use super::*;

    struct DropSignal(Arc<AtomicBool>, Arc<Notify>);

    #[derive(Clone)]
    struct TestLogWriter(Arc<Mutex<Vec<u8>>>);

    struct TestLogGuard(Arc<Mutex<Vec<u8>>>);

    impl io::Write for TestLogGuard {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TestLogWriter {
        type Writer = TestLogGuard;

        fn make_writer(&'a self) -> Self::Writer {
            TestLogGuard(self.0.clone())
        }
    }

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
            self.1.notify_waiters();
        }
    }

    #[derive(Clone)]
    struct CancellationMock {
        calls: Arc<AtomicUsize>,
        started: Arc<Notify>,
        dropped: Arc<AtomicBool>,
        dropped_notify: Arc<Notify>,
        bodies: Arc<Mutex<Vec<String>>>,
    }

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
            )
            .route(
                "/api/services/light/turn_on",
                route_post(|| async { Json(json!({"raw_secret":"must-not-leak"})) }),
            )
            .route(
                "/api/config/scene/config/evening_scene",
                route_post(|| async { Json(json!({"result":"ok"})) }),
            )
            .route(
                "/api/config/automation/config/arrival_lights",
                route_post(|| async { Json(json!({"result":"ok"})) }),
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
                            "camera.front_door":{"conversation":true},
                            "light.kitchen":{"conversation":true}
                        }
                    }),
                    "config/entity_registry/get_entries" => json!({"sensor.allowed":null}),
                    "config/device_registry/list" | "config/area_registry/list" => json!([]),
                    "validate_config" => json!({
                        "triggers":{"valid":true,"error":null},
                        "actions":{"valid":false,"error":"must-not-leak-validation-error"}
                    }),
                    "trace/list" => json!([{
                        "run_id":"run-safe","state":"stopped","script_execution":"failed",
                        "timestamp":{"start":"2026-08-10T11:00:00Z","finish":"2026-08-10T11:00:02Z"},
                        "domain":"automation","item_id":"arrival_lights","not_triggered":false,
                        "error":{"message":"must-not-leak-trace-error"},
                        "last_step":"action/0","config":{"secret":"must-not-leak-config"},
                        "trace":{"variables":{"secret":"must-not-leak-variable"}}
                    }]),
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
    async fn discovery_lists_query_and_exec_with_distinct_annotations() {
        let (endpoint, task) = endpoint().await;
        let (_, response) = post(&endpoint, request("tools/list", "list", json!({}))).await;
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 6);
        let query = tools.iter().find(|tool| tool["name"] == TOOL_NAME).unwrap();
        let exec = tools
            .iter()
            .find(|tool| tool["name"] == EXEC_TOOL_NAME)
            .unwrap();
        assert_eq!(query["annotations"]["readOnlyHint"], true);
        assert_eq!(
            exec["annotations"],
            json!({
                "readOnlyHint":false,
                "destructiveHint":true,
                "idempotentHint":false,
                "openWorldHint":true
            })
        );
        for name in [THREAD_QUERY_TOOL_NAME, MATTER_QUERY_TOOL_NAME] {
            let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
            assert_eq!(tool["annotations"]["destructiveHint"], false);
        }
        for name in [THREAD_EXEC_TOOL_NAME, MATTER_EXEC_TOOL_NAME] {
            let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
            assert_eq!(tool["annotations"]["readOnlyHint"], false);
            assert_eq!(tool["annotations"]["destructiveHint"], true);
        }
        task.abort();
    }

    #[tokio::test]
    async fn authoring_and_evidence_discovery_catalogs_are_explicit_and_closed() {
        let (endpoint, task) = endpoint().await;
        let (_, response) = post(&endpoint, request("tools/list", "list", json!({}))).await;
        let tools = response["result"]["tools"].as_array().unwrap();
        let query = tools.iter().find(|tool| tool["name"] == TOOL_NAME).unwrap();
        let query_schema = serde_json::to_string(&query["inputSchema"]).unwrap();
        for action in [
            "entity.list",
            "device.list",
            "state.get",
            "history.get",
            "camera.snapshot",
            "automation.validate",
            "automation.traces",
        ] {
            assert!(query_schema.contains(action), "query missing {action}");
        }
        assert!(query_schema.contains("\"additionalProperties\":false"));

        let (_, help) = post(
            &endpoint,
            request(
                "tools/call",
                "help.automation",
                json!({"name":TOOL_NAME,"arguments":{"action":"help.automation"}}),
            ),
        )
        .await;
        let actions = help["result"]["structuredContent"]["actions"]
            .as_array()
            .unwrap();
        assert_eq!(actions.len(), 2);
        assert!(
            actions
                .iter()
                .any(|entry| entry["action"] == "automation.validate")
        );
        assert!(
            actions
                .iter()
                .any(|entry| entry["action"] == "automation.traces")
        );
        task.abort();
    }

    #[tokio::test]
    async fn scene_upsert_caller_cancellation_drops_upstream_and_releases_capacity_privately() {
        let mock = CancellationMock {
            calls: Arc::new(AtomicUsize::new(0)),
            started: Arc::new(Notify::new()),
            dropped: Arc::new(AtomicBool::new(false)),
            dropped_notify: Arc::new(Notify::new()),
            bodies: Arc::new(Mutex::new(Vec::new())),
        };
        let router = Router::new()
            .route(
                "/api/config/scene/config/cancel_private_key",
                route_post(delayed_config_upsert),
            )
            .with_state(mock.clone());
        let (origin, home_assistant_task) = serve(router).await;
        let client = HomeAssistantClient::for_test(
            url::Url::parse(&origin).unwrap(),
            Secret("test-token".to_owned()),
            Duration::from_secs(10),
        );
        let handler = Arc::new(SmarthomeMcp {
            services: Arc::new(Services::new(client.clone())),
        });
        let logs = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_writer(TestLogWriter(logs.clone())),
        );
        let dispatch = tracing::Dispatch::new(subscriber);
        let (mut input_writer, input_reader) = tokio::io::duplex(64 * 1024);
        let (mut output_reader, output_writer) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(
            mcp::server::serve_stream(handler, input_reader, output_writer)
                .with_subscriber(dispatch),
        );
        let call = request(
            "tools/call",
            "cancel-authoring",
            json!({
                "name":EXEC_TOOL_NAME,
                "arguments":{
                    "action":"scene.upsert",
                    "input":{
                        "config_key":"cancel_private_key",
                        "config":{"id":"cancel_private_key","secret":"native-private-sentinel"}
                    }
                }
            }),
        );
        let started = mock.started.notified();
        input_writer
            .write_all(format!("{call}\n").as_bytes())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), started)
            .await
            .unwrap();
        let cancellation = request(
            "notifications/cancelled",
            "unused",
            json!({"requestId":"cancel-authoring","reason":"caller stopped"}),
        );
        let mut cancellation = cancellation;
        cancellation.as_object_mut().unwrap().remove("id");
        let dropped = mock.dropped_notify.notified();
        input_writer
            .write_all(format!("{cancellation}\n").as_bytes())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), dropped)
            .await
            .unwrap();
        assert!(mock.dropped.load(Ordering::Relaxed));
        assert!(client.has_full_test_capacity());
        assert_eq!(
            client
                .upsert_scene(
                    &crate::integrations::home_assistant::actions::ConfigUpsert {
                        config_key: "cancel_private_key".to_owned(),
                        config: json!({"id":"cancel_private_key"}),
                    }
                )
                .await
                .unwrap(),
            json!({"action":"scene.upsert","config_key":"cancel_private_key","accepted":true})
        );

        input_writer.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let mut output = String::new();
        output_reader.read_to_string(&mut output).await.unwrap();
        if !output.trim().is_empty() {
            assert_eq!(
                serde_json::from_str::<Value>(output.trim()).unwrap(),
                json!({
                    "jsonrpc":"2.0",
                    "id":"cancel-authoring",
                    "error":{"code":-32603,"message":"request cancelled"}
                })
            );
        }
        assert_eq!(mock.calls.load(Ordering::Relaxed), 2);
        let bodies = mock.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2);
        drop(bodies);
        let telemetry = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        for sentinel in [
            "cancel_private_key",
            "native-private-sentinel",
            "raw-upstream-sentinel",
        ] {
            assert!(!output.contains(sentinel));
            assert!(!telemetry.contains(sentinel));
        }
        home_assistant_task.abort();
    }

    async fn delayed_config_upsert(State(mock): State<CancellationMock>, body: Bytes) -> Response {
        let call = mock.calls.fetch_add(1, Ordering::Relaxed);
        mock.bodies
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(&body).into_owned());
        if call > 0 {
            return Json(json!({"result":"ok"})).into_response();
        }
        let _drop = DropSignal(mock.dropped.clone(), mock.dropped_notify.clone());
        mock.started.notify_waiters();
        std::future::pending::<()>().await;
        Json(json!({"result":"ok","raw":"raw-upstream-sentinel"})).into_response()
    }

    #[tokio::test]
    async fn all_authoring_and_evidence_actions_dispatch_with_safe_results() {
        let (home_assistant_origin, home_assistant_task) = home_assistant().await;
        let (endpoint, task) = endpoint_for(home_assistant_origin).await;
        for (tool, action, input, expected) in [
            (
                EXEC_TOOL_NAME,
                "scene.upsert",
                json!({"config_key":"evening_scene","config":{"id":"evening_scene","secret":"must-not-leak-scene"}}),
                json!({"action":"scene.upsert","config_key":"evening_scene","accepted":true}),
            ),
            (
                EXEC_TOOL_NAME,
                "automation.upsert",
                json!({"config_key":"arrival_lights","config":{"id":"arrival_lights","secret":"must-not-leak-automation"}}),
                json!({"action":"automation.upsert","config_key":"arrival_lights","accepted":true}),
            ),
            (
                TOOL_NAME,
                "automation.validate",
                json!({"triggers":[],"actions":[]}),
                json!({"action":"automation.validate","sections":{
                    "triggers":{"valid":true,"error_present":false},
                    "actions":{"valid":false,"error_present":true}
                }}),
            ),
            (
                TOOL_NAME,
                "automation.traces",
                json!({"item_id":"arrival_lights","limit":1}),
                json!({
                    "action":"automation.traces","item_id":"arrival_lights","total":1,"truncated":false,
                    "traces":[{"run_id":"run-safe","start":"2026-08-10T11:00:00Z",
                        "finish":"2026-08-10T11:00:02Z","duration_ms":2000,"state":"stopped",
                        "script_execution":"failed","not_triggered":false,"error_present":true,
                        "error_category":"execution_error"}]
                }),
            ),
        ] {
            let (_, response) = post(
                &endpoint,
                request(
                    "tools/call",
                    action,
                    json!({"name":tool,"arguments":{"action":action,"input":input}}),
                ),
            )
            .await;
            assert_eq!(response["result"]["structuredContent"], expected);
            let serialized = serde_json::to_string(&response).unwrap();
            for forbidden in ["must-not-leak", "last_step", "variables", "raw_error"] {
                assert!(
                    !serialized.contains(forbidden),
                    "{action} leaked {forbidden}"
                );
            }
        }
        task.abort();
        home_assistant_task.abort();
    }

    #[tokio::test]
    async fn thread_and_matter_discovery_have_exact_first_slice_catalogs() {
        let (endpoint, task) = endpoint().await;
        let (_, response) = post(&endpoint, request("tools/list", "list", json!({}))).await;
        let tools = response["result"]["tools"].as_array().unwrap();
        for (name, actions, forbidden) in [
            (
                THREAD_QUERY_TOOL_NAME,
                vec!["network.list", "router.discover", "readiness.get"],
                vec!["get_dataset_tlv", "dataset.delete", "dataset.import"],
            ),
            (
                THREAD_EXEC_TOOL_NAME,
                vec!["network.set_preferred", "router.set_preferred"],
                vec!["delete", "import", "tlv"],
            ),
            (
                MATTER_QUERY_TOOL_NAME,
                vec![
                    "readiness.get",
                    "device.list",
                    "device.diagnostics",
                    "device.ping",
                ],
                vec!["commission", "fabric", "window"],
            ),
            (
                MATTER_EXEC_TOOL_NAME,
                vec!["device.interview"],
                vec!["commission", "fabric", "window", "remove"],
            ),
        ] {
            let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
            let schema = serde_json::to_string(&tool["inputSchema"]).unwrap();
            for action in actions {
                assert!(schema.contains(action), "{name} missing {action}");
            }
            for action in forbidden {
                assert!(!schema.contains(action), "{name} exposed {action}");
            }
            assert!(schema.contains("\"additionalProperties\":false"));
        }
        task.abort();
    }

    #[tokio::test]
    async fn thread_and_matter_help_and_semantic_errors_are_safe() {
        let (endpoint, task) = endpoint().await;
        for (tool, namespace_help, action) in [
            (THREAD_QUERY_TOOL_NAME, "help.network", "network.list"),
            (THREAD_QUERY_TOOL_NAME, "help.router", "router.discover"),
            (THREAD_QUERY_TOOL_NAME, "help.readiness", "readiness.get"),
            (
                THREAD_EXEC_TOOL_NAME,
                "help.network",
                "network.set_preferred",
            ),
            (MATTER_QUERY_TOOL_NAME, "help.device", "device.list"),
            (MATTER_EXEC_TOOL_NAME, "help.device", "device.interview"),
        ] {
            let (_, response) = post(
                &endpoint,
                request(
                    "tools/call",
                    namespace_help,
                    json!({"name":tool,"arguments":{"action":namespace_help}}),
                ),
            )
            .await;
            let actions = response["result"]["structuredContent"]["actions"]
                .as_array()
                .unwrap();
            assert!(actions.iter().any(|entry| entry["action"] == action));
        }

        for (tool, action, input) in [
            (
                THREAD_QUERY_TOOL_NAME,
                "router.discover",
                json!({"duration_seconds":0}),
            ),
            (
                THREAD_EXEC_TOOL_NAME,
                "network.set_preferred",
                json!({"dataset_id":"bad/id"}),
            ),
            (
                MATTER_QUERY_TOOL_NAME,
                "device.ping",
                json!({"device_id":"bad id"}),
            ),
        ] {
            let (_, response) = post(
                &endpoint,
                request(
                    "tools/call",
                    action,
                    json!({"name":tool,"arguments":{"action":action,"input":input}}),
                ),
            )
            .await;
            assert_eq!(
                response["result"]["structuredContent"]["error"]["code"],
                "invalid_arguments"
            );
            assert_eq!(response["result"]["isError"], true);
        }
        task.abort();
    }

    #[tokio::test]
    async fn exec_discovery_has_the_exact_catalog_and_closed_input_schemas() {
        let (endpoint, task) = endpoint().await;
        let (_, discovery) = post(&endpoint, request("tools/list", "list", json!({}))).await;
        let exec = discovery["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == EXEC_TOOL_NAME)
            .unwrap();
        let schema = &exec["inputSchema"];
        let serialized = serde_json::to_string(schema).unwrap();
        for action in [
            "scene.activate",
            "scene.upsert",
            "automation.upsert",
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
            assert!(serialized.contains(action), "missing {action}");
        }
        assert_eq!(serialized.matches("additionalProperties").count(), 26);
        assert!(serialized.contains("\"additionalProperties\":false"));
        for forbidden in ["toggle", "confirmation", "preset", "source", "template"] {
            assert!(!serialized.contains(forbidden));
        }
        task.abort();
    }

    #[tokio::test]
    async fn exec_dispatches_a_fixed_control_and_never_returns_upstream_contents() {
        let (home_assistant_origin, home_assistant_task) = home_assistant().await;
        let (endpoint, task) = endpoint_for(home_assistant_origin).await;
        let (_, response) = post(
            &endpoint,
            request(
                "tools/call",
                "exec",
                json!({
                    "name":EXEC_TOOL_NAME,
                    "arguments":{
                        "action":"light.turn_on",
                        "input":{"entity_id":"light.kitchen","brightness_pct":75}
                    }
                }),
            ),
        )
        .await;
        assert_eq!(
            response["result"]["structuredContent"],
            json!({
                "action":"light.turn_on",
                "entity_id":"light.kitchen",
                "success":true
            })
        );
        assert!(
            !serde_json::to_string(&response)
                .unwrap()
                .contains("must-not-leak")
        );

        let (_, wrong_domain) = post(
            &endpoint,
            request(
                "tools/call",
                "wrong-domain",
                json!({
                    "name":EXEC_TOOL_NAME,
                    "arguments":{
                        "action":"lock.unlock",
                        "input":{"entity_id":"light.kitchen"}
                    }
                }),
            ),
        )
        .await;
        assert_eq!(
            wrong_domain["result"]["structuredContent"]["error"]["code"],
            "invalid_arguments"
        );

        let (_, unknown_field) = post(
            &endpoint,
            request(
                "tools/call",
                "unknown-field",
                json!({
                    "name":EXEC_TOOL_NAME,
                    "arguments":{
                        "action":"light.turn_off",
                        "input":{"entity_id":"light.kitchen","service":"unlock"}
                    }
                }),
            ),
        )
        .await;
        assert!(unknown_field.get("error").is_some());
        task.abort();
        home_assistant_task.abort();
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
    async fn authoring_semantic_validation_rejects_before_home_assistant_contact() {
        let (endpoint, task) = endpoint().await;
        for (tool, action, input) in [
            (
                EXEC_TOOL_NAME,
                "scene.upsert",
                json!({"config_key":"bad/key","config":{}}),
            ),
            (
                TOOL_NAME,
                "automation.validate",
                json!({"trigger":{},"triggers":[]}),
            ),
            (
                TOOL_NAME,
                "automation.traces",
                json!({"item_id":"bad/id","limit":10}),
            ),
        ] {
            let (_, response) = post(
                &endpoint,
                request(
                    "tools/call",
                    action,
                    json!({"name":tool,"arguments":{"action":action,"input":input}}),
                ),
            )
            .await;
            assert_eq!(
                response["result"]["structuredContent"]["error"]["code"],
                "invalid_arguments"
            );
        }
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
            "help.automation",
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
            ("help.automation", "automation.validate"),
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
            "automation.validate",
            "automation.traces",
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

    #[test]
    fn unfiltered_query_results_put_complete_json_in_text_and_structured_content() {
        for output in [
            json!({"action":"device.list","devices":[],"truncated":false}),
            json!({"action":"entity.list","entities":[],"truncated":false}),
            json!({"action":"state.get","entities":[]}),
            json!({"action":"history.get","history":[]}),
        ] {
            let result = query_result(output.clone()).unwrap();
            let text = result.raw["content"][0]["text"].as_str().unwrap();
            assert_eq!(serde_json::from_str::<Value>(text).unwrap(), output);
            assert_eq!(result.raw["structuredContent"], output);
        }
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
        let text = dispatched["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            dispatched["result"]["structuredContent"]
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
