# Observability Operations

## Configuration

`SMARTHOME_MCP_DEPLOYMENT_ENVIRONMENT` is required even when export and profiling are disabled.

| Variable | Requirement |
| --- | --- |
| `SMARTHOME_MCP_DEPLOYMENT_ENVIRONMENT` | Required non-empty environment name. |
| `SMARTHOME_MCP_K8S_NAMESPACE` | Optional resource attribute and profile tag. |
| `SMARTHOME_MCP_K8S_POD_NAME` | Optional resource attribute. |
| `SMARTHOME_MCP_K8S_POD_UID` | Optional source for `k8s.pod.uid` and `service.instance.id`. |
| `SMARTHOME_MCP_PYROSCOPE_URL` | Optional credential-free HTTPS root origin enabling 100 Hz CPU profiling. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Optional shared OTLP/HTTP endpoint enabling trace and metric export. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Optional trace endpoint; without the shared endpoint, the metric endpoint is also required. |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | Optional metric endpoint; without the shared endpoint, the trace endpoint is also required. |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | `http/protobuf` in the declared deployment. |
| `RUST_LOG` | Optional JSON log filter for only the approved `smarthome_mcp` and `mcp` target trees. |

The declared deployment sends OTLP/HTTP to Alloy port 4318 and profiles to port 4040. Network policy permits both ports. Never print environment variables or rendered Secrets during diagnosis.

## Runtime Behavior

With no OTLP endpoints, the service uses local JSON telemetry. A shared endpoint enables traces and metrics. Both signal-specific endpoints also enable export; exactly one fails startup.

Startup fails before bind for invalid telemetry configuration, exporter construction, subscriber setup, or profiler startup. Export or profile delivery failures after startup do not stop request handling. The hard target allowlist suppresses dependency diagnostics, so backend freshness and collector health are the evidence for delivery.

SIGTERM or Ctrl-C starts graceful HTTP shutdown. Cleanup stops before bounded Pyroscope and OpenTelemetry shutdown. Cancellation guards finalize HTTP and Home Assistant operations as `cancelled` and restore active counters.

`/health` and `/ready` intentionally emit no request telemetry. Diagnose probes through Kubernetes status and direct endpoint behavior.

## Data Safety

Home Assistant telemetry uses only fixed action and outcome labels. Spans use target `smarthome_mcp::home_assistant` and name `home_assistant.query`.

Do not record entity IDs, friendly names, domains derived from caller data, states, attributes, search terms, history ranges, Home Assistant URLs, access tokens, authorization headers, WebSocket messages, or upstream bodies. MCP and host metrics likewise use bounded protocol and route dimensions.

## Validation

After an authorized deployment, use the observability backends without including credentials or household data.

Logs in Loki:

```logql
{service_name="smarthome-mcp"} | json | deployment_environment_name="preview"
```

HTTP request rate in Mimir:

```promql
sum by (http_route, http_outcome) (rate(http_server_request_count{service_name="smarthome-mcp"}[5m]))
```

Home Assistant failures in Mimir:

```promql
sum by (action, outcome) (rate(smarthome_mcp_home_assistant_requests_total{service_name="smarthome-mcp",outcome!="success"}[5m]))
```

Service traces in Tempo:

```traceql
{ resource.service.name = "smarthome-mcp" }
```

Select `smarthome-mcp` and CPU profile type in Pyroscope. Filter by `service_namespace="smarthome"` and the target deployment environment. Backend translation may replace metric dots with underscores and append `_total` to counters; confirm final names through metric discovery.

## Troubleshooting

| Symptom | Checks |
| --- | --- |
| Process exits before listen | Check safe errors for required configuration, paired OTLP endpoints, valid origins, or profiler initialization. |
| JSON logs exist but traces do not | Confirm approved targets and INFO-or-higher admission, OTLP HTTP configuration, Alloy reachability, and backend health. `RUST_LOG` does not control trace admission. |
| Probe telemetry is absent | Expected for `/health` and `/ready`; use Kubernetes probe evidence. |
| Trace IDs are absent from logs | Startup events legitimately omit IDs. Confirm request events occur inside instrumented MCP or Home Assistant spans. |
| Parent traces do not connect | Confirm the caller and proxies preserve a valid W3C `traceparent`. |
| Home Assistant failures rise | Group only by action and bounded outcome; use correlated traces without adding entity or payload data. |
| Profiles are absent | Confirm the Pyroscope origin and egress to port 4040, then inspect collector/backend health. |

`RUST_LOG` cannot enable dependency target trees or bypass redaction. Use backend freshness rather than expecting local dependency-export diagnostics.
