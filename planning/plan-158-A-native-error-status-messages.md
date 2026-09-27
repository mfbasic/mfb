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

This sub-plan builds a runtime-formatted error message, carrying the failing call's
name and the OS status value, and adopts it across the Windows audio backend.
**Outcome:** a failed Windows audio call raises its usual error code, and `e.message`
reads `Audio device open, configuration, or stream operation failed.
(IAudioRenderClient::GetBuffer: 0x88890004)`.

References:

- `.ai/compiler.md`, `.ai/codegen-invariants.md` (register clobbers across `bl`),
  `.ai/arch-abi.md` §"Windows (PE / console / audio)" and §"WASAPI capture carry-over"
- `src/codegen/error/emission/park_error_helper.rs` (module doc: the park funnel),
  `src/codegen/error/emission/builder_error_emission.rs`
  (`emit_stamp_current_error_source`, `emit_park_error_block_from_registers`,
  `emit_build_error_inline`)
- `src/codegen/error/constants/error_constants.rs` (arena-state layout)
- `mfb spec language errors` / `src/docs/spec/stdlib/11_audio.md`,
  `src/docs/spec/diagnostics/02_error-codes.md`

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Tree builds at HEAD | `cargo build --release` → `Finished` | MET (2026-09-26) |
| Windows box reachable (password `password`) | `ssh test@192.168.1.206 ver` → `Microsoft Windows [Version 10.0.26200…]` | MET (2026-09-26) |

Everything below is written for a world where both hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- A native helper can fail with `"<registered message> (<call>: 0x<8 hex digits>)"`.
  The prefix is fixed text chosen at compile time; the status is the runtime value.
  The error code stays the error's own, so an existing `TRAP`/`CASE errorCode::…`
  match is unaffected.
- Every Windows audio failure site that has an OS status reports it this way.
- No message string is ever leaked or double-freed, including under inline `TRAP`,
  propagation, worker threads, and OOM while building the message.

### Non-goals (explicit constraints)

- No new error codes; `02_error-codes.md` and the generated `errorCode::` constants
  are unchanged.
- `ARENA_STATE_SIZE` must not grow (growing it changes every program's code).
- `raise_error_into` / `emit_fail` output is byte-identical for every site that does
  not adopt the new call.
- The `Error` record layout (`ERROR_OBJECT_SIZE`, inline message copy) is unchanged.
- macOS and ALSA adoption is plan-158-B. The Windows exclusive-mode fixes are
  plan-158-C. Neither is in scope here.

## 2. Current State

- **Raise:** `raise_error_into(from, error_name, …)` (`data_objects.rs`) loads the
  code immediate, `RESULT_ERR_TAG`, and the address of the static
  `_mfb_str_error_*` object into `RESULT_ERROR_MESSAGE_REGISTER`
  (`error_constants.rs`: `abi::mfb_return(2)`). `emit_fail`
  (`src/codegen/error/emission/native_fail.rs`) wraps it and branches to `done`.
- **Message emission:** the fixed messages are data objects emitted after codegen by a
  relocation scan (`src/codegen/engine/builder/mod.rs`, the bug-545 block that filters
  `relocation.to.starts_with("_mfb_str_error_")`).
- **Call site, propagating:** `builder_emit_helpers.rs` (runtime-call path) runs
  `emit_stamp_current_error_source` → `emit_park_error_call(StampedCallSite)` →
  `_mfb_rt_park_error` → `emit_park_error_block_from_registers` →
  `emit_build_error_inline`. That copies the message bytes into the new owned `Error`
  block, then **restores the original message pointer** into the loose register
  (`emit_park_error_block_from_registers`, the reload after
  `emit_release_parked_error_source`). The top-level exit printer reads that loose
  register (`src/codegen/engine/function/entry.rs`, `emit_write_string_object`).
- **Call site, inline `TRAP`:** the `raw` branch in the same function goes to
  `materialize_current_result` (`builder_arena_transfer.rs`) without stamping; it
  adopts a parked block or rebuilds one with `emit_build_error_inline`.
- **Ownership today:** the helper's message pointer is never freed by anyone. That is
  sound only because every helper message is static. `_mfb_arena_free`
  (`src/codegen/memory/arena/arena.rs`: `lower_arena_free`) bins whatever pointer it
  is given, so freeing a static object corrupts the free list.
- **Precedent for runtime messages:** none among the helpers. The nearest owned-copy
  precedent is the worker path, `lower_thread_finish` + `emit_finalize_worker_error_source`.
  Integer formatting precedent: `_mfb_rt_int_to_string`
  (`src/codegen/string/format/int_format.rs`: `lower_int_to_string_helper`). Hex
  nibble precedent: `emit_hex_nibble_to_path` (`src/codegen/builtins/fs/gen_temp_file.rs`).
- **Windows backend:** `gen_windows_open.rs` (`lower_open`, `lower_close`),
  `gen_windows_io.rs` (`lower_write`, `lower_read`, `lower_query`),
  `gen_windows_devices.rs`. The failing COM calls share `dev_fail` / `unavailable`
  labels, and `lower_open`'s `dev_fail` runs `emit_open_cleanup` (COM `Release` ×4)
  before raising. The HRESULT is in `abi::return_register()` right after `com_call`
  (sign-extended by `ole_call`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `emit_fail(` calls in audio backends | 48 (Windows 13, macOS 18, ALSA 17) | `cat src/codegen/builtins/audio/gen_{windows,macos,alsa}*.rs \| grep -c 'emit_fail('` → 48; per file via `grep -c` |
| …of which `ErrAudioDevice` / `ErrAudioUnavailable` | 11 / 10 | `grep -A2 'emit_fail(' gen_*.rs \| grep -oE '"Err[A-Za-z]+"' \| sort \| uniq -c` |
| OS-status branches to a fail/unavailable label, Windows | 21 (open 12, io 5, devices 4) | `grep -cE 'branch_(eq\|ne\|lt\|le\|gt\|ge\|lo\|hs\|hi\|ls)\(&?[a-z_]*(fail\|unavailable)' gen_windows*.rs` |
| same, macOS / ALSA (plan-158-B) | 20 / 13 | same command on `gen_macos*.rs` / `gen_alsa*.rs` |
| `emit_build_error_inline` callers | 3 | `grep -rn 'emit_build_error_inline(' src/codegen \| grep -v 'fn emit_build_error_inline' \| wc -l` → 3 |
| `raw_result_discard_error` references | 10 | `grep -rn raw_result_discard_error src/codegen \| wc -l` → 10 |
| Windows audio emit-inspection tests | 5 | `grep -c '#\[test\]' src/codegen/builtins/audio/gen_windows_tests.rs` → 5 |

### Verified properties

- **A spare per-thread word exists at `ARENA_FLAG_RHS_OFFSET + 8`.** `ARENA_FLAG_RHS_OFFSET`
  reserves a 16-byte v128 slot (`ARENA_CURRENT_ERROR_OFFSET = ARENA_FLAG_RHS_OFFSET + 16`),
  and the rv64 flag snapshot uses only the first 8 bytes. Verified:
  `grep -rn ARENA_FLAG_RHS_OFFSET src` shows only rv64 `select.rs` addressing the base
  offset, and `grep -rn 'FLAG_RHS_OFFSET *+' src` shows only the `+ 16` definition.
  The word is zeroed by the whole-`ARENA_STATE_SIZE` clear on entry and thread spawn
  (doc comment on `ARENA_CURRENT_ERROR_OFFSET`). UNVERIFIED: that no rv64 codegen
  writes 16 bytes there. Phase 1 task.
- **The Windows failures this plan exists to diagnose were measured with a throwaway
  instrumented compiler** (reverted), on box 192.168.1.206 (Win11 26200, devices
  "ELEFW195 (Intel(R) Display Audio)" and "Speakers (2- Synaptics HD Audio)"):
  a SHARED 48000/1 stream's `GetBuffer` returned `0x88890004` after another stream was
  opened on the same device; an EXCLUSIVE 48000/2 or 44100/2 stream's first
  `audio::write` never returned (not in `WaitForSingleObject`; a 2 s timeout did not
  end it). Plan-158-C owns the fixes.

## 3. Design Overview

Three pieces, layered:

1. **Status message builder.** A runtime helper `_mfb_rt_error_status_message(prefix,
   status)`. `prefix` is a static `mfb.string.v1` holding `"<registered message> (<call>: "`,
   emitted like the fixed messages (symbol-encoded, relocation-scan gated). The helper
   allocates `len(prefix) + 11 + 9` bytes in the current arena, copies the prefix,
   appends `0x` + 8 lowercase hex digits of the low 32 bits of `status` and `)`,
   writes the NUL, and returns the String. It records the pointer in the per-thread
   **pending-owned-message word** (`ARENA_PENDING_ERROR_MESSAGE_OFFSET =
   ARENA_FLAG_RHS_OFFSET + 8`). Before allocating, it frees any stale pointer still in
   that word. On OOM it returns the static `ErrOutOfMemory` message and leaves the word
   at 0.
2. **Ownership at the copy point.** `emit_build_error_inline` is the single routine
   every consumer uses to copy a helper message into an owned `Error` block (3 callers,
   measured above). After the copy, if `message == pending word`, it frees the message
   (`len + 9`), zeroes the word, and rewrites its `message_slot` to the block's inline
   copy (`base + ERROR_OBJECT_SIZE`), so every caller that restores the loose register
   from that slot sees a live pointer. Static messages never equal the word (it holds an
   arena address or 0), so they are never freed.
3. **Raise API + adoption.** `emit_fail_status(symbol, error_name, call, status_reg,
   ins, rel, done)` in `native_fail.rs`. For shared cleanup tails, a per-site form:
   each failing check branches to its own stub. The stub stores the HRESULT and the
   prefix address in two frame slots and branches to the shared tail. The tail runs
   cleanup, then calls the builder and raises. Windows adopts it at all 21 OS-status
   sites.

**Correctness risk** is in piece 2: a wrong free corrupts the arena. It is guarded by
exact pointer equality with a word only piece 1 writes, and tested before any backend
uses it. **Design uncertainty** is in the spare word (verify first) and in whether
every path reaches `emit_build_error_inline`. A path that doesn't only leaves the
block pending until the next status message frees it: one bounded block, never a
corruption. Phase 1 settles both.

**Gate class:** behavior changes. Byte-identity is NOT the gate. Expected diffs: every
`*.windows-x86_64.ncodesum` golden whose program uses `audio::` (in the tree today:
`tests/byte-identity/audio/golden/audio_codegen_cover_rt.windows-x86_64.ncodesum`),
plus every `ncodesum` for any target whose program raises through
`emit_build_error_inline`, because piece 2 adds instructions there. A diff anywhere
else is a bug to root-cause (objdump one fixture), not a design failure.

**Rejected alternatives** (all tried or examined on 2026-09-26):

- *Static detail only* (`emit_fail_message` with a compile-time string): names the call
  but can never show the status, which is the point. Built and reverted.
- *Process-global writable message buffer:* races between threads failing at once.
- *Owned flag in the tag or code register:* every consumer of `RESULT_TAG_REGISTER` /
  the code (`RESULT_ERR_BLOCK_TAG` handling, `TRAP` materialization, worker copy) would
  need auditing, a far larger blast radius than one exact-equality test.
- *Growing `ARENA_STATE_SIZE`:* shifts every arena-state offset and churns every golden
  on every target.
- *Unconditional free after the park:* `_mfb_arena_free` would bin static objects.

## 4. Detailed Design

### 4.1 Prefix data object

`error_status_prefix_symbol(error_name, call) = "_mfb_str_errstatus_<ErrorName>_<hex(call)>"`.
A post-codegen relocation scan in `builder/mod.rs`, next to the bug-545 block, decodes
each referenced symbol into `string_data_object(symbol, format!("{message} ({call}: "))`
(message from `registry::runtime_error`), iterating a `BTreeSet` so the layout is
deterministic. A malformed symbol panics: it can only come from the compiler itself.

### 4.2 `_mfb_rt_error_status_message`

- Inputs: `c_arg(0)` = prefix String pointer, `c_arg(1)` = status.
- Output: the String pointer in `RESULT_ERROR_MESSAGE_REGISTER`.
- Emitted once per module, only when referenced (same gate style as the park:
  referenced symbol → helper function in the plan).
- Body: vregs + `finalize_vreg_body_with_locals`, like `lower_int_to_string_helper`.
  - Free a stale pending word first (`arena_free(ptr, [ptr]+9)`).
  - Then `_mfb_arena_alloc(len+20, 8)`; OOM → static `ErrOutOfMemory` message, word
    left 0.
  - Copy the prefix bytes, write `0x`, 8 nibbles (`+48` / `+87`), `)`, NUL; store len.
  - Store the pointer into the pending word via `ARENA_STATE_REGISTER` (the offset is
    past rv64's 12-bit immediate, so compute the address in a register, as the
    current-error slot accessors do).
- Every value live across the two arena calls is a vreg: the arena helpers clobber
  every integer register.

### 4.3 Copy-point ownership (`emit_build_error_inline`)

After the message copy, add:

- load the pending word;
- compare it with `[message_slot]`; skip if not equal;
- `arena_free([message_slot], [[message_slot]] + 9)`;
- zero the word;
- store `base + ERROR_OBJECT_SIZE` into `message_slot`.

That is valid because the inline copy starts with the same `{len, bytes, NUL}` header
at `messageOffset`. Confirm the offset from `emit_build_error_inline`'s own
`messageOffset@8 (=24)` store when implementing.

### 4.4 Raise API

- `emit_fail_status(symbol, error_name, call, status, ins, rel, done)`: load the prefix
  address into `c_arg(0)`, move `status` into `c_arg(1)`, `bl
  _mfb_rt_error_status_message` (plus internal-call relocation), set the code
  immediate and `RESULT_ERR_TAG` (the message is already in its register), branch to
  `done`. Shape the ordering around the call clobbering x0/x1.
- `StatusFailSites` for shared tails. It is built with `(symbol, error_name,
  status_slot, prefix_slot, tail)`. `site(call)` returns a fresh label. `emit()` lays out
  one stub per site: store the status register into `status_slot`, store the prefix
  address into `prefix_slot`, branch to `tail`. The tail runs cleanup, then
  `emit_fail_status_from_slots`.
- Windows frame slots: offsets 160 and 168 are unused in `gen_windows.rs`'s frame
  table (152 = `COUNT_OFF`, 176 = `DEADLINE_OFF`). Re-verify with `grep -n 'const
  [A-Z_]*_OFF: usize' gen_windows*.rs` before use.

### 4.5 Windows call names

Use the COM method names: `IMMDeviceEnumerator::GetDefaultAudioEndpoint`,
`IMMDeviceEnumerator::GetDevice`, `IMMDevice::Activate`, `IAudioClient::Initialize`,
`IAudioClient::GetMixFormat`, `IAudioClient::GetBufferSize`,
`IAudioClient::GetCurrentPadding`, `IAudioClient::SetEventHandle`,
`IAudioClient::GetService`, `IAudioClient::Start`, `IAudioRenderClient::GetBuffer`,
`IAudioRenderClient::ReleaseBuffer`, `IAudioCaptureClient::GetBuffer`,
`IAudioCaptureClient::ReleaseBuffer`, `CoCreateInstance`, `CreateEventW` (status =
`GetLastError()`). The non-HRESULT open checks (mix rate ≠ requested, mix bits ≠ 32,
mix channels < requested) carry the observed value: `mix rate` shows the device rate,
which is exactly what a developer needs.

## Compatibility / Format Impact

- `e.message` text for Windows audio failures changes (fixed prefix + status suffix).
  `e.code` does not.
- `.mfp` format, `Error` layout, `ARENA_STATE_SIZE`, error-code table: unchanged.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work; `- [~]` for partial; mark moot tasks `~~struck~~ — moot: <evidence>`; fill
> `Commit:` when a phase lands. An unticked box means NOT DONE.

### Phase 1 — Verify the premises (no code change)

- [ ] Confirm no target writes `ARENA_FLAG_RHS_OFFSET + 8`: read `src/arch/riscv64/select.rs`
      (the flag-rhs store/load helpers) and `src/arch/riscv64/v128.rs`; record the width
      of every access in §Verified properties.
- [ ] Confirm every consumer of a runtime-helper error message reaches
      `emit_build_error_inline`: read the three callers plus the `raw_result_discard_error`
      skip (`builder_error_emission.rs`, `emit_error_register_return`) and the worker
      path. List any path that copies a helper message some other way. Each such path
      becomes a Phase 2 task, or is recorded as a bounded pending block.

Acceptance: §Verified properties has no UNVERIFIED entry left for the spare word or the
consumer list.
  Check: `grep -n UNVERIFIED planning/plan-158-A-native-error-status-messages.md` → no
  lines (est. 1 min).
Commit: —

### Phase 2 — Status message primitive (no backend caller yet except a test fixture)

- [ ] `src/codegen/error/constants/error_constants.rs`: add
      `ARENA_PENDING_ERROR_MESSAGE_OFFSET = ARENA_FLAG_RHS_OFFSET + 8` with a doc comment
      citing this plan, and a `const _: () = assert!(…)` that it is below
      `ARENA_CURRENT_ERROR_OFFSET`.
- [ ] `src/codegen/memory/data/data_objects.rs`: `error_status_prefix_symbol`,
      `error_status_prefix_data_object` (decode), unit tests for the round trip and for
      rejecting malformed symbols.
- [ ] `src/codegen/engine/builder/mod.rs`: relocation scan emitting prefix objects
      (§4.1) and gating the helper (§4.2) on any `_mfb_str_errstatus_` reference.
- [ ] New `src/codegen/error/emission/status_message.rs`: `lower_error_status_message_helper`
      (§4.2); register it where `lower_park_error_helper` is registered.
- [ ] `builder_error_emission.rs`: `emit_build_error_inline` ownership block (§4.3).
- [ ] `native_fail.rs`: `emit_fail_status`, `StatusFailSites`, `emit_fail_status_from_slots`.
- [ ] Test: a runtime test that a helper raising through `emit_fail_status` delivers
      `"<msg> (<call>: 0x…)"` in `e.message`, under inline `TRAP`, under propagation to
      a caller's `TRAP`, and uncaught (stderr line). Prefer a real Windows audio call in
      Phase 3 over a synthetic helper; if no host-runnable caller exists before Phase 3,
      fold this test into Phase 3 rather than adding a test-only helper.
- [ ] Test (leak): the same failure in a loop of 10 000 iterations under `--debug`
      arena accounting shows zero net growth (`mfb build --debug`; the arena debug
      counters from plan-130-C).

Acceptance: unit tests pass, and the full byte-identity gate shows diffs only in
fixtures whose programs reach `emit_build_error_inline` (expected, §3).
  Check: `cargo test --bin mfb error_status` → all pass (est. 3 min);
  `scripts/artifact-gate.sh` → every DIFF is on a fixture whose program raises an
  error (inspect one per target with `-ncode` to confirm the delta is the §4.3 block)
  (est. 15 min: the copy point is shared by every raising program, so no scoped subset
  would show an unexpected diff elsewhere).
Commit: —

### Phase 3 — Windows audio adoption + box proof

- [ ] `gen_windows_open.rs` `lower_open`: replace the 12 OS-status branches with
      `StatusFailSites` sites (names §4.5). The `dev_fail` / `unavailable` tails keep
      `emit_open_cleanup` and end in `emit_fail_status_from_slots`.
- [ ] `gen_windows_io.rs` (`lower_write`, `lower_read`, `lower_query`): 5 sites.
- [ ] `gen_windows_devices.rs`: 4 sites.
- [ ] `gen_windows_tests.rs`: an emit-inspection test that `lower_open` / `lower_write`
      contain no `emit_fail("ErrAudioDevice")` tail left and reference the status helper.
- [ ] Regenerate `tests/byte-identity/audio/golden/audio_codegen_cover_rt.windows-x86_64.ncodesum`
      (expected diff). Verify with `-ncode` that the delta is confined to the audio
      helpers and the §4.3 block.
- [ ] Box proof on 192.168.1.206: a probe program under `/tmp` that (a) opens a SHARED
      48000/1 stream, (b) opens and closes a second stream on the same device, and
      (c) writes to the first; and one that writes to an EXCLUSIVE 44100/2 stream with
      a watchdog. Record the exact `e.message` lines in §Corrections of this plan and
      hand them to plan-158-C.

Acceptance: on the box, the probe's failed write prints
`… (IAudioRenderClient::GetBuffer: 0x88890004)`. The spec and man text below are updated.
  Check: `ssh test@192.168.1.206 "cd probe && wp.exe Speakers 48000 1 1 48000"` →
  line contains `: 0x88890004)` (est. 2 min).
Commit: —

## Validation Plan

- Tests: data-object unit tests (Phase 2); runtime message and leak tests (Phase 2/3);
  Windows emit-inspection test (Phase 3).
- Coverage check: the Windows audio helpers are in the gate's denominator through
  `audio_codegen_cover_rt.windows-x86_64.ncodesum`. The runtime path is covered only by
  the box probe; record it.
- Runtime proof: the Phase 3 box probe output, pasted into §Corrections.
- Doc sync:
  - `src/docs/spec/stdlib/11_audio.md` (error messages now carry the call and OS
    status, cite `emit_fail_status`);
  - `mfb spec language errors` / the park section (the pending-owned-message word and
    the §4.3 free; cite `ARENA_PENDING_ERROR_MESSAGE_OFFSET`);
  - `.ai/codegen-invariants.md` (a helper message is owned only when it equals the
    pending word);
  - man: the `audio` package descriptor's `ErrAudioDevice` description says the message
    names the failing call and status. Check with `mfb man audio write`, then
    `scripts/man-census.sh --memory-scope` → 0 unclassified.
- Final gate (once, after Phase 3): `cargo test` plus `scripts/artifact-gate.sh`
  (est. 45 min).

## Open Decisions

- Status format: `0x%08x` for everything (recommended; HRESULT, OSStatus and negative
  errno are all 32-bit and hex is how Microsoft/Apple document them) vs. per-backend
  decimal. plan-158-B inherits the choice.
- Should ALSA use `snd_strerror` text instead of a number? Deferred to plan-158-B §Open
  Decisions; this primitive supports only a number.

## Corrections

## Summary

The engineering risk is the ownership rule at the copy point (§4.3). It is one
exact-pointer-equality test against a word only the new helper writes, and it is
proven in Phase 2 before any backend depends on it. Untouched: error codes, the
`Error` layout, arena-state size, and every non-adopting helper's bytes.
