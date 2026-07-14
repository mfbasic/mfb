# plan-14-B: `fs::` per-handle buffering + remove `io::flushError`

Last updated: 2026-07-05 (reference refresh — see overview banner)
Effort: medium

Part **B** of plan-14 (Opt-In Output Buffering). Mirrors the buffering trio per `File` handle (with
the stricter flush-on-close rule) and removes the now-redundant `io::flushError`. Shared design
lives in the overview: [plan-14-io-buffering.md](plan-14-io-buffering.md).

- **Depends on:** plan-14-A (reuses the buffer machinery); the `io::flushError` removal (Phase B2)
  is independent and landable in either order.
- **Spec/design:** overview §4.5 (`fs::` per-handle), §4.2.1 (`flushError` removal).

## Phases

### Phase B1 — `fs::` per-handle buffering (§4.5)

Mirror the trio per `File`, with the stricter flush-on-close rule.

- [ ] Add `fs::setBuffered`/`isBuffered`/`flush` builtins (per-`File`).
- [ ] Add the buffer field set (ptr/fill/enabled) to the `File` resource runtime layout.
- [ ] Buffer `fs::writeAll`/`writeAllBytes` (copy into the handle buffer, drain on buffer-full); leave whole-file `writeText`/`writeBytes`/`append*`/`*Atomic` unbuffered.
- [ ] Wire the **mandatory flush-on-close/drop** into the `File` resource teardown (both `fs::close` and lexical scope-drop) so on-disk data is never stranded.
- [ ] Tests: `tests/func_fs_setBuffered_*`, `tests/func_fs_isBuffered_*`, `tests/func_fs_flush_*` (`_valid/**` + `_invalid/**`, `File` arg).

Acceptance: a loop of small `fs::writeAll`s on a buffered handle issues ~1 `write` per 4 KiB; the on-disk file is byte-identical to the unbuffered run after `fs::close`/scope exit (proving flush-on-close); an early `fs::close` and a scope-drop both flush; an unbuffered handle is unchanged from today.
Commit: —

### Phase B2 — Remove `io::flushError` (§4.2.1)

Independent of the buffering mechanism; landable in either order.

- [ ] Drop `FLUSH_ERROR` from the builtin/runtime/codegen touch points (`src/builtins/io.rs:8`; the `io.flushError` runtime spec/symbol in `src/target/shared/runtime/io_specs.rs:80`; the `"io.flushError"` dispatch arms; app-mode bodies; platform plans — all cited in §4.2.1).
- [ ] Delete `src/docs/man/builtins/io/flushError.txt` and every `io::flushError` cross-ref + spec-list entry (§4.2.1).
- [ ] Update the two affected tests/goldens (`tests/native_io_runtime.rs:442`; the `package-import-as` fixture + `.ast`/`.ir` goldens) to use `io::flush` / `console::flush`; add a `tests/func_io_flush_*` proving the drain.

Acceptance: `io::flushError` is gone from the builtin set, every cited touch point (codegen/runtime/man/spec) is updated, the affected tests/goldens use `io::flush`, and the full suite + acceptance pass.
Commit: —
