# Home Assistant Blueprint Contract

## Status

The repository implements this contract and local Rust and Python tests cover its bounded behavior. No live Home Assistant compatibility, component installation, deployment, or external-operation evidence exists.

## Home Assistant Integration

The repository includes a custom integration at `custom_components/smarthome_mcp/`, embedded into the MCP binary for private deployment. One no-input config flow creates one config entry. The integration supports a single entry and treats another flow as already configured.

The integration registers one administrator-only WebSocket command, `smarthome_mcp/blueprint/get`. Its closed input contains one validated automation blueprint path. The command does not accept a filesystem path, URL, domain, or command name.

The command resolves the path through Home Assistant's internal `DomainBlueprints.async_get_blueprint` model for the automation domain. It serializes the resolved blueprint through `Blueprint.yaml()` and returns semantic YAML. The result preserves blueprint meaning, but comments, scalar style, key order, and other formatting can change. Byte-identical source retrieval is not a supported contract.

The command rejects traversal, absolute paths, control characters, non-automation domains, invalid blueprint paths, and YAML above 256 KiB. It returns safe errors without the path, YAML, inputs, entity references, or raw upstream errors. It provides no generic filesystem access.

## Actions

| Tool action | Input | Fixed upstream operation |
| --- | --- | --- |
| `home_assistant_query` `blueprint.list` | A closed wrapper with bounded optional search and result limit. | Home Assistant's native automation blueprint list command, with a bounded projection. |
| `home_assistant_query` `blueprint.get` | One validated automation blueprint path. | `smarthome_mcp/blueprint/get`. |
| `home_assistant_exec` `blueprint.save` | One validated automation blueprint path and semantic YAML no larger than 256 KiB. | Home Assistant's native automation blueprint save command. |
| `home_assistant_exec` `automation.from_blueprint` | One safe automation config key, one validated blueprint path, and bounded native input values. | Native blueprint substitution preflight, then the fixed automation config endpoint. |
| `home_assistant_exec` `smarthome_mcp.setup` | No input. | Start only the `smarthome_mcp` config flow, or return an already-configured result. |
| `home_assistant_exec` `home_assistant.restart` | Exact `confirm: true`. | Home Assistant's fixed restart operation. |

All wrappers reject unknown fields. Paths, search text, result counts, config keys, input keys, input values, and entity references have explicit finite schema bounds. Native JSON has a 256 KiB encoded limit and maximum depth 32. Shared transport, output, deadline, and concurrency limits also apply.

## Semantics

`blueprint.list` returns only bounded blueprint metadata. It does not return source YAML, arbitrary registry data, or upstream wrappers. `blueprint.get` returns the requested authorized semantic YAML and bounded metadata.

`blueprint.save` always replaces the selected blueprint path. It never merges with the prior document. Callers must preserve prior YAML outside the service before replacement when rollback matters.

`automation.from_blueprint` first invokes Home Assistant's native substitution behavior with the selected blueprint and inputs. It stops without a config write when substitution fails. On success, it writes a compact native automation object through the current fixed automation endpoint. The stored object contains `use_blueprint` with the path and inputs, plus only required identity fields. It does not persist the expanded automation returned by preflight.

`smarthome_mcp.setup` starts only this integration's config flow. An entry that exists before the flow returns idempotent success. If another request creates the entry while this flow is active, the resulting already-configured completion also returns idempotent success. The action does not install files, restart Home Assistant, configure another integration, or claim that a downloaded integration is loaded.

`home_assistant.restart` never runs implicitly. The action requires the exact boolean `confirm: true`; strings and truthy substitutes fail. Install, update, blueprint save, automation creation, and setup never trigger it. A successful request returns minimal acknowledgment because the connection can terminate before a richer response arrives.

Component file installation is defined separately by the [component deployment contract](component-deployment.md). Deploy, restart, and setup are three independently invoked operations.

## Authority And Safety

Endpoint-wide `mcp:use` and the server-owned Home Assistant administrator token authorize these actions. They do not use Assist exposure or recursively authorize blueprint inputs and entity references. No separate management OAuth scope exists.

This authority permits blueprint source reads, always-replace writes, config-flow creation, automation writes, and a full Home Assistant restart. A client with `mcp:use` therefore receives broad administrator, filesystem-mutation, and availability authority in addition to entity capabilities.

Blueprint source, YAML, inputs, paths, config keys, repository identifiers, versions, entity references, and raw upstream errors must remain absent from logs, traces, metric labels, and safe errors. An authorized result can contain the requested bounded YAML or projected data.

## Exclusions And Compatibility

The feature excludes blueprint delete, blueprint import, script blueprints, generic blueprint domains, generic config flows, generic WebSocket commands, caller-selected HTTP routes, and implicit restart. It provides no generic Home Assistant or filesystem proxy.

`DomainBlueprints.async_get_blueprint`, `Blueprint.yaml()`, native blueprint commands, and config-flow behavior are version-sensitive Home Assistant internals. Local tests prove validation, routing, bounds, projection, privacy, and controlled integration behavior only. Support claims require recorded evidence against a disposable Home Assistant version. No repository evidence currently proves live compatibility, installation, deployment, or external operation.
