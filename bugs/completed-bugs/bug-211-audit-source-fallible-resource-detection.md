# bug-211: audit source collector under-reports fallibility and resources (LINK-gated calls, reassignment/TRAP acquisitions)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness

Status: Fixed (2026-07-15) — (1) new link_fallible_calls(ast) collects `<alias>.<func>` for every LINK function with a SUCCESS_ON gate, and fallible_functions seeds its `names` set with them before the fixpoint, so a user function whose only error source is such a native call is reported fallible and its call appears in the Control-flow section. (2) collect_resources now detects acquisitions via Statement::Assign (reassignment) and unwraps Expression::Trapped to its inner call (a new acquisition_callee helper), instead of matching only a bare Expression::Call under Statement::Let. Regression Test: verified `h = fs::openFile(...)` reassignment now appears in the Resources section (previously only the LET did), and an inline-TRAP acquisition is listed. 76 audit tests pass.

Follow-up (2026-07-16): the 76 audit unit tests were run but not the acceptance
golden suite, and this change invalidated the only two acceptance goldens
covering LINK fallibility — `syntax/packages/audit-capabilities` and
`rt-behavior/native/native-link-free-rt` — leaving them red on `main` from this
commit (c6ce07ed) until they were synced. The goldens were stale, not the code:
both already listed the `sql.*` calls under Permissions with `capability:
native` and already marked `main` fallible, while omitting those same calls from
Control flow — i.e. they claimed a call may fail yet refused to list it as
fallible. The new output is also precise rather than blanket: it adds the six
`SUCCESS_ON`-gated calls and correctly omits `sql.expandedSql`, the one LINK
function with no `SUCCESS_ON`. Synced additively (+56 lines, 0 deletions); full
acceptance now 949/949. Both fixtures declare `LINK "sqlite3" AS sql` inline, so
this resolves from the AST and is not environment-dependent.

Two related gaps in `src/audit/collect/source.rs` cause `mfb audit` to
under-report:

- `is_fallible_call` (`:567-581`): a call to a native `LINK` function gated by
  `SUCCESS_ON` (which raises a trappable error) is never treated as fallible, so
  a user function whose only error source is such a call is reported as pure and
  its call omitted from the Control-flow section. Trigger:
  `LINK "x" AS db ... FUNC open(...) SUCCESS_ON status = 0 ...`, then a user
  `FUNC f() ... db::open(...)` — `f.fallible` is false though a native failure
  propagates.
- `collect_resources` (`:110-154`): resource acquisition is only detected for
  `Statement::Let` whose value is a bare `Expression::Call`; acquisitions via
  reassignment (`Statement::Assign`) or an inline-`TRAP` value
  (`Expression::Trapped` wrapping the call) are missed, so the Resources section
  and close-may-fail findings are under-reported. Trigger: `h = fs::open("p")`
  or `LET h = fs::open("p") TRAP(e) ... END TRAP`.

Fix: mark a call fallible when its package is a `LINK` alias whose target has
`success_on.is_some()`; and in `collect_resources` also inspect
`Statement::Assign` values and unwrap `Expression::Trapped` to its inner call.
