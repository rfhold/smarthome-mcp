---
name: control-home
description: Operate one Assist-exposed entity using fixed controls, or explicitly trigger one editor-managed stored automation live with confirmation and a current config revision.
---
# Control Home

1. Read [control guidance](references/controls.md) and the current `execute` schema.
2. Resolve the exact target through `smarthome://entities` and its state links; establish user authority for the requested physical effect.
3. Invoke only the requested fixed action, then report the minimal acknowledgment without claiming a verified resulting state.

Submit each authorized mutation once. Do not retry automatically. Timeout, cancellation, or an unconfirmed response can follow execution and leave the outcome unknown. Inspect available state before any repeat. Obtain a fresh explicit user decision for the exact target and action before any repeat. A backend `retryable` error does not grant permission to repeat.

Entity controls use fresh exact `conversation: true` exposure. Previous reads do not authorize later writes. For `automation.trigger`, instead read the exact automation resource, review its text and revision, and establish explicit user authority for its live effects. Clients with `mcp:use` can run any resolvable editor-managed automation with confirmation and revision under the configured administrator Home Assistant token; nested effects are not recursively Assist-exposure-authorized. Do not author or edit an automation to bypass a denied entity control. This skill grants no mutation permission; unlocking and opening can have physical safety consequences even though entity controls have no additional confirmation field.
