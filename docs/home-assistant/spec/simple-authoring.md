# Simple Native Authoring

The [resource-first interface](resource-first.md) owns the public schemas. Authoring
uses the existing Home Assistant native APIs, targeting 2026.9.3 without Core or
component authoring changes. Blueprint reads still need the existing semantic
blueprint reader integration.

## Writer Assumption

Only one writer may change scene, automation, or blueprint configuration during an
authoring operation. Resource revisions are observations of exact displayed UTF-8
text, not native revisions or compare-and-swap tokens. Native GET/list preflight and
an immediate second read detect some intervening changes, but cannot prevent a
change after the second observation. No locks, transactions, or atomic create-only
guarantees are claimed. In particular scene/automation POST can upsert in a race.

## Native Endpoints

Scene and automation resources use `GET /api/config/{scene|automation}/config/{key}`
and deterministic pretty JSON. Edits and creation write the complete validated
object with POST to that same fixed endpoint. Only a verified GET 404 permits
creation; all other read failures stop the operation.

Blueprint resources use `smarthome_mcp/blueprint/get` semantic YAML. Creation checks
the complete native `blueprint/list` result directly, including invalid blueprint
entries, rather than the bounded public catalog. Native `blueprint/save` uses fixed
domain `automation`, validated path, complete YAML, and `allow_override: false` for
creation or `true` for editing. Its acknowledgment must contain a boolean
`overrides_existing`; creation cannot acknowledge an override.

## Validation Boundary

The wrapper checks input envelopes, URIs, identifiers, edit operations, unique
matches, and all text byte bounds locally. JSON candidates use the current
native-object validator, including depth 32 and identifier consistency.

Blueprint candidates retain the management path/text constraints and the 256 KiB
bound. The local `serde_yaml_ng` parser requires one YAML document with a mapping
root. It rejects syntax errors, duplicate keys, invalid aliases, and malformed
trees, with its default recursion protection intact. Its semantic tree preserves
Home Assistant `!input` tags. The wrapper compares parsed trees before and after
edits; comments, key order, or formatting alone do not authorize a native save.

Local JSON/YAML parsing does not implement the complete Home Assistant domain
schemas. Native config POST and `blueprint/save` still validate those schemas
before persistence. A locally valid tree can therefore receive a native schema
rejection. The wrapper does not rewrite source or silently strip tags.

## Outcomes

Every candidate is submitted at most once. A valid acknowledgment means native
persistence accepted the configuration and reload may be scheduled, not that reload
or activation completed. No returned revision is fabricated from the submitted
candidate: read the resource again to obtain the exact normalized text/revision.
For native blueprint saves, only valid `invalid_format` and `already_exists`
rejection envelopes count as definite rejection. Core 2026.9.3 emits those codes
before persistence. `unknown_error` can follow a partial write; unknown codes and
malformed rejection envelopes also leave the outcome unknown. This classification
applies only to authoring saves, not general WebSocket reads or controls.
Timeout, cancellation, lost connection, or malformed acknowledgment after possible
dispatch is `mutation_outcome_unknown` with `retryable: false`. Inspect state and
obtain a new explicit decision rather than retrying the write. Cancellation does
not undo persistence.

Transport cancellation can close the response before the wrapper delivers an
error. A missing response also leaves the mutation outcome unknown; it does not
authorize a retry. The client connection and local admission permit are released,
but Home Assistant can still complete persistence.
