# Index

| Path | Info |
| --- | --- |
| [src/](src/) | Rust 1.96 application composition, hosted OAuth, MCP server, shared services, and integration-owned adapters. |
| [custom_components/smarthome_mcp/](custom_components/smarthome_mcp/) | Home Assistant custom integration for bounded semantic blueprint reads. |
| [Dockerfile](Dockerfile) | Multi-stage Rust build and non-root Debian runtime image. |
| [infra/pulumi/](infra/pulumi/) | Preview and production deployment declarations plus mock tests. |
| [.tekton/](.tekton/) | Preview pipeline declaration. |
| [docs/](docs/) | Repository architecture, Home Assistant contracts, operations, and quality evidence. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Verified local commands and contribution boundaries. |

# Hints

- Read [docs/README.md](docs/README.md) before work in this repository.
- Treat Home Assistant as a fixed upstream, never as a caller-selected HTTP or service proxy.
- Treat component deployment and blueprint operations as fixed administrator capabilities, never as generic filesystem or service proxies.
- Every entity read must use a fresh Assist exposure lookup and fail closed unless `conversation` is explicitly `true`.
- The runtime has exactly six progressive tools.
- Local implementation and tests do not establish live Home Assistant compatibility, component deployment, installation, or external-operation evidence.
- The repository is intentionally unlicensed; do not add a license or claim an open-source license.
- Keep the Kuri `mcp` dependency pinned to a reviewed immutable Git revision before delivery.
- Deployment declarations exist; do not claim live validation without recorded evidence.
- Use Rust 1.96 or the documented container fallback for Rust commands.
- Use `agentic-documentation` for documentation or `AGENTS.md` changes.
- Use `planning-changes` before service, infrastructure, or security implementation.
