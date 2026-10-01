---
name: maintain-home-integration
description: Maintain the fixed embedded smarthome_mcp Home Assistant integration through separately authorized deployment, restart, and setup decisions.
---
# Maintain Home Integration

Read [lifecycle guidance](references/lifecycle.md) before using `execute` maintenance actions. Identify the exact configured target and state what each requested operation changes.

Submit each authorized mutation once. Do not retry automatically. Timeout, cancellation, or an unconfirmed response can follow execution and leave the outcome unknown. Inspect available state before any repeat. Obtain a fresh explicit user decision for the exact target and action before any repeat. A backend `retryable` error does not grant permission to repeat.

Deploy, restart, and setup are three independent explicit decisions. A deploy response saying `restart_required` is information, not authority to restart. Stop at the authorized boundary; never combine all three implicitly. This skill includes no deploy scripts, runtime credentials, or installation payload.
