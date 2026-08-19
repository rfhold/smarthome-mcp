# Operations

These documents describe deployment declarations, current pipeline behavior, private component deployment, and operator validation.

The runtime uses the reviewed immutable Kuri Git revision, and the container and pipeline declarations can resolve it. Production remains excluded with zero resources.

| Document | Covers |
| --- | --- |
| [Deployment](deployment.md) | Pulumi stacks, Kubernetes resources, credentials, delivery inputs, and operational constraints. |
| [Component deployment](component-deployment.md) | Credential bootstrap, SSH prerequisites, deploy/restart/setup sequence, recovery, and evidence boundaries. |
| [Observability](observability.md) | Telemetry configuration, data safety, backend validation, and troubleshooting. |
