# smarthome-mcp

`smarthome-mcp` is a Rust MCP server for authenticated, bounded smarthome integrations.

The current runtime implements hosted OAuth, stateless MCP, and six progressive Home Assistant, Thread, and Matter tools. Entity operations require current Assist exposure. Fixed administrator actions support authoring, blueprints, private component deployment, setup, and confirmed restart.

The Home Assistant component is embedded in the MCP binary for deployment to one server-owned SFTP target. Local Rust and Python tests cover the bounded behavior, but no live SSH/SFTP or Home Assistant component deployment evidence exists. Endpoint-wide `mcp:use` grants broad administrator, fixed machine-filesystem mutation, and availability authority.

Implementation entry points are [the library](src/lib.rs), [the Axum process](src/main.rs), [the container build](Dockerfile), [Pulumi](infra/pulumi/), and [the preview pipeline](.tekton/smarthome-mcp-preview.yaml).

## Private Component Deployment

1. Seed the protected deployment credential Stashes through an authorized Pulumi bootstrap.
2. Invoke `home_assistant_exec` action `smarthome_mcp.deploy` with `confirm: true`.
3. If the result reports `restart_required: true`, invoke `home_assistant.restart` separately with `confirm: true`.
4. After Home Assistant is ready, invoke `smarthome_mcp.setup` separately.
5. Verify that Home Assistant registers `smarthome_mcp/blueprint/get`.

Deployment neither restarts Home Assistant nor creates the integration config entry. See the [component deployment runbook](docs/operations/component-deployment.md) for prerequisites, recovery, and evidence limits.

Start with the [documentation index](docs/README.md) for current behavior, validation evidence, deployment status, and remaining gates.

The repository intentionally has no license file and makes no open-source license claim.
