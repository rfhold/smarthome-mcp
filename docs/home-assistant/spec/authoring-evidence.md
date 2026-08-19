# Home Assistant Authoring and Evidence Contract

This contract defines eight narrow authoring and evidence actions within the six progressive tools. They add authoring, stored-config reads, and projected evidence without generic Home Assistant access. The [blueprint contract](blueprints.md) defines separate implemented actions.

## Action Catalog

| Tool action | Input | Fixed upstream operation |
| --- | --- | --- |
| `home_assistant_exec` `scene.upsert` | A caller-selected `config_key` and one complete native Home Assistant scene JSON object. | `POST /api/config/scene/config/{config_key}` |
| `home_assistant_exec` `automation.upsert` | A caller-selected `config_key` and one complete native Home Assistant automation JSON object. | `POST /api/config/automation/config/{config_key}` |
| `home_assistant_query` `scene.list` | Optional case-insensitive `query` and bounded `limit`. | `GET /api/states`, projected to editor-managed scene metadata. |
| `home_assistant_query` `scene.get` | One validated `config_key`. | `GET /api/config/scene/config/{config_key}` |
| `home_assistant_query` `automation.list` | Optional case-insensitive `query` and bounded `limit`. | `GET /api/states`, projected to editor-managed automation metadata. |
| `home_assistant_query` `automation.get` | One validated `config_key`. | `GET /api/config/automation/config/{config_key}` |
| `home_assistant_query` `automation.validate` | A closed wrapper for native trigger, condition, or action configuration accepted by `validate_config`. | WebSocket command `validate_config` |
| `home_assistant_query` `automation.traces` | A closed wrapper with one automation item ID. | WebSocket command `trace/list` with fixed domain `automation` and caller item ID. |

Each outer input wrapper rejects unknown fields. A config key or item ID must be one safe, bounded path segment. It cannot contain separators, traversal syntax, control characters, or an empty value.

Native scene and automation JSON remains arbitrary inside the accepted object, except for the reserved top-level `id`. An absent `id` is valid. A present `id` must be a string exactly equal to `config_key`; another value or type fails validation. The service otherwise enforces only an object or action-specific shape, encoded-byte bounds, nesting-depth bounds, and shared transport limits. It does not invent a smaller Home Assistant schema or accept an unbounded document.

## Authority

These eight actions deliberately do not use Assist exposure. Endpoint-wide `mcp:use` plus the configured Home Assistant administrator token authorizes each request. The service performs no `homeassistant/expose_entity/list` lookup and does not recursively authorize entities, devices, areas, services, templates, or other references inside native JSON.

This policy expands authority beyond exposure-gated entity operations. Any client with `mcp:use` can discover editor-managed scenes and automations, read their complete native stored configuration, replace the entry with a matching config key, validate native automation fragments, and obtain projected evidence for an automation item ID. Existing entity reads and controls retain fresh exact `conversation: true` authorization.

## Configuration Read Semantics

`scene.list` and `automation.list` use `/api/states` only as bounded discovery metadata. They select the matching entity domain only when attributes contain an `id` that passes the same config-key validation as upsert and get. YAML-only entries without such an editor config ID are omitted and are not claimed as retrievable. Each result contains only config key, entity ID, and a bounded friendly name or config-key fallback. Arbitrary state, timestamps, attributes, and context are discarded. Search is case-insensitive over the three projected fields; results are deterministically sorted, counted, and limited to a default of 50 and maximum of 100.

`scene.get` and `automation.get` construct only the fixed config GET path from a validated key. A successful response must be an object within the native JSON byte and nesting-depth bounds and shared response/output limits. The authorized result contains the complete native object, including embedded values that may be sensitive. The service does not recursively authorize references in that object.

## Upsert Semantics

The server constructs the fixed path from the validated config key. Callers cannot select the origin, path prefix, method, headers, credential, or another config domain.

After the reserved `id` check, an upsert sends the complete caller object to Home Assistant. Home Assistant validates it and inserts or replaces the matching path key. Replacement is intentional. The service does not merge with, read, or return prior configuration. This rule prevents the native object's `id` from selecting a different stored config entry.

A successful config POST proves only that Home Assistant accepted and wrote the object. Home Assistant schedules its reload hook asynchronously after the write. The response does not prove that the scene or automation became active, that triggers attached, or that future execution will succeed.

## Validation Semantics

`automation.validate` uses only WebSocket `validate_config`. It returns a bounded projection of acceptance for the supplied trigger, condition, or action fields. It does not read stored automation configuration.

Acceptance describes the current Home Assistant instance and its current integrations. It does not prove future triggering, condition outcomes, service availability, template results, or successful execution.

## Trace Projection

`automation.traces` uses only `trace/list` with domain `automation` and the supplied item ID. It returns a bounded list of status summaries. Each summary can contain only:

- run ID;
- run time;
- duration;
- state;
- bounded execution status;
- a not-triggered indication; and
- error presence or a generic bounded error category.

The projection excludes raw errors, last steps, automation config, variables, state data, service data, contexts, and the full trace. It also excludes fields that cannot map safely to the allowlist.

Trace summaries describe retained past runs only. No trace does not prove that an automation failed to load or trigger. A successful past run does not guarantee another trigger or successful future execution.

## Privacy and Exclusions

Telemetry can contain only fixed action names, bounded outcomes, and shared safe request metadata. Config keys, entity IDs, names, item IDs, native config, trace fields, raw errors, and upstream bodies must not appear in logs, traces, metrics, or error text.

The tools do not expose config deletes, generic config listing routes, `trace/get`, trace contexts, trace debugging, breakpoint or trigger subscriptions, script execution, generic service calls, arbitrary WebSocket commands, or caller-selected HTTP paths. Exact config gets deliberately return complete native config; list and trace actions do not return native config or full trace data.

## Compatibility Evidence

The config endpoints, state discovery metadata, and trace commands are internal administrator APIs. Home Assistant frontend source confirms fixed GET and POST config paths, but unit and mock tests can prove only routing, projection, bounds, and privacy. They cannot prove compatibility with a supported Home Assistant release.

Acceptance requires recorded tests against a disposable Home Assistant instance. Evidence must cover administrator enforcement, editor-managed discovery, exact complete config reads, create and replacement behavior, native validation, reload scheduling, `validate_config`, `trace/list`, projection, and all exclusions. No repository evidence currently proves this live compatibility.
