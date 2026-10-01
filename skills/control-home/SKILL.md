---
name: control-home
description: Operate one Assist-exposed Home Assistant entity using fixed light, switch, fan, cover, climate, media-player, lock, or scene controls.
---
# Control Home

1. Read [control guidance](references/controls.md) and the current `execute` schema.
2. Resolve the exact target through `smarthome://entities` and its state links; establish user authority for the requested physical effect.
3. Invoke only the requested fixed action, then report the minimal acknowledgment without claiming a verified resulting state.

Submit each authorized mutation once. Do not retry automatically. Timeout, cancellation, or an unconfirmed response can follow execution and leave the outcome unknown. Inspect available state before any repeat. Obtain a fresh explicit user decision for the exact target and action before any repeat. A backend `retryable` error does not grant permission to repeat.

Every control uses fresh exact `conversation: true` exposure. Previous reads do not authorize later writes. This skill grants no mutation permission; unlocking and opening can have physical safety consequences even though the API has no additional confirmation field.
