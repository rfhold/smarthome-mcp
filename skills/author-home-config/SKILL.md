---
name: author-home-config
description: Read native scenes, automations, and semantic automation blueprints; create or revision-edit through existing native APIs under a single-writer assumption, validate fragments, and inspect retained trace summaries.
---
# Author Home Config

1. Read [authoring guidance](references/authoring.md), resource catalogs/templates, and the `query` schema.
2. Follow canonical scene, automation, blueprint, or trace links. Treat complete native content as potentially sensitive.
3. Validate fragments with `query` action `automation.validate` without claiming future success.
4. For an authorized create, use the advertised `create` schema and complete native text. For an authorized edit, read the exact text and observed revision, then submit direct `{uri, expected_revision, edits}` arguments once. The system assumes no concurrent writers; revision and existence checks are best-effort, not atomic. Never substitute a legacy upsert or command.

Submit each authorized mutation once. Do not retry automatically. Timeout, cancellation, or an unconfirmed response can follow execution and leave the outcome unknown. Inspect available state before any repeat. Obtain a fresh explicit user decision for the exact target and action before any repeat. A backend `retryable` error does not grant permission to repeat.

These are administrator exceptions: they do not use Assist exposure or recursively authorize native references. Complete native config and semantic blueprint YAML may contain sensitive values. Neither this skill nor discovery confers mutation authority.
