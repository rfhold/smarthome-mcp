#![allow(clippy::useless_vec)]

use std::sync::Arc;

use axum::Router;
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
        actions::{GetHistoryInput, GetStatesInput, ListEntitiesInput},
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
        description = "Read explicitly Assist-exposed Home Assistant entities and bounded history.",
        annotations = json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": true
        })
    )
)]
impl SmarthomeMcp {
    /// List current states for entities explicitly exposed to Home Assistant's
    /// conversation assistant. Results may be searched, filtered by domain,
    /// and are deterministically limited.
    #[action(tool = "home_assistant_query", name = "list_entities")]
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
    #[action(tool = "home_assistant_query", name = "get_states")]
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
    #[action(tool = "home_assistant_query", name = "get_history")]
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
}

fn query_result(noun: &str, output: serde_json::Value) -> McpToolResult {
    let count = output
        .get("entities")
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
        let client = HomeAssistantClient::for_test(
            url::Url::parse("http://127.0.0.1:1/").unwrap(),
            Secret("test-token".to_owned()),
            Duration::from_millis(100),
        );
        let handler = Arc::new(SmarthomeMcp {
            services: Arc::new(Services::new(client)),
        });
        let (origin, task) = serve(mcp::server::streamable_http_router(handler)).await;
        (format!("{origin}/mcp"), task)
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
        let payload = text
            .lines()
            .rev()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap_or(&text);
        (status, serde_json::from_str(payload).unwrap())
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
                    "arguments": {"action":"get_states", "input":{"entity_ids":[]}}
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
}
