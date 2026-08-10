# Index

| Path | Info |
| --- | --- |
| [src/](src/) | Rust 1.96 application composition, hosted OAuth, MCP server, shared services, and integration-owned adapters. |
| [Dockerfile](Dockerfile) | Multi-stage Rust build and non-root Debian runtime image. |
| [infra/pulumi/](infra/pulumi/) | Preview and production deployment declarations plus mock tests. |
| [.tekton/](.tekton/) | Main-branch preview build and deployment pipeline; no release pipeline exists. |
| [docs/](docs/) | Repository architecture, Home Assistant contracts, operations, and quality evidence. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Verified local commands and contribution boundaries. |

# Hints

- Read [docs/README.md](docs/README.md) before work in this repository.
- Treat Home Assistant as a fixed upstream, never as a caller-selected HTTP or service proxy.
- Every entity read must use a fresh Assist exposure lookup and fail closed unless `conversation` is explicitly `true`.
- Keep the Kuri `mcp` dependency pinned to a reviewed immutable Git revision before delivery.
- Deployment declarations exist; do not claim live validation without recorded evidence.
- Use Rust 1.96 or the documented container fallback for Rust commands.
- Use `agentic-documentation` for documentation or `AGENTS.md` changes.
- Use `planning-changes` before service, infrastructure, or security implementation.
