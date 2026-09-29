# bug-778: every inline `TRAP` evaluation heap-allocates a `Result` block per fallible node, even when nothing fails

Last updated: 2026-09-28
Effort: large (3h–1d)
Severity: MEDIUM
Class: Other (performance: an arena allocation + free + entropy scrub per trapped call on the success path)

Status: Open
Regression Test: `tests/runtime/rt_inline_trap_success_alloc.rs` (to be added, Phase 1)

`LET x = f(a) TRAP(e) … END TRAP` desugars (`mfb spec language error-model` §8.8) to a bind of a compiler temporary
`$trap_res = CallResult(f(a))`, which is then read by `ResultIsOk` / `ResultValue` / `ResultError`. Native codegen
represents that temporary as a **heap** flat `Result` block (`{tag, size, payload}`), allocated on **every** evaluation,
including every successful one. It is freed at the end of the statement, and the arena's always-on entropy fill scrubs
the freed payload. A trapped expression with k fallible nodes (calls **and** raising operators such as `+`, `*` and
`MOD`, which the bug-457/bug-471 desugar wraps in `Checked`) allocates **k** such blocks per evaluation. For a `String`
payload, the success value is also copied into the block and then back out.

On the success path the cost is 1–3 extra allocations and 9–29 ns per evaluation, 4–10× the untrapped statement, at every
optimization level. No one can observe the temporary: it is synthesized by the desugar, never escapes the statement, and
on success only its tag and payload are read.

**The single correct behavior a fix produces:** the success path of an inline-`TRAP`'d expression performs **no arena
allocation beyond what the same expression performs without the `TRAP`**. `--debug` `arena.0.alloc_calls` is equal
for the trapped and the plain form of every shape in the table below, at every `-O` level. The cost moves to the error
path, which still materializes the `Error` value the handler binds (`mfb spec memory fallible-call-abi`: "trapping
materializes a 3-field `Error` record from `x1`/`x2`/`x3`"). Time target: the trapped form's success path is within 1.5×
the plain form (plus 2 ns for the tag test) on the reference host.

References:

- `mfb spec language error-model` §8 rule 11 and §8.8 (the inline-`TRAP` desugar, `MATCH g(y)`); `src/ir/lower.rs:
  lower_inline_trap` doc comment (the "one check" and "check chain" shapes).
- `mfb spec memory fallible-call-abi`: a failed call leaves its error in `x1`/`x2`/`x3`, and trapping materializes the
  `Error` record. Nothing requires a heap `Result` on success.
- `mfb spec memory arenas` → *Entropy Fill* (each freed block past 16 B is scrubbed).
- Split out of `bugs/bug-772-toscalars-fromscalars-allocate-per-scalar.md`, where the per-element `toScalar(cp) TRAP(err)` in
  `__strings_toScalars` was the ~1 alloc/scalar. Found during the cluster-D benchmark triage, 2026-09-28.

## Failing Reproduction

Harness `/tmp/dbD-tr/gen.py`. One program per shape, built `-O 1` with and without `--debug`, run in a scratch project:

```
IMPORT io
IMPORT datetime
FUNC userInt(i AS Integer) AS Integer
  IF i < 0 THEN
    FAIL error(77050002, "neg")
  END IF
  RETURN i + 1
END FUNC
FUNC userStr(i AS Integer) AS String
  IF i < 0 THEN
    FAIL error(77050002, "neg")
  END IF
  RETURN "v"
END FUNC
FUNC work(n AS Integer) AS Integer
  MUT s AS Integer = 0
  FOR i = 1 TO n
    LET sc AS Scalar = toScalar(65 + (i MOD 26)) TRAP(err)
      RETURN -1
    END TRAP
    s = s + toInt(sc)
  NEXT
  RETURN s
END FUNC
FUNC main AS Integer
  LET t0 = datetime::monotonicNanos()
  LET r = work(1000000)
  LET t1 = datetime::monotonicNanos()
  io::print(toString(r) & " " & toString((t1 - t0) / 1000000) & " ns/eval")
  RETURN 0
END FUNC
```

The other rows swap the loop body's trapped statement (`LET v … = <expr> TRAP(err) RETURN -1 END TRAP`) and its plain
twin (`LET v … = <expr>`). No evaluation fails in any row, so every handler is dead at run time. Results for 10⁶
evaluations, `target/release/mfb` at `7e53c82b9`, macOS aarch64. Absolute times are from a host shared with other
agents; the allocation counts are exact:

| trapped expression | `alloc_calls` trapped / plain | `alloc_bytes` trapped / plain | ns/eval trapped / plain |
|---|---:|---:|---:|
| `toScalar(65 + (i MOD 26))` | 3,000,005 / 5 | 96,000,144 / 144 | 32 / 3 |
| same, `-O 3` | 3,000,003 / — | — | 39 / 2 |
| `userInt(i)` (user FUNC, `Integer`) | 1,000,005 / 5 | 32,000,144 / 144 | 12 / 3 |
| `userStr(i)` (user FUNC, `String`) | 4,000,005 / 1,000,005 | 80,000,128 / 16,000,128 | 37 / 11 |
| `toInt("123")` (fallible builtin parse) | 1,000,005 / 5 | 32,000,144 / 144 | 16 / 8 |
| `userInt(5)` (constant argument) | 1,000,005 / — | 32,000,128 / — | 19 / — |
| `i * 2 + 3` (no call; see *Adjacent finding*) | 2,000,005 / 5 | 64,000,160 / 144 | 27 / 2 |

Each extra allocation is 32 B (a 24 B flat `Result` of a scalar payload, rounded up to the granule), and
`fill_free_calls` rises by the same count (1 scrub of 16 B each). `toScalar(65 + (i MOD 26))` allocates **3**: one
`Checked` for `MOD` (it can divide by zero), one for `+` (it can overflow), and one `CallResult` for `toScalar`.
`userStr` allocates 3 extra: the `Result` block, which holds the String inline, plus the copies in and out.

Contrast: the same calls without `TRAP`, or under a **function-level** `TRAP`, allocate nothing extra. A failed call
travels in registers (`mfb spec memory fallible-call-abi`).

## Root Cause

1. **Desugar** (`src/ir/lower.rs:lower_inline_trap`, plus `hoist_trap_calls`): the scrutinee's outermost fallible node
   becomes `Bind $trap_res : Result OF T = CallResult(...)`, or `Checked(...)` for a raising operator (bug-471). Each
   nested fallible call or raising operator is lifted into its own `Bind $trap_res_k = CallResult/Checked(...)` plus
   `If ResultIsOk($trap_res_k)` (bug-457). The temporaries are ordinary `Result OF T` locals.
2. **Codegen** gives every `Result OF T` value the heap representation:
   - `NirValue::CallResult` (`src/codegen/engine/value/builder_values.rs:CodeBuilder::lower_value`, the `CallResult` arm
     and its function-value twin, "bug-448: the inline-TRAP machinery consumes a boxed `Result`") calls the callee raw,
     spills the four result registers, and builds a block with
     `src/codegen/memory/arena/builder_arena_transfer.rs:CodeBuilder::emit_build_result_inline` / `materialize_current_result`.
     That means an `arena_alloc` of `16 + payload`, with a block payload (String/record/collection) copied inline at +16.
   - `NirValue::Checked` (`builder_values.rs:CodeBuilder::lower_checked_value`) runs the operator under a raw-result capture,
     then boxes the outcome the same way.
   - `NirValue::ResultIsOk` loads the tag at +0; `ResultValue` loads (scalar) or aliases (block) +16, after which the
     bind copies a block payload out; `ResultError` aliases +16.
   - The `$trap_res` local is then dropped at the statement's end (`arena_free`, and the scrub).

   None of these readers needs a heap object. They read three words the call already left in `x0`–`x3`, which step 2
   spills to frame slots anyway (`raw_result_tag`/`_value`/`_message`/`_source`). The box exists because the NIR models
   the temporary as a first-class `Result` value, and codegen has one representation for that type.

The `-O 3` row shows that no optimizer pass removes it: the `CallResult`/`Checked` nodes are opaque to opt1
(`src/optimizer/opt1/inline.rs` deliberately leaves a `CallResult` call alone).

## Goal

- For every shape in the table: trapped `alloc_calls` equals plain `alloc_calls` at `-O 0`–`-O 3`.
- The trapped success path is ≤ 1.5× plain (+2 ns).
- Error-path behavior is unchanged: the handler's `e` has the same code, message and `ErrorLoc`, `RECOVER` works, and
  the check-chain short-circuit (`outer` is not called when `inner` fails) and `Float` observation-boundary semantics
  (`lower_checked_value`'s `observe_float`) are unchanged.

### Non-goals (must NOT change)

Owner constraint: no language-spec or memory-model change.

- Inline-`TRAP` semantics (§8 rules 11–14) are unchanged: what is covered, what is rejected, the order of evaluation,
  the short-circuit on the first failure, `RECOVER` delivery, and the warnings (`TYPE_INLINE_TRAP_DEAD_HANDLER`).
- The heap `Result` representation is unchanged wherever a `Result` is a **program-visible value**: a user `Result OF T`
  binding, return, argument, collection element, or `thread::waitFor`'s `WorkerArena` path. Only the desugar's own
  temporaries, whose every use is `ResultIsOk`/`ResultValue`/`ResultError` inside the same statement, become unboxed.
- The fallible-call register ABI and the `Error` record layout are unchanged. On failure, the `Error` the handler binds
  is still materialized exactly as today.
- `.ir` goldens are unchanged if the fix lives in codegen (recommended), since the desugar's IR stays byte-identical. If it
  lives in the IR, see Open Decisions.
- **Tempting wrong fixes, forbidden:** removing `TRAP`s from library bodies as "the fix". That is legitimate cleanup
  where a handler is provably dead (bug-772 does it for the scalar seam), but it does not fix this bug. Also forbidden:
  skipping the scrub for "small" blocks, and pooling `Result` blocks (that changes allocator behavior, not the cost model).

## Blast Radius

**User code** is the main consumer: every inline `TRAP` in a loop pays this. It is documented as the idiomatic way to
handle a per-item parse (`toInt`, `toScalar`, `json::parse`…).

**Builtin `.mfb` bodies with an inline `TRAP`**. I listed them with a scan of `src/codegen/builtins/**/*.rs` for
` TRAP(` inside source bodies, recording each site's loop depth within its FUNC, and excluding the `EX` man examples
(`[main]`). Each entry has a verdict:

| site | executes | verdict |
|---|---|---|
| `strings/helper_scalar_seam.rs:__strings_toScalars` `toScalar(cp) TRAP` | **per scalar** (loop depth 1) | hot. bug-772 removes the TRAP from this path; this bug removes the cost everywhere |
| `json/helper_parse_hex_quad.rs:__json_parseHexQuad` `toInt(toString(quad), 16) TRAP` | per `\uXXXX` escape (called from `helper_parse_unicode_escape.rs`) | warm, latent, fixed by this bug. No D/F row isolates it |
| `http/helper_linger_net.rs`, `helper_linger_tls.rs`, `helper_read_request_net.rs`, `helper_read_request_tls.rs` `tcp/tls::read(...) TRAP` | per 64 KiB read, in a loop | per I/O syscall, so negligible (≤ 30 ns against a syscall). Fixed by this bug |
| `http/helper_hex_to_int.rs:__http_hexToInt` (called per chunk by `helper_chunked_scan.rs`/`helper_dechunk_bytes.rs`), `helper_dec_to_int.rs`, `helper_framing_length.rs`, `helper_request_framing.rs`, `helper_frame_advance.rs` (2), `helper_frame_complete.rs`, `helper_build_response.rs`, `helper_parse_request.rs`, `helper_invoke_handler.rs`, `helper_read_net.rs`, `helper_read_tls.rs`, `func_respond_path.rs`, `func_handle_request.rs` (10) | per request / per chunk | negligible against network I/O. Fixed by this bug |
| `net/helper_decode_query_component.rs` (per query component, from `func_parse_query.rs`), `net/helper_parse_port.rs` (per URL) | per component | cold. Fixed by this bug |
| `regex/helper_parse_hex_escape.rs` (2), called from `helper_parse_literal_escape.rs` | per escape at **compile** time | cold. Fixed by this bug |
| `audio/helper_mml_parse_uint.rs` (called per MML number by `helper_mml_note.rs`/`_expand.rs`/`_req_int.rs`) | per note | cold. Fixed by this bug |
| `canvas/helper_font_name.rs` (2), `helper_png.rs`, `helper_surface.rs` (2) | per file / per frame-stats write | cold. Fixed by this bug |
| `strings/helper_scalar_seam.rs:__strings_fromScalars` `utf32Decode(cps) TRAP` | once per call | cold. bug-772 removes it |

No builtin site is on a D/F benchmark row other than through bug-772. The value of this fix is user code and the general
cost model.

## Fix Design

Stay entirely inside codegen, so the IR, the spec'd desugar and the goldens are unchanged. Codegen learns an **unboxed
trap result**:

1. **Recognize the temporaries.** A `Bind` of a `CallResult`/`Checked` whose binding is a desugar temporary
   (`make_temp_local_name(…, "trap_res")`). Better, and more robust than a name test: a NIR flag the lowering already
   has the information to set. Every use of the binding is `ResultIsOk`/`ResultValue`/`ResultError` in the same
   statement group. Verify that property rather than assume it, and fall back to the boxed form when it does not hold.
2. **Represent it in the frame.** Four stack slots: tag, value, message, source. These are exactly the `raw_result_*`
   slots the `CallResult` arm already spills. `CallResult`/`Checked` store the raw registers there and stop, with no
   `emit_build_result_inline`.
3. **Readers.** `ResultIsOk` compares the tag slot. `ResultValue` takes the value slot, and for a block payload **moves**
   the callee's returned block into the binding, so there is no copy in or out and the temporary no longer owns it.
   `ResultError` materializes the `Error` record from the message/source slots with the existing loose-error
   materialization (`materialize_current_result`'s error half). This runs only on the `else` branch.
4. **Drop.** The unboxed temporary owns nothing on success, because the payload was moved. On failure, the materialized
   `Error` follows today's handler-scope ownership.

Where the risk sits: (a) block-payload ownership, meaning the move out of `ResultValue` must not double-free with the
temporary's drop. `rt_scope_drop_leaks.rs` and `tests/rt-behavior/collections/hof-callback-failure-rt` cover it.
(b) `Float` observation in `lower_checked_value`, which must keep running before the tag is set. (c) `Nothing` payloads
(bug-744's special case). (d) the worker-arena path (`TrappedErrorTagClass::WorkerArena`), which must stay boxed.

Rejected: (1) Changing the desugar to not bind a `Result`. That changes the IR, which is spec-described (§8.8) and pinned
by many `.ir` goldens, and it gains nothing over the codegen representation. (2) A small-block freelist for `Result` blocks
only makes the allocation cheaper; it does not remove it.

## Adjacent finding (not fixed here — filed as bug-779, `bugs/bug-779-inline-trap-on-a-call-free-operator-chain-compiles.md`)

While building the table, `LET v AS Integer = i * 2 + 3 TRAP(err)` (no call at all) **compiled**, as do `(i * 2) + 3` and
`i / 2 + 1`, while `i + 1` and `i * 2` are correctly rejected with `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`. Checked with
`target/release/mfb build -q` of a 12-line program per expression, `/tmp/dbD-cf`. §8 rule 11 says a scrutinee "with no
runtime call to trap anywhere in it — a call-free expression … (`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`)" is rejected, so
`LET d = 1 / z TRAP(e)` is. The likely mechanism: once `hoist_trap_calls` lifts a nested raising operator, `check_root`
treats the handler as "demonstrably live", and `ir::verify`'s `check_inline_trap_scrutinee` no longer sees the bare
non-call bind it keys on. That is a guess; I have not traced it. **This is a diagnostics correctness bug** (an invalid
program is accepted), independent of the allocation cost. It needs its own number.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_inline_trap_success_alloc.rs`, modeled on
      `tests/runtime/rt_debug_arena.rs::fill_counters_match_known_allocations` (`--debug` counters at N and 2N). For each
      shape in the table, assert `delta(alloc_calls)` of the trapped program equals the plain program's. It fails today
      (+1 to +3 per evaluation).
- [ ] Add error-path guards to the same file: each shape with an input that fails, checking the handler's `e.code` /
      `e.message`, and `live_bytes == 0` at exit. These pass today and must keep passing.

Acceptance: the success-path assertions fail by exactly the measured counts; the error-path guards pass.
Commit: —

### Phase 2 — the fix

- [ ] Codegen: the unboxed trap-result representation (`builder_values.rs` `CallResult`/`Checked`/`ResultIsOk`/
      `ResultValue`/`ResultError` arms, `builder_arena_transfer.rs`), with the boxed fallback for every non-temporary `Result`.

Acceptance: Phase 1 passes; `rt_scope_drop_leaks.rs`, `rt_callback_failure_reaches_trap.rs`, every
`tests/rt-behavior/**/*trap*` fixture and the error-model acceptance goldens pass unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `.ir` goldens: expect **no** change, since the desugar is untouched. Any `.ir` diff is a bug-hunt trigger.
      `.ncode`/codegen goldens shift only inside trapped statements. Localize one fixture before accepting any diff
      (AGENTS.md).
- [ ] Re-run the table: equal allocation counts, and time within target.
- [ ] `./benchmark/run.sh 10`: no row regresses; the scalarbench rows (bug-772) and JSON/HTTP rows move only within noise
      or better.
- [ ] Full suite.

Acceptance: full suite green; the golden deltas are exactly the intended lowering.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_inline_trap_success_alloc.rs`.
- Runtime proof: the table re-measured at `-O 1` and `-O 3`.
- Doc sync: `mfb spec memory fallible-call-abi` gains one sentence stating that a desugar temporary is unboxed (frame
  slots) and the `Error` is materialized only on the error path. No `mfb man` change.
- Full suite: `cargo test` per `.ai/compiler.md` *Hard Completion Gate*, plus `artifact-gate`/`test-accept`
  (`.ai/testing-gates.md`).

## Open Decisions

- How to recognize the temporaries: by name, or by a NIR marker? Recommended: a marker (e.g. a flag on the `Bind`) set by
  `lower_inline_trap`, with a codegen-side verification that every use is one of the three readers. The name test is the
  fallback. The alternative, a codegen-only use-analysis with no marker, is more fragile across package IR decode.

## Summary

The cost is one heap round-trip per fallible node per trapped evaluation, paid on the success path by every program that
uses the idiomatic per-item inline `TRAP`. The fix is a codegen representation change for compiler temporaries only, with
the risk in block-payload ownership and the `Float`/`Nothing`/worker-arena special cases. The IR and the language are
untouched.
