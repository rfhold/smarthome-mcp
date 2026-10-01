# MCP Skills Catalog

The runtime publishes the official `io.modelcontextprotocol/skills` extension through an immutable validated `Arc<SkillCatalog>`. [`src/skills.rs`](../../../src/skills.rs) registers each authored file explicitly with `include_bytes!`; startup performs no filesystem scan. Catalog validation is fallible and reports only a safe construction error before routing.

| Skill | Workflow |
| --- | --- |
| [inspect-home](../../../skills/inspect-home/SKILL.md) | Exposure-gated entity, device, state, history, and camera inspection. |
| [control-home](../../../skills/control-home/SKILL.md) | Exact single-entity controls and physical-effect authority. |
| [author-home-config](../../../skills/author-home-config/SKILL.md) | Complete native config and semantic blueprint reads/replacements, validation, and historical trace evidence. |
| [maintain-home-integration](../../../skills/maintain-home-integration/SKILL.md) | Separate fixed deployment, restart, and setup decisions. |
| [inspect-thread-matter](../../../skills/inspect-thread-matter/SKILL.md) | Bounded network/device inspection and explicitly authorized preferred-selection/interview maintenance. |

Every root is `skill://smarthome/<name>/SKILL.md`. Each skill includes its entrypoint and one linked reference under `references/`. `skills/list` returns five complete entries in deterministic URI order; `skills/get` accepts an exact entrypoint URI. Every manifest includes all explicitly registered files with raw-byte size and lowercase `sha256:` digest. Frontmatter is parsed from the exact embedded entrypoint without discarding author-defined fields. Resource reads return exact UTF-8 content; clients can read any listed URI directly without first calling list or get. No directory-read extension is advertised. Results use `resultType: complete`, `ttlMs: 0`, and private cache scope. Unknown skills/cursors fail with invalid parameters, and unknown resources fail as resource-not-found.

Generated root and namespace help actions remain absent, without aliases. The [resource-first interface](resource-first.md) owns public create/edit/query/execute schemas and dynamic resource discovery. Private adapter names are not public tools. Use tools/list and resources/list/templates for discovery, and Skills for workflow guidance; authoring Skills explain the single-writer assumption and never suggest retrying uncertain writes.

Skills are static guidance, not permissions, executable payloads, deployment scripts, or runtime-secret sources. Hosted `/mcp` authentication, exact resource binding, endpoint-wide `mcp:use`, and origin protection apply to skill methods and resource reads just as to tools. There is no new per-tool administrator scope. Entity operations keep fresh exact Assist exposure; native authoring, blueprint, lifecycle, Thread, and Matter operations keep their documented administrator exceptions. Skill integrity does not establish trust or authorize a mutation.

Each mutation-capable entrypoint requires one submission per authorized decision and prohibits automatic retries. Timeout, cancellation, or an unconfirmed response can follow execution and leave the outcome unknown. Before any repeat, inspect available state. Obtain a fresh explicit user decision for the exact target and action. Backend `retryable` errors do not grant repeat authority.

Docker copies the authored assets into the build stage so compile-time embedding works; the runtime binary requires no skill directory. Repository tests compare every served manifest file to source bytes and reject removed help and unknown resources. Live Home Assistant compatibility and deployment remain separate, unverified boundaries.
