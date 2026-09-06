# bug-548: two paths in `builder_values.rs::lower_value` are reached by no program in the tree

Last updated: 2026-09-05 (second path added)
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

## The second path: `CallResult` on a function-typed LOCAL

`lower_value`'s `NirValue::CallResult` arm opens with

```rust
if let Some(local) = self.locals.get(target).cloned() {
    if matches!(local.type_, ParameterType::Func(_, _, false)) {
```

and about ninety lines follow it: load the callable out of its stack slot, call
through it, materialize the `Result`. It is the fallible indirect call — `f(x)`
where `f` is a binding of function type and the callee can `FAIL`.

**The outer `if let` never binds.** An `eprintln!` at the top of the arm,
printing `target` and `self.locals.get(target)`, across every in-process program
— the 424-fixture corpus, the 63 package-bearing fixtures, and every
hand-written suite — reports `local=None` on every single hit. The targets are
all function names (`toInt`, `tls.read`, `#http_buildResponse`, user functions);
not one is a local.

This is not for want of a program that does it.
`tests/rt-behavior/functions/function-value-error-propagates-rt` is in the
corpus and is precisely this shape — it exists for plan-120-E, "a FAIL inside a
function called through a FUNC-typed VALUE must propagate to the caller's TRAP"
— and it lowers through the same arm with `local=None`. So the indirect fallible
call reaches codegen as something other than a `CallResult` naming its callable,
and this branch is waiting for a shape the front end does not produce.

Same disposition as the union path above: measured dead, not proven unreachable,
and the same two ways to settle it. If a shape does produce it, that fixture is
the place to look for why it does not today; if none can, ninety lines go, and
whatever DOES lower `function-value-error-propagates-rt` becomes the single
documented path.

## How it was found

`planning/tests.md` (the per-file coverage gate task), while ranking the
remaining gaps — `builder_values.rs` is the largest one left (373 lines short,
80.17%) and these two paths are about half of it. Both probes are described
above; both were reverted, and
`src/codegen/engine/value/builder_values.rs` is byte-identical to HEAD.
