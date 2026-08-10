# Observability

The host emits correlated OpenTelemetry traces and metrics, structured JSON logs, and optional CPU profiles. Health and readiness probes intentionally bypass request telemetry.

Home Assistant operations emit `home_assistant.query` spans and these metrics:

| Metric | Dimensions |
| --- | --- |
| `smarthome_mcp.home_assistant.requests` | `action`, `outcome` |
| `smarthome_mcp.home_assistant.duration` | `action`, `outcome` |
| `smarthome_mcp.home_assistant.in_flight` | `action` |

Action and outcome values pass through fixed allowlists. Cancellation finalizes in-flight telemetry through RAII. No signal records entity IDs, names, states, attributes, search text, history times, bearer tokens, upstream URLs, response bodies, or raw errors.

Only the exact `smarthome_mcp`, `smarthome_mcp::*`, `mcp`, and `mcp::*` tracing target trees are admitted. Other dependency events remain suppressed even under permissive `RUST_LOG` settings.
