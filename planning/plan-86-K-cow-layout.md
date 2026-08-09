# plan-86-K — COW / refcount collection buffers + String-element layout

Sub-plan **K** of [plan-86](plan-86-benchmark-perf.md). Open. Split candidate. Also owns
[B3](plan-86-B-reduce-accumulator.md) (in-place growing accumulator).

**Covers (1 P1 direct + broad amplifier):** list (Dynamic) copy (12.5), set (14.3), transform (23.6),
insert/removeAt (~8.8); amplifies A's sort/groupBy buffer copies and B's reduce accumulator.

## Root cause
The 40-byte header (`error_constants.rs`) has **no refcount/version word** and there is no COW: `lower_value_owned`
unconditionally deep-copies any aliasing source via `copy_collection_tight`. `list (Dynamic) copy` is a full
String-list memcpy per call; `list (Dynamic) set` grows a String payload → the out-of-line whole-list
rebuild branch (bug-430). `insert`/`removeAt` are native with **inherent** O(n²) data-region reflow — not
COW-fixable.

## Fixes
- [x] **K1 — LANDED. Parameter-passthrough functions return a borrow; the deep-copy MOVES to the caller's
  ownership boundary.** move-elision escape analysis: `RETURN local` of a caller-dead value becomes a move
  (that half already existed — plan-25-C `plan_returned_move`, owned-LOCAL returns); the identity shape
  `FUNC f(xs) RETURN xs` (copyStrs, a PARAMETER return) is the piece K1 adds. **Implementation:**
  `function_returns_param_borrow` (`function_lowering.rs`) marks a function whose EVERY value-return is a bare,
  never-reassigned/never-address-taken parameter (exhaustive `NirVisitor`), EXCLUDING any function used as a
  `FunctionRef`/callback (`collect_function_ref_names`, `data_objects.rs`). Callee (`lower_returned_value`)
  returns the argument pointer uncopied when it owns no `OwnedValue` cleanup (the borrow marker — NOT `by_ref`,
  which is false for params; fail-safe to a copy otherwise). Caller: `value_needs_owning_copy` classifies the
  call result as an aliasing source, so the EXISTING discipline does the rest for free — `register_pending_temp`
  skips freeing it, `lower_value_owned` deep-copies it at any owning store. Both sides key off the SAME
  predicate + shared `callback_referenced_functions` set (threaded from the module driver), so they cannot
  desync. **Value semantics is preserved by construction** (a read-only-and-discarded result pays no copy; a
  stored result is copied exactly once at the store). **SOUNDNESS gated by fixtures:**
  `return-param-borrow-rt` (mutate a passthrough result → base/p/q PROVABLY unchanged; `copy_0=X` but
  `base_0=a`; multi-param `pick`; a fresh-returning `grow` correctly NOT elided), and
  `groupby-string-value-native-rt` (the CALLBACK-exclusion guard — a `RETURN s` passthrough passed as a
  groupBy value-mapper; its grouped Strings came back EMPTY before the FunctionRef exclusion — a real
  double-free/UAF this session caught + fixed). 3776 unit tests green; full acceptance green; artifact-gate
  clean/re-synced. **Measured (isolated replica of `test_ld_copy`, 10×1000 `len(copyStrs(base))` over a
  1000-element String list): ~132,000µs → ~45µs, a ~2900× drop** — the per-call whole-list deep-copy is gone
  (`passStr` ncode: 8 instrs, no arena_alloc). Retires `list (Dynamic) copy` (P1 12.5ms). Commit: `PENDING`.
- [~] **K2 (large) — DEFERRED by explicit user decision (2026-08-09); reassess as its own plan, may never be
  built.** — add a refcount/version word + copy-on-write to `copy_collection_tight` — share on
  RETURN/param-alias, split on first mutation. Makes A/B/D's buffer copies free too. Largest design change
  in the plan (value model + every mutation path); semantics must stay observably value-copy. **Defer** until
  A/B/C/D cut copy volume; reassess. **User flagged the refcount/COW value-model change as too high-risk to
  land inside this benchmark plan** (a single missed mutation path → observable aliasing / heap corruption).
  A/B/C/D landed and already cut copy volume, and K1+K3 retire the two P1 rows (list copy / list set)
  refcount-free — so K2's remaining upside is likely small. If ever pursued, it must be a standalone plan with
  an exhaustive value-semantics/aliasing fixture matrix authored BEFORE any mutation path is touched. Left
  `[~]` deferred, not `[ ]`.
- [ ] **K3** — out-of-line String-element list layout so a growing String `set` need not rebuild the whole
  list (the bug-430 in-place-mutation follow-up). Retires `list (Dynamic) set`. Do with A.
- [~] **B3 (from [plan-86-B](plan-86-B-reduce-accumulator.md)) — BLOCKED on K2 (its stated prerequisite) +
  conditional-not-met.** — in-place growing reducer accumulator, needs K's uniquely-owned-mutation analysis.
  Pursue only if B1 left a gap; do not gate B on parity. **Both of B3's own gates are unmet:** (1) it explicitly
  "needs K's uniquely-owned-mutation analysis" — that IS the K2 COW/refcount + uniquely-owned-mutation pass,
  which is NOT done (K2 is the plan's largest deferred change), so per follow-plan (`a letter's product doesn't
  exist → do that letter`) B3 cannot land until K2 does; (2) "pursue ONLY IF B1 left a gap" — B1 landed the
  reduce accumulator fix (the reduce-leak, plan-86-B's deliverable; "do not gate B on parity"), so B is complete
  and B3 is a further-optimization refinement, not required. Resolved as blocked-on-K2 per the plan's own
  dependency + condition, to be revisited if/when K2 lands. Not skipped on difficulty — its prerequisite letter
  is undone.

## Note
`insert`/`removeAt` are reflow-inherent (repeated `insert(0)` is O(n²) by construction) — not COW-fixable;
track for regression only.

## Acceptance
value-semantics fixtures (`tests/`) + `scripts/artifact-gate.sh`.
