# smarthome-mcp

`smarthome-mcp` is a Rust MCP server for authenticated, bounded smarthome integrations.

The repository implements hosted OAuth with Authentik browser authentication, stateless MCP, and the progressive read-only `home_assistant_query` tool. Home Assistant reads require current Assist exposure, strict schemas, bounded REST and WebSocket clients, and data-minimal results. Results include normalized entity data or one validated camera image.

Implementation entry points are [the library](src/lib.rs), [the Axum process](src/main.rs), [the container build](Dockerfile), [Pulumi](infra/pulumi/), and [the preview pipeline](.tekton/smarthome-mcp-preview.yaml).

Start with the [documentation index](docs/README.md) for current behavior, validation evidence, deployment status, and remaining gates.
