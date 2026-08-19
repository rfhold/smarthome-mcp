# Observability Testing

## Current Evidence

Local tests cover host and integration telemetry contracts. No deployed observability evidence is claimed.

## Commands

Run the Rust and Pulumi commands in [Testing](testing.md), then:

```bash
git diff --check -- docs/architecture/observability.md docs/operations/observability.md docs/quality/observability-testing.md
```

## Local Coverage

Tests cover:

- local OTLP mode, shared endpoints, paired signal-specific endpoints, and rejection of an incomplete pair;
- approved resource identity and pod UID mapping;
- JSON trace/span correlation, bounded routes, and safe completion events;
- total telemetry bypass for `/health` and `/ready`;
- cancellation-safe host and Home Assistant in-flight metrics;
- fixed trace admission for the `smarthome_mcp` and `mcp` target trees independent of `RUST_LOG`;
- bounded Pyroscope tags and cleanup outcomes;
- exact Home Assistant and component deployment metric names with allowlisted action and outcome labels;
- child-process exporter coverage for one-span REST parentage, `traceparent` propagation, success, 4xx, 3xx, cancellation, and sensitive-attribute exclusion;
- deployment environment and downward API wiring; and
- declared Alloy and Pyroscope egress ports.

Required authoring and evidence coverage must prove fixed action labels. It must also prove that config keys, entity IDs, names, item IDs, native config, trace fields, and raw errors remain absent.

Blueprint and component deployment coverage must prove fixed action and outcome labels. It must exclude paths, YAML, inputs, component bytes, versions, hashes, credentials, host-key material, result data, entity references, and raw errors.

## Post-Deployment Evidence

After an authorized deployment, record timestamps and redacted results for each area:

| Area | Required evidence |
| --- | --- |
| JSON logs | Startup and request completion parse as single objects containing approved fields only. |
| Signal filtering | The application HTTP request span parents `home_assistant.query`; permissive `RUST_LOG` still admits only application-owned JSON targets. |
| W3C correlation | A known `traceparent` joins the caller trace and matching request log IDs. |
| Resources | Service, namespace, environment, Kubernetes namespace/pod/UID, and instance ID are present. |
| HTTP metrics | Count, duration, active requests, bounded routes/outcomes, cancellation balance, and probe bypass. |
| MCP metrics | Generic request count, duration, in-flight, and bounded protocol outcomes. |
| Home Assistant metrics | Request count, duration, in-flight, fixed action labels, and bounded outcomes without entity dimensions. |
| Component deployment metrics | Request count, duration, in-flight, and bounded outcomes without host, path, version, hash, or credential dimensions. |
| Profiles | A 100 Hz CPU profile appears with only approved stable tags. |
| Failure safety | Controlled failures contain no token, URL, entity ID, key, blueprint YAML or input, component bytes, deployment version or hash, SSH credential or host key, search or result data, state, native config, trace field, image, path, header, upstream body, or raw error. |
| Export outage | HTTP remains available while backend freshness and collector health expose delivery failure. |
| Shutdown | Bounded profile and OpenTelemetry shutdown completes with sanitized status only. |

Use the [operations queries](../operations/observability.md#validation) for backend checks and the [Home Assistant specifications](../home-assistant/README.md) for action behavior. Never retain credentials or raw household responses as evidence.
