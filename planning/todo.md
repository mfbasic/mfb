No changes made — this is all read-only analysis. Here's the picture of what's below the `// Everything below this should be depricated` line (lines 985–1352, before the `#[cfg(test)]` block).

## The headline finding

**None of the free functions below the line own a real algorithm.** Everything algorithmic already lives *on the registry types* as methods — `RegistryFunction::select` (with the private `unify`/`substitute` engine), `::arity`, `::declares_error`, `Body::rewrite_target`, `Registry::resolve_func`/`resolve_type`/`augment_project`. The free functions are all one of two things:

1. **Trivial delegators** — call one registry method, adapt the result (`is_some()`, `.map(field)`).
2. **String↔`ParameterType` boundary adapters** — marshal the type-checker's `String` arg-types into a `CallShape`, call a real method, and marshal back out (often via `Box::leak`).

That distinction is the whole answer to your migration question, and it's cleaner than "wrapper vs. real work."

## Bucket 1 — Pure/thin wrappers (7) → delete, repoint callers to `registry().*`

Each delegates to something that already exists on a type. Nothing to "move" — the caller just calls the method directly.

| shim | delegates to | callers |
|---|---|---|
| `augment_project` (989) | `registry().augment_project` (1:1) | ir/lower:94, syntaxcheck:163, resolver:109 |
| `is_member` (1000) | `resolve_func().is_some()` | ir/lower:2098, builtins:397,622 |
| `owning_package` (1006) | `resolve_func().map(.package.import_name)` | ir/lower:2099, verify/compat:299, builtins:437, process:206 |
| `arity` (1198) | `RegistryFunction::arity` | builtins:447 |
| `declares_error` (1229) | `RegistryFunction::declares_error` | builder_error_emission:20 |
| `qualified_builtin_type` (1216) | `resolve_type()` + enum→name | builtins:166 |
| `is_builtin_type` (1206) | packages/records/unions scan (no method yet, but a trivial query) | builtins:113 |

## Bucket 2 — Boundary adapters (7) → the *shim* dies, the logic does NOT move

These look like "real work," but the only work is string marshalling + `Box::leak` (and a couple of overload guards). The algorithm they wrap is already a method.

| shim | wraps | the "work" it adds |
|---|---|---|
| `resolve_call` (1176) | `RegistryFunction::select` | build `CallShape`, echo `Arg(n)` string. *Not marked deprecated* — author flags it a permanent boundary. |
| `rewrite_target` (1241) | `select` + `Body::rewrite_target` | `CallShape` + single-overload fallback |
| `native_lower` (1262) | scan impls for `Body::Native.common` | *Not marked deprecated* — the codegen dual-path seam |
| `call_return_type` (1019) | `select`/`resolve_func` | `contains_var` guard + `Box::leak` |
| `expected_arguments` (1279) | first param's `.ty.name()` | overload-count guard + `Box::leak` |
| `call_param_names` (1298) | params/aliases | overload guard + table build |
| `default_argument_padding` (1327) | params past `provided` | `Fill` filter + `Box::leak` |

The critical point: the four `Box::leak` ones (`call_return_type`, `expected_arguments`, `default_argument_padding`, and the string side of `resolve_call`/`rewrite_target`) exist **only because the type checker and IR still speak `String`, not `ParameterType`**. Their own comments say "this leaks, once migration is complete it goes away." So they don't get relocated onto a type — they **evaporate** once the callers (all in `src/builtins/mod.rs` and `src/ir/lower.rs`) are converted to consume `ParameterType`/`Selection` directly.

## Bucket 3 — Private unification engine (5) → keep; it's the type system

`unify` (1056), `substitute` (1127), `leaf_matches` (1043), `resource_base_eq` (1115), `contains_var` (1151). These are real work and permanent — but they're **not shims**. `select` (line 461, on `RegistryFunction`) already consumes `unify`/`substitute`; they're module-private free fns purely by style. Only `contains_var` is currently reached from a shim (`call_return_type`). Leave these; optionally fold them into the `impl` region for clarity.

## What this means for your cleanup

- **Nothing needs to be "moved into the registry" first.** The algorithms are already on the types. This is the good news — you're not blocked on relocating logic before migrating callers.
- **The migration is a caller-conversion, not a logic-relocation.** All 14 shims funnel through three consumers: `src/builtins/mod.rs` (the old dispatch hub — 10 of the call sites), `src/ir/lower.rs` (5), and `src/target/shared/code/*` (2). Convert those to `registry().resolve_func(...)` / `.select(...)` and the shims delete.
- **Sequence:** Bucket 1 can be deleted piecemeal today (pure delegators). Bucket 2 blocks on making the type checker / IR carry `ParameterType` instead of `String` — do that boundary change and 6 of the 7 leak-adapters vanish rather than migrate. `resolve_call` and `native_lower` are the two the author deliberately left un-`#[deprecated]`; treat them as the intended permanent seams unless you also push `CallShape`/`Selection` up into the callers.

Want me to trace exactly what each of the `src/builtins/mod.rs` call sites needs from these (e.g. whether `builtins::resolve_call`'s caller can take a `Selection` directly), so you can see how much of the old `targets/shared` branch collapses when the string boundary moves? That's the piece that tells you how much of the second branch disappears.

---

Here's the order I'd follow, with the gating reason for each boundary:

## Phase 0 — free prep (do first, independent of everything) (done)

- **Delete Bucket 1** (the 7 pure delegators: `is_member`, `owning_package`, `arity`, `declares_error`, `qualified_builtin_type`, `is_builtin_type`, `augment_project`). These are gated on *nothing* — repoint callers to `registry().resolve_func(...)` today. Doing this now shrinks the shim surface so the later phases are legible.
- **Extract `ParameterType` to its neutral module** (the leaf move). Do it *before* migrating more packages, so every package you migrate next already references the type from its final home — otherwise you re-churn `use` paths across all of them later.

Neither of these touches the compiler's string currency, so they're low-risk and unlock the rest.

## Phase 0.5 — package-scope resources ([plan-97], bug-441) — BEFORE the resource-owning packages migrate (done)

Builtin resource type names (`File`, `Socket`, `Process`, …) are a global, unqualified **bare-name** reservation, so a user `TYPE Process` collides with the builtin (bug-441). Make them `pkg::Name` (`process::Process`), like every other builtin surface. See `planning/plan-97-resources-package-scoped.md`.

**Why here, not later:** the cost scales with migrated resources. `fs`/`net`/`tls`/`audio` (File/Socket/Listener/UdpSocket/AudioInput/AudioOutput/TlsSocket/TlsListener) all migrate in Phase 1 and each calls `add_resource` with a *bare* name — exactly as `process` already did. Fix the scoping *first* and each of those migrations adopts qualified resources for free; fix it *after* (or at Phase 3) and you re-qualify ~8 more resources plus their syntax/spec/goldens, and double-touch `ParameterType::Named` (bare then qualified). Not Phase 3.

- The cheap non-breaking interim (bug-441 Phase 2a — a "name collides with a builtin resource" diagnostic) can land anytime, independently.
- plan-97 (the real package-qualification) is breaking/spec-touching — run it as its own plan, sequenced right after Phase 0's `ParameterType` extraction and before the `fs`/`net`/`tls`/`audio` migrations below.

## Phase 1 — finish the package migration (`target/shared → codegen`)

Migrate each remaining builtin into the registry, **leaning on the Bucket 2 shims as the bridge**. This is the key realization: Bucket 2 is *scaffolding* — its whole purpose is to let the still-String-speaking compiler consume registry packages. You keep it alive precisely so you can migrate packages *without* touching the compiler's currency yet. Per package: parity-test against the old path, then delete that package's old path.

> ⚠️ The resource-owning packages here (`fs`/`net`/`tls`/`audio`) depend on **Phase 0.5 / plan-97** landing first — otherwise they register bare-name resources that plan-97 must then re-qualify. ✅ Phase 0.5 is done, so this dependency is satisfied.

Already migrated (on the registry, `src/codegen/builtins/`): `encoding`, `collections`, `csv`, `json`, `regex`, `datetime`, `process`. The 21 packages below still have old-branch files (`src/builtins/<pkg>.rs` + descriptors in `target/shared`) and need migrating. Ordered by `planning/migratelist.md` tiers (cheapest → heaviest); the coupled clusters must migrate together. Playbook: `planning/migrate.md`.

Tier 2 — source companion + light native:
- [x] `money` — DONE (registry `add_enum(Rounding)` + 3 `Body::native` NativeLower funcs; e6ab61d9b, merged)
- [x] `os` — DONE (15 native OS-seam members; `resourcePath` consumes build-context; 0 resource; 0a274639f, merged)
- [x] `io` — DONE (15 members; consumes arena ctx for TUI routing + cooked-mode; needed the OsLowerCtx extension; 0f67f66fc, merged 505d634a0). Bundled OS-seam context into `OsLowerCtx { build_mode, module_name, term_state_offset, presentation_mode_offset }` (c76a70db9). fs `.ncode` byte-identity debt also cleared (768b1a5d9).
- [x] `fs` — DONE (41 members, `File` resource; 73374e779, merged d98f31872). Surfaced + fixed a registry-core gap: strict `leaf_matches` now requires `resource_base_eq` for nominal-vs-nominal so a resource UNION can't satisfy a concrete resource close-op param (`fs::close(<union>)` rejected).
- [x] `app` — DONE (9989e4bca). 2 members (`getMode`/`setMode`) + `Mode` enum (`add_enum`, money/datetime idiom). NOT name-overloaded in practice — the two members are plain distinct names, NO resolver/selector needed. Native OS-seam: both `Body::native` posix/win over `native::lower_app_helper` reading `ctx.presentation_mode_offset` (the io-added `OsLowerCtx` field), routed generically via `dispatch_runtime_helper` (crypto/audio idiom); runtime specs DERIVED (deleted `app_specs.rs`). The cross-package `ErrWrongMode` gate (`prepend_wrong_mode_gate`, gates term/io) STAYS in the shared code layer (`code/app.rs`, slimmed). Count 7→6. Gate: `.run`/`.ast` + all 3 `macos-app-mode-*` `.app.ncode` byte-identical; regen 1 `.ir` (Mode enum injected line 24→1).
- [x] `vector` — DONE (9 `add_record` Vec types, 19 members, 42 constants w/ components, 9 toString overrides; 6eaa54117, merged). SIMD carrier (`builder_vector_inline.rs`) KEPT SHARED (codegen-wide hook, not per-call). NOTABLE: the coarse value-nominal matcher (needed for json union widening) CAN'T distinguish concrete records (Float3 vs Integer3), so vector keeps a THIN exact selector in `vector/mod.rs` over its registered overload data (reproduces VectorResolver byte-for-byte, excluded from the generic path, ZERO shared-matcher change). `.run` byte-identical (35 accept), 5 `.ncode` line-shift diffs deferred.

Tier 3 — coupled clusters (migrate together to avoid half-cut seams):
- [x] `net` + `http` — DONE (ff61adf44/ee0359bee, merged bda41d03f). net: 19 native OS-seam members (poll 2-overload, connectTcp 4-overload) + 3 source rewrites + Socket/Listener/UdpSocket resources + Address/Datagram records + Url source-type + toString(Url) override. http: all Body::Rewrite over source, handleRequest 2-overload (net.Listener vs tls.TlsListener) via generic resolver, value types via add_source_types. Count 11→8. Carried a `registry::rewrite_target` STRICT-first fix (lenient treated net.Listener/tls.TlsListener as interchangeable) — verified behavior-neutral (full acceptance 0 .run mismatch; targets byte-identical). Regenerated 22 net-source-injection sidecars.
- [x] `strings` — DONE (035653578, merged de6125808). HelperGate infra (PART A, f44c80eb8) + 39 members (29 native → shared `lower_strings_package_call`, 3 intrinsic find/mid/replace collections-coupled, 7 seam `Body::Rewrite` via `WhenUsed`+`WhenImported("astrings")` gated helper carrying seam+`__strings_genCat`). Native lowering KEPT SHARED (like vector's carrier). AttributedString Tier resolver → co-located IR-level rewrite. `artifact-gate strings`+`collections` 0 diffs, full acceptance 1263 `.run` byte-identical. Count 6→5.
- [x] `astrings` — DONE (39a25d6ca, merged). 18 members (fromString + readSpans/writeSpans/scalarLen native `internal_only:true`; bold/italic/…/clearAttributes(2-overload) `Body::Rewrite`); companion via `RegistryHelper::always`; `builder_astrings.rs` kept shared. Added `internal_only: bool` to RegistryFunction (honored via `is_internal_only_member`). 3683 tests, full acceptance byte-identical (11 `.ir` temp-renumber sidecars regen'd — astrings uses strings::toScalars so gencat always injected, legacy-consistent). Count 5→4.
- [x] `term` — DONE (ffa896ae6/f763cee24/dc005a42a, merged bb35edc96). 24 native OS-seam members (`Body::native_os_seam`, hand-authored `expected_arguments`; heavy emitters `lower_term_helper`/`emit_app_term_helper`/`console_data_objects`/`term_grid` stay SHARED like strings/vector carriers; dispatcher relocated to `term/native/`). TermColor/TermSize `add_record` (read-only via `term::is_read_only_record`), LineStyle/FillStyle `add_source_types`. The `term_astrings_bridge.mfb` __term_drawTextAttr → gated helper: the task's `WhenImported("astrings")` OVER-injected the bridge (it references `term::`/`TermColor`) into astrings-only programs → generalized to a new `HelperGate::WhenBothImported("term","astrings")` restoring legacy `bridge_uses_package`; drawText(String) vs drawText(AttributedString) = co-located `ir::lower` rewrite on `term::DRAW_TEXT`. Fixed one real bug: term routed through the arg-typed return oracle mis-resolved Byte-param setters called with Integer literals (`setForeground(255,128,0)` → Nothing regressed to Unknown) — excluded term from the arg-typed path so it resolves by name (matching legacy `TermResolver`). Presentation-mode gate stays shared. Count 4→3. `artifact-gate term`+`strings` 0 diffs; full acceptance 1263/1263 `.run` byte-identical (18 `.ir`/`.app.nir` regen'd — benign `add_record` companion TYPE-decl injection shift).
- [x] `crypto` — DONE (33 members: source `Body::Rewrite` + native per-backend; `_bytes`/`_text` selection = distinct overloads + rewrite_target, NO custom resolver; f149af4e6, merged). Confirmed the design's no-infra prediction. Gate 0 diffs, KAT vectors byte-identical.
- [x] `audio` — DONE (768002802, merged ca1857369). 11 members (native OS-seam devices/open/read/write/poll/available/xruns/close + source render/play), AudioInput/AudioOutput resources, 3 records. The 5 overload-split cases (named-device opens, timed read/poll, per-direction close) via an IR-level `audio::runtime_overload_name` (the tls.closeListener idiom), NOT the code-layer os_alias path (which broke required-helper emission + macOS import planning). 7926-line per-backend code relocated to `audio/native/`. Count 10→7 (with net/http). Gate 0 diffs, `.run` byte-identical.

Tier 4 — descriptor / data-only:
- [x] `errorcode` — DONE (43 constants; extended `RegistryConstant` with message+symbol to repoint the whole error-emission path; 52598bd4f/2c5999457, merged). Byte-identity 0 diffs (codes/messages/symbols registry-sourced); regenerated 1 io-cascade golden.
- [x] `testing` — DONE (5fc230f75, merged ae57236b8). 12 `Body::Intrinsic` assertions in a real-named `testing` clean-room package; desugar UNCHANGED at `src/testing/desugar/expect.rs` (invoked `ir/lower.rs`); `is_testing_call` → `resolve_package("testing").function(name)`; `expectTrap` optional `code` via `DefaultValue::Optional` (arity (1,2)). Added the REUSABLE man2 gate: `RegistryPackage::unqualified_global: bool` (default false) + `mark_unqualified_global()` + `is_unqualified_global()`; `man2.rs render_all_markdown` filters them (direct `mfb man2 testing` still resolves; only `--all` catalog skips). Count 2→1. 3675 tests, acceptance 1263/0 byte-identical (32 `artifact-gate all` diffs proven pre-existing on the base binary; NO `testing` byte-identity fixture exists — assertions are front-end desugars, not a `.ncode` package). GENERAL reuses `mark_unqualified_global()`.
- [ ] `general` — overridable global builtins (`len`/`toString`/`typeName`/conversions/predicates). DESIGNED (a954f3ce report). Register a REAL-NAMED `general` clean-room package for registry-answered facts (`declares_error`/membership/arity); co-located BESPOKE resolver (`resolve_call`, vector/strings idiom) for the argument-dependent returns + id/predicate/param-name surfaces; `general.`-prefix confined to the general lookup BOUNDARY so the callee stays BARE through IR/codegen and the `toString(VecN)`/`toString(Url)` overrides fire unchanged. Load-bearing: add the general branch to `resolve_call_return_type`/`call_return_type_name` BEFORE teardown, and wire `declares_error("general.toByte",…)` for the `raise_error` debug-assert.
- [x] `resource` — DONE (50dbf6083, merged 05021a89e). Delete-and-repoint: clean-room `RegistryResource` is a strict SUPERSET of legacy `ResourceInfo` (all 9 builtin resources already registered via `add_resource`), so `BUILTIN_RESOURCES` was redundant. Repointed `with_builtins()` + the four `is_builtin_*`/close-fn helpers onto `registry()`, deleted the LazyLock + empty `RESOURCE` module, collapsed `is_resource_type_name` to the bare-name scan, repointed 1 stale man citation. Count 3→2. 3675 tests, acceptance 1263/0 byte-identical (the 32 `artifact-gate all` diffs are pre-existing deferred `.ncode` debt, proven identical on the base binary).

DESIGN DECISION (both unqualified-global specials): REJECT empty-name package (`""`) — `resolve_package` has NO dup-name guard so two `""` packages (general+testing) collide, and it would edit the hot shared `resolve_func`/`resolve_package` `split_once('.')` path used by all 24 packages. USE the hybrid: real-named package + calls stay UNQUALIFIED end-to-end (qualified query surface requires a `.`, so bare `expect`/`toString` are inert to it). man2 auto-renders `registry().packages()` → needs a skip gate for unqualified-global packages (unwritable `testing::expect` spelling).
- [x] `bits` — DONE (native-inline; Body::native common-slot; cc86a30a3)

Tier 5 — heavy native leaves (do last; most code, highest byte-identity risk):
- [ ] `os` — 5 files (syscalls)
- [~] `math` — SLICE B DONE (70f4694ed, merged 01f4fa80f): enumerated concrete overloads (per-member Float/Fixed-vs-all-4, list-before-scalar ordering) + 14 constants + full rewire; descriptor+resolution on the registry, byte-identical (`artifact-gate math` 0 diffs, all `func_math_*_invalid` goldens green). SLICE C DEFERRED to the final polish pass: relocate the math-specific lowering from `src/target/shared/code/builder_{fixed,simd_float,simd_fixed,simd}_math.rs` + the `lower_math_*` half of `builder_math.rs` into `src/codegen/builtins/math/common/` (currently reached via the `CodeBuilder::lower_math_call` pub(crate) shim). KEEP in shared: `observe_float`/`observe_promoted_float`/`emit_float_result_check(+_fp)`/`emit_float_exponent_classify`/`FloatInfinityError`/`float_arith_node` (builder_math.rs, ~30 non-math callers), `emit_alloc_result_list` (builder_simd_math.rs, builder_pow:422), all of builder_pow/builder_money_math/rng_pcg64. Repoint const-pool emission (code/mod.rs:1861-1870) + man citations. Byte-identity-inert (bodies unchanged); gate `artifact-gate math` = 0 after.
- [ ] `fs` — 9 files (filesystem syscalls; resource-owning, Phase 0.5 satisfied)
- [ ] `io` — 11 files (print/read/stdin, per-arch)
- [x] `thread` — DONE (two-overload model: `start` 4 overloads on the msg×res matrix; resource-only `accept`/`transfer`; the strict-Nothing guard rejects data-handles from accept automatically; `ParameterType::ThreadHandle` + #4 Unknown-refinement; d2d51fd08, merged 5149c67be). Descriptor+resolver on the registry, ThreadResolver deleted; runtime lowering kept SHARED as `Body::Intrinsic → RuntimeHelper::Thread` (like strings/collections — thread's direction-split/cleanup/stdin lowering is concurrency-runtime infra). 84/84 thread+io `.run` OUTPUT byte-identical, full acceptance 1263/1263. FOLLOW-UP (optional, byte-identity-risky, low value): fold the parent/worker direction split into overload rewrite-targets for full native_os_seam uniformity.
- [x] `tls` — DONE (9 members, TlsSocket/TlsListener resources; poll list/scalar = 2 overloads; e50ec8abc, merged dcfa75d1a). Confirmed no-infra prediction. Carried a `leaf_matches` poll fix that I NARROWED to resource-leaf patterns (de1323783) to avoid perturbing every package's lenient overload dispatch.

## Phase 2 — delete the old branch

Once no package resolves through `target/shared`, delete the plan-72 descriptor vocabulary (including its degenerate `Named(&'static str)` `ParameterType`) and the hand-written free-function fallbacks in `builtins/mod.rs`. The `registry::X(name).or(old(name))` dual-paths collapse to a single registry call. **Now the registry is the one source of truth.**

## Phase 3 — flip the compiler currency to `ParameterType`

Change the type checker / `ir` / `syntaxcheck` to carry `ParameterType` (and `Selection`) across the registry boundary instead of `String`.

⚠️ **This is the one ordering trap:** do *not* do Phase 3 before Phase 2. If you flip the currency while the old string path still exists, you just relocate the `String↔ParameterType` boundary onto the old path instead of eliminating it — you'd be building a new adapter at the same time you're trying to delete one.

## Phase 4 — Bucket 2 falls out

The 6 leak-adapters (`resolve_call`, `rewrite_target`, `call_return_type`, `expected_arguments`, `call_param_names`, `default_argument_padding`) now have no callers doing string marshalling — they evaporate. This isn't a separate effort; it's the *consequence* of Phase 3. (Decide separately whether `resolve_call`/`native_lower` — the two the author left un-`#[deprecated]` — stay as intended permanent `CallShape`/`Selection` seams or get inlined.)

### Phase 1 — migration status & the infra prerequisite (2026-08-16)

- DONE: `bits` (cc86a30a3), `money` (e6ab61d9b), OS-seam build-context infra (a49568620), `os` (0a274639f), `fs` (73374e779). Wave 1 syscall migrations COMPLETE + acceptance-clean.
- `fs` surfaced + fixed a registry-core gap (4a42c74f2): strict `leaf_matches` tightened to require `resource_base_eq` ONLY for RESOURCE params (a value-UNION param like `Json` stays coarse so a variant widens in — the broad first cut over-rejected `json::stringify(JsonNull)`, caught by acceptance). See [[registry-strict-matcher-resource-vs-value-union]].
- 16 stale sidecar goldens regenerated (e76b2b741), all proven benign (fs.close Nothing, qualified fs.File, more precise diagnostics, bug-443 widening) — zero behavior/output changes.
- STILL PENDING cleanup: fs (+ net) have pre-existing stale BYTE-IDENTITY `.ncode` goldens (~39 branch-wide per fs agent) — separate from the acceptance goldens above; regenerate in a final golden-cleanup sweep once migrations land (byte-identity is a signal, not a cargo-test gate).
- `math` split design captured in `planning/math-migration.md` (decision: add a lean `NumericVar` type-class; serial, main thread).
- **BLOCKED on registry infra** (a `vector` migration attempt proved these are hard gaps, not per-package work): `vector`, `math`, `errorcode`, `net` need TWO new registry subsystems first —
  1. **package-constant API** — `RegistryPackage::add_constant` (name + type + component/value data) + a `registry()`-backed dual-path for `is_package_constant`/`package_constant_type_name`/`package_constant_value`/`constant_components` (`src/builtins/mod.rs:648-669`, `src/ir/lower.rs:2567-2591`). Serves `math`/`errorCode`/`vector`.
  2. **general-override API** — `RegistryPackage::add_override((builtin, arg_type) → helper)` + a `registry()`-backed dual-path for `general_override_target` (`src/builtins/mod.rs:148-156`). Serves `vector` (`toString(VecN)`) + `net` (`toString(Url)`).
  - Plus `vector`'s SIMD inline carrier (`builder_vector_inline.rs`, `VECTOR_NATIVE_MARKER`) needs a registry home (not a per-call `Body::native`).
- Also special (not plain native/source): `general` (IS the overridable-builtin subsystem), `resource` (RES), `testing` (desugar).
- Tractable now (no constants/overrides/resolver-context): `money`, `os`, `io`, `fs`, `thread`, `tls`, `audio`, `http`(w/ net), `strings`/`astrings`/`term`(cluster), `app`(name-overloaded — care). `crypto` has a context-dependent resolver — verify before attempting.

### Phase 1 — a second infra prerequisite surfaced (os attempt, 2026-08-16)

- `os` is 14/15 members clean; `os.resourcePath` alone needs per-compilation build context (`build_mode` + `module_name`) that the OS-seam `OsLower` contract can't carry. Fix (greenlit, do BEFORE re-attempting os / io / fs / thread / tls, which likely share the need): extend `OsLower` (`src/codegen/registry/mod.rs:49`), `os_helper` (`:1640`), and `codegen/os/dispatch_runtime_helper` (`src/codegen/os/mod.rs:25`) to carry `build_mode: NativeBuildMode` + `module_name: &str`; thread from the two `code/mod.rs` dispatch sites (~2024 os, ~2382 process); update the ~18 existing OsLower emitters (process/datetime) to accept-and-ignore. Additive/mechanical; keeps os uniform with process. (`code/os/paths.rs:186-215` is the sole build-dependent member.)
- **Infra sequencing (serial — all touch registry/mod.rs):** (1) constant + override API [in flight], (2) OS-seam build-context, THEN the blocked migrations: constant/override → vector/math/errorcode/net; OS-seam-context → os (and de-risks io/fs/thread/tls).

### Phase 1 — constant/override infra LANDED (74c08e745)

- DONE: registry package-constant API (`add_constant`/`is_package_constant`/`constant_type_name`/`constant_value`/`constant_components`) + general-override API (`add_override`/`general_override_target`), dual-pathed through `builtins/mod.rs` + `ir/lower.rs`, byte-identical (registry empty until a package migrates). Unblocks the constant/override half of `errorcode`/`math`/`net`/`vector`.
- Migration note for `vector`/record-constants: a migrated package's record types (e.g. `Float3`) must register as a `RegistryRecord` with element-typed props (the registry record-constant path reads element types from the record's fields in declaration order).
- Pre-existing: `net` has ~5 STALE byte-identity goldens on `worktree-builtin` (verified on clean base, unrelated to any migration) — regenerate when `net` migrates or as a standalone cleanup.

### Phase 1 — wave 1 + arena COMPLETE; resolver-heavy packages are the next infra frontier (2026-08-16)

- DONE + acceptance-green: `bits`, `money`, `os`, `fs`, `io` (+ pre-existing csv/json/regex/process/datetime/encoding/collections). Registry currency: `REGISTRY.modules().len()` down to 16.
- Infra landed: constant/override APIs, OS-seam build-context, `OsLowerCtx` (build_mode/module_name/term_state_offset/presentation_mode_offset), strict-matcher resource-vs-value-union fix. Byte-identity debt for fs `.ncode` cleared.
- **The next frontier is RESOLVER-HEAVY packages** — `thread`/`tls`/`crypto` carry custom `BuiltinResolver`s the registry's generic overload/return machinery may not model:
  - `thread`: `ThreadResolver` parses PARAMETRIC handle types (`Thread OF Msg TO Out`, `ThreadWorker OF Msg TO Out`) to compute return types (`receive→Msg`, `result→Out`) + a resource plane (transfer/accept/emit/read → parent-vs-worker `*Resource` targets). Likely needs a `ParameterType` extension for parametric opaque handles + `unify` rules. DESIGN ANALYSIS in flight.
  - **UPDATE (thread design done, `planning/thread-migration.md`): `tls` and `crypto` need NO new infra** — their custom resolvers are pure arity/exact-nominal dispatch the clean-room `select()` already handles (datetime/process proved multi-overload return variance). Migrate them as multi-overload members. ONLY `thread` needs a new `ParameterType::ThreadHandle` variant (+6 mechanical arms parallel to List/Map/Func; select() untouched). Order: tls → crypto (cheap, no infra) → ThreadHandle infra → thread.
- DONE: `errorcode` (b488ca72f), `crypto` (f149af4e6), `tls` (e50ec8abc) — all acceptance-green (sidecars). 8 packages migrated this session (bits/money/os/fs/io/errorcode/crypto/tls). Legacy `REGISTRY` down to 13.
- **DEFERRED byte-identity `.ncode` debt (final-sweep TODO):** the full byte-identity sweep (`artifact-gate all`) shows ~31 `.ncode`/`.ncodesum` diffs in packages that CALL migrated packages (http/thread call tls; json/crypto/audio native-form ripple). PROVEN BENIGN: http `.ir` shows `tls.writeText`→Nothing / `tls.poll`→Boolean (correct return-type resolution vs pre-migration Unknown); crypto-ec-valid/json `.ir` are IDENTICAL (native-form-only) and their program OUTPUT passes. These will RE-SHIFT with every remaining migration (caller return-type ripple), so regenerating now is churn — do ONE controlled `regen-ncodesum` + rt/app `.ncode` sweep at the END, verify behavior-preserved. Per-migration, verify OUTPUT/run (not sidecar) preservation. (byte-identity is a SIGNAL, not a hard gate — user-confirmed.)
- NEXT after tls/crypto: `ParameterType::ThreadHandle` infra + `thread` (per `planning/thread-migration.md`); then `math` (`NumericVar`, per `planning/math-migration.md`); then `net`/`http`, `vector`, `audio`, strings-cluster (`strings`/`astrings`/`term`), `app`; then specials `general`/`resource`/`testing`.
- LESSON: run FULL acceptance after each migration merge, not just a package sample — the io migration's lenient-`Unknown` behavior (io.print no longer rejects `Unknown` args, bug-443) dropped a cascading diagnostic in a `resources/` fixture that only the full sweep caught.

### Phase 1 — os LANDED; pre-existing fs byte-identity red is BENIGN (2026-08-16)

- `os` migrated + merged (0a274639f, merge 38eb9cdfd): 15 native OS-seam members, `os.resourcePath` consumes the build-context (validates the OS-seam extension end-to-end), owns NO resource, 0 diffs on os/datetime/process. `3814` tests green. Migrated-package count now: csv/json/regex + process/datetime + bits + money + os.
- **Pre-existing `fs` byte-identity red is SAFE, not a bug:** `artifact-gate fs` shows 5/7 `fs_codegen_cover_rt.ncode` reds across ALL arches with fs UNCHANGED. Verified benign: **all 94 fs acceptance tests pass** (rt-behavior/rt-error/syntax via `test-accept.sh`), so behavior+diagnostics are correct — the `.ncode` shift is a codegen-FORM change (registration-order/symbol-numbering ripple from the migrations changing the registered-package set, OR a forgotten 13092026c-era regen). Golden last regen'd at `13092026c` (pre-migration). Disposition: regenerate the fs byte-identity golden WHEN fs migrates (the in-flight fs agent does this as part of relocation) — do NOT treat fs's red gate as a migration failure. Same class as the `net` pre-existing reds noted at 967d58ec9.

### Phase 1 — io needs an ARENA-context OsLower extension (io attempt, 2026-08-16)

- `io` migration STOPPED at a real infra gap (correct stop, no code changed). io is 15 members; the stdin↔thread coupling is CLEAN (io readers reach thread-owned broadcast state by the linked symbol `_mfb_rt_stdin_next_byte`, NOT shared compile-time layout — thread stays out of scope).
- **The gap:** `io.print`/`io.write` (plan-35-B TUI shadow-grid routing, `io_stdout.rs:364` `lower_io_write_helper`) and `io.readLine`/`io.input` (bug-149 cooked-mode restore, `io_stdin.rs:799` `lower_io_read_line_helper`) bake the **per-compilation** `arena_layout.term_state_offset` (+ post-dispatch `presentation_mode_offset` wrong-mode gate) into their runtime-helper bodies. The `OsLower` contract (`registry/mod.rs:55`) carries `build_mode`/`module_name` but NOT arena layout. `app_mode` is fine (derivable from `build_mode.is_app()`). Precedents miss it: process/datetime OsLower emitters never read arena state; money reads arena state only at a FIXED offset via the `NativeLower`/`CodeBuilder` path, not the `OsLower` runtime-helper path.
- **Fix (greenlit, do AFTER os/fs land — same OsLower-signature serialization):** extend the OS-seam contract to thread the arena context (`term_state_offset` + `presentation_mode_offset`, computed in the dispatch at `code/mod.rs:1132-1147`), existing process/datetime emitters accept-and-ignore. **Design note:** OsLower now grows to ~6 params — bundle `build_mode`/`module_name`/arena into a single `OsLowerCtx` struct at the same time (lean end-state per migrate.md), re-touching the process/datetime emitters once. Then re-run the io migration (its analysis is done: 15 members, io_stdout/io_stdin/io_terminal emission, `io.` dispatch arm at `code/mod.rs:2033`, `io_specs.rs`).

### Phase 1 — remaining-package difficulty audit + fan-out plan (2026-08-16)

Every remaining package is more entangled than `bits`/`money` (the easy leaves). Audited shapes:
- **math** — NOT a leaf. ~280KB of SIMD lowering (`builder_{math,fixed_math,simd_math,simd_float_math,simd_fixed_math}.rs`) is **shared core numeric infra** — ~12 core files (`builder_pow`, `builder_numeric`, `mir`, `builder_value_semantics`, `entry`, `data_objects`…) call `math.*` helpers directly. Migration = decide what stays core vs. moves; only the descriptor + constants (`math.pi`… via `add_constant`, Float) + call-dispatch move. `builder_money_math.rs` is core Money-scalar operator codegen — stays. `vector` depends on math only by call-name (`math.sqrt`/`math.clamp`), safe. SERIAL, judgment-heavy.
- **crypto** — custom `CryptoResolver` (arg-dependent `_bytes`/`_text` impl selection, `resolve_return_type`, `default_padding`), 5 `.mfb` companions, per-backend native (`crypto_ec/{cng,macos,openssl}`). SERIAL, resolver-modeling needed.
- **Syscall batch** (all via `OsLower`/`native_os_seam`; all clash with the OS-seam signature change → land AFTER it, then fan out):
  - `os` (220-line desc; env/introspect/paths; resource-owning) — needs the OS-seam build-context infra itself
  - `io` (219; stdin/stdout/terminal; no resolver/resource) — stdin↔thread broadcast coupling (`stdin_broadcast.rs`)
  - `fs` (568; fs/atomic/io/paths; resource-owning) — self-contained, tractable
  - `thread` (1089; +resolver +resource +concurrency runtime +stdin broadcast) — HARDEST of the batch
  - `tls` (715; +resolver +resource +per-backend macos/openssl/schannel; net-coupled) — backend-heavy
- **FAN-OUT ORDER once OS-seam lands:** first wave (tractable, parallel) `os` + `fs` + `io`; second wave (resolver+coupling, more care) `thread`, `tls`. Then the serial infra-heavy set: `math`, `crypto`, `errorcode` (error-emission table → extend `RegistryConstant` with message+symbol, repoint `runtime_error*`), `net`/`http` (resource+`Url` type+`toString(Url)` override), `vector` (RegistryRecord + SIMD carrier home), `audio` (resource+MML source). Finally the specials: `general`/`resource`/`testing`.

### Phase 1 — thread obstacle resolution (2026-08-16, collaborative w/ user)
- thread PART A (ThreadHandle variant) landed clean; PART B hit 4 proven obstacles (the res-slot optional-vs-required contradiction + strict-Nothing guard + Unknown re-occur + ISOLATED FUNC parse).
- RESOLVED without matcher-core surgery or a resolver hook (user-designed): #2+#3 via SIGNATURE-LEVEL OVERLOAD SPLIT (two `start` overloads data/resource; resource-only `accept`/`transfer` — the strict-Nothing guard then rejects a data-handle from `accept` automatically, reproducing legacy). #4 = Unknown-refinement in the Var arm (LANDED de834e95b, verified no dispatch shift: acceptance 1263/0, byte-identity unchanged). #1 = bounded ISOLATED FUNC parse fix.
- thread agent resumed to implement PART B on this model. `planning/thread-migration.md` "PART B obstacle resolution" is the spec.

---

> resource can not be stored as a collection element

I need to review all documentation and remove this claim as it is false.
