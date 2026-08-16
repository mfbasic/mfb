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

> ⚠️ The resource-owning packages here (`fs`/`net`/`tls`/`audio`) depend on **Phase 0.5 / plan-97** landing first — otherwise they register bare-name resources that plan-97 must then re-qualify.

## Phase 2 — delete the old branch

Once no package resolves through `target/shared`, delete the plan-72 descriptor vocabulary (including its degenerate `Named(&'static str)` `ParameterType`) and the hand-written free-function fallbacks in `builtins/mod.rs`. The `registry::X(name).or(old(name))` dual-paths collapse to a single registry call. **Now the registry is the one source of truth.**

## Phase 3 — flip the compiler currency to `ParameterType`

Change the type checker / `ir` / `syntaxcheck` to carry `ParameterType` (and `Selection`) across the registry boundary instead of `String`.

⚠️ **This is the one ordering trap:** do *not* do Phase 3 before Phase 2. If you flip the currency while the old string path still exists, you just relocate the `String↔ParameterType` boundary onto the old path instead of eliminating it — you'd be building a new adapter at the same time you're trying to delete one.

## Phase 4 — Bucket 2 falls out

The 6 leak-adapters (`resolve_call`, `rewrite_target`, `call_return_type`, `expected_arguments`, `call_param_names`, `default_argument_padding`) now have no callers doing string marshalling — they evaporate. This isn't a separate effort; it's the *consequence* of Phase 3. (Decide separately whether `resolve_call`/`native_lower` — the two the author left un-`#[deprecated]` — stay as intended permanent `CallShape`/`Selection` seams or get inlined.)
