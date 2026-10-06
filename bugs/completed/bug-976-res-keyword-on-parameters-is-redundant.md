# bug-976: `RES` on a parameter is an optional keyword that the type already implies — remove it; a parameter is a resource because its type is one

Last updated: 2026-10-06
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (language surface; owner-confirmed)

Status: Fixed (bs17-agent-e)
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

## Measurement (phase 1, base compiler `468ff0f94`, `/tmp/main-dbg-target/release/mfb`)

Fifteen probe programs, each built twice — once with `RES` on the parameter and
once bare — and compared on diagnostics, `--ir`, `--ncode`, the run output and
the `--debug` report (timing lines excluded). Harness: `/tmp/bs17e/measure.sh`.

| probe | shape | RES vs bare |
| --- | --- | --- |
| p01 | `RES g = f` alias in the callee, caller writes after | identical IR, ncode, run, debug counts |
| p02 | `RETURN f` as `AS RES fs::File`, caller binds and writes | identical |
| p03 | `append(xs, f)` into a returned `List OF RES fs::File` | identical |
| p04 | `fs::close(f)` in the callee | identical |
| p05 | `Holder["made", f]` returned | identical |
| p06 | `LET g = f` | both `TYPE_RESOURCE_REQUIRES_RES` |
| p07 | `saved = [f]` into a module global | identical |
| p08 | `x AS Integer` | identical (`RES x AS Integer` is accepted today) |
| p09 | `f AS fs::File STATE Cur` | RES builds and prints 1; bare is `MFB_PARSE_UNEXPECTED_TOKEN` |
| p10 | stateful argument into a bare parameter | identical |
| p11 | resource-union parameter | identical |
| p12 | `x AS Integer STATE Cur` | RES: accepted at the declaration, errors at the uses; bare: parse error |
| p13/p14 | `ThreadWorker` / `Thread` parameter | identical |
| p15 | `LAMBDA(RES x AS fs::File)` | identical |
| LINK | `FUNC close(db AS Db)` as a `CLOSE BY` op | RES builds; bare is `RESOURCE_CLOSE_SIGNATURE` |

So for a FUNC parameter the keyword changed no emitted code: `resource_escape`
resolves every parameter `Local` either way (a depth-0 parameter can only float to
`MODULE_DEPTH`, which `solve` skips for parameters, and phase 1's ordering gate
skips parameters), and IR/NIR params carry no flag. The observable differences
were exactly three: STATE parsing, the LINK close-op signature rule, and the
`.ast` dump.

## Resolution

- **Parser** (`src/ast/items.rs:parse_params`): a `RES` before a parameter name
  is the new located parse error `1-102-0015 MFB_PARSE_PARAMETER_RES` ("a
  parameter takes no RES; its type already makes it a resource"), reported on the
  keyword with a detail naming the parameter and its type; the parser recovers
  and keeps parsing the parameter. `STATE` parses after any parameter type; the
  plan-156-E `thread_view` carve-out is deleted. `ast::Param.resource` is gone.
- **Derivation** (`src/hir/resource_params.rs`): `HirParam.resource` and
  `HirLinkParam.resource` are derived from the parameter's type with
  `ir::verify`'s binding predicate (`is_resource_or_resource_union`: a close op —
  built-in, parent `Thread`, project `RESOURCE`, imported `RESOURCE_TABLE` row —
  or a union with such a variant), or a `STATE` clause (exactly what `RES … STATE`
  meant). `hir::elaborate` derives it from the sources (resolver and test paths);
  the build driver derives it again on the concrete post-monomorph HIR with the
  imported packages' resources and unions folded in (`src/cli/build/mod.rs`,
  mirrored in `testutil`). FUNC, LINK and LAMBDA parameters are all covered. The
  consumers (`resource_escape`, `lower_link`, the resolver's close-op rule) are
  unchanged.
- **AST dump**: a parameter prints no `resource` key; a `STATE` prints as
  `"state": …` (it was dropped for the unkeyworded thread-view STATE before).
- **Diagnostics wording**: `RESOURCE_CLOSE_SIGNATURE` reads "a close op must
  consume exactly one parameter of its resource"; the resolver detail, and three
  `ir::verify` details that said "bare `RES` parameter", now say "parameter" /
  "resource parameter".
- **Spec**: `diagnostics/01_rule-codes` (new row, reworded 0091),
  `language/15` (parameter rule, the `RES b = p` row, the STATE table header,
  the marker paragraph), `16` (worker entry), `19` (param production), `17`,
  `14`, `08`, `21`, `architecture/23`, `memory/04`, `package/12`; man `link`,
  `variable`.
- **In-tree rewrite**: every parameter-position `RES` (403 sites in 174 files:
  fixtures, `packages/`, `examples/`, `tools/`, `spikes/`, `bugs/repro/`, the
  injected builtin sources under `src/codegen/builtins`, Rust test strings, spec
  and man examples, `.ai` docs). `bugs/completed/`, `planning/` and other open
  bug docs are history and were left as written.

Regression tests: `tests/syntax/resources/param-res-keyword-invalid` (FUNC and
LAMBDA `RES` → two located `MFB_PARSE_PARAMETER_RES`), and
`tests/rt-behavior/resources/param-resource-inferred-rt` (bare alias, bare
`STATE` write visible to the owner, stateful argument to an opaque parameter,
handing the parameter back, storing it in a returned record, a resource-union
parameter, a callee close): prints `state=41 holder=held union=file size=4`,
`--debug` 17 allocs / 17 frees / 0 live bytes — the same as its RES-spelled twin
on the base compiler, whose `--ncode` and `--ir` are byte-identical to the bare
version on the fixed compiler. Unit tests:
`hir::resource_params::tests::*` (decision table, LAMBDA, imported) and
`ast::tests::a_res_parameter_keyword_is_a_parse_error_and_state_needs_no_keyword`.

## Golden impact (not regenerated)

Full `scripts/test-accept.sh` with the fixed compiler: 72 mismatches, every one
in a class below (`/tmp/bs17e/classify.py` normalizes the golden by the class and
compares it to the actual; all 72 match their class exactly). No `.ir` and no
`.run` output changes.

1. **`.ast`, RES removed from a param list in the fixture source (60).** The
   parameter's `, "resource": true, "state": null` suffix is gone, or
   `"resource": true, "state": "T"` becomes `"state": "T"`. Fixtures:
   rt-behavior/collections/bug752-res-element-callbacks-rt; rt-behavior/fs/
   func_fs_openData_valid, func_fs_readBytesAt_valid, func_fs_seek_valid,
   func_fs_size_valid; rt-behavior/native/ libsnd-open-file-info-rt,
   libsnd-read-samples-rt, native-closed-guard-rt, native-link-and-truthy-rt,
   native-link-free-rt, native-link-inline-trap-rt, native-link-nested-success-rt,
   native-link-sqlite-rt, native-private-resource-rt,
   native-resource-in-collection-rt, native-resource-scope-drop-rt,
   native-resource-state-rt, native-resource-thread-accept-rt,
   native-resource-transfer-fail-usable-rt, native-stateless-record-rt;
   rt-behavior/process/close-input-keeps-handle; rt-behavior/resources/
   bug424_state_accum_inplace, bug487_state_mutating_operand,
   bug643-collection-helpers-keep-caller-handles-rt, native-resource-import-valid,
   p121d-state-ops-rt, p121d-state-reach-rt, p121d-state-splice-rt,
   record-res-field-return-rt, res-rebind-alias-runtime, resource-owner-scope-rt,
   resource-pointer-across-ops-valid, resource-state-bare-param-valid,
   resource-state-field-assign-valid, resource-state-mutation-valid,
   resource-union-state-access-valid, state-field-source-generic-arg-valid,
   state-scalar-inplace-decline-rt; rt-behavior/scope/
   float-arith-on-global-and-state-field-valid; rt-behavior/threads/
   bug691-res-thread-param, bug693-worker-state; rt-behavior/tls/ tls-poll-rt,
   tls-read-timeout-rt, tls-write-after-read-closed-raises-rt;
   rt-behavior/udp/func_udp_receive_valid; syntax/native/ native-bind-state-valid,
   native-res-state-inline-trap-valid, native-resource-state-export-valid;
   syntax/packages/audit-capabilities; syntax/resources/
   native-resource-link-valid, native-resource-state-link-valid,
   resource-invalidate-not-owner-valid, resource-non-owner-return-valid,
   resource-union-param-valid, resource-union-state-valid,
   state-opaque-narrow-valid, user-resource-shadows-builtin-valid; syntax/tls/
   connect_valid, endpoints_valid, listen_valid.
2. **`.ast`, an unkeyworded `STATE` now printed (1).**
   rt-behavior/threads/thread-view-state-independent: `worker AS ThreadWorker …
   STATE W` gains `"state": "W"` (the old printer only printed STATE beside
   `RES`).
3. **`build.log`, RES removed from the quoted source line and/or the reworded
   RES-parameter message (10).** syntax/native/ native-abi-no-result-invalid,
   native-abi-unbound-slot-invalid, native-resource-state-mismatch-invalid;
   syntax/resources/ native-resource-close-signature-invalid (also the reworded
   0091 message and resolver detail), resource-param-state-invalid,
   resource-state-bare-param-read-invalid and -write-invalid (reworded
   `values.rs` detail), resource-state-unknown-param-invalid,
   state-opaque-narrow-bind-invalid and -return-invalid (reworded `calls.rs` /
   `ops.rs` detail).
4. **`.nir`, a previously BARE resource-typed parameter gains a `local`
   `resourceOwners` entry (1 golden).** syntax/app/app-mouse-surface
   `.macos-aarch64.app.nir`: canvas's injected companion has a bare
   `font AS canvas::Font` parameter, now a resource parameter. `Local` is what an
   absent entry already meant, and the fixture's four `.ncodesum` goldens are
   unchanged.

`scripts/artifact-gate.sh target/release/mfb all`: 2280 goldens checked, 62 diffs
= class 1 + 2 (61 `.ast`) + class 4 (1 `.nir`); no `.ncode`/`.ncodesum`/`.nplan`
golden moved.

Byte-identity beyond the goldens: `scripts/artifact-baseline.sh` captured on a
detached base worktree with the base compiler and verified on this tree with the
fixed compiler (`--targets macos-aarch64`, 1657 fixtures, every dump and every
linked `.out`). Besides the two new fixtures, the only differences are:

- 13 fixtures' `.nir`, all class 4 (a `local` `resourceOwners` entry for a
  parameter that was bare: `t AS Thread OF …` parameters in the thread tests, and
  bare resource parameters in injected package sources). Their `.ncode`, `.mir`
  and `.out` are byte-identical.
- rt-behavior/collections/bug752-res-element-callbacks-rt `.mir`/`.ncode`/`.out`:
  three error-location column constants (`mov_imm %argC2`) each drop by 4,
  because removing `RES ` from `LAMBDA(RES x AS fs::File) -> …` shifts every
  later column on those source lines. No code shape changed.

The committed `.mfp` copies of `packages/sqlite3` and `packages/libsnd` embed doc
signature lines that still spell `RES db AS Db`; they change only in that text on
the next `scripts/sync-package-mfp.sh` (not run here).

## Found, not fixed (needs an owner decision)

A `STATE` clause on a parameter whose type is not a resource is accepted at the
declaration. Today `FUNC f(RES x AS Integer STATE Cur)` builds when `f` is never
called (measured on the base compiler), and its uses fail downstream with
unrelated errors (`TYPE_BINARY_OPERATOR_MISMATCH`, `TYPE_STATE_MISMATCH`). After
this change the same holds for `x AS Integer STATE Cur`, which used to be a parse
error. Rejecting it needs a diagnostic the docs do not settle (a new code, or
`TYPE_RES_REQUIRES_RESOURCE` with new wording), so it is reported rather than
invented here.
