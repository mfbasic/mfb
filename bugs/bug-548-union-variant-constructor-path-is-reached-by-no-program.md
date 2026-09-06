# bug-548: the union-variant `Constructor` path in `builder_values.rs` is reached by no program in the tree

Last updated: 2026-09-05
Effort: small (30m–1h) to decide; medium if it turns out reachable
Severity: LOW
Class: Dead code (suspected) / Coverage

Status: **OPEN — measured dead, not yet proven unreachable.** Filed rather than
deleted: two independent sweeps agree nothing reaches it, but "no committed
program reaches it" and "no valid program can reach it" are different claims,
and only the second justifies deleting ~85 lines of codegen.

## What it is

`CodeBuilder::lower_value`'s `NirValue::Constructor` arm has two halves
(`src/codegen/engine/value/builder_values.rs`, the arm beginning
`NirValue::Constructor { type_, args }`):

```rust
if self.type_model.record_fields.contains_key(type_) {
    ...                 // the record path — allocates, inlines String fields, returns
}
let register = self.allocate_register();
let tag = self.type_model.union_variant_tags.get(type_)...   // <-- from here down
```

Everything after that `if` is the **union-variant** path: it looks the type up in
`union_variant_tags`, derives the including union's name from `union_variants`,
sizes the block as `8 * (1 + max_payload)` the way `UnionWrap` does (the
`bug-175 C` comment), and writes the tag. About 85 lines.

It runs only for a `Constructor` whose `type_` is **absent from
`record_fields`** and **present in `union_variant_tags`** — a union variant that
is not a declared record. Every union member in the language is declared with
`TYPE ... END TYPE` (`mfb spec language types`, "A union may include the members
of another concrete union"), so it is not obvious what produces that pairing
today.

## The measurement

Two independent sweeps, both zero, with an `eprintln!` at the first line of the
union half:

1. **Every unit test.** `cargo test -p mfb --bin mfb` — 3,900+ tests, including
   the 424-fixture lowering corpus across five backends and the 417-fixture
   diagnostic corpus. `grep -c PROBE-UNION-CONSTRUCTOR` = **0**.
2. **Every committed program.** A probe-instrumented `mfb build -ncode
   -target linux-x86_64` over every `project.json` under `tests/` and
   `examples/` — around 700 projects. **0** of them reached it.

The union fixtures were checked individually first, in case the corpus list was
the gap rather than the code: `tests/rt-behavior/arena/flat-union` (`LET a AS
Shape = Dot[7]`, the textbook construct-into-a-union form) and
`tests/rt-error/types/types-union` (which also exercises `UNION ... INCLUDES`)
both reach the **record** path and return before the union half. Both are in the
corpus already.

## Why it matters, and why it is only LOW

It is not a correctness bug on its face — nothing miscompiles, because nothing
runs it. It matters for two reasons:

* **AGENTS.md forbids dead code.** If no program can reach it, it should be
  deleted, and `UnionWrap` documented as the one path that tags a union value.
* **It is ~85 lines of the per-file coverage gate** that no test can close,
  which is what turned it up. `src/codegen/engine/value/builder_values.rs` is
  the largest remaining gap in that task (381 lines short, 79.74%), and this is
  a quarter of it.

The risk of getting it wrong points the other way, which is why this is a report
and not a deletion: if some shape DOES reach it — a variant declared in an
imported `.mfp` package, say, where `record_fields` is populated from the
importing module's own types — then deleting it turns a working construction
into `native code union variant '...' does not resolve`.

## What would settle it

Either half is enough:

* **Reachable.** Find one program that reaches it. The imported-package
  direction is the most promising: `testutil::check_src_with_imports` and
  `ir::ImportedTypeDef` exist precisely because a type can arrive from a `.mfp`
  without the importing module declaring it, and `record_fields` is built from
  the module's own type table. If a `.mfp` exporting a union variant produces a
  `Constructor` for it, that is the shape — write the fixture and the path is
  live.
* **Unreachable.** Show that `record_fields` is populated for every type that
  can appear in `union_variant_tags` (they are both built in
  `codegen/engine/types`'s `TypeModel`), then delete the half and let the
  corpus + `test-accept.sh` prove nothing moved.

## How it was found

`planning/tests.md` (the per-file coverage gate task), while ranking the
remaining gaps. The probe is described above; it was reverted, and
`src/codegen/engine/value/builder_values.rs` is byte-identical to HEAD.
