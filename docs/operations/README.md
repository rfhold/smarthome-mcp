# Operations

These documents describe deployment declarations, current pipeline behavior, private component deployment, and operator validation.

The runtime uses Waltr tag `v0.3.1` with an exact lockfile SHA. Container and pipeline declarations retain private Git credential support. The production release pipeline promotes exact preview digests; production deployment requires separate authority and protected credential bootstrap.

| Document | Covers |
| --- | --- |
| [Deployment](deployment.md) | Pulumi stacks, Kubernetes resources, credentials, preview builds, signed-tag production promotion, and operational constraints. |
| [Component deployment](component-deployment.md) | Credential bootstrap, SSH prerequisites, deploy/restart/setup sequence, recovery, and evidence boundaries. |
| [Observability](observability.md) | Telemetry configuration, data safety, backend validation, and troubleshooting. |
