---
name: inspect-thread-matter
description: Inspect bounded Thread networks, border routers, and Matter registry or device diagnostics; use narrowly authorized preferred-selection or interview maintenance only when explicitly requested.
---
# Inspect Thread And Matter

1. Read [network guidance](references/networks.md) and current Thread/Matter schemas.
2. Read `smarthome://thread/networks` and `smarthome://matter/devices`; use `query` for computed readiness, discovery, diagnostics, or ping. Use exact returned identifiers and report only observed facts.
3. Stop before preferred-network/router selection or device interview unless the user explicitly authorizes that exact administrator mutation.

Submit each authorized mutation once. Do not retry automatically. Timeout, cancellation, or an unconfirmed response can follow execution and leave the outcome unknown. Inspect available state before any repeat. Obtain a fresh explicit user decision for the exact target and action before any repeat. A backend `retryable` error does not grant permission to repeat.

Assist entity exposure does not authorize these non-entity operations. Skills grant no extra permission. Never request operational TLVs, Thread credentials, unsupported commissioning, fabric removal, or commissioning windows.
