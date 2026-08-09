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
- [ ] **K1 (interim, contained)** — move-elision escape analysis: `RETURN local` of a caller-dead value
  becomes a move; the identity shape `FUNC f(xs) RETURN xs` (copyStrs) elides the copy entirely. Retires
  `list (Dynamic) copy` cheaply.
- [ ] **K2 (large)** — add a refcount/version word + copy-on-write to `copy_collection_tight` — share on
  RETURN/param-alias, split on first mutation. Makes A/B/D's buffer copies free too. Largest design change
  in the plan (value model + every mutation path); semantics must stay observably value-copy. **Defer** until
  A/B/C/D cut copy volume; reassess.
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
