---
name: author-home-config
description: Read or replace native editor-managed Home Assistant scenes, automations, or automation blueprints, validate fragments, and inspect retained trace summaries.
---
# Author Home Config

1. Read [authoring guidance](references/authoring.md) and current query/exec schemas.
2. Identify the exact config key or blueprint path, read the existing complete content when replacing it, and preserve a private rollback copy if needed.
3. Explain the complete replacement and obtain target-specific write authority before submitting. Validate relevant fragments or blueprint substitution without claiming future success.
4. Distinguish accepted writes, asynchronous reload, activation, and retained historical trace evidence.

Submit each authorized mutation once. Do not retry automatically. Timeout, cancellation, or an unconfirmed response can follow execution and leave the outcome unknown. Inspect available state before any repeat. Obtain a fresh explicit user decision for the exact target and action before any repeat. A backend `retryable` error does not grant permission to repeat.

These are administrator exceptions: they do not use Assist exposure or recursively authorize native references. Complete native config and semantic blueprint YAML may contain sensitive values. Neither this skill nor discovery confers mutation authority.
