# plan-158-A: Native error status messages + Windows audio adoption

Last updated: 2026-09-26
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: nothing

A native runtime helper that fails today can only return a fixed message, one per
error code (`raise_error_into`, `src/codegen/memory/data/data_objects.rs`). Every
failed WASAPI, Core Audio and ALSA call therefore reaches the MFBASIC developer as the
same `77050018 ErrAudioDevice — "Audio device open, configuration, or stream operation
failed."`, and the OS status that says *why* is discarded. During the 2026-09-26
session the only way to learn that a Windows stream had failed with
`AUDCLNT_E_DEVICE_INVALIDATED` was to patch the compiler.

MFBASIC code can already raise an error with a runtime-built message: `FAIL` of an
`Error` value builds one owned `Error` block, parks it in the per-thread current-error
slot, and returns `RESULT_ERR_BLOCK_TAG`. This plan lets a native helper use that same
route. **Outcome:** a failed Windows audio call raises its usual error code, and
`e.message` reads `Audio device open, configuration, or stream operation failed.
(IAudioRenderClient::GetBuffer: 0x88890004)`.

References:

- `src/codegen/error/constants/error_constants.rs` (`RESULT_ERR_BLOCK_TAG` and its doc
  comment: the parked-block protocol)
- `src/codegen/error/emission/park_error_helper.rs` (module doc; `lower_park_error_helper`)
- `src/codegen/error/emission/builder_error_emission.rs`
  (`emit_park_error_block_from_registers`, `emit_stamp_current_error_source`,
  `emit_free_error_block_from_slot`, `emit_store_current_error`)
- `src/codegen/memory/arena/builder_arena_transfer.rs` (`TrappedErrorTagClass`)
- `.ai/codegen-invariants.md`, `.ai/arch-abi.md` §Windows audio
- `src/docs/spec/stdlib/11_audio.md`, `mfb spec language errors`

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Tree builds at HEAD | `cargo build --release` → `Finished` | MET (2026-09-26) |
| Windows box reachable (user `test`, password `password`) | `ssh test@192.168.1.206 ver` → `Microsoft Windows [Version 10.0.26200…]` | MET (2026-09-26) |

Everything below is written for a world where both hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- A native helper can fail with `"<registered message> (<call>: 0x<8 hex digits>)"`,
  delivered as a parked `Error` block (tag 3), with the error's own code.
- Every Windows audio failure site that has an OS status reports it this way.
- The failure keeps its call-site source location (`e.source`), as helper errors do today.
- No message or `Error` block is leaked or freed twice, including under OOM.

### Non-goals (explicit constraints)

- No new error codes; the error-code table and generated `errorCode::` constants are unchanged.
- No new ownership rule and no arena-state change: the parked-block protocol is used as it is.
- `raise_error_into` / `emit_fail` output is byte-identical for every non-adopting site.
- `Error` block layout unchanged.
- macOS and ALSA adoption is plan-158-B; the Windows exclusive-mode fixes are plan-158-C.

## 2. Current State

- **User-level runtime messages already work.** `FAIL <Error value>` builds an owned
  block and parks it (`builder_exits.rs`, the `RESULT_ERR_BLOCK_TAG` store at the
  `emit_build_error_inline` call).
- **Tag 3 handling.** Every "is it not Ok?" test is `tag != 0`, so tag 3 propagates like
  tag 1 (`error_constants.rs`, doc on `RESULT_ERR_BLOCK_TAG`). An inline `TRAP` adopts
  the parked block instead of rebuilding (`trapped_error_tag_class` →
  `TrappedErrorTagClass::ParkedBlock`).
- **The park helper.** `_mfb_rt_park_error` reads code / message / source from
  `RESULT_VALUE_REGISTER` / `RESULT_ERROR_MESSAGE_REGISTER` /
  `RESULT_ERROR_SOURCE_REGISTER`, builds the block (message copied inline), stores it in
  the current-error slot, releases the source `ErrorLoc` (bug-573), restores the loose
  code and message registers, and sets tag 3 (`emit_park_error_block_from_registers`).
  On OOM it returns the loose tag 1 instead (`building_error_block` guard, module doc).
  It is emitted iff some function relocates against `PARK_ERROR_SYMBOL`
  (`builder/mod.rs`, `uses_park_error`).
- **Error block layout:** `{code@0, messageOffset@8, sourceOffset@16, [message][ErrorLoc]}`;
  `ERROR_OBJECT_SIZE = 24` (`error_constants.rs`).
- **The gap.** The propagating runtime-call path (`builder_emit_helpers.rs`, the
  `emit_stamp_current_error_source()` branch after `runtime_call_ok`) stamps and parks
  **unconditionally** for any non-Ok helper result. A helper that returned tag 3 would be
  parked a second time; the first block is overwritten in the slot and leaks, and the
  loose message register would point into it.
- **Windows backend:** `gen_windows_open.rs` (`lower_open`, `lower_close`),
  `gen_windows_io.rs` (`lower_write`, `lower_read`, `lower_query`),
  `gen_windows_devices.rs`. Failing COM calls share `dev_fail` / `unavailable` tails;
  `lower_open`'s `dev_fail` runs `emit_open_cleanup` before raising. The HRESULT is in
  `abi::return_register()` after `com_call` (sign-extended by `ole_call`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `emit_fail(` calls in audio backends | 48 (Windows 13, macOS 18, ALSA 17) | `cat src/codegen/builtins/audio/gen_{windows,macos,alsa}*.rs \| grep -c 'emit_fail('` → 48 |
| …`ErrAudioDevice` / `ErrAudioUnavailable` | 11 / 10 | `grep -A2 'emit_fail(' src/codegen/builtins/audio/gen_*.rs \| grep -oE '"Err[A-Za-z]+"' \| sort \| uniq -c` |
| OS-status branches to a fail/unavailable label, Windows | 21 (open 12, io 5, devices 4) | `grep -cE 'branch_(eq\|ne\|lt\|le\|gt\|ge\|lo\|hs\|hi\|ls)\(&?[a-z_]*(fail\|unavailable)' src/codegen/builtins/audio/gen_windows*.rs` |
| `emit_stamp_current_error_source()` call sites | 2 | `grep -rn 'emit_stamp_current_error_source()' src/codegen` → `builder_emit_helpers.rs`, `builder_thread_cleanup.rs` |
| Windows audio emit-inspection tests | 5 | `grep -c '#\[test\]' src/codegen/builtins/audio/gen_windows_tests.rs` |

### Verified properties

- Park inputs/outputs and tag 3: read `emit_park_error_block_from_registers` (2026-09-26).
- `TRAP` adopts tag 3: read `trapped_error_tag_class` (2026-09-26).
- UNVERIFIED: that runtime helper functions are in `code_functions` before the
  `uses_park_error` gate runs, so a helper's `bl _mfb_rt_park_error` triggers emission
  of the park. Phase 1 task.
- UNVERIFIED: what `e.source` a trapped helper error carries today on the raw
  (inline-`TRAP`) path, which does not stamp. Phase 1 records it, so tag 3 keeps parity.
- UNVERIFIED: whether the `builder_thread_cleanup.rs` stamp site can receive a tag-3
  helper result. Phase 1 task.

## 3. Design Overview

1. **Helper side: `emit_fail_status(symbol, error_name, call, status, …)`**, in
   `native_fail.rs`.
   - Build the message in the arena: a static prefix `"<registered message> (<call>: "`
     (a data object whose symbol encodes error name + call, emitted by a post-codegen
     relocation scan like the fixed `_mfb_str_error_*` messages), then `0x` + 8 hex
     digits + `)`.
   - Load the code, the message, and source `0` into the three registers; `bl
     _mfb_rt_park_error`. The park copies the message into the block.
   - Free the helper's temporary message; point `RESULT_ERROR_MESSAGE_REGISTER` at the
     parked block's inline copy (`[slot] + [[slot]+8]`); return tag 3.
   - OOM while formatting → plain `emit_fail(error_name)` (tag 1, static). OOM inside
     the park (tag 1 back) → free the temporary, raise the static `ErrOutOfMemory`.
   - The formatting and park call live in one out-of-line runtime helper
     (`_mfb_rt_fail_status`, inputs: prefix ptr, status, code), so each site is a few
     instructions.
2. **Call-site side:** in the propagating runtime-call path, branch on the tag. Tag 1
   keeps today's stamp. Tag 3 re-stamps to attach the call-site location:
   - save the parked block base from the current-error slot;
   - run `emit_stamp_current_error_source` (it builds the `ErrorLoc` and re-parks,
     copying code and message from the old block);
   - free the old block (`emit_free_error_block_from_slot`);
   - point the loose message register at the new block's inline message.
   The inline-`TRAP` path already adopts tag 3; Phase 1 decides whether it needs the
   location too.
3. **Shared cleanup tails:** `StatusFailSites`. Each failing check branches to its own
   stub. The stub stores the status and the prefix address in two frame slots, then
   branches to the shared tail; the tail runs cleanup and calls `_mfb_rt_fail_status`.
4. **Windows adoption** at all 21 status sites.

**Correctness risk** is in piece 2 (a free on the error path) and in the OOM arms of
piece 1. Both are covered by tests before the backend conversion. The error-path
re-park costs one extra copy only when an error is raised.

**Gate class:** behavior changes; byte-identity is not the gate. Expected diffs:
`tests/byte-identity/audio/golden/audio_codegen_cover_rt.windows-x86_64.ncodesum`, and
every `ncodesum` whose program makes a propagating runtime-helper call, because piece 2
adds a tag compare there. A diff elsewhere is a bug to root-cause with `-ncode` on one
fixture.

**Rejected:** a separate owned-message scheme (a pending-message arena word plus a
conditional free in `emit_build_error_inline`). It duplicates what the parked-block
protocol already does. The design from the first draft of this plan.

## 4. Detailed Design

### 4.1 Prefix data object

`error_status_prefix_symbol(error_name, call) = "_mfb_str_errstatus_<ErrorName>_<hex(call)>"`.
A post-codegen relocation scan in `builder/mod.rs`, next to the bug-545
`_mfb_str_error_` block, decodes each referenced symbol into
`string_data_object(symbol, format!("{message} ({call}: "))`, iterating a `BTreeSet`
for a deterministic layout.

### 4.2 `_mfb_rt_fail_status`

- Inputs: `c_arg(0)` = prefix String, `c_arg(1)` = status, `c_arg(2)` = code.
- Returns: tag 3 with message = the parked block's inline copy, or tag 1 with a static
  message on OOM.
- Body: vregs with `finalize_vreg_body_with_locals` (precedent
  `lower_int_to_string_helper`, `src/codegen/string/format/int_format.rs`).
  1. `_mfb_arena_alloc(len+20, 8)`; copy the prefix; append `0x`, 8 nibbles (`+48` / `+87`,
     precedent `emit_hex_nibble_to_path` in `gen_temp_file.rs`), `)`, NUL; store the length.
  2. Set the code, message and `0` source registers; `bl _mfb_rt_park_error`.
  3. `arena_free(temp, len+9)`; then, if tag 3, message = slot base + messageOffset.
     If tag 1, message = static `ErrOutOfMemory`, code = its code.
- Emitted iff referenced (relocation gate, like the park). Its own `bl` to the park keeps
  the park emitted (Phase 1 verifies the order).

### 4.3 Call-site tag-3 branch

In `builder_emit_helpers.rs`, after `runtime_call_ok`:

- `cmp tag, 3; b.ne legacy`;
- save the slot base in a stack object;
- `emit_stamp_current_error_source()`;
- `emit_free_error_block_from_slot(old)`;
- reload the message register from the new slot base;
- `b exit`;
- `legacy:` the existing stamp.

Mirror it at `builder_thread_cleanup.rs` if Phase 1 finds that site can see tag 3.

### 4.4 Windows call names

`IMMDeviceEnumerator::GetDefaultAudioEndpoint`, `IMMDeviceEnumerator::GetDevice`,
`IMMDevice::Activate`, `IAudioClient::Initialize`, `IAudioClient::GetMixFormat`,
`IAudioClient::GetBufferSize`, `IAudioClient::GetCurrentPadding`,
`IAudioClient::SetEventHandle`, `IAudioClient::GetService`, `IAudioClient::Start`,
`IAudioRenderClient::GetBuffer`, `IAudioRenderClient::ReleaseBuffer`,
`IAudioCaptureClient::GetBuffer`, `IAudioCaptureClient::ReleaseBuffer`,
`CoCreateInstance`, `CreateEventW` (status = `GetLastError()`). The non-COM open checks
(mix rate ≠ requested, mix bits ≠ 32, mix channels < requested) pass the observed device
value as the status: `mix rate: 0x0000bb80` tells the developer the device runs at 48 kHz.
Windows frame slots 160 and 168 are unused (`gen_windows.rs`: 152 = `COUNT_OFF`,
176 = `DEADLINE_OFF`); re-verify before use.

## Compatibility / Format Impact

- `e.message` for Windows audio failures gains `(<call>: 0x<status>)`; `e.code` unchanged.
- `.mfp` format, `Error` layout, arena state, error-code table: unchanged.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work; `- [~]` for partial; mark moot tasks `~~struck~~ — moot: <evidence>`; fill
> `Commit:` when a phase lands. An unticked box means NOT DONE.

### Phase 1 — Settle the three UNVERIFIED items (no code change)

- [ ] Read the `builder/mod.rs` function that pushes runtime helpers into
      `code_functions`; record whether that happens before `uses_park_error`. If not,
      note the reorder or the extra gate term Phase 2 needs.
- [ ] Build a probe that TRAPs a helper error on the raw path (e.g.
      `fs::readText("/nonexistent") TRAP(e)`). Print `e.source` and record it.
- [ ] Read `builder_thread_cleanup.rs` around its `emit_stamp_current_error_source()`
      call; record whether a tag-3 result can reach it.

Acceptance: no UNVERIFIED line remains in §Verified properties.
  Check: `grep -c UNVERIFIED planning/plan-158-A-native-error-status-messages.md` → 0 (est. 1 min).
Commit: —

### Phase 2 — Primitive + call-site branch

- [ ] `data_objects.rs`: `error_status_prefix_symbol`, decode function, unit tests
      (round trip, malformed symbol rejected).
- [ ] `builder/mod.rs`: prefix relocation scan; emission gate for `_mfb_rt_fail_status`.
- [ ] New `src/codegen/error/emission/fail_status.rs`: `lower_fail_status_helper` (§4.2).
- [ ] `native_fail.rs`: `emit_fail_status`, `StatusFailSites`.
- [ ] `builder_emit_helpers.rs`: the tag-3 branch (§4.3); plus the thread-cleanup site
      per Phase 1.
- [ ] Unit test: the helper body contains exactly one `bl _mfb_rt_park_error` and one
      `bl _mfb_arena_free` after it.

Acceptance: unit tests pass; the byte-identity gate diffs only where §3 predicts.
  Check: `cargo test --bin mfb fail_status` → pass (est. 3 min); `scripts/artifact-gate.sh`
  → every DIFF is a program with a propagating runtime-helper call. Inspect one per
  target with `-ncode` (est. 15 min: the tag compare lands in every such program, so no
  scoped subset would show a stray diff).
Commit: —

### Phase 3 — Windows adoption + box proof

- [ ] `gen_windows_open.rs` `lower_open`: 12 sites via `StatusFailSites` (names §4.4);
      the tails keep `emit_open_cleanup`.
- [ ] `gen_windows_io.rs`: 5 sites. `gen_windows_devices.rs`: 4 sites.
- [ ] `gen_windows_tests.rs`: emit-inspection test that no `ErrAudioDevice` /
      `ErrAudioUnavailable` raise in these helpers bypasses `_mfb_rt_fail_status`.
- [ ] Regenerate `audio_codegen_cover_rt.windows-x86_64.ncodesum`; confirm with `-ncode`
      that the delta is confined to the audio helpers and the call-site branch.
- [ ] Box proof on 192.168.1.206. Probe under `/tmp`: open a SHARED 48000/1 stream, open
      and close a second stream on the same device, write to the first. Print `e.message`
      and `e.source` under TRAP, and once uncaught. Run the same failure 10 000 times in
      a loop under `mfb build --debug`: arena counters show no net growth. Record the
      output in §Corrections and hand the statuses to plan-158-C.

Acceptance: the box prints `… (IAudioRenderClient::GetBuffer: 0x88890004)` with the
call-site line in `e.source`; the loop shows no growth.
  Check: box probe output contains `: 0x88890004)` (est. 3 min).
Commit: —

## Validation Plan

- Tests: data-object unit tests, helper-shape test, Windows emit-inspection test; box
  runtime proof, including the leak loop.
- Coverage: the Windows audio helpers are in the gate through the Windows audio
  ncodesum. The runtime path is proven only on the box; record it.
- Doc sync:
  - `src/docs/spec/stdlib/11_audio.md` (message format; cite `_mfb_rt_fail_status`);
  - `mfb spec language errors` / the park section (native helpers may return tag 3; the
    call site re-stamps it);
  - man: the `audio` package's `ErrAudioDevice` description. Check with
    `mfb man audio write`, then `scripts/man-census.sh --memory-scope` → 0 unclassified.
- Final gate (once, after Phase 3): `cargo test` plus `scripts/artifact-gate.sh` (est. 45 min).

## Open Decisions

- Status format: `0x%08x` everywhere (recommended; HRESULT, OSStatus and negative errno
  are all 32-bit and documented in hex) vs. per-backend decimal. plan-158-B inherits it.

## Corrections

## Summary

The helper reuses the existing parked-`Error` protocol that `FAIL` uses. The only new
error-path logic is the call site re-stamping a tag-3 helper result so it keeps its
source location, and the helper's OOM arms. Untouched: error codes, the `Error` layout,
arena state, and every non-adopting helper.
