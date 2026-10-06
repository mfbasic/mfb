# bug-976: `RES` on a parameter is an optional keyword that the type already implies — remove it; a parameter is a resource because its type is one

Last updated: 2026-10-05
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (language surface; owner-confirmed)

Status: Open
Regression Test: tests/syntax/resources/param-res-keyword-invalid (new parse error); tests/rt-behavior/resources/param-resource-inferred-rt (a bare `h AS fs::File` parameter behaves exactly as `RES h AS fs::File` does today)

**Owner decision (2026-10-05):** remove the `RES` requirement *and* the option on
parameters. Resource-ness is inferred from the parameter's type: `fs::File` is a resource, so
`FUNC sz(h AS fs::File)` is a resource parameter and there is no need to say `RES` again. The
change is limited to the spec and the parser. It must not change any other aspect of
resource semantics. Thread views follow the same rule with no special case (bug-975):
`worker AS ThreadWorker OF Msg TO Out [STATE W]` and `t AS Thread OF …` are resource
parameters because their types are resources.

## Today (measured, HEAD 846aa55fd, `/tmp/hardening/mfb-165`, `/tmp/hardening/coord/rp`)

With a plain `fs::File`, both spellings build and print `TRUE`:

```basic
FUNC sz(h AS fs::File) AS Integer        ' accepted
FUNC sz(RES h AS fs::File) AS Integer    ' accepted
```

The spec says the opposite of the "optional" behavior. `language/15_resource-management.md`:
"A resource value may be passed to a function whose parameter is declared `RES` … such as
`RES f AS fs::File`". plan-156-E §3 instead relied on "A resource parameter needs no `RES`
(verified 2026-09-24)" to let a worker parameter carry `STATE` without `RES`.

Census of the keyword in parameter position (`grep -rEo '(\(|, *)RES [A-Za-z_][A-Za-z0-9_]* AS '`):
246 uses in 107 `.mfb` files, 139 in Rust sources (man examples, test strings), 25 in
`src/docs`.

## The keyword is not only spelling — it drives a flag

`src/ast/items.rs:parse_params` sets `Param.resource = match_keyword(Keyword::Res)`, and that
flag is consumed downstream:

- `src/ir/resource_escape.rs`: "`RES` parameters are resources owned at function-entry depth".
  Only params with `resource` set are `declare`d, put in `res_names`/`res_params`, and given
  `res_depth` 0.
- `src/ir/lower_link.rs`: a parameter's `STATE` is applied (`type_.with_state(state)`) only when
  `param.resource` is true.
- `src/resolver/mod.rs`: `param_resource` is recorded per function from `param.resource`.
- `src/hir/mod.rs`: carries `resource` into HIR params.
- `src/ast/serialize.rs`: the `.ast` dump prints `RES ` before the name.
- `parse_params` gates `STATE` parsing on `resource || thread_view` (plan-156-E's thread
  special case).

So `h AS fs::File` and `RES h AS fs::File` may already differ in escape analysis, a latent
inconsistency. The first phase must measure it: return or store a bare-`fs::File` parameter
versus a `RES` one, and compare diagnostics and the `--debug` cleanup counts.

## Goal

- The parser rejects `RES` in a parameter list with a located parse error that says the
  parameter's type already makes it a resource.
- `Param.resource` (and its HIR/resolver copies) is **derived from the parameter's resolved
  type** with the same predicate that decides resource-ness for bindings. It is true exactly
  when the type is a resource (or a resource union, or `STATE`-carrying resource). Every
  consumer listed above stays unchanged and sees the same value it saw for a `RES`-spelled
  parameter. If the derivation must live after name resolution rather than in the parser,
  that is the minimal place; the consumers do not change.
- `STATE` on a parameter parses after any resource-typed parameter, with no `thread_view`
  special case.
- Spec: `language/15_resource-management.md` (parameter rules, the "declared `RES`"
  sentence, the `RES b = p` table row), `16_threads.md` (worker entry signature), `19_grammar.md`
  (parameter production), and every `mfb man` example drop `RES` from parameter lists.
- Every parameter-position `RES` in `.mfb` sources, Rust test strings and man examples is
  removed (census above). The `.ast` goldens that print `RES ` for a parameter change only by
  that token.

### Non-goals (must NOT change)

- `RES` on bindings (`RES f = fs::open(…)`), on return types (`AS RES T`), in collection
  element / record field / map value types (`List OF RES T`, `t AS RES Thread…`), and in
  thread resource planes (`Thread OF Msg RES Res TO Out`). Only parameter lists change.
- Resource semantics: lending on pass, scope ownership, moves, cleanup, `STATE` agreement.
  Emitted code and runtime behavior are byte-identical for every program that used
  `RES` on its parameters.
- Forbidden wrong fix: keeping `RES` as an accepted no-op on parameters.

## Blast Radius

- bug-975 (`ThreadWorker` not treated as a resource): once resource-ness of a worker view
  is fixed there, this bug makes its parameter a resource purely by type.
- plan-156-E's `thread_view` STATE carve-out in `parse_params` is deleted.
