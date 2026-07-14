# bug-231: syntaxcheck resource-union arms iterate empty variant.fields → union misclassified as non-resource/copyable

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness

Status: Open

`contains_resource_or_thread_with_seen` (`src/syntaxcheck/resources.rs:99-119`)
and `is_copyable_type_with_seen` (`:211-236`) both iterate `variant.fields`, which
is always empty for a resource-union variant (the variant name is a registered
resource, not a record), so a resource union is misclassified as containing no
resource and as freely copyable — the bug-173-F pattern fixed only in
`is_thread_sendable_type_with_seen`.

Trigger: `UNION Handle` / `File` / `END UNION` (File is a registered resource)
used as a Map key: `Map OF Handle TO Integer`. `contains_resource_or_thread`
returns false, so `TYPE_COLLECTION_OWNERSHIP_VIOLATION` is not emitted here.

Impact is LOW: the map-key case is backstopped by `ir::verify` (which uses
`is_resource_or_resource_union`), and every `is_copyable_type` caller checks
`is_resource_type` first — so the user still gets rejected, just from a later
pass. Still a genuine discrepancy vs the sibling and the ir::verify twin.

Fix: in both Union arms, short-circuit per variant on
`self.resource_registry.is_resource(&variant.name)` (contains → true; copyable →
false), mirroring `is_thread_sendable_type_with_seen`.
