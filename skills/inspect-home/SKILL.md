---
name: inspect-home
description: Inspect Assist-exposed Home Assistant entities, devices, current states, history, or camera snapshots without changing the home.
metadata:
  authority: assist-exposure
x-smarthome:
  workflow: inspection
---
# Inspect Home

1. Read [query guidance](references/queries.md) and the current `home_assistant_query` input schema.
2. Discover exact entities with `entity.list` or grouped devices with `device.list`; do not guess targets.
3. Request only the needed states, history interval, or camera frame. Report observed facts and freshness limitations.

Each entity request refreshes Assist exposure and fails closed unless the exact target has `conversation: true`. Earlier discovery is not authorization for a later request. Skills do not grant permission or change endpoint-wide `mcp:use` authority. Do not call removed `help` actions.
