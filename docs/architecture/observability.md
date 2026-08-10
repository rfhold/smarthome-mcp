# Observability

The host emits correlated OpenTelemetry traces and metrics, structured JSON logs, and optional CPU profiles. Health and readiness probes intentionally bypass request telemetry.

Home Assistant operations emit `home_assistant.query` spans. Each service-owned Home Assistant REST request emits one child `http.client.request` client span and propagates that span's W3C trace context upstream. The span covers request send and response headers. For an accepted success status, it also covers bounded body transfer and ends before JSON or schema validation. Status and declared-size rejections finalize at headers. A cancellation guard finalizes a dropped request. The WebSocket exposure lookup is outside this client instrumentation boundary.

The Home Assistant integration emits these metrics:

| Metric | Dimensions |
| --- | --- |
| `smarthome_mcp.home_assistant.requests` | `action`, `outcome` |
| `smarthome_mcp.home_assistant.duration` | `action`, `outcome` |
| `smarthome_mcp.home_assistant.in_flight` | `action` |

Action and outcome values pass through fixed allowlists. Cancellation finalizes in-flight telemetry through RAII. REST client outcomes are `success`, `http_error`, `transport_error`, `response_error`, or `cancelled`. `response_error` represents a bounded response-size rejection, not a transport failure. Only 4xx and 5xx status responses set OpenTelemetry HTTP error status; 1xx and 3xx responses retain unset status. REST client spans contain only the request method, response status when available, bounded outcome, and standard application span metadata. No signal records entity IDs, names, states, attributes, search text, history times, bearer tokens, headers, server address or port, URL components, response bodies, or raw errors.

Only the exact `smarthome_mcp`, `smarthome_mcp::*`, `mcp`, and `mcp::*` tracing target trees are admitted. Other dependency events remain suppressed even under permissive `RUST_LOG` settings.
