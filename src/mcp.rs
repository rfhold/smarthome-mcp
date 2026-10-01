#![allow(clippy::useless_vec)]

mod authoring;

use std::{future::Future, pin::Pin, sync::Arc};

use axum::Router;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use mcp::server::ServerHandler as _;
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
        DeployInput, Error as HomeAssistantError,
        actions::{
            AutomationFromBlueprintInput, AutomationTracesInput, AutomationValidateInput,
            BlueprintGetInput, BlueprintListInput, BlueprintSaveInput, CameraSnapshotInput,
            ClimateTemperatureInput, ConfigGetInput, ConfigListInput, ConfigUpsertInput,
            ConfirmInput, Control, ControlAction, CoverPositionInput, DiscoverRoutersInput,
            EmptyInput, EntityControlInput, FanPercentageInput, GetHistoryInput, GetStatesInput,
            LightTurnOnInput, ListDevicesInput, ListEntitiesInput, ListMatterDevicesInput,
            MatterDeviceInput, MatterEmptyInput, MediaPlayerVolumeInput, SetPreferredDatasetInput,
            SetPreferredRouterInput, ThreadEmptyInput,
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
struct SmarthomeMcp {
    services: Arc<Services>,
    catalog: Arc<mcp::skills::SkillCatalog>,
}

impl SmarthomeMcp {
    fn new(services: Arc<Services>) -> Result<Self, String> {
        let catalog = crate::skills::catalog()
            .map_err(|_| "invalid embedded MCP skill catalog".to_owned())?;
        Ok(Self {
            services,
            catalog: Arc::new(catalog),
        })
    }
}

pub fn router(
    config: &OAuthConfig,
    services: Arc<Services>,
    oauth: &OAuthAuthorizationServer,
) -> Result<Router, String> {
    let handler = Arc::new(ResourceFirstMcp(SmarthomeMcp::new(services)?));
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

// Keep the published dependency's generated action handlers private behind the
// resource-first protocol boundary; the wrapper owns discovery and dispatch.
#[derive(Clone)]
struct ResourceFirstMcp(SmarthomeMcp);

const RESOURCE_CATALOGS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "entities",
        "Entities",
        "home_assistant_query",
        "entity.list",
        "entities",
    ),
    (
        "devices",
        "Devices grouped by exposed entity",
        "home_assistant_query",
        "device.list",
        "devices",
    ),
    (
        "scenes",
        "Stored scenes",
        "home_assistant_query",
        "scene.list",
        "entries",
    ),
    (
        "automations",
        "Stored automations",
        "home_assistant_query",
        "automation.list",
        "entries",
    ),
    (
        "blueprints",
        "Semantic automation blueprints",
        "home_assistant_query",
        "blueprint.list",
        "blueprints",
    ),
    (
        "thread/networks",
        "Stored Thread networks",
        "thread_query",
        "network.list",
        "networks",
    ),
    (
        "matter/devices",
        "Registered Matter devices",
        "matter_query",
        "device.list",
        "devices",
    ),
];

fn public_action(tool: &str, action: &str) -> Option<(&'static str, String)> {
    match tool {
        "home_assistant_query" if matches!(action, "history.get" | "automation.validate") => {
            Some(("query", action.into()))
        }
        "thread_query" if matches!(action, "router.discover" | "readiness.get") => {
            Some(("query", format!("thread.{action}")))
        }
        "matter_query"
            if matches!(
                action,
                "readiness.get" | "device.diagnostics" | "device.ping"
            ) =>
        {
            Some(("query", format!("matter.{action}")))
        }
        "home_assistant_exec"
            if matches!(
                action,
                "scene.activate"
                    | "smarthome_mcp.deploy"
                    | "smarthome_mcp.setup"
                    | "home_assistant.restart"
                    | "light.turn_on"
                    | "light.turn_off"
                    | "switch.turn_on"
                    | "switch.turn_off"
                    | "fan.turn_on"
                    | "fan.turn_off"
                    | "fan.set_percentage"
                    | "cover.open"
                    | "cover.close"
                    | "cover.stop"
                    | "cover.set_position"
                    | "climate.turn_on"
                    | "climate.turn_off"
                    | "climate.set_temperature"
                    | "media_player.turn_on"
                    | "media_player.turn_off"
                    | "media_player.play"
                    | "media_player.pause"
                    | "media_player.stop"
                    | "media_player.volume_set"
                    | "lock.lock"
                    | "lock.unlock"
            ) =>
        {
            Some(("execute", action.into()))
        }
        "thread_exec" if matches!(action, "network.set_preferred" | "router.set_preferred") => {
            Some(("execute", format!("thread.{action}")))
        }
        "matter_exec" if action == "device.interview" => {
            Some(("execute", format!("matter.{action}")))
        }
        _ => None,
    }
}

impl ResourceFirstMcp {
    async fn catalog_value(
        &self,
        path: &str,
        context: ServerContext,
    ) -> ServerResult<serde_json::Value> {
        let (_, _, tool, action, field) = RESOURCE_CATALOGS
            .iter()
            .find(|entry| entry.0 == path)
            .ok_or_else(|| ServerError::invalid_params("unknown resource catalog"))?;
        let input = if *action == "network.list" {
            json!({})
        } else {
            json!({"limit":100})
        };
        let result = self
            .0
            .call_tool(
                mcp::McpToolCall::new(*tool, json!({"action":action,"input":input})),
                context,
            )
            .await?;
        let mut value = resource_output(result)?;
        let entries = value[*field]
            .as_array_mut()
            .ok_or_else(|| ServerError::internal("invalid resource catalog"))?;
        for entry in entries {
            let id = match path {
                "entities" => entry["entity_id"].as_str(),
                "devices" => entry["entities"][0]["entity_id"].as_str(),
                "scenes" | "automations" => entry["config_key"].as_str(),
                "blueprints" => entry["path"].as_str(),
                "thread/networks" => entry["dataset_id"].as_str(),
                "matter/devices" => entry["device_id"].as_str(),
                _ => None,
            }
            .ok_or_else(|| ServerError::internal("invalid resource catalog"))?
            .to_owned();
            let uri = item_uri(path, &id);
            entry["uri"] = json!(uri);
            if path == "entities" {
                entry["state_uri"] = json!(item_uri("states", &id));
                if id.starts_with("camera.") {
                    entry["camera_uri"] = json!(item_uri("cameras", &id));
                }
            } else if path == "automations" {
                entry["traces_uri"] = json!(format!("{}/traces", item_uri(path, &id)));
            }
        }
        Ok(value)
    }

    async fn dynamic_resource(
        &self,
        uri: &str,
        context: ServerContext,
    ) -> ServerResult<mcp::McpResourceResult> {
        let path = uri
            .strip_prefix("smarthome://")
            .ok_or_else(|| ServerError::invalid_params("invalid resource URI"))?;
        if RESOURCE_CATALOGS.iter().any(|entry| entry.0 == path) {
            return bounded_text_resource(mcp::McpResourceResult::text(
                uri,
                "application/json",
                resource_json_text(&self.catalog_value(path, context).await?)?,
            ));
        }
        let (catalog, encoded_id, traces) = if let Some(id) = path
            .strip_prefix("automations/")
            .and_then(|rest| rest.strip_suffix("/traces"))
        {
            ("automations", id, true)
        } else {
            let (catalog, id) = if let Some(id) = path.strip_prefix("thread/networks/") {
                ("thread/networks", id)
            } else if let Some(id) = path.strip_prefix("matter/devices/") {
                ("matter/devices", id)
            } else {
                path.split_once('/')
                    .ok_or_else(|| ServerError::invalid_params("invalid resource URI"))?
            };
            (catalog, id, false)
        };
        let id = decode_item_id(encoded_id)?;
        if item_uri(catalog, &id)
            != if traces {
                uri.strip_suffix("/traces")
                    .ok_or_else(|| ServerError::invalid_params("invalid resource URI"))?
            } else {
                uri
            }
        {
            return Err(ServerError::invalid_params("noncanonical resource URI"));
        }
        if matches!(catalog, "scenes" | "automations" | "blueprints") && !traces {
            authoring::target(uri)?;
            return self.authoring_resource(uri, context).await;
        }
        let (tool, action, input) = match catalog {
            "entities" | "states" => (
                "home_assistant_query",
                "state.get",
                json!({"entity_ids":[id]}),
            ),
            "cameras" => (
                "home_assistant_query",
                "camera.snapshot",
                json!({"entity_id":id}),
            ),
            "scenes" | "automations" | "blueprints" | "devices" | "thread/networks"
            | "matter/devices" => {
                let value = self.catalog_value(catalog, context.clone()).await?;
                let field = RESOURCE_CATALOGS
                    .iter()
                    .find(|entry| entry.0 == catalog)
                    .ok_or_else(|| ServerError::internal("invalid resource catalog"))?
                    .4;
                let canonical = item_uri(catalog, &id);
                let entry = value[field]
                    .as_array()
                    .and_then(|entries| entries.iter().find(|entry| entry["uri"] == canonical))
                    .ok_or_else(|| ServerError::resource_not_found("unavailable resource"))?;
                if matches!(catalog, "devices" | "thread/networks" | "matter/devices") {
                    return bounded_text_resource(mcp::McpResourceResult::text(
                        uri,
                        "application/json",
                        resource_json_text(entry)?,
                    ));
                }
                match catalog {
                    "scenes" => (
                        "home_assistant_query",
                        "scene.get",
                        json!({"config_key":id}),
                    ),
                    "automations" if traces => (
                        "home_assistant_query",
                        "automation.traces",
                        json!({"item_id":id,"limit":50}),
                    ),
                    "automations" => (
                        "home_assistant_query",
                        "automation.get",
                        json!({"config_key":id}),
                    ),
                    _ => ("home_assistant_query", "blueprint.get", json!({"path":id})),
                }
            }
            _ => return Err(ServerError::resource_not_found("unavailable resource")),
        };
        let result = self
            .0
            .call_tool(
                mcp::McpToolCall::new(tool, json!({"action":action,"input":input})),
                context,
            )
            .await?;
        if catalog == "cameras" {
            if result.raw["isError"] == true {
                return Err(ServerError::resource_not_found("unavailable camera"));
            }
            let image = result.raw["content"]
                .as_array()
                .and_then(|blocks| blocks.iter().find(|block| block["type"] == "image"))
                .ok_or_else(|| ServerError::internal("invalid camera resource"))?;
            return Ok(mcp::McpResourceResult::new(
                json!({"contents":[{"uri":uri,"mimeType":image["mimeType"],"blob":image["data"]}]}),
            ));
        }
        let value = resource_output(result)?;
        let text = resource_json_text(&value)?;
        if text.len() > 256 * 1024 {
            return Err(ServerError::internal("resource text exceeds size limit"));
        }
        let result = mcp::McpResourceResult::text(uri, "application/json", &text);
        bounded_text_resource(result)
    }
}

const MAX_TEXT_RESOURCE_BYTES: usize = 2 * 1024 * 1024;

fn resource_json_text(value: &serde_json::Value) -> ServerResult<String> {
    serde_json::to_string_pretty(value)
        .map_err(|_| ServerError::internal("unable to serialize resource text"))
}

fn bounded_text_resource(result: mcp::McpResourceResult) -> ServerResult<mcp::McpResourceResult> {
    let serialized = serde_json::to_vec(&result.raw)
        .map_err(|_| ServerError::internal("unable to serialize resource response"))?;
    if serialized.len() > MAX_TEXT_RESOURCE_BYTES {
        return Err(ServerError::internal(
            "resource response exceeds size limit",
        ));
    }
    Ok(result)
}

fn resource_output(result: McpToolResult) -> ServerResult<serde_json::Value> {
    if result.raw["isError"] == true {
        return Err(ServerError::resource_not_found("unavailable resource"));
    }
    result
        .raw
        .get("structuredContent")
        .cloned()
        .ok_or_else(|| ServerError::internal("invalid resource response"))
}

fn item_uri(catalog: &str, id: &str) -> String {
    let mut encoded = String::new();
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("smarthome://{catalog}/{encoded}")
}

fn decode_item_id(encoded: &str) -> ServerResult<String> {
    if encoded.is_empty() || encoded.len() > 768 {
        return Err(ServerError::invalid_params("invalid resource URI"));
    }
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|part| std::str::from_utf8(part).ok())
                .and_then(|part| u8::from_str_radix(part, 16).ok())
                .ok_or_else(|| ServerError::invalid_params("invalid resource URI"))?;
            decoded.push(hex);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let id = String::from_utf8(decoded)
        .map_err(|_| ServerError::invalid_params("invalid resource URI"))?;
    if id.chars().any(char::is_control) {
        return Err(ServerError::invalid_params("invalid resource URI"));
    }
    Ok(id)
}

impl mcp::server::ServerHandler for ResourceFirstMcp {
    fn server_info(&self) -> mcp::server::ServerInfo {
        self.0.server_info()
    }
    fn capabilities(&self) -> mcp::server::ServerCapabilities {
        self.0.capabilities()
    }
    fn skill_catalog(&self) -> Option<Arc<mcp::SkillCatalog>> {
        Some(self.0.catalog.clone())
    }
    fn instructions(&self) -> Option<String> {
        Some("Read smarthome:// catalogs and resource templates for current data. Use query for probes and execute for physical/lifecycle commands. Create and direct revision-checked edit use existing native APIs under a single-writer assumption; checks are best-effort, not atomic. Never retry an unknown mutation outcome. No destroy tool exists.".into())
    }
    fn list_tools(
        &self,
        cursor: Option<String>,
        context: ServerContext,
    ) -> mcp::server::BoxFuture<ServerResult<mcp::McpToolList>> {
        let this = self.clone();
        Box::pin(async move {
            let private = this.0.list_tools(cursor, context).await?;
            let mut tools = Vec::new();
            for name in ["query", "execute"] {
                let mut actions = Vec::new();
                let mut inputs = Vec::new();
                for tool in &private.tools {
                    let old_actions = tool.input_schema["properties"]["action"]["enum"]
                        .as_array()
                        .unwrap();
                    let old_inputs = tool.input_schema["properties"]["input"]["oneOf"]
                        .as_array()
                        .unwrap();
                    for (action, input) in old_actions.iter().zip(old_inputs) {
                        let action = action.as_str().unwrap();
                        if let Some((target, public)) = public_action(&tool.name, action)
                            && target == name
                        {
                            let mut input = input.clone();
                            input["description"] = json!(format!("Use with action `{public}`."));
                            actions.push(public.clone());
                            inputs.push(input);
                        }
                    }
                }
                let schema = json!({"type":"object","additionalProperties":false,"required":["action"],"properties":{
                    "action":{"type":"string","enum":actions},
                    "input":{"oneOf":inputs},
                    "filter":{"type":["string","null"],"description":"Optional jq-compatible output filter."}
                }});
                let mut tool = mcp::progressive::tool_definition(
                    name,
                    if name == "query" {
                        "Bounded temporal history, validation, discovery, ping and computed diagnostics."
                    } else {
                        "Fixed physical, lifecycle, Thread selection and Matter maintenance commands."
                    },
                    schema,
                );
                tool.annotations = Some(
                    json!({"readOnlyHint":name=="query","destructiveHint":name=="execute","idempotentHint":name=="query","openWorldHint":true}),
                );
                tools.push(tool);
            }
            tools.extend(authoring::tools());
            Ok(mcp::McpToolList {
                tools,
                next_cursor: None,
            })
        })
    }
    fn call_tool(
        &self,
        call: mcp::McpToolCall,
        context: ServerContext,
    ) -> mcp::server::BoxFuture<ServerResult<McpToolResult>> {
        let this = self.clone();
        Box::pin(async move {
            if matches!(call.name.as_str(), "create" | "edit") {
                return this.author(&call.name, &call.arguments, context).await;
            }
            let action = call.arguments["action"]
                .as_str()
                .ok_or_else(|| ServerError::invalid_params("missing action"))?;
            for tool in [
                "home_assistant_query",
                "home_assistant_exec",
                "thread_query",
                "thread_exec",
                "matter_query",
                "matter_exec",
            ] {
                let private_action = if tool.starts_with("thread_") {
                    action.strip_prefix("thread.")
                } else if tool.starts_with("matter_") {
                    action.strip_prefix("matter.")
                } else {
                    Some(action)
                };
                if let Some(private_action) = private_action
                    && public_action(tool, private_action)
                        .is_some_and(|(name, public)| name == call.name && public == action)
                {
                    let mut arguments = call.arguments.clone();
                    arguments["action"] = json!(private_action);
                    return this
                        .0
                        .call_tool(
                            mcp::McpToolCall {
                                name: tool.into(),
                                arguments,
                                progress_token: call.progress_token,
                            },
                            context,
                        )
                        .await;
                }
            }
            Err(ServerError::invalid_params("unknown tool or action"))
        })
    }
    fn list_resources(
        &self,
        cursor: Option<String>,
        context: ServerContext,
    ) -> mcp::server::BoxFuture<ServerResult<mcp::McpResourceList>> {
        let this = self.clone();
        Box::pin(async move {
            let mut page = this.0.list_resources(cursor, context).await?;
            let extra = mcp::McpResourceList::parse(&json!({"resources":RESOURCE_CATALOGS.iter().map(|(path,name,_,_,_)| json!({"uri":format!("smarthome://{path}"),"name":name,"mimeType":"application/json"})).collect::<Vec<_>>()})).map_err(|_| ServerError::internal("invalid resource definitions"))?;
            page.resources.extend(extra.resources);
            Ok(page)
        })
    }
    fn list_resource_templates(
        &self,
        cursor: Option<String>,
        _: ServerContext,
    ) -> mcp::server::BoxFuture<ServerResult<mcp::McpResourceTemplateList>> {
        Box::pin(async move {
            if cursor.is_some() {
                return Err(ServerError::invalid_params(
                    "invalid resource template cursor",
                ));
            }
            let paths = [
                "entities/{entity_id}",
                "states/{entity_id}",
                "devices/{entity_id}",
                "cameras/{entity_id}",
                "scenes/{config_key}",
                "automations/{config_key}",
                "automations/{config_key}/traces",
                "blueprints/{path}",
                "thread/networks/{dataset_id}",
                "matter/devices/{device_id}",
            ];
            mcp::McpResourceTemplateList::parse(&json!({"resourceTemplates":paths.iter().map(|path| json!({"uriTemplate":format!("smarthome://{path}"),"name":path})).collect::<Vec<_>>()})).map_err(|_| ServerError::internal("invalid resource templates"))
        })
    }
    fn read_resource(
        &self,
        uri: String,
        context: ServerContext,
    ) -> mcp::server::BoxFuture<ServerResult<mcp::McpResourceResult>> {
        let this = self.clone();
        Box::pin(async move {
            if uri.starts_with("skill://") {
                return this.0.read_resource(uri, context).await;
            }
            this.dynamic_resource(&uri, context).await
        })
    }
}

#[mcp::progressive_server(
    skills = self.catalog,
    name = "smarthome-mcp",
    version = "0.2.0",
    description = "Authenticated, policy-bounded smart-home tools.",
    tool(
        name = "home_assistant_query",
        description = "Read bounded Home Assistant entity and editor-managed config data, validate automation sections, and summarize automation traces.",
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
        namespace(automation, description = "Read stored automations, validate sections, and summarize traces."),
        namespace(blueprint, description = "Read bounded automation blueprints."),
        namespace(scene, description = "Read stored Home Assistant scenes.")
    ),
    tool(
        name = "home_assistant_exec",
        description = "Operate Assist-exposed entities, manage bounded native configs, and deploy the embedded smarthome_mcp integration.",
        annotations = json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": true
        }),
        namespace(scene, description = "Activate or upsert Home Assistant scenes."),
        namespace(automation, description = "Upsert Home Assistant automations or create one from a blueprint."),
        namespace(blueprint, description = "Save automation blueprints."),
        namespace(smarthome_mcp, description = "Deploy or set up the smarthome_mcp integration."),
        namespace(home_assistant, description = "Run separately confirmed Home Assistant lifecycle operations."),
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
    /// List bounded automation blueprint metadata and input definitions.
    #[action(tool = "home_assistant_query", name = "blueprint.list")]
    async fn list_blueprints(
        &self,
        input: BlueprintListInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        dispatch_query(
            self.services
                .home_assistant
                .list_blueprints(&match input.validate() {
                    Ok(v) => v,
                    Err(()) => {
                        return Ok(tool_error(
                            "list blueprints",
                            HomeAssistantError::InvalidArguments,
                        ));
                    }
                }),
            "list blueprints",
            context,
        )
        .await
    }

    /// Get bounded semantic YAML for one automation blueprint.
    #[action(tool = "home_assistant_query", name = "blueprint.get")]
    async fn get_blueprint(
        &self,
        input: BlueprintGetInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        dispatch_query(
            self.services
                .home_assistant
                .get_blueprint(&match input.validate() {
                    Ok(v) => v,
                    Err(()) => {
                        return Ok(tool_error(
                            "get blueprint",
                            HomeAssistantError::InvalidArguments,
                        ));
                    }
                }),
            "get blueprint",
            context,
        )
        .await
    }

    /// Replace one automation blueprint with bounded semantic YAML.
    #[action(tool = "home_assistant_exec", name = "blueprint.save")]
    async fn save_blueprint(
        &self,
        input: BlueprintSaveInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        dispatch_exec(
            self.services
                .home_assistant
                .save_blueprint(&match input.validate() {
                    Ok(v) => v,
                    Err(()) => {
                        return Ok(tool_error(
                            "save blueprint",
                            HomeAssistantError::InvalidArguments,
                        ));
                    }
                }),
            "save blueprint",
            context,
        )
        .await
    }

    /// Preflight blueprint substitution, then create a compact automation.
    #[action(tool = "home_assistant_exec", name = "automation.from_blueprint")]
    async fn automation_from_blueprint(
        &self,
        input: AutomationFromBlueprintInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        dispatch_exec(
            self.services
                .home_assistant
                .automation_from_blueprint(&match input.validate() {
                    Ok(v) => v,
                    Err(()) => {
                        return Ok(tool_error(
                            "create automation from blueprint",
                            HomeAssistantError::InvalidArguments,
                        ));
                    }
                }),
            "create automation from blueprint",
            context,
        )
        .await
    }

    /// Start only the smarthome_mcp integration config flow when needed.
    #[action(tool = "home_assistant_exec", name = "smarthome_mcp.setup")]
    async fn setup_smarthome_mcp(
        &self,
        _: EmptyInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        dispatch_exec(
            self.services.home_assistant.setup_smarthome_mcp(),
            "set up smarthome_mcp",
            context,
        )
        .await
    }

    /// Deploy the embedded smarthome_mcp integration after exact confirmation.
    #[action(tool = "home_assistant_exec", name = "smarthome_mcp.deploy")]
    async fn deploy_smarthome_mcp(
        &self,
        input: DeployInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        if let Err(error) = input.validate() {
            return Ok(error.into_tool_error().into_mcp_result());
        }
        let result = tokio::select! {
            result = self.services.component_deployer.deploy() => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => Ok(control_result(
                serde_json::to_value(output).expect("deploy output is serializable"),
            )),
            Err(error) => Ok(error.into_tool_error().into_mcp_result()),
        }
    }

    /// Restart Home Assistant only after exact boolean confirmation.
    #[action(tool = "home_assistant_exec", name = "home_assistant.restart")]
    async fn restart_home_assistant(
        &self,
        input: ConfirmInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        if input.validate().is_err() {
            return Ok(tool_error(
                "restart Home Assistant",
                HomeAssistantError::InvalidArguments,
            ));
        }
        dispatch_exec(
            self.services.home_assistant.restart_home_assistant(),
            "restart Home Assistant",
            context,
        )
        .await
    }

    /// List editor-managed scenes using bounded state metadata. YAML-only scenes
    /// without a safe stored config ID are not included.
    #[action(tool = "home_assistant_query", name = "scene.list")]
    async fn list_scenes(
        &self,
        input: ConfigListInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "list scenes",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.list_scenes(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("list scenes", error)),
        }
    }

    /// Get one complete native editor-managed scene configuration by stable key.
    #[action(tool = "home_assistant_query", name = "scene.get")]
    async fn get_scene(
        &self,
        input: ConfigGetInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "get scene",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.get_scene(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("get scene", error)),
        }
    }

    /// List editor-managed automations using bounded state metadata. YAML-only
    /// automations without a safe stored config ID are not included.
    #[action(tool = "home_assistant_query", name = "automation.list")]
    async fn list_automations(
        &self,
        input: ConfigListInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "list automations",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.list_automations(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("list automations", error)),
        }
    }

    /// Get one complete native editor-managed automation configuration by stable key.
    #[action(tool = "home_assistant_query", name = "automation.get")]
    async fn get_automation(
        &self,
        input: ConfigGetInput,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let query = match input.validate() {
            Ok(query) => query,
            Err(()) => {
                return Ok(tool_error(
                    "get automation",
                    HomeAssistantError::InvalidArguments,
                ));
            }
        };
        let result = tokio::select! {
            result = self.services.home_assistant.get_automation(&query) => result,
            () = context.cancelled() => return Err(ServerError::internal("request cancelled")),
        };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error("get automation", error)),
        }
    }

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

fn dispatch_query<'a>(
    future: impl Future<Output = Result<serde_json::Value, HomeAssistantError>> + Send + 'a,
    description: &'static str,
    context: ServerContext,
) -> Pin<Box<dyn Future<Output = ServerResult<McpToolResult>> + Send + 'a>> {
    Box::pin(async move {
        let result = tokio::select! { result = future => result, () = context.cancelled() => return Err(ServerError::internal("request cancelled")) };
        match result {
            Ok(output) => query_result(output),
            Err(error) => Ok(tool_error(description, error)),
        }
    })
}

fn dispatch_exec<'a>(
    future: impl Future<Output = Result<serde_json::Value, HomeAssistantError>> + Send + 'a,
    description: &'static str,
    context: ServerContext,
) -> Pin<Box<dyn Future<Output = ServerResult<McpToolResult>> + Send + 'a>> {
    Box::pin(async move {
        let result = tokio::select! { result = future => result, () = context.cancelled() => return Err(ServerError::internal("request cancelled")) };
        match result {
            Ok(output) => Ok(control_result(output)),
            Err(error) => Ok(tool_error(description, error)),
        }
    })
}

fn control_result(output: serde_json::Value) -> McpToolResult {
    let action = output["action"].as_str().unwrap_or("control");
    let text = if action == "smarthome_mcp.deploy" {
        format!("Completed {action}.")
    } else {
        format!("Acknowledged {action}; state, reload completion, and readiness are not verified.")
    };
    McpToolResult::new(json!({
        "content": [{"type":"text","text":text}],
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
        extract::{State, WebSocketUpgrade, ws},
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

    use crate::{
        config::Secret,
        integrations::home_assistant::{ComponentDeployer, HomeAssistantClient},
    };

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

    async fn resource_endpoint(home_assistant_origin: url::Url) -> (String, JoinHandle<()>) {
        resource_endpoint_with_timeout(home_assistant_origin, Duration::from_secs(2)).await
    }

    async fn resource_endpoint_with_timeout(
        home_assistant_origin: url::Url,
        timeout: Duration,
    ) -> (String, JoinHandle<()>) {
        let client = HomeAssistantClient::for_test(
            home_assistant_origin,
            Secret("test-token".into()),
            timeout,
        );
        let handler = Arc::new(ResourceFirstMcp(
            SmarthomeMcp::new(Arc::new(Services::new(client))).unwrap(),
        ));
        let (origin, task) = serve(mcp::server::streamable_http_router(handler)).await;
        (format!("{origin}/mcp"), task)
    }

    #[tokio::test]
    async fn resource_first_discovery_and_dispatch_are_closed() {
        let (endpoint, task) =
            resource_endpoint(url::Url::parse("http://127.0.0.1:1/").unwrap()).await;
        let (_, response) = post(&endpoint, request("tools/list", "tools", json!({}))).await;
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 4);
        let query = &tools[0];
        let execute = &tools[1];
        assert_eq!(query["name"], "query");
        assert_eq!(execute["name"], "execute");
        assert_eq!(
            query["inputSchema"]["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            7
        );
        assert_eq!(
            execute["inputSchema"]["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            29
        );
        for tool in tools {
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
            let schema = tool["inputSchema"].to_string();
            for forbidden in [
                "entity.list",
                "state.get",
                "blueprint.save",
                "automation.upsert",
                "scene.upsert",
                "automation.from_blueprint",
            ] {
                assert!(!schema.contains(forbidden), "{schema}");
            }
        }
        for (name, action, input) in [
            ("query", "entity.list", json!({})),
            (
                "query",
                "automation.traces",
                json!({"item_id":"arrival_lights"}),
            ),
            (
                "execute",
                "scene.upsert",
                json!({"config_key":"a","config":{}}),
            ),
            ("home_assistant_query", "history.get", json!({})),
            ("edit", "scene.edit", json!({})),
            ("create", "scene.create", json!({})),
            ("destroy", "scene.destroy", json!({})),
            ("execute", "smarthome_mcp.deploy", json!({"confirm":false})),
            (
                "execute",
                "thread.network.set_preferred",
                json!({"dataset_id":"../unsafe"}),
            ),
        ] {
            let (_, result) = post(
                &endpoint,
                request(
                    "tools/call",
                    "call",
                    json!({"name":name,"arguments":{"action":action,"input":input}}),
                ),
            )
            .await;
            assert!(
                result.get("error").is_some() || result["result"]["isError"] == true,
                "{result}"
            );
        }
        let (_, resources) =
            post(&endpoint, request("resources/list", "resources", json!({}))).await;
        assert_eq!(
            resources["result"]["resources"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|resource| resource["uri"]
                    .as_str()
                    .unwrap()
                    .starts_with("smarthome://"))
                .count(),
            7
        );
        let (_, templates) = post(
            &endpoint,
            request("resources/templates/list", "templates", json!({})),
        )
        .await;
        assert_eq!(
            templates["result"]["resourceTemplates"]
                .as_array()
                .unwrap()
                .len(),
            10
        );
        task.abort();
    }

    #[test]
    fn resource_uris_round_trip_with_strict_canonical_encoding() {
        for id in [
            "sensor.allowed",
            "vendor/motion.yaml",
            "device #1",
            "caf\u{e9}",
        ] {
            let uri = item_uri("blueprints", id);
            assert_eq!(
                decode_item_id(uri.strip_prefix("smarthome://blueprints/").unwrap()).unwrap(),
                id
            );
        }
        for invalid in ["", "%", "%ZZ", "%FF", "%00", "a\n", &"a".repeat(769)] {
            assert!(decode_item_id(invalid).is_err());
        }
        assert_eq!(
            item_uri("blueprints", "vendor/motion.yaml"),
            "smarthome://blueprints/vendor%2Fmotion.yaml"
        );
    }

    #[test]
    fn final_text_resource_bound_includes_escaping_and_metadata() {
        let uri = "smarthome://devices/sensor.allowed";
        let empty = mcp::McpResourceResult::text(uri, "application/json", "");
        let overhead = serde_json::to_vec(&empty.raw).unwrap().len();
        let at_limit = mcp::McpResourceResult::text(
            uri,
            "application/json",
            "x".repeat(MAX_TEXT_RESOURCE_BYTES - overhead),
        );
        assert_eq!(
            serde_json::to_vec(&at_limit.raw).unwrap().len(),
            MAX_TEXT_RESOURCE_BYTES
        );
        assert!(bounded_text_resource(at_limit.clone()).is_ok());
        let mut with_metadata = at_limit;
        with_metadata.raw["contents"][0]["_meta"] = json!({"revision":"extra"});
        assert!(bounded_text_resource(with_metadata).is_err());
        assert!(
            bounded_text_resource(mcp::McpResourceResult::text(
                uri,
                "application/json",
                "x".repeat(MAX_TEXT_RESOURCE_BYTES - overhead + 1),
            ))
            .is_err()
        );
        let escaped = "\"".repeat(MAX_TEXT_RESOURCE_BYTES / 2);
        assert!(escaped.len() < MAX_TEXT_RESOURCE_BYTES);
        assert!(
            bounded_text_resource(mcp::McpResourceResult::text(
                uri,
                "application/json",
                escaped
            ))
            .is_err()
        );
    }

    #[tokio::test]
    async fn blueprint_catalog_rejects_final_pretty_expansion_without_payload_leak() {
        let input = json!({"a":{"b":{"c":{"d":{"e":vec![0;100_000]}}}}});
        assert!(serde_json::to_vec(&input).unwrap().len() < 256 * 1024);
        let upstream = json!({"vendor/large.yaml":{"metadata":{
            "domain":"automation","name":"private-size-marker","input":input
        }}});
        assert!(serde_json::to_vec(&upstream).unwrap().len() < 1024 * 1024);
        let response = upstream.clone();
        let router = Router::new().route("/api/websocket", get(move |upgrade: WebSocketUpgrade| {
            let response = response.clone();
            async move { upgrade.on_upgrade(move |mut socket| async move {
                use axum::extract::ws::Message;
                use futures_util::StreamExt as _;
                socket.send(Message::Text(json!({"type":"auth_required"}).to_string().into())).await.unwrap();
                socket.next().await.unwrap().unwrap();
                socket.send(Message::Text(json!({"type":"auth_ok"}).to_string().into())).await.unwrap();
                let Message::Text(command) = socket.next().await.unwrap().unwrap() else { panic!("expected command"); };
                let command: Value = serde_json::from_str(&command).unwrap();
                assert_eq!(command["type"], "blueprint/list");
                socket.send(Message::Text(json!({"id":command["id"],"type":"result","success":true,"result":response}).to_string().into())).await.unwrap();
            }) }
        }));
        let (origin, ha) = serve(router).await;
        let client = HomeAssistantClient::for_test(
            url::Url::parse(&origin).unwrap(),
            Secret("test-token".into()),
            Duration::from_secs(2),
        );
        let query = BlueprintListInput {
            search: None,
            limit: Some(100),
        }
        .validate()
        .unwrap();
        let compact = client.list_blueprints(&query).await.unwrap();
        assert_eq!(compact["blueprints"].as_array().unwrap().len(), 1);
        assert!(serde_json::to_vec(&compact).unwrap().len() < MAX_TEXT_RESOURCE_BYTES);
        assert!(resource_json_text(&compact).unwrap().len() > MAX_TEXT_RESOURCE_BYTES);
        let (endpoint, task) = resource_endpoint(url::Url::parse(&origin).unwrap()).await;
        let (_, result, wire_size) = post_with_wire_size(
            &endpoint,
            request(
                "resources/read",
                "oversize",
                json!({"uri":"smarthome://blueprints"}),
            ),
        )
        .await;
        assert!(result.get("error").is_some(), "{result}");
        assert!(result.get("result").is_none());
        assert!(wire_size < 8 * 1024);
        assert!(!result.to_string().contains("private-size-marker"));
        assert!(!result.to_string().contains("large.yaml"));
        task.abort();
        ha.abort();
    }

    #[tokio::test]
    async fn resource_first_reads_linked_entities_camera_and_immutable_skills() {
        let (ha_origin, ha) = home_assistant().await;
        let (endpoint, task) = resource_endpoint(ha_origin).await;
        for (name, action, input) in [
            (
                "query",
                "automation.validate",
                json!({"triggers":[],"actions":[]}),
            ),
            ("query", "matter.readiness.get", json!({})),
            (
                "execute",
                "light.turn_on",
                json!({"entity_id":"light.kitchen"}),
            ),
            (
                "execute",
                "thread.network.set_preferred",
                json!({"dataset_id":"dataset-a"}),
            ),
        ] {
            let (_, result) = post(
                &endpoint,
                request(
                    "tools/call",
                    "dispatch",
                    json!({"name":name,"arguments":{"action":action,"input":input}}),
                ),
            )
            .await;
            assert!(
                result.get("error").is_none() && result["result"]["isError"] != true,
                "{action}: {result}"
            );
            assert!(!result.to_string().contains("must-not-leak"));
        }
        let (_, catalog) = post(
            &endpoint,
            request(
                "resources/read",
                "catalog",
                json!({"uri":"smarthome://entities"}),
            ),
        )
        .await;
        let value: Value =
            serde_json::from_str(catalog["result"]["contents"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(
            value["entities"][0]["uri"],
            "smarthome://entities/sensor.allowed"
        );
        assert_eq!(
            value["entities"][0]["state_uri"],
            "smarthome://states/sensor.allowed"
        );
        let (_, state) = post(
            &endpoint,
            request(
                "resources/read",
                "state",
                json!({"uri":"smarthome://states/sensor.allowed"}),
            ),
        )
        .await;
        assert!(
            state["result"]["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("sensor.allowed")
        );
        let (_, camera) = post(
            &endpoint,
            request(
                "resources/read",
                "camera",
                json!({"uri":"smarthome://cameras/camera.front_door"}),
            ),
        )
        .await;
        assert_eq!(camera["result"]["contents"][0]["mimeType"], "image/png");
        assert_eq!(
            STANDARD
                .decode(camera["result"]["contents"][0]["blob"].as_str().unwrap())
                .unwrap(),
            b"\x89PNG\r\n\x1a\nframe"
        );
        let (_, skill) = post(
            &endpoint,
            request(
                "resources/read",
                "skill",
                json!({"uri":"skill://smarthome/inspect-home/SKILL.md"}),
            ),
        )
        .await;
        assert!(
            skill["result"]["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Inspect Home")
        );
        for uri in [
            "smarthome://states/sensor.foreign",
            "smarthome://states/%73ensor.allowed",
            "smarthome://states/sensor.allowed?filter=x",
            "smarthome://blueprints/../unsafe.yaml",
            "smarthome://unknown",
            "https://example.com/",
        ] {
            let (_, result) = post(
                &endpoint,
                request("resources/read", "invalid", json!({"uri":uri})),
            )
            .await;
            assert!(result.get("error").is_some(), "{uri}: {result}");
        }
        task.abort();
        ha.abort();
    }

    #[tokio::test]
    async fn resource_first_config_reads_use_native_admin_boundary_and_digest_text() {
        let present = Arc::new(AtomicBool::new(true));
        let reads = Arc::new(AtomicUsize::new(0));
        let states_present = present.clone();
        let config_reads = reads.clone();
        let router = Router::new()
            .route("/api/states", get(move || {
                let present = states_present.clone();
                async move { Json(if present.load(Ordering::Relaxed) { json!([{"entity_id":"scene.evening","attributes":{"id":"evening_scene","friendly_name":"Evening"}}]) } else { json!([]) }) }
            }))
            .route("/api/config/scene/config/evening_scene", get(move || {
                let reads = config_reads.clone();
                async move { reads.fetch_add(1, Ordering::Relaxed); Json(json!({"name":"Evening","id":"evening_scene"})) }
            }));
        let (origin, ha) = serve(router).await;
        let (endpoint, task) = resource_endpoint(url::Url::parse(&origin).unwrap()).await;
        let (_, result) = post(
            &endpoint,
            request(
                "resources/read",
                "config",
                json!({"uri":"smarthome://scenes/evening_scene"}),
            ),
        )
        .await;
        let content = &result["result"]["contents"][0];
        let text = content["text"].as_str().unwrap();
        assert_eq!(
            text,
            "{\n  \"id\": \"evening_scene\",\n  \"name\": \"Evening\"\n}"
        );
        use sha2::{Digest as _, Sha256};
        let digest = Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(content["_meta"]["revision"], format!("sha256:{digest}"));
        assert_eq!(content["_meta"]["editable"], true);
        assert_eq!(content["_meta"]["concurrency"], "single-writer");
        present.store(false, Ordering::Relaxed);
        let (_, removed) = post(
            &endpoint,
            request(
                "resources/read",
                "removed",
                json!({"uri":"smarthome://scenes/evening_scene"}),
            ),
        )
        .await;
        assert!(removed.get("error").is_none());
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        task.abort();
        ha.abort();
    }

    async fn endpoint_for(home_assistant_origin: url::Url) -> (String, JoinHandle<()>) {
        endpoint_for_with_timeout(home_assistant_origin, Duration::from_millis(100)).await
    }

    #[tokio::test]
    async fn native_authoring_http_roundtrip_without_component_bridge() {
        let config = Arc::new(Mutex::new(Some(
            json!({"id":"evening_scene","name":"Evening"}),
        )));
        let writes = Arc::new(AtomicUsize::new(0));
        let get_config = config.clone();
        let put_config = config.clone();
        let put_writes = writes.clone();
        let router = Router::new().route(
            "/api/config/scene/config/evening_scene",
            get(move || {
                let config = get_config.clone();
                async move {
                    match config.lock().unwrap().clone() {
                        Some(config) => (StatusCode::OK, Json(config)),
                        None => (StatusCode::NOT_FOUND, Json(json!({}))),
                    }
                }
            })
            .post(move |Json(value): Json<serde_json::Value>| {
                let config = put_config.clone();
                let writes = put_writes.clone();
                async move {
                    *config.lock().unwrap() = Some(value);
                    writes.fetch_add(1, Ordering::Relaxed);
                    Json(json!({"result":"ok"}))
                }
            }),
        );
        let (origin, ha) = serve(router).await;
        let (endpoint, task) = resource_endpoint(url::Url::parse(&origin).unwrap()).await;
        let (_, discovery) = post(&endpoint, request("tools/list", "list", json!({}))).await;
        assert_eq!(
            discovery["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["query", "execute", "create", "edit"]
        );
        let (_, read) = post(
            &endpoint,
            request(
                "resources/read",
                "read",
                json!({"uri":"smarthome://scenes/evening_scene"}),
            ),
        )
        .await;
        let item = &read["result"]["contents"][0];
        assert_eq!(item["_meta"]["editable"], true);
        assert_eq!(item["_meta"]["revision_check"], "best-effort");
        let args = json!({"uri":"smarthome://scenes/evening_scene","expected_revision":item["_meta"]["revision"],"edits":[{"operation":"replace","old_text":"Evening","new_text":"Night"},{"operation":"insert","placement":"after","anchor":"Night","text":"!"}]});
        let (_, edited) = post(
            &endpoint,
            request(
                "tools/call",
                "edit",
                json!({"name":"edit","arguments":args}),
            ),
        )
        .await;
        assert_eq!(
            edited["result"]["structuredContent"]["accepted"], true,
            "{edited}"
        );
        assert_eq!(config.lock().unwrap().as_ref().unwrap()["name"], "Night!");
        let (_, conflict) = post(
            &endpoint,
            request(
                "tools/call",
                "edit",
                json!({"name":"edit","arguments":args}),
            ),
        )
        .await;
        assert_eq!(
            conflict["result"]["structuredContent"]["error"]["code"],
            "revision_conflict"
        );
        assert_eq!(writes.load(Ordering::Relaxed), 1);
        let create = json!({"name":"create","arguments":{"action":"scene.create","input":{"config_key":"evening_scene","text":"{\"id\":\"evening_scene\"}"}}});
        let (_, exists) = post(&endpoint, request("tools/call", "exists", create.clone())).await;
        assert_eq!(
            exists["result"]["structuredContent"]["error"]["code"],
            "already_exists"
        );
        *config.lock().unwrap() = None;
        let (_, created) = post(&endpoint, request("tools/call", "create", create)).await;
        assert_eq!(created["result"]["structuredContent"]["accepted"], true);
        assert_eq!(writes.load(Ordering::Relaxed), 2);
        task.abort();
        ha.abort();
    }

    #[tokio::test]
    async fn native_authoring_rejects_invalid_candidates_before_writing() {
        let writes = Arc::new(AtomicUsize::new(0));
        let put_writes = writes.clone();
        let router = Router::new().route(
            "/api/config/scene/config/x",
            get(|| async { Json(json!({"id":"x","name":"one"})) }).post(move || {
                let writes = put_writes.clone();
                async move {
                    writes.fetch_add(1, Ordering::Relaxed);
                    Json(json!({"result":"ok"}))
                }
            }),
        );
        let (origin, ha) = serve(router).await;
        let (endpoint, task) = resource_endpoint(url::Url::parse(&origin).unwrap()).await;
        let (_, read) = post(
            &endpoint,
            request(
                "resources/read",
                "read",
                json!({"uri":"smarthome://scenes/x"}),
            ),
        )
        .await;
        let revision = read["result"]["contents"][0]["_meta"]["revision"].clone();
        for edits in [
            vec![json!({"operation":"insert","placement":"start","text":" ","anchor":null})],
            vec![json!({"operation":"replace","old_text":"one","new_text":"two","unknown":true})],
            vec![
                json!({"operation":"replace","old_text":"one","new_text":"two"}),
                json!({"operation":"replace","old_text":"missing","new_text":"private"}),
            ],
            vec![json!({"operation":"replace","old_text":"one","new_text":"one"})],
            vec![json!({"operation":"insert","placement":"start","text":" "})],
            vec![json!({"operation":"replace","old_text":"\"x\"","new_text":"\"wrong\""})],
            vec![json!({"operation":"insert","placement":"start","text":"private invalid source"})],
            vec![json!({"operation":"insert","placement":"end","text":"x".repeat(256*1024)})],
        ] {
            let (_, result) = post(&endpoint, request("tools/call","invalid",json!({"name":"edit","arguments":{"uri":"smarthome://scenes/x","expected_revision":revision,"edits":edits}}))).await;
            assert!(
                result.get("error").is_some() || result["result"]["isError"] == true,
                "{result}"
            );
            assert!(!result.to_string().contains("private"));
            assert_eq!(writes.load(Ordering::Relaxed), 0);
        }
        task.abort();
        ha.abort();
    }

    #[tokio::test]
    async fn native_authoring_preflight_errors_and_reread_conflicts_never_write() {
        for mode in ["error", "edit_race", "create_race"] {
            let reads = Arc::new(AtomicUsize::new(0));
            let writes = Arc::new(AtomicUsize::new(0));
            let get_reads = reads.clone();
            let put_writes = writes.clone();
            let router = Router::new().route(
                "/api/config/scene/config/x",
                get(move || {
                    let reads = get_reads.clone();
                    async move {
                        let index = reads.fetch_add(1, Ordering::Relaxed);
                        if mode == "error" {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({"secret":"private"})),
                            )
                        } else if mode == "create_race" && index == 0 {
                            (StatusCode::NOT_FOUND, Json(json!({})))
                        } else {
                            (
                                StatusCode::OK,
                                Json(
                                    json!({"id":"x","name":if index == 0 { "one" } else { "two" }}),
                                ),
                            )
                        }
                    }
                })
                .post(move || {
                    let writes = put_writes.clone();
                    async move {
                        writes.fetch_add(1, Ordering::Relaxed);
                        Json(json!({"result":"ok"}))
                    }
                }),
            );
            let (origin, ha) = serve(router).await;
            let (endpoint, task) = resource_endpoint(url::Url::parse(&origin).unwrap()).await;
            let arguments = if mode == "edit_race" {
                let (_, read) = post(
                    &endpoint,
                    request(
                        "resources/read",
                        "read",
                        json!({"uri":"smarthome://scenes/x"}),
                    ),
                )
                .await;
                reads.store(0, Ordering::Relaxed);
                json!({"name":"edit","arguments":{"uri":"smarthome://scenes/x","expected_revision":read["result"]["contents"][0]["_meta"]["revision"],"edits":[{"operation":"replace","old_text":"one","new_text":"three"}]}})
            } else {
                json!({"name":"create","arguments":{"action":"scene.create","input":{"config_key":"x","text":"{\"id\":\"x\"}"}}})
            };
            let (_, result) = post(&endpoint, request("tools/call", "call", arguments)).await;
            assert_eq!(result["result"]["isError"], true, "{mode}: {result}");
            assert_eq!(writes.load(Ordering::Relaxed), 0);
            assert!(!result.to_string().contains("private"));
            if mode == "edit_race" {
                assert_eq!(
                    result["result"]["structuredContent"]["error"]["code"],
                    "revision_conflict"
                );
            }
            task.abort();
            ha.abort();
        }
    }

    #[tokio::test]
    async fn native_authoring_uncertain_ack_is_nonretryable_and_never_repeated() {
        for slow in [false, true] {
            let writes = Arc::new(AtomicUsize::new(0));
            let put_writes = writes.clone();
            let router = Router::new().route(
                "/api/config/scene/config/x",
                get(|| async { StatusCode::NOT_FOUND }).post(move || {
                    let writes = put_writes.clone();
                    async move {
                        writes.fetch_add(1, Ordering::Relaxed);
                        if slow {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                        Json(json!({"private":"invalid ack"}))
                    }
                }),
            );
            let (origin, ha) = serve(router).await;
            let (endpoint, task) = resource_endpoint_with_timeout(
                url::Url::parse(&origin).unwrap(),
                Duration::from_millis(100),
            )
            .await;
            let (_, result) = post(&endpoint,request("tools/call","create",json!({"name":"create","arguments":{"action":"scene.create","input":{"config_key":"x","text":"{\"id\":\"x\"}"}}}))).await;
            let error = &result["result"]["structuredContent"]["error"];
            assert_eq!(error["code"], "mutation_outcome_unknown", "{result}");
            assert_eq!(error["retryable"], false);
            assert!(!result.to_string().contains("private"));
            assert_eq!(writes.load(Ordering::Relaxed), 1);
            task.abort();
            ha.abort();
        }
    }

    #[tokio::test]
    async fn blueprint_authoring_roundtrips_reader_text_and_uses_only_native_save() {
        let native = Arc::new(Mutex::new(Some(
            "blueprint:\n  name: One\n  domain: automation\naction:\n  target: !input chosen_entity\n".to_owned(),
        )));
        let commands = Arc::new(Mutex::new(Vec::<Value>::new()));
        let ws_native = native.clone();
        let ws_commands = commands.clone();
        let router = Router::new().route("/api/websocket", get(move |upgrade: WebSocketUpgrade| {
            let native = ws_native.clone(); let commands = ws_commands.clone(); async move {
                upgrade.on_upgrade(move |mut socket| async move {
                    socket.send(ws::Message::Text(json!({"type":"auth_required"}).to_string().into())).await.unwrap();
                    socket.recv().await.unwrap().unwrap();
                    socket.send(ws::Message::Text(json!({"type":"auth_ok"}).to_string().into())).await.unwrap();
                    while let Some(Ok(message)) = socket.recv().await {
                        let Ok(message) = message.into_text() else { break; };
                        let command: Value = serde_json::from_str(&message).unwrap();
                        commands.lock().unwrap().push(command.clone());
                        if command["type"] == "blueprint/save" && command["yaml"].as_str().is_some_and(|yaml| yaml.contains("unknown_native_schema")) {
                            socket.send(ws::Message::Text(json!({"id":command["id"],"type":"result","success":false,"error":{"code":"invalid_format","message":"private native schema details"}}).to_string().into())).await.unwrap();
                            continue;
                        }
                        let result = match command["type"].as_str().unwrap() {
                            "smarthome_mcp/blueprint/get" => json!({"path":"local/test.yaml","yaml":native.lock().unwrap().as_ref().unwrap()}),
                            "blueprint/list" => if native.lock().unwrap().is_some() { json!({"local/test.yaml":{"metadata":{"name":"One"}}}) } else { json!({}) },
                            "blueprint/save" => {
                                let previous = native.lock().unwrap().is_some();
                                assert!(command["allow_override"] == true || !previous);
                                *native.lock().unwrap() = Some(command["yaml"].as_str().unwrap().to_owned());
                                json!({"overrides_existing":previous})
                            }
                            other => panic!("unexpected native command: {other}"),
                        };
                        if socket.send(ws::Message::Text(json!({"id":command["id"],"type":"result","success":true,"result":result}).to_string().into())).await.is_err() { break; }
                    }
                })
            }
        }));
        let (origin, ha) = serve(router).await;
        let (endpoint, task) = resource_endpoint(url::Url::parse(&origin).unwrap()).await;
        let uri = "smarthome://blueprints/local%2Ftest.yaml";
        let (_, read) = post(
            &endpoint,
            request("resources/read", "read", json!({"uri":uri})),
        )
        .await;
        assert_eq!(
            read["result"]["contents"][0]["text"],
            native.lock().unwrap().as_ref().unwrap().as_str(),
            "{read}"
        );
        let original = native.lock().unwrap().as_ref().unwrap().clone();
        for replacement in [
            format!("# private comment\n{original}"),
            "action: {target: !input chosen_entity}\nblueprint: {domain: automation, name: One}\n"
                .to_owned(),
            "blueprint: [private broken".to_owned(),
            "[private, sequence]".to_owned(),
            "---\nblueprint: {}\n---\nprivate: true\n".to_owned(),
            "blueprint: {}\nblueprint: {}\n".to_owned(),
            "blueprint:\n  name: One\n  name: Two\n".to_owned(),
        ] {
            let before = commands.lock().unwrap().len();
            let (_, invalid) = post(&endpoint,request("tools/call","invalid-yaml",json!({"name":"edit","arguments":{"uri":uri,"expected_revision":read["result"]["contents"][0]["_meta"]["revision"],"edits":[{"operation":"replace","old_text":original,"new_text":replacement}]}}))).await;
            assert!(invalid.get("error").is_some(), "{invalid}");
            assert!(!invalid.to_string().contains("private"));
            let calls = commands.lock().unwrap();
            assert_eq!(calls.len(), before + 1);
            assert_eq!(calls.last().unwrap()["type"], "smarthome_mcp/blueprint/get");
            assert!(!calls.iter().any(|call| call["type"] == "blueprint/save"));
            assert_eq!(native.lock().unwrap().as_ref().unwrap(), &original);
        }
        let (_, edited) = post(&endpoint,request("tools/call","edit",json!({"name":"edit","arguments":{"uri":uri,"expected_revision":read["result"]["contents"][0]["_meta"]["revision"],"edits":[{"operation":"replace","old_text":"One","new_text":"Two"}]}}))).await;
        assert_eq!(
            edited["result"]["structuredContent"]["accepted"], true,
            "{edited}"
        );
        assert!(native.lock().unwrap().as_ref().unwrap().contains("Two"));
        assert!(
            native
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("!input chosen_entity")
        );
        let create = json!({"name":"create","arguments":{"action":"blueprint.create","input":{"path":"local/test.yaml","text":"blueprint:\n  name: Three\n  domain: automation\n"}}});
        let (_, exists) = post(&endpoint, request("tools/call", "exists", create.clone())).await;
        assert_eq!(
            exists["result"]["structuredContent"]["error"]["code"],
            "already_exists"
        );
        *native.lock().unwrap() = None;
        for source in [
            "blueprint: [private broken",
            "[sequence]",
            "---\na: 1\n---\nb: 2\n",
            "a: 1\na: 2\n",
        ] {
            let before = commands.lock().unwrap().len();
            let (_, invalid) = post(&endpoint,request("tools/call","invalid-create",json!({"name":"create","arguments":{"action":"blueprint.create","input":{"path":"local/test.yaml","text":source}}}))).await;
            assert!(invalid.get("error").is_some(), "{invalid}");
            assert!(!invalid.to_string().contains("private"));
            let calls = commands.lock().unwrap();
            assert_eq!(calls.len(), before + 1);
            assert_eq!(calls.last().unwrap()["type"], "blueprint/list");
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| call["type"] == "blueprint/save")
                    .count(),
                1
            );
        }
        let (_, created) = post(&endpoint, request("tools/call", "create", create)).await;
        assert_eq!(
            created["result"]["structuredContent"]["accepted"], true,
            "{created}"
        );
        *native.lock().unwrap() = None;
        let (_, rejected) = post(&endpoint,request("tools/call","native-schema",json!({"name":"create","arguments":{"action":"blueprint.create","input":{"path":"local/test.yaml","text":"unknown_native_schema: true\n"}}}))).await;
        assert_eq!(
            rejected["result"]["structuredContent"]["error"]["code"], "request_rejected",
            "{rejected}"
        );
        assert_eq!(
            rejected["result"]["structuredContent"]["error"]["retryable"],
            false
        );
        assert!(!rejected.to_string().contains("private"));
        assert!(native.lock().unwrap().is_none());
        let calls = commands.lock().unwrap();
        let saves = calls
            .iter()
            .filter(|call| call["type"] == "blueprint/save")
            .collect::<Vec<_>>();
        assert_eq!(saves.len(), 3);
        assert_eq!(saves[0]["allow_override"], true);
        assert_eq!(saves[1]["allow_override"], false);
        assert_eq!(saves[2]["allow_override"], false);
        for call in saves {
            assert_eq!(call["path"], "local/test.yaml");
            assert_eq!(call["domain"], "automation");
        }
        task.abort();
        ha.abort();
    }

    #[tokio::test]
    async fn blueprint_authoring_native_error_outcomes_and_creation_preflights() {
        for operation in ["create", "edit"] {
            for mode in [
                "unknown",
                "missing_code",
                "invalid_code",
                "missing_error",
                "invalid_error",
                "invalid_message",
                "malformed_ack",
                "lost_ack",
                "invalid_format",
                "already_exists",
                "invalid_existing",
                "list_error",
                "list_race",
            ] {
                if operation == "edit"
                    && matches!(mode, "invalid_existing" | "list_error" | "list_race")
                {
                    continue;
                }
                let commands = Arc::new(Mutex::new(Vec::<Value>::new()));
                let list_reads = Arc::new(AtomicUsize::new(0));
                let ws_commands = commands.clone();
                let ws_reads = list_reads.clone();
                let router = Router::new().route("/api/websocket",get(move |upgrade: WebSocketUpgrade| {
                    let commands = ws_commands.clone(); let reads = ws_reads.clone(); async move {
                        upgrade.on_upgrade(move |mut socket| async move {
                            socket.send(ws::Message::Text(json!({"type":"auth_required"}).to_string().into())).await.unwrap();
                            socket.recv().await.unwrap().unwrap();
                            socket.send(ws::Message::Text(json!({"type":"auth_ok"}).to_string().into())).await.unwrap();
                            while let Some(Ok(message)) = socket.recv().await {
                                let Ok(message) = message.into_text() else { break; };
                                let command: Value = serde_json::from_str(&message).unwrap();
                                commands.lock().unwrap().push(command.clone());
                                let mut response = json!({"id":command["id"],"type":"result","success":true});
                                match command["type"].as_str().unwrap() {
                                    "smarthome_mcp/blueprint/get" => response["result"] = json!({"path":"local/test.yaml","yaml":"blueprint:\n  name: One\n  domain: automation\n"}),
                                    "blueprint/list" => {
                                        let index = reads.fetch_add(1,Ordering::Relaxed);
                                        if mode == "list_error" { response["success"] = json!(false); response["error"] = json!({"code":"unknown_error","message":"private list token"}); }
                                        else if mode == "invalid_existing" || (mode == "list_race" && index > 0) { response["result"] = json!({"local/test.yaml":{"error":"private invalid blueprint"}}); }
                                        else { response["result"] = json!({}); }
                                    }
                                    "blueprint/save" => {
                                        if mode == "lost_ack" { socket.send(ws::Message::Close(None)).await.unwrap(); break; }
                                        if mode == "malformed_ack" { response["result"] = json!({"overrides_existing":"private invalid ack"}); }
                                        else {
                                            response["success"] = json!(false);
                                            let error = match mode {
                                                "unknown" => json!({"code":"unknown_error","message":"private native token"}),
                                                "missing_code" => json!({"message":"private native token"}),
                                                "invalid_code" => json!({"code":17,"message":"private native token"}),
                                                "invalid_error" => json!("private malformed error"),
                                                "invalid_message" => json!({"code":"invalid_format","message":17}),
                                                "invalid_format" | "already_exists" => json!({"code":mode,"message":"private native token"}),
                                                "missing_error" => Value::Null,
                                                other => panic!("unexpected save mode {other}"),
                                            };
                                            if mode != "missing_error" { response["error"] = error; }
                                        }
                                    }
                                    other => panic!("unexpected native command {other}"),
                                }
                                if socket.send(ws::Message::Text(response.to_string().into())).await.is_err() { break; }
                            }
                        })
                    }
                }));
                let (origin, ha) = serve(router).await;
                let (endpoint, task) = resource_endpoint(url::Url::parse(&origin).unwrap()).await;
                let arguments = if operation == "create" {
                    json!({"name":"create","arguments":{"action":"blueprint.create","input":{"path":"local/test.yaml","text":"blueprint:\n  name: Two\n  domain: automation\n"}}})
                } else {
                    let (_, read) = post(
                        &endpoint,
                        request(
                            "resources/read",
                            "read",
                            json!({"uri":"smarthome://blueprints/local%2Ftest.yaml"}),
                        ),
                    )
                    .await;
                    json!({"name":"edit","arguments":{"uri":"smarthome://blueprints/local%2Ftest.yaml","expected_revision":read["result"]["contents"][0]["_meta"]["revision"],"edits":[{"operation":"replace","old_text":"One","new_text":"Two"}]}})
                };
                let (_, result) = post(&endpoint, request("tools/call", "author", arguments)).await;
                let error = &result["result"]["structuredContent"]["error"];
                assert_eq!(
                    result["result"]["isError"], true,
                    "{operation}/{mode}: {result}"
                );
                assert!(!result.to_string().contains("private"), "{result}");
                let expected_saves =
                    if matches!(mode, "invalid_existing" | "list_error" | "list_race") {
                        0
                    } else {
                        1
                    };
                assert_eq!(
                    commands
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|command| command["type"] == "blueprint/save")
                        .count(),
                    expected_saves,
                    "{operation}/{mode}"
                );
                match mode {
                    "invalid_existing" | "list_race" => assert_eq!(error["code"], "already_exists"),
                    "list_error" => {}
                    "invalid_format" | "already_exists" => {
                        assert_eq!(error["code"], "request_rejected");
                        assert_eq!(error["retryable"], false);
                    }
                    _ => {
                        assert_eq!(
                            error["code"], "mutation_outcome_unknown",
                            "{operation}/{mode}: {result}"
                        );
                        assert_eq!(error["retryable"], false);
                    }
                }
                task.abort();
                ha.abort();
            }
        }
    }

    #[tokio::test]
    async fn native_authoring_cancellation_never_retries_and_releases_capacity() {
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
                get(|| async { StatusCode::NOT_FOUND }).post(delayed_config_upsert),
            )
            .with_state(mock.clone());
        let (origin, ha) = serve(router).await;
        let client = HomeAssistantClient::for_test(
            url::Url::parse(&origin).unwrap(),
            Secret("test-token".into()),
            Duration::from_secs(10),
        );
        let handler = Arc::new(ResourceFirstMcp(
            SmarthomeMcp::new(Arc::new(Services::new(client.clone()))).unwrap(),
        ));
        let (mut input_writer, input_reader) = tokio::io::duplex(64 * 1024);
        let (output_reader, output_writer) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(mcp::server::serve_stream(
            handler,
            input_reader,
            output_writer,
        ));
        let started = mock.started.notified();
        let call = request(
            "tools/call",
            "cancel-native",
            json!({"name":"create","arguments":{"action":"scene.create","input":{"config_key":"cancel_private_key","text":"{\"id\":\"cancel_private_key\",\"secret\":\"private-source\"}"}}}),
        );
        input_writer
            .write_all(format!("{call}\n").as_bytes())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), started)
            .await
            .unwrap();
        let mut cancel = request(
            "notifications/cancelled",
            "unused",
            json!({"requestId":"cancel-native","reason":"caller stopped"}),
        );
        cancel.as_object_mut().unwrap().remove("id");
        let dropped = mock.dropped_notify.notified();
        input_writer
            .write_all(format!("{cancel}\n").as_bytes())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), dropped)
            .await
            .unwrap();
        assert_eq!(mock.calls.load(Ordering::Relaxed), 1);
        assert!(client.has_full_test_capacity());
        drop(output_reader);
        drop(input_writer);
        server.abort();
        ha.abort();
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
        let handler = Arc::new(SmarthomeMcp::new(Arc::new(Services::new(client))).unwrap());
        let (origin, task) = serve(mcp::server::streamable_http_router(handler)).await;
        (format!("{origin}/mcp"), task)
    }

    async fn endpoint_with_component_deployer(
        component_deployer: ComponentDeployer,
    ) -> (String, JoinHandle<()>) {
        let client = HomeAssistantClient::for_test(
            url::Url::parse("http://127.0.0.1:1/").unwrap(),
            Secret("test-token".to_owned()),
            Duration::from_millis(100),
        );
        let handler = Arc::new(
            SmarthomeMcp::new(Arc::new(Services::new_with_component_deployer(
                client,
                component_deployer,
            )))
            .unwrap(),
        );
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
            .route("/api/states/sensor.allowed", get(|| async { Json(json!({
                "entity_id":"sensor.allowed","state":"1","attributes":{},
                "last_changed":"2026-08-10T00:00:00Z","last_updated":"2026-08-10T00:00:00Z"
            })) }))
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
                get(|| async { Json(json!({"id":"evening_scene","name":"Evening","secret":"authorized-scene"})) })
                    .post(|| async { Json(json!({"result":"ok"})) }),
            )
            .route(
                "/api/config/automation/config/arrival_lights",
                get(|| async { Json(json!({"id":"arrival_lights","alias":"Arrival","secret":"authorized-automation"})) })
                    .post(|| async { Json(json!({"result":"ok"})) }),
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
                    "thread/set_preferred_dataset" | "thread/set_preferred_border_agent" => json!({}),
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
        } else if method == "resources/read" {
            request = request.header("mcp-name", body["params"]["uri"].as_str().unwrap());
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
    async fn discovery_reports_the_cargo_package_version() {
        let (endpoint, task) = endpoint().await;
        let (_, response) = post(
            &endpoint,
            request("server/discover", "discover-version", json!({})),
        )
        .await;

        assert_eq!(
            response["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
            env!("CARGO_PKG_VERSION")
        );
        task.abort();
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
    async fn deployment_and_blueprint_catalogs_are_progressive_and_closed() {
        let (endpoint, task) = endpoint().await;
        let (_, response) = post(&endpoint, request("tools/list", "list", json!({}))).await;
        let tools = response["result"]["tools"].as_array().unwrap();
        let exec = tools
            .iter()
            .find(|tool| tool["name"] == EXEC_TOOL_NAME)
            .unwrap();
        let exec_schema = serde_json::to_string(&exec["inputSchema"]).unwrap();
        for action in [
            "blueprint.save",
            "automation.from_blueprint",
            "smarthome_mcp.deploy",
            "smarthome_mcp.setup",
            "home_assistant.restart",
        ] {
            assert!(exec_schema.contains(action));
        }
        let query = tools.iter().find(|tool| tool["name"] == TOOL_NAME).unwrap();
        let query_schema = serde_json::to_string(&query["inputSchema"]).unwrap();
        assert!(query_schema.contains("blueprint.list"));
        assert!(query_schema.contains("blueprint.get"));
        assert!(exec_schema.contains("\"const\":true"));

        task.abort();
    }

    #[tokio::test]
    async fn confirmations_reject_false_without_upstream_contact() {
        let (endpoint, task) = endpoint().await;
        for (tool, action, input) in [
            (
                EXEC_TOOL_NAME,
                "home_assistant.restart",
                json!({"confirm":false}),
            ),
            (
                EXEC_TOOL_NAME,
                "smarthome_mcp.deploy",
                json!({"confirm":false}),
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
    async fn confirmed_deploy_dispatches_once_and_returns_only_the_safe_projection() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (endpoint, task) =
            endpoint_with_component_deployer(ComponentDeployer::successful_for_test(calls.clone()))
                .await;

        let (_, rejected) = post(
            &endpoint,
            request(
                "tools/call",
                "deploy-false",
                json!({
                    "name":EXEC_TOOL_NAME,
                    "arguments":{"action":"smarthome_mcp.deploy","input":{"confirm":false}}
                }),
            ),
        )
        .await;
        assert_eq!(
            rejected["result"]["structuredContent"]["error"]["code"],
            "invalid_arguments"
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);

        let (_, response) = post(
            &endpoint,
            request(
                "tools/call",
                "deploy-true",
                json!({
                    "name":EXEC_TOOL_NAME,
                    "arguments":{"action":"smarthome_mcp.deploy","input":{"confirm":true}}
                }),
            ),
        )
        .await;
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            response["result"]["structuredContent"],
            json!({
                "action":"smarthome_mcp.deploy",
                "operation":"install",
                "changed":true,
                "installed_version":env!("CARGO_PKG_VERSION"),
                "restart_required":true
            })
        );
        assert_eq!(
            response["result"]["content"][0]["text"],
            "Completed smarthome_mcp.deploy."
        );
        let serialized = serde_json::to_string(&response).unwrap();
        for private in ["127.0.0.1", "/config", "password", "ssh-ed25519"] {
            assert!(!serialized.contains(private));
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
            "automation.list",
            "automation.get",
            "scene.list",
            "scene.get",
        ] {
            assert!(query_schema.contains(action), "query missing {action}");
        }
        assert!(query_schema.contains("\"additionalProperties\":false"));

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
        let handler = Arc::new(SmarthomeMcp::new(Arc::new(Services::new(client.clone()))).unwrap());
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
            (
                TOOL_NAME,
                "automation.list",
                json!({"query":"arrival","limit":1}),
                json!({
                    "action":"automation.list","entries":[],"total":0,"truncated":false
                }),
            ),
            (
                TOOL_NAME,
                "automation.get",
                json!({"config_key":"arrival_lights"}),
                json!({
                    "action":"automation.get","config_key":"arrival_lights",
                    "config":{"id":"arrival_lights","alias":"Arrival","secret":"authorized-automation"}
                }),
            ),
            (
                TOOL_NAME,
                "scene.list",
                json!({}),
                json!({"action":"scene.list","entries":[],"total":0,"truncated":false}),
            ),
            (
                TOOL_NAME,
                "scene.get",
                json!({"config_key":"evening_scene"}),
                json!({
                    "action":"scene.get","config_key":"evening_scene",
                    "config":{"id":"evening_scene","name":"Evening","secret":"authorized-scene"}
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
    async fn thread_and_matter_semantic_errors_are_safe() {
        let (endpoint, task) = endpoint().await;
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
        assert_eq!(serialized.matches("additionalProperties").count(), 31);
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
            (TOOL_NAME, "scene.get", json!({"config_key":"bad/id"})),
            (TOOL_NAME, "automation.list", json!({"limit":101})),
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
    async fn skills_list_get_and_every_resource_round_trip_exact_embedded_bytes() {
        use mcp::skills::{McpSkillList, McpSkillResources, parse_skill_frontmatter};
        use sha2::{Digest as _, Sha256};

        let (endpoint, task) = endpoint().await;
        let (_, discovery) = post(
            &endpoint,
            request("server/discover", "skills-capability", json!({})),
        )
        .await;
        assert!(
            discovery["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/skills"]
                .is_object()
        );
        assert_ne!(
            discovery["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/skills"]["directoryRead"],
            true
        );
        let (_, response) = post(&endpoint, request("skills/list", "skills", json!({}))).await;
        let list: McpSkillList = serde_json::from_value(response["result"].clone()).unwrap();
        assert_eq!(list.skills.len(), 5);
        assert_eq!(list.ttl_ms, 0);
        assert_eq!(list.cache_scope, mcp::skills::SkillCacheScope::Private);
        assert!(list.next_cursor.is_none());
        let mut names = Vec::new();
        for skill in &list.skills {
            let name = skill.frontmatter["name"].as_str().unwrap();
            names.push(name);
            assert_eq!(skill.uri, format!("skill://smarthome/{name}/SKILL.md"));
            let (_, response) = post(
                &endpoint,
                request("skills/get", "get-skill", json!({"uri":skill.uri})),
            )
            .await;
            assert_eq!(
                response["result"]["skill"],
                serde_json::to_value(skill).unwrap()
            );
            assert_eq!(response["result"]["resultType"], "complete");
            assert_eq!(response["result"]["ttlMs"], 0);
            assert_eq!(response["result"]["cacheScope"], "private");
            let McpSkillResources::Files(files) = &skill.resources else {
                panic!("catalog must be immutable");
            };
            assert_eq!(files.len(), 2);
            for file in files {
                let (_, response) = post(
                    &endpoint,
                    request("resources/read", "read-skill", json!({"uri":file.uri})),
                )
                .await;
                assert_eq!(response["result"]["resultType"], "complete");
                assert_eq!(response["result"]["ttlMs"], 0);
                assert_eq!(response["result"]["cacheScope"], "private");
                let contents = response["result"]["contents"].as_array().unwrap();
                assert_eq!(contents.len(), 1);
                assert_eq!(contents[0]["uri"], file.uri);
                let bytes = contents[0]["text"].as_str().unwrap().as_bytes();
                let relative = file
                    .uri
                    .strip_prefix(&format!("skill://smarthome/{name}/"))
                    .unwrap();
                let expected = std::fs::read(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("skills")
                        .join(name)
                        .join(relative),
                )
                .unwrap();
                assert_eq!(bytes, expected);
                assert_eq!(file.size, bytes.len() as u64);
                let hex = Sha256::digest(bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                assert_eq!(file.digest, format!("sha256:{hex}"));
                mcp::skills::verify_skill_bytes(skill, &file.uri, bytes).unwrap();
                if file.uri == skill.uri {
                    assert_eq!(parse_skill_frontmatter(bytes).unwrap(), skill.frontmatter);
                }
            }
        }
        assert_eq!(
            names,
            [
                "author-home-config",
                "control-home",
                "inspect-home",
                "inspect-thread-matter",
                "maintain-home-integration"
            ]
        );
        assert_eq!(
            list.skills
                .iter()
                .find(|skill| skill.frontmatter["name"] == "inspect-home")
                .unwrap()
                .frontmatter["metadata"]["authority"],
            "assist-exposure"
        );
        assert_eq!(
            list.skills
                .iter()
                .find(|skill| skill.frontmatter["name"] == "inspect-home")
                .unwrap()
                .frontmatter["x-smarthome"],
            json!({"workflow":"inspection"})
        );
        task.abort();
    }

    #[tokio::test]
    async fn hosted_skill_methods_keep_bearer_scope_and_origin_protection() {
        use mcp::server::{McpHostedTokenValidation, McpTokenAuthorization};

        let (ha_origin, ha) = home_assistant().await;
        let client = HomeAssistantClient::for_test(
            ha_origin,
            Secret("test-token".to_owned()),
            Duration::from_millis(100),
        );
        let handler = Arc::new(ResourceFirstMcp(
            SmarthomeMcp::new(Arc::new(Services::new(client))).unwrap(),
        ));
        let metadata = McpProtectedResourceMetadata::new(
            "https://mcp.example/mcp",
            ["https://mcp.example/oauth"],
        )
        .with_scopes(["mcp:use"]);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let authorization = StreamableHttpAuthorization::hosted(metadata, move |token, context| {
            let seen = seen.clone();
            Box::pin(async move {
                seen.fetch_add(1, Ordering::Relaxed);
                assert_eq!(context.required_scopes, ["mcp:use"]);
                assert_eq!(context.resource, "https://mcp.example/mcp");
                match token.0.as_str() {
                    "valid" => McpHostedTokenValidation::Authorized(McpTokenAuthorization {
                        principal_id: mcp::McpPrincipalId::new("skill-test").unwrap(),
                        expires_at: None,
                        revocation: None,
                    }),
                    "wrong-scope" => McpHostedTokenValidation::InsufficientScope {
                        required_scopes: vec!["mcp:use".to_owned()],
                        error_description: None,
                    },
                    _ => McpHostedTokenValidation::Unauthorized {
                        error_description: None,
                    },
                }
            })
        })
        .unwrap()
        .with_required_scopes(["mcp:use"]);
        let (origin, task) = serve(streamable_http_router_with_options(
            handler,
            StreamableHttpOptions::default()
                .without_root_protected_resource_metadata()
                .with_authorization(authorization),
        ))
        .await;
        for (method, params) in [
            ("skills/list", json!({})),
            (
                "skills/get",
                json!({"uri":"skill://smarthome/inspect-home/SKILL.md"}),
            ),
            (
                "resources/read",
                json!({"uri":"skill://smarthome/inspect-home/references/queries.md"}),
            ),
            ("resources/list", json!({})),
            ("resources/read", json!({"uri":"smarthome://entities"})),
        ] {
            for (token, request_origin, expected, error) in [
                (None, None, StatusCode::UNAUTHORIZED, None),
                (
                    Some("wrong"),
                    None,
                    StatusCode::UNAUTHORIZED,
                    Some("invalid_token"),
                ),
                (
                    Some("wrong-scope"),
                    None,
                    StatusCode::FORBIDDEN,
                    Some("insufficient_scope"),
                ),
                (
                    Some("valid"),
                    Some("https://evil.example"),
                    StatusCode::FORBIDDEN,
                    None,
                ),
                (Some("valid"), None, StatusCode::OK, None),
            ] {
                let mut request_builder = Client::new()
                    .post(format!("{origin}/mcp"))
                    .header("accept", "application/json, text/event-stream")
                    .header("content-type", "application/json")
                    .header("mcp-protocol-version", MCP_PROTOCOL_VERSION)
                    .header("mcp-method", method);
                if method == "resources/read" {
                    request_builder =
                        request_builder.header("mcp-name", params["uri"].as_str().unwrap());
                }
                if let Some(token) = token {
                    request_builder = request_builder.bearer_auth(token);
                }
                if let Some(origin) = request_origin {
                    request_builder = request_builder.header("origin", origin);
                }
                let response = request_builder
                    .json(&request(method, "protected", params.clone()))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), expected, "{method}");
                if expected == StatusCode::UNAUTHORIZED || error.is_some() {
                    let challenge = mcp::McpBearerChallenge::parse_any(
                        response
                            .headers()
                            .get_all("www-authenticate")
                            .iter()
                            .map(|v| v.to_str().unwrap()),
                    )
                    .unwrap();
                    assert_eq!(challenge.error.as_deref(), error);
                    assert!(challenge.resource_metadata.is_some());
                    if error == Some("insufficient_scope") {
                        assert_eq!(challenge.scope.as_deref(), Some("mcp:use"));
                    }
                }
                if expected == StatusCode::OK {
                    let expected_text = if params["uri"] == "smarthome://entities" {
                        "sensor.allowed"
                    } else if method == "resources/list" {
                        "smarthome://entities"
                    } else {
                        "inspect-home"
                    };
                    let body = response.text().await.unwrap();
                    assert!(body.contains(expected_text), "{method}: {body}");
                }
            }
        }
        assert!(calls.load(Ordering::Relaxed) >= 9);
        task.abort();
        ha.abort();
    }

    #[tokio::test]
    async fn skill_resources_read_directly_and_unknown_requests_fail() {
        let (endpoint, task) = endpoint().await;
        let (_, direct) = post(
            &endpoint,
            request(
                "resources/read",
                "direct",
                json!({"uri":"skill://smarthome/control-home/references/controls.md"}),
            ),
        )
        .await;
        assert!(
            direct["result"]["contents"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("# Controls\n")
        );
        for (method, params, code) in [
            (
                "skills/get",
                json!({"uri":"skill://smarthome/missing/SKILL.md"}),
                -32602,
            ),
            ("skills/list", json!({"cursor":"unknown"}), -32602),
            (
                "resources/read",
                json!({"uri":"skill://smarthome/control-home/references/missing.md"}),
                -32602,
            ),
            (
                "resources/read",
                json!({"uri":"skill://smarthome/control-home/../inspect-home/SKILL.md"}),
                -32602,
            ),
        ] {
            let (_, response) = post(&endpoint, request(method, "unknown", params)).await;
            assert_eq!(response["error"]["code"], code, "{response}");
        }
        task.abort();
    }

    #[tokio::test]
    async fn all_tool_schemas_have_exact_domain_enums_and_reject_removed_help() {
        let (endpoint, task) = endpoint().await;
        let (_, response) = post(&endpoint, request("tools/list", "list", json!({}))).await;
        let catalogs: [(&str, &[&str]); 6] = [
            (
                TOOL_NAME,
                &[
                    "entity.list",
                    "device.list",
                    "state.get",
                    "history.get",
                    "camera.snapshot",
                    "automation.list",
                    "automation.get",
                    "automation.validate",
                    "automation.traces",
                    "scene.list",
                    "scene.get",
                    "blueprint.list",
                    "blueprint.get",
                ],
            ),
            (
                EXEC_TOOL_NAME,
                &[
                    "scene.activate",
                    "scene.upsert",
                    "automation.upsert",
                    "blueprint.save",
                    "automation.from_blueprint",
                    "smarthome_mcp.deploy",
                    "smarthome_mcp.setup",
                    "home_assistant.restart",
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
                ],
            ),
            (
                THREAD_QUERY_TOOL_NAME,
                &["network.list", "router.discover", "readiness.get"],
            ),
            (
                THREAD_EXEC_TOOL_NAME,
                &["network.set_preferred", "router.set_preferred"],
            ),
            (
                MATTER_QUERY_TOOL_NAME,
                &[
                    "readiness.get",
                    "device.list",
                    "device.diagnostics",
                    "device.ping",
                ],
            ),
            (MATTER_EXEC_TOOL_NAME, &["device.interview"]),
        ];
        for (name, expected) in catalogs {
            let tool = response["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap();
            let mut actual = tool["inputSchema"]["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>();
            let mut expected = expected.to_vec();
            actual.sort_unstable();
            expected.sort_unstable();
            assert_eq!(actual, expected, "{name}");
            let serialized = tool["inputSchema"].to_string();
            assert!(!serialized.contains("help"));
            assert!(!serialized.contains("runbooks"));
            for action in [
                "help",
                "help.entity",
                "help.device",
                "help.state",
                "help.history",
                "help.camera",
                "help.automation",
                "help.scene",
                "help.blueprint",
                "help.smarthome_mcp",
                "help.home_assistant",
                "help.light",
                "help.switch",
                "help.fan",
                "help.cover",
                "help.climate",
                "help.media_player",
                "help.lock",
                "help.network",
                "help.router",
                "help.readiness",
                "help.runbooks",
                "runbooks.load",
            ] {
                let (_, response) = post(
                    &endpoint,
                    request(
                        "tools/call",
                        "removed",
                        json!({"name":name,"arguments":{"action":action}}),
                    ),
                )
                .await;
                assert_eq!(
                    response["error"]["code"], -32602,
                    "{name} accepted {action}"
                );
            }
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

    #[test]
    fn deploy_result_contains_only_the_bounded_projection() {
        let output = json!({
            "action":"smarthome_mcp.deploy",
            "operation":"install",
            "changed":true,
            "installed_version":"0.2.0",
            "restart_required":true,
        });
        let result = control_result(output.clone());
        assert_eq!(result.raw["structuredContent"], output);
        assert_eq!(
            result.raw["content"][0]["text"],
            "Completed smarthome_mcp.deploy."
        );
        assert!(
            !result.raw["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("/config")
        );
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
