---
name: inspect-home
description: Inspect Assist-exposed Home Assistant entities, devices, current states, history, or camera snapshots without changing the home.
metadata:
  authority: assist-exposure
x-smarthome:
  workflow: inspection
---
# Inspect Home

1. Read [query guidance](references/queries.md), resource discovery, and the current `query` input schema.
2. Read `smarthome://entities` or `smarthome://devices` and follow canonical links; do not guess targets.
3. Request only the needed states, history interval, or camera frame. Report observed facts and freshness limitations.

Each entity request refreshes Assist exposure and fails closed unless the exact target has `conversation: true`. Earlier discovery is not authorization for a later request. Skills do not grant permission or change endpoint-wide `mcp:use` authority. Do not call removed `help` actions.
