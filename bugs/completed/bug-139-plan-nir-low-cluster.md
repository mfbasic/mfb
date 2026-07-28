# bug-139 — plan/nir LOW cluster: dead constant-fold machinery, dead CallKind::Import, dedup drops literals, dumps omit LINK/provenance, CallResult default-arg gap, link-thunk symbol collision

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, plan/nir slice). Independent
LOW / latent / docs findings in `src/target/shared/plan/` and
`src/target/shared/nir/`, batched per goal-02. (The HIGH `.result` finding from
this slice is folded into bug-119.)

## 1. Plan-layer constant-fold machinery is dead code with a divergence trap (dead-code)

`plan/symbols.rs:395-523 + 622-737` (`native_constant_value`,
`native_static_string_value`, `native_static_graphemes_value`,
`native_primitive_text`); `plan/function_builder.rs:241-252,119-235`. The skip
guards can never fire: every foldable target (`toString`,
`strings.upper/lower/caseFold/normalizeNfc`, `strings.graphemes`) is in
`runtime::is_native_direct_call`, so nir/lower.rs always produces them as
`NirValue::Call`, never `RuntimeCall`. Also keeps loop-entry constants live
through While/DoUntil bodies, diverging from codegen's bug-57
`clear_local_constants()` — inert today (verified: fold in a WHILE body compiles
and runs correctly) but a real miscompile/missing-symbol trap if a foldable
helper-backed call is ever added. Fix: delete the dead fold machinery or wire it
to the real fold path.

## 2. `CallKind::Import` is unreachable (dead-code)

`plan/lower.rs:26` (`import_symbols = HashMap::new()`),
function_builder.rs:381-382, plan/mod.rs:114-119 (enum), json.rs:146.
`lower_module_for_platform` inserts all module imports into `function_symbols`
and always passes an empty `import_symbols`, so `add_call` can never classify a
call as `CallKind::Import`; the variant, its validation arm (mod.rs:307), and
its JSON name are dead. Fix: remove the variant.

## 3. `push_call_with_literals` dedup drops later call-sites' string literals (correctness — model artifact only)

`plan/function_builder.rs:415-434`. A second call to the same (target, symbol)
early-returns and discards the new call's `string_literals` instead of merging
them, so the `.nobj` object-plan `__cstring` model under-reports strings. Real
binaries are unaffected (string data comes from the code layer's
data_objects.rs) — inaccurate parallel model/gate artifact, not a miscompile.
Fix: merge the literal sets.

## 4. `-nplan`/`-nir` dumps silently omit LINK and provenance fields (docs)

`plan/mod.rs:215-243` (`NativePlan::to_json` omits `link_symbols`);
`nir/json.rs:3-43` (`NirModule::to_json` omits `link_functions`), :279-314
(`NirFunction::to_json` omits `file` and `resource_owners`). The structs carry
load-bearing fields (object plan, error-loc, §15.6 ownership) but the debug
dumps drop them, so a LINK-using program's artifacts are incomplete. No parser
reads these dumps back → no round-trip corruption. Fix: emit the fields.

## 5. CallResult lowering skips the Call arm's default-argument normalization (latent)

`nir/lower.rs:437-443` (`IrValue::CallResult` arm) vs :394-415 (Call arm
fixups). The Call arm appends the defaulted `"read"` mode for 1-arg
`fs.openFile`/`openFileNoFollow` and the `fs.tempDirectory` arg for 0-arg
`fs.createTempFile`; the CallResult arm (from `lower_inline_trap`, preserves args
verbatim) does not, so an inline-TRAP'd 1-arg `fs::openFile(p)` would reach
codegen with the wrong arity. Currently unreachable — inline TRAP on any
File-returning builtin fails earlier ("native code cannot materialize default
value for type 'File'"). Fix: apply the same default-arg normalization in the
CallResult arm.

## 6. `link_thunk_symbol` sanitization can collide two distinct LINK bindings (footgun, latent)

`nir/mod.rs:38-51`. `_mfb_linker_{sanitize(alias)}_{sanitize(name)}` with `_` as
both separator and replacement char is ambiguous: alias `a_b` + name `c` and
alias `a` + name `b_c` both produce `_mfb_linker_a_b_c`. Two valid distinct LINK
bindings collide; the object plan's duplicate-defined-symbol guard turns this
into a confusing build error on valid source (not a silent misroute). Fix: use
an unambiguous separator/escaping. (Distinct from bug-79 §2, duplicate labels
inside `lower_link_thunk`.)
