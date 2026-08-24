# bug-452: native-interop backends read the C-call result from the MFB-return bank (not the C-return bank) on x86-64 / Win64 — tls/OpenSSL, audio/ALSA, audio/WASAPI

Last updated: 2026-08-23
Effort: large (3h–1d — ~100 result-read sites across 3 backends + goldens + box verification)
Severity: HIGH (TLS — a security library — returns garbage / crashes on linux-x86_64; audio capture/playback broken on linux-x86_64 and win-x86_64; all uncatchable at the language level)
Class: Correctness / Security (valid programs silently misbehave or crash on supported platforms)

Status: Open
Regression Test: to be added (codegen-inspection per backend, mirroring
`tests/codegen_crypto_ec_c_return_x86_64.rs`; plus a box run where a runtime
environment exists — see Validation Plan)

The `tls`, `audio` (ALSA), and `audio` (WASAPI) built-in backends generate their
platform-library call sequences by hand: they load a resolved function pointer
(dlsym'd libssl/libcrypto/libasound symbol, or a COM vtable slot) and call it via
a raw indirect `blr` (`abi::branch_link_register`), then read the call's return
value from `abi::return_register()` — the aligned **MFB**-return bank. On the
x86-64 ABIs that bank is **not** where a C function returns:

- x86-64 SysV: `return_register()` = `mfb_return(0)` = **`rdi`**; a C function
  returns in **`rax`** (`c_return(0)`).
- Win64: `return_register()` = **`rcx`**; a C function returns in **`rax`**.

So every "result" these backends read back is actually the **stale first
argument** (or the `this` pointer, for COM), not the return value. On AArch64 and
RISC-V the argument and result banks are the same register (`x0`/`a0`), which is
why the macOS-aarch64 runtime — the only place these paths are *executed* in the
automated suite — works, and the x86-64/Win64 paths (compile-only in the suite)
are silently broken. **The single correct behavior a fix produces:** every
external C-call result is read from `abi::c_return(0)`.

This is the exact bug class already fixed for `crypto::generate/sign/verify` in
**[[bug-450]]** and already documented as a rule in `.ai/arch-abi.md`
("x86 native-call return uses c_return, not the aligned bank"). These three
backends predate/violate that rule and were never caught because their x86-64
execution is never exercised (byte-identity goldens verify drift, not semantics;
the `.run` fixtures for tls run only on the macOS host via the Security.framework
backend, and audio opens devices absent in the test env).

<!-- When the fix fully lands, add:
       ## STATUS: FIXED (<commit hash>)
     then archive this file to bugs/completed/. -->

References:

- `.ai/arch-abi.md` — §"x86-64 (SysV) → x86 native-call return uses c_return, not
  the aligned bank" (the rule these backends violate; states the SysV/Win64 bank
  split and the "works on macOS aarch64, broken on both x86-64 ABIs" trap).
- `.ai/net-tls.md`, `.ai/arch-abi.md` (Windows PE/console/audio) — backend docs.
- bugs/completed/bug-450-crypto-generate-nist-curve-segfaults-linux-x86_64.md —
  the sibling instance, fixed and box-proven; found this class.
- Found during the bug-450 latent-audit follow-up (2026-08-23).

## Failing Reproduction

No runtime environment was available to execute these paths (the reachable
x86-64 boxes 2227/2228 have no outbound network for a TLS handshake and no audio
hardware; no Windows run was performed). The defect is proven **statically** from
the generated code — the same oracle that localized bug-450: after an external
`blr`, the result must be consumed from `rax`, never `rdi` (SysV) / `rcx` (Win64).

TLS (linux-x86_64):

```
target/release/mfb build -q -ncode -target linux-x86_64 tests/byte-identity/tls
python3 - <<'PY'
import json,glob
d=json.load(open(glob.glob('tests/byte-identity/tls/*.ncode')[0]))
for fn in d['functions']:
    ins=fn.get('instructions',[])
    bad=sum(1 for i,x in enumerate(ins) if x.get('op')=='blr'
            and (ins[i+1].get('src') or ins[i+1].get('lhs'))=='rdi')
    if bad: print(fn['symbol'], 'result-reads-from-rdi=', bad)
PY
```

- Observed (linux-x86_64): `_mfb_rt_tls_tls_connect` reads its result from `rdi`
  (1 `str_u64` + 7 `cmp_imm`); `tls_listen` 1+5; `tls_accept` 3; `tls_read`,
  `tls_write`, `tls_readText`, `tls_writeText`, `tls_poll` each `sxtw rdi`
  (sign-extend the wrong register). 0 reads from `rax`.
- Observed (linux-x86_64, audio ALSA): every `_mfb_rt_audio_*` reads from `rdi`
  (`openInput`/`openOutput`/`openInputDevice`/`openOutputDevice` 18 each,
  `devices` 4, `readTimeout` 6, `read`/`write`/`close*` 2 each,
  `available`/`poll`/`pollTimeout` 1 each). 0 from `rax`.
- Observed (win-x86_64, audio WASAPI): `com_call`
  (`src/codegen/builtins/audio/gen_windows.rs`) does
  `sign_extend_word(return_register(), return_register())` after the `blr` —
  sign-extends `rcx` (the stale `this` pointer) as if it were the HRESULT (`rax`).
- Expected: every result read comes from `abi::c_return(0)` (`rax`), so
  `tls::connect` returns the real `SSL_CTX*`/`SSL*`, `audio::openInput` returns
  the real `snd_pcm_t*`, and the WASAPI HRESULT check tests the real status.

Contrast cases that are correct today (bound the bug):

- macOS aarch64: the same source runs correctly — `return_register()` and
  `c_return(0)` are both `x0` (Security.framework/CoreAudio backends execute in
  the acceptance suite and pass).
- `crypto::generate/sign/verify` on linux-x86_64: fixed in bug-450, box-proven on
  2227 (musl) and 2228 (glibc).
- Internal (MFB-convention) indirect calls are unaffected — the thread worker
  trampoline (`runtime_helpers.rs:1073` calls the worker closure), HOF/closure
  invocation (`collections/gen_flow.rs`, `builder_emit_helpers.rs`) — their result
  legitimately lives in the MFB result bank.
- The LINK/FFI thunk (`link_thunk.rs`) already stages `c_return` (5 uses).

| Environment | arch / ABI | Backend exercised | Result |
| --- | --- | --- | --- |
| macOS | aarch64 | Security.framework / CoreAudio | works ✓ |
| Linux | x86-64 SysV | tls/OpenSSL, audio/ALSA | broken ✗ (garbage/crash, unverified at runtime — no net/audio) |
| Linux | aarch64 | tls/OpenSSL, audio/ALSA | works ✓ (banks coincide) |
| Windows | x86-64 Win64 | audio/WASAPI | broken ✗ (unverified at runtime — no Windows run) |

## Root Cause

Identical mechanism to bug-450. Each backend emits, per external call:

```
load return_register() <- <arg0 slot>     ; arg0  (rdi on SysV — correct: mfb_return(0)==c_arg(0)==rdi)
load <v> <- <fnptr slot>
blr <v>                                     ; call the C function; result -> rax (SysV) / rax (Win64)
store return_register() -> <result slot>    ; reads rdi/rcx == the STALE arg, NOT rax   <-- BUG
```

`abi::return_register()` == `abi::mfb_return(0)`, which `realize_abi_operand`
(`src/arch/x86_64/select.rs`) maps to the aligned call-argument bank
(`rdi` SysV / `rcx` Win64). The C ABI result is `abi::c_return(0)` == `rax`.
Nothing stages `rax` into the aligned bank after a raw `blr` (only the direct-`bl`
path, `emit_linux_c_call`, and `link_thunk` do). Arg *passing* via
`return_register()` works by coincidence (the aligned MFB and C arg banks are the
same register); only the result *read* is wrong. AArch64/RISC-V are immune
because their arg and result banks are one register.

Cited sites (result read via `return_register()` immediately after an external
`blr`):

- `src/codegen/builtins/tls/gen_openssl.rs` — `return_register()`×151,
  `c_return`×0; e.g. `:445` `blr` then `:446` `store return_register() -> CTX`.
- `src/codegen/builtins/audio/gen_alsa_io.rs` — `return_register()`×53, `c_return`×0.
- `src/codegen/builtins/audio/gen_alsa_shared.rs` — `return_register()`×19, `c_return`×0.
- `src/codegen/builtins/audio/gen_windows.rs` — `com_call` `:261` `blr` then
  `:262` `sign_extend_word(return_register(), return_register())`.

## Goal

- On linux-x86_64 and win-x86_64, `tls::*` and `audio::*` read every
  external-library call result from `abi::c_return(0)`; the generated code has
  **zero** external-`blr`-then-read-`rdi`/`rcx`-result sites (ncode oracle).
- Byte-identical on linux-aarch64 / macos-aarch64 (both banks are `x0`), so only
  the x86-64 / win-x86_64 goldens shift.

### Non-goals (must NOT change)

- Arg *setup* via `return_register()` before a call (correct on all ABIs — it is
  the aligned arg bank; do not churn it).
- The macOS backends (`tls/gen_macos/*`, `macos_aarch64/tls.rs`,
  CoreAudio) — aarch64-only, correct. (They *would* break on a hypothetical
  macos-x86_64 target, but none exists; leave them.)
- Any semantic/wire behavior, resource lifetimes, DER/encoding, or the internal
  MFB-convention indirect calls (thread worker, closures) — untouched.
- Do NOT "fix" by weakening a byte-identity golden or a test; regenerate goldens
  only after the code fix, proving the delta is the intended register change.

## Blast Radius

Found by grepping every `branch_link_register` site and classifying each by
whether its `blr` targets an external C function AND its result is read from the
aligned bank (measured via `-ncode` post-`blr` reads):

- `tls/gen_openssl.rs` — **in scope** (linux-x86_64; ~40 result-read sites).
- `audio/gen_alsa_io.rs`, `audio/gen_alsa_shared.rs` — **in scope** (linux-x86_64).
- `audio/gen_windows.rs` `com_call` — **in scope** (win-x86_64; every COM method).
- `crypto/*` — already fixed (bug-450).
- `thread/runtime_helpers.rs:1073` — unaffected (calls the MFB worker closure;
  result is the MFB result bank, correctly read).
- `collections/gen_flow.rs:117`, `builder/builder_emit_helpers.rs:301` —
  unaffected (internal closure/function-value calls; MFB convention).
- `link/thunk/link_thunk.rs:1185,1622` — unaffected (already reads `c_return`).
- `tls/gen_macos/*`, `macos_aarch64/tls.rs` — unaffected (aarch64 only; banks
  coincide). Latent-but-safe: would break if a macos-x86_64 target is ever added.

## Fix Design

Per backend, at each site that reads an external C-call result, replace
`abi::return_register()` with `abi::c_return(0)` (leaving arg-setup uses alone) —
exactly the bug-450 change. `c_return(0)` renders `x0` on aarch64/riscv
(byte-identical) and `rax` on both x86-64 ABIs (correct). Verify per file with the
ncode oracle (zero post-`blr` reads of `rdi`/`rcx` as a result) before/after,
and add a codegen-inspection test per backend modeled on
`tests/codegen_crypto_ec_c_return_x86_64.rs`.

Rejected: a systemic "stage `mov mfb_return, c_return` after every external `blr`"
hook — there is no choke point (calls are hand-emitted inline) and selection
cannot distinguish an external `blr` from an internal MFB-convention `blr`, so it
would corrupt the thread/closure result paths. The per-site `c_return` read is the
sanctioned pattern in `.ai/arch-abi.md`.

Expected output shift: the linux-x86_64 (tls, audio) and win-x86_64 (audio)
`.ncode`/`.ncodesum` goldens for the affected fixtures change; aarch64/macos are
byte-identical.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a codegen-inspection test per backend (tls, audio-alsa on linux-x86_64;
      audio-wasapi on win-x86_64) asserting no external `blr` result is read from
      the aligned bank; confirm RED against current codegen.
- [ ] Confirm the Blast Radius verdicts above still hold at HEAD.

Acceptance: the new tests fail for the documented reason; audit complete.
Commit: —

### Phase 2 — the fix

- [ ] `tls/gen_openssl.rs`: result reads → `c_return(0)`.
- [ ] `audio/gen_alsa_io.rs` + `gen_alsa_shared.rs`: result reads → `c_return(0)`.
- [ ] `audio/gen_windows.rs` `com_call`: read HRESULT from `c_return(0)`.

Acceptance: Phase-1 tests pass; aarch64/macos ncode unchanged; Non-goals intact.
Commit: —

### Phase 3 — regenerate goldens + full validation

- [ ] Regenerate the shifted linux-x86_64 / win-x86_64 goldens
      (`byte-identity/{tls,audio}`, any tls/audio rt fixtures); confirm the delta
      is only the register change and aarch64/macos sums are unchanged.
- [ ] Full `cargo test` + `artifact-gate.sh all`.
- [ ] Box runtime proof where possible (see Validation Plan).

Acceptance: full suite green; deltas exactly the intended change.
Commit: —

## Validation Plan

- Regression test(s): per-backend codegen-inspection tests (RED→GREEN), as bug-450.
- Runtime proof: **needs an environment not available in this session** —
  a TLS handshake on an x86-64 Linux box *with outbound network* (build
  `tests/rt-behavior/tls/tls-connect-*` for linux-x86_64, run on 2227/2228 with
  network), audio capture/playback on an x86-64 Linux box with ALSA hardware, and
  a WASAPI capture/playback run on box 2230 (Win11). Until then the static ncode
  oracle + the bug-450 end-to-end proof of the identical mechanism stand in.
- Doc sync: none expected (the rule is already in `.ai/arch-abi.md`; optionally
  add these backends to its "known-fixed" list once landed).
- Full suite: `cargo test` and `scripts/artifact-gate.sh <mfb> all`.

## Open Decisions

- Runtime verification depth — (recommended) fix ncode-verified now and add the
  three box runtime proofs when a networked x86-64 box / audio HW / Win box run is
  available, vs. block the fix on full runtime verification. Given the identical
  mechanism is already box-proven via bug-450, ncode-verified is a sound gate.

## Summary

The engineering risk is entirely in the mechanical breadth (~100 result-read
sites across three hand-written backends) and in *not* touching the correct
arg-setup uses of `return_register()`. The mechanism is settled (bug-450, and the
`.ai/arch-abi.md` rule); the fix is the same one-token-per-site change, verified by
the ncode oracle. Nothing about wire formats, resource handling, the macOS
backends, or internal MFB-convention calls changes.
