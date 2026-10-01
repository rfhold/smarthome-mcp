# Documentation

This index routes readers to current runtime contracts, operational guidance, and validation boundaries.

| Document | Covers |
| --- | --- |
| [Architecture](architecture/README.md) | Service boundaries, components, data flows, and authentication. |
| [Home Assistant](home-assistant/README.md) | Resource-first query/execute interface, authored MCP Skills, and domain adapter contracts. |
| [Operations](operations/README.md) | Service and component deployment, bootstrap, recovery, and external-action boundaries. |
| [Quality](quality/README.md) | Verified local commands, current evidence, and remaining validation. |

The current runtime defines hosted OAuth, authenticated `/mcp`, create/edit/query/execute tools, and dynamic smart-home resources. Authoring uses existing native APIs under a single-writer assumption and makes no atomic concurrency guarantees. Local code and tests do not establish live compatibility, installation, SSH/SFTP deployment, or external behavior.
