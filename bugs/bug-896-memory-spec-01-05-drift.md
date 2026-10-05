# bug-896: memory spec (heap-values, collections) drift — stale sort description, set literal "tight", union tag, map stride, grow-family list, dangling §15.1

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (documentation)

Status: Open
Regression Test: none (spec text); verification is rendering `mfb spec memory <topic>`

Six statements in `src/docs/spec/memory/03_heap-values.md` and
`src/docs/spec/memory/05_collections.md` disagree with the code at HEAD and with
what a compiled program actually does. The code is correct in every case (the
observable value semantics are right); the spec is stale or imprecise. **The
correct outcome is the spec text rewritten to match the code**, as listed per
item below.

References:

- `.ai/spec-content.md` / `.ai/specifications.md` — spec must match code (Hard Completion Gate).
- Related: bug-857 (language spec §9–§15 drift, which covers the nonexistent `§15.4`); item 6 here is the same missing-heading problem from the memory spec.
- Found during hardening batch B07.

Tooling used for the layout observations: `mfb dbg` on a `--debug -O0` build,
breaking on a one-line helper `FUNC pk(m AS …) AS Integer` and dumping `show mem x0 <n>`
(the debugger needs a signed copy: `cp /tmp/hardening/mfb /tmp/hardening/B07/mfbdbg && sh scripts/sign-debugger.sh /tmp/hardening/B07/mfbdbg`).

## 1. Union tag is a global canonical id, not the "active variant index"

Spec (`03_heap-values.md`, *Union*): "`+0   U64 tag       ; active variant index`".

Code: `src/codegen/engine/validation/validation.rs` (doc comment above
`impl TypeKinds for TypeModel`, and `module_tables`): "The tag is assigned
**globally-canonically** (sorted variant name) by `recompute_canonical_variant_tags`,
independent of a variant's position within any including union" (bug-80).

Repro (`/tmp/hardening/B07/lay4`):

```
TYPE Inn … END TYPE
TYPE R … END TYPE
UNION U
  R
  Inn
END UNION
' LET u AS U = Inn[9, "ab"]   -> tag word 0
' LET u2 AS U = r              -> tag word 1
```

Observed `show mem x0 32` at a `pu(u AS U)` breakpoint: `Inn` value starts
`00 00 00 00 00 00 00 00 2b 00 …` (tag 0, size 43), `R` value starts `01 00 … a0 00 …`
(tag 1) — `R` is declared first, so a positional "variant index" would be 0.

Fix: say the tag is the variant's canonical id (its rank among all variant names
in the program, sorted), the same in every union that includes it; `MATCH`
compares against that per-variant constant. Same wording for the resource-union
`activeMemberTag`.

## 2. `collections::sort` source body is a merge sort, not an insertion sort

Spec (`05_collections.md`, end of *`set`*): "The source `collections::sort` is an
insertion sort built on `set`, so its per-swap `items = collections::set(items, j, …)`
overwrites run in place: the sort is a stable in-place O(n²)-comparison / O(1)-swap
pass over a copy of the argument".

Code: `src/codegen/builtins/collections/func_sort.rs` `BODY` begins
"' Stable bottom-up merge sort — O(n log n). Equal elements keep their original
order … matching the previous insertion sort's stability". It merges `src` into
`dst` with `dst = collections::set(dst, k, …)` per width pass. `sortBy`'s body
(`func_sort_by.rs`) is likewise a keyed merge.

Fix: describe the body as the stable bottom-up merge sort (O(n log n)
comparisons) that writes each pass with `set`.

## 3. Copying `sort` of a `List OF String` does NOT leave the data region untouched

Spec (`05_collections.md`, *Payload Order*): "`collections::sort` on a `List OF String`
relies on the permutation directly — it swaps the fixed-size entry records and
leaves the data region untouched." And (*`set`*): a copying sort whose shape the
word merge declines "sorts that copy with the self-update arm's stable
index-permutation merge".

Observed (`/tmp/hardening/B07/lay6`, `xs = ["pear", "apple", "fig"]`):

- `LET s = collections::sort(xs)` → data region `a p p l e f i g p e a r` — the
  payloads are gathered into sorted order (the String "index merge with a gather",
  `func_sort.rs:sort_fast_path` → `sort_word_path_accepts` →
  `lower_collection_sort_call`).
- `MUT t = xs` / `t = collections::sort(t)` (self-update) → data region
  `p e a r a p p l e f i g` unchanged, entries permuted — the statement is true
  only for this form.

Fix: restrict the "swaps entries, data untouched" sentence to the in-place
self-update (`emit_sort_buffer` → `lower_list_permute_in_place`), and say the
copying `sort` of `String` (and signed 8-byte) elements gathers a fresh, packed list.

## 4. A `Set` literal is not tight

Spec (`05_collections.md`, *Sets*): "a set literal `Set OF T { … }` builds a tight
block (`capacity == count`) with each distinct element inserted once." Also
*Capacity Headroom*: "Literals … allocate exactly (`capacity == count`,
`dataCapacity == dataLength`)" and *Header Fields*: `bucketsReady` "is `0` on every
fresh … map".

Code: `src/codegen/collection/layout/builder_collection_layout.rs:lower_set_literal`
starts from `lower_empty_collection` and inserts every element with
`lower_map_set_in_place`, i.e. the geometric grow path.

Observed header words (`/tmp/hardening/B07/lay7`):

| literal | count | capacity | dataLength | dataCapacity | bucketsReady |
| --- | --- | --- | --- | --- | --- |
| `Set OF Integer { 1, 2, 3 }` | 3 | 4 | 41 | 64 | 1 |
| `Set OF Integer { 1, a, 1, 5, 2 }` (a = 5) | 3 | 4 | 41 | 64 | 1 |
| `Set OF String { "x", "yy", "zzz", "w", "v" }` | 5 | 8 | 13 | 64 | 0 |
| contrast `Map OF String TO Integer { "Ada" := 36 }` | 1 | 1 | 16 | 16 | 0 |

Fix: say a set literal is built by inserting each element into a growing buffer
(so duplicates collapse) and carries the grow path's headroom and, possibly, a built
index; only `List`/`Map` literals are exact. (Or make `lower_set_literal` tighten —
owner's call; nothing observable depends on it.)

## 5. Fixed-width map entry stride formula

Spec (`05_collections.md`, *Capacity Headroom and Growth*): for a `Map`/`Set` entry
whose key and value are both fixed-width, "`dataCapacity = capacity × stride`, where
the stride is … for a map entry the key and value each padded to their alignment."

Code: `src/codegen/collection/map/map_mutate.rs` (grow arm of
`lower_map_set_in_place`): `stride = (key_width.next_multiple_of(value_align) +
value_width).next_multiple_of(key_align)` — the value is padded up to the *key's*
alignment so the next entry's key is aligned.

Observed (`/tmp/hardening/B07/lay5`, `Map OF Integer TO Byte` built by five `set`s):
entries at data offsets 0/8, 16/24, … (stride 16), `capacity 8`, `dataCapacity 128`.
"Each padded to their alignment" gives 8 + 1 = 9.

Fix: state the formula the code uses: key, padded to the value's alignment, then
the value, the whole rounded up to the key's alignment.

## 6. `String` grow family list is incomplete; `§15.1` is dangling

(a) Spec (`05_collections.md`, *Self-updates*): "**grow** — `strings::padLeft`,
`padRight`, `padLeftToWidth`, `padRightToWidth`, `repeat`, `os::appResourcePath`".
Code: `src/codegen/collection/assign/string_self_update.rs:STRING_GROW_FNS` also
lists `os::appDataPath`, `os::appCachePath`, `os::userHomePath`,
`os::userDocumentsPath` (`GrowKind::HostPath`, plan-157-B). Observed: `s = os::appDataPath(s)`
equals the copying form (`/tmp/hardening/B07/hp`). Fix: add the four host-path members.

(b) Spec (`03_heap-values.md`, *The owner word*): "(`./mfb spec language
resource-management` §15.1)". `grep -n "^#" src/docs/spec/language/15_resource-management.md`
shows no `15.1` heading (headings are `### The owner of a resource`, `## 15.5`, `## 15.6`, …),
the same defect bug-857 records for `§15.4`. Fix: cite the *The owner of a resource* section by name.
