# plan-47-S: raise the platform seam off POSIX

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-47-P (branches already exhaustive), plan-47-C (one real Windows surface
to validate the raised seam against). Feature-wide precondition: master §Prerequisites.
Produces: intent-level `CodegenPlatform` methods replacing 21 POSIX constant accessors,
and the deletion of C's poison-value wall. **Blocks E and G.**

`CodegenPlatform` requires 21 methods that describe POSIX ABI details — `termios_size()`,
`dirent_name_offset()`, `stat_mode_offset()`, `sol_socket()`, `eagain()` and the rest.
Their *consumers* in shared lowering build the POSIX struct inline: `io_helpers.rs:866`
computes `slots.modified + platform.termios_lflag_offset()` and then calls `tcsetattr`.

Windows has no `termios`, no `dirent`, and no `struct stat` in that shape. **There is no
set of integers a Windows platform can return that makes that consumer correct.** So the
constants cannot be parameterized — the seam has to move up, from "tell me your struct
offsets" to "do the thing".

The single behavioral outcome: no `CodegenPlatform` method describes a POSIX struct
layout or constant, every existing target emits byte-identical code, and 47-C's 21
poison values are gone.

References (read first):

- The master §3.1 — the four categories of POSIX coupling and why this is category 1.
- `src/target/shared/code/io_helpers.rs:866` — the consumer that proves constants are
  insufficient.
- `src/target/shared/code/net/{mod,io,poll}.rs`, `tls/{mod,openssl}.rs` — the socket
  constant consumers.
- `planning/plan-47-C-win32-runtime-floor.md` §Phase 3 — the poison-value wall this
  sub-plan removes.
- `planning/plan-47-P-platform-family-match.md` — lands first; its exhaustive matches are
  what make this sub-plan's edits safe.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-47-P has landed | `rg -n 'enum PlatformFamily' src/` | **NOT MET** |
| plan-47-C has landed (a real Windows surface to validate against) | `ls src/target/win_x86_64/code.rs` | **NOT MET** |
| Byte-identity goldens exist for all four existing targets | `find tests -path '*/golden/*' -name '*.ncode*' \| while read f; do b="${f##*/}"; b="${b%.*}"; echo "${b##*.}"; done \| sort -u` | **NOT MET — `linux-riscv64` has 0** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before continuing and again before deciding to stop. If you stop, report all
> three statuses.

Row 3 is a hard blocker here for the same reason as in 47-P: this sub-plan's acceptance
is byte-identity across all four targets, and a target with no goldens cannot produce a
diff.

## 1. Goal

- The 21 POSIX constant accessors are **gone** from `CodegenPlatform`, replaced by
  intent-level methods each OS implements its own way.
- Shared lowering contains no `termios`, `dirent` or `struct stat` layout knowledge.
- All four existing targets emit **byte-identical** code — this is the whole acceptance
  criterion.
- 47-C's poison-value wall is deleted; no fabricated constant remains in the Windows
  platform.
- Windows implementations are **not** written here — E and G write them. This sub-plan
  leaves `unreachable!("47-E owns this")` arms, exactly as 47-P did.

### Non-goals (explicit constraints)

- **No Windows behavior.** Not one `GetConsoleMode` call. This is a refactor of the seam;
  E and G fill the Windows side.
- **No change to the ~125 hardcoded POSIX symbol literals** (master §3.1 category 2).
  Those are E1/F1/G1's chokepoint work, and mixing them into this diff would make the
  byte-identity signal unattributable.
- **No behavior change anywhere.** If a diff appears, the raise is wrong — do not
  rebaseline.
- **Do not extend this to the app-mode or TLS methods.** They are already defaulted and
  are not POSIX-shaped.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| POSIX constant accessors on the trait | **21** | `awk '/pub\(crate\) trait CodegenPlatform/,0' src/target/shared/code/types.rs \| awk '/^}/{exit} /^    fn /{sub(/^    fn /,""); sub(/[(<].*/,""); print}' \| grep -cE '^(termios_\|dirent_\|stat_\|eagain\|einprogress\|emsgsize\|o_nonblock\|so_\|sol_\|addrinfo_)'` |
| — `termios_*` | 8 | same, `grep -c '^termios_'` |
| — `dirent_*` + `stat_mode_offset` | 3 | same |
| — socket constants | 10 | same |
| Required trait methods before / after | 54 → **54 − 21 + N** | N = the intent-level replacements (§3) |
| Backends to update | 2 impls (`linux_common/code.rs:302`, `macos_aarch64/code.rs:38`) + the Windows one | `rg -n 'impl (code::)?CodegenPlatform for' src/` |

Note the impl count: because Linux is **one** `impl<A: LinuxArch>` shared by three
arches, this sub-plan updates **two** real implementations, not four. That is why it is
medium rather than large.

### 2.2 The consumers, by group

| Group | Consumer | What it builds inline today |
|---|---|---|
| `termios_*` (8) | `io_helpers.rs:866` and the raw-mode enter/exit sequence | a `struct termios` at `slots.modified + offset`, then `tcsetattr` |
| `dirent_*`, `stat_mode_offset` (3) | `fs_helpers_paths.rs:922`, `:1039` and the `emit_path_stat` path | reads `d_name`/`st_mode` at an offset |
| socket constants (10) | `net/mod.rs`, `net/io.rs`, `net/poll.rs`, `tls/mod.rs`, `tls/openssl.rs` | `setsockopt` levels/names, `fcntl` flags, `errno` comparisons |

### 2.3 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| The constants are required (no defaults) | **CONFIRMED** | `termios_size` etc. end in `;` at `types.rs:241-244`; all 21 are in the 54 required |
| Their consumers build POSIX structs inline | **CONFIRMED** | `io_helpers.rs:866` computes an offset into a `termios` and calls `tcsetattr` |
| No integer return makes `io_helpers.rs:866` correct for Windows | **CONFIRMED** | Windows raw mode is `SetConsoleMode(h, DWORD)` — a bitmask on a handle, not a struct write |
| Only 2 real `CodegenPlatform` impls exist | **CONFIRMED** | `linux_common/code.rs:302` (generic over 3 arches), `macos_aarch64/code.rs:38` |
| Winsock defines `SOL_SOCKET`/`SO_*` compatibly | **CONFIRMED (partially)** | it does — but `O_NONBLOCK` is `ioctlsocket(FIONBIO)` and errors are `WSAE*`, so the socket group is *not* uniformly liftable (§3.1) |
| The raise is byte-neutral | **UNVERIFIED — this is the acceptance criterion** | proven by 0-diff goldens per phase |

## 3. Design Overview

Replace each constant *group* with the smallest intent-level method that covers its
consumers.

| Today (21 constants) | Becomes | Why |
|---|---|---|
| 8 × `termios_*` | `emit_set_raw_mode(enable)` + `emit_is_terminal` (exists) | The consumer's whole purpose is enter/exit raw mode. macOS/Linux implement it with `tcgetattr`/`tcsetattr`; Windows with `GetConsoleMode`/`SetConsoleMode`. |
| 3 × `dirent_*`/`stat_*` | `emit_read_dir_entry(...) -> name+len` and `emit_stat_is_dir(...)` | The consumer wants a name and a file kind, not offsets into someone's struct. |
| 10 × socket constants | **Keep 6, lift 4** — see §3.1 | Not uniformly liftable; pretending otherwise is how this sub-plan would go wrong. |

### 3.1 The socket group does not lift cleanly — and that is the finding

`SOL_SOCKET`, `SO_REUSEADDR`, `SO_RCVTIMEO`, `SO_SNDTIMEO`, `SO_ERROR`,
`ADDRINFO_ADDR_OFFSET` **are genuinely portable**: Winsock defines the same names with
compatible semantics, and `getaddrinfo` has the same layout. Those 6 stay as constants —
lifting them would be churn for nothing.

`O_NONBLOCK`, `EAGAIN`, `EINPROGRESS`, `EMSGSIZE` **do not port**:

- `O_NONBLOCK` is an `fcntl` flag; Windows uses `ioctlsocket(s, FIONBIO, &1)` — a
  different *call*, not a different constant.
- `EAGAIN`/`EINPROGRESS`/`EMSGSIZE` are `errno` values; Winsock reports
  `WSAEWOULDBLOCK`/`WSAEINPROGRESS`/`WSAEMSGSIZE` via `WSAGetLastError()`, not `errno`.

So those 4 lift to `emit_set_nonblocking(fd)` and `emit_classify_socket_error(...) ->
{WouldBlock, InProgress, MsgSize, Other}`. **Net: 21 → 6 constants + ~5 intent methods.**

Getting this split right is the sub-plan's actual design work. Lifting all 10 would be
gratuitous churn in shared net lowering; lifting none leaves Windows unimplementable.

**Where design uncertainty concentrates:** the socket split above, and nowhere else. The
termios and dirent groups are unambiguous — their consumers already have a single clear
intent. Settle the socket split by writing the Windows arm of
`emit_classify_socket_error` as a *throwaway sketch* before committing to the signature;
if the sketch needs a seventh constant, the split is wrong.

**Where correctness risk concentrates:** in the two existing implementations. Every
lifted method must reproduce today's emitted bytes exactly on macOS and on all three
Linux arches. The termios sequence is the dangerous one — it is a read-modify-write of a
struct with per-field offsets, and folding it into one method means re-emitting that
sequence from inside the platform impl rather than from shared code.

**Rejected alternative:** *branch in the consumer* (`if platform.has_termios()`).
Rejected: it is the same binary shape 47-P just removed, and it grows a new fork per
surface. It also leaves POSIX struct knowledge in shared code permanently.

**Rejected alternative:** *have Windows return synthetic offsets into a fake struct it
then interprets.* Rejected on sight — it encodes a lie in the platform seam, and the
first person to read `termios_size()` on Windows would reasonably believe it.

**Rejected alternative:** *do this before 47-C.* Rejected: without one real Windows
surface, the raised seam is designed against an imagined implementation. C's poison-value
wall is uncomfortable but short-lived, and it makes the requirements concrete.

## 4. Detailed Design

### 4.1 `emit_set_raw_mode`

The consumer at `io_helpers.rs:866` becomes a single call. The POSIX implementation moves
*verbatim* into `linux_common` and `macos_aarch64` — same instructions, same order, same
`tcgetattr`/`tcsetattr` symbols, just relocated. **That is what makes byte-identity
achievable**: nothing is rewritten, only moved.

Signature carries the slot the current code uses for the saved-state buffer, so the
caller still owns the storage and the platform only decides what to write into it.

### 4.2 `emit_read_dir_entry` / `emit_stat_is_dir`

Same move: `fs_helpers_paths.rs:922`/`:1039`'s `d_namlen`-vs-strlen fork moves into the
two platform impls. The `if platform.family() == Linux` that 47-P created here
disappears entirely — which is the tell that this raise is correct rather than cosmetic.

### 4.3 The socket methods

`emit_set_nonblocking` and `emit_classify_socket_error` per §3.1. The 6 portable
constants stay.

## Compatibility / Format Impact

- **Changed:** `CodegenPlatform` loses 21 required methods and gains ~5; the two real
  impls each move a handful of emission sequences in from shared code. Required-method
  count goes 54 → ~38.
- **Unchanged:** every existing target's emitted bytes; the ~125 POSIX symbol literals
  (E1/F1/G1's work); the language, IR and schemas.

## Phases

One group per phase, byte-identity gate after each, so a regression is attributable.

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — the socket split decision (settles the only uncertainty)

- [ ] Write a throwaway sketch of the Windows arm of `emit_classify_socket_error` and
      `emit_set_nonblocking` against the real Winsock calls.
- [ ] If the sketch needs a constant not in the "keep 6" list, revise §3.1 before
      writing any production code.
- [ ] Record the settled split in §Corrections.

Acceptance: the split is written down with the sketch that justified it. No production
code lands in this phase.
Commit: —

### Phase 2 — the dirent/stat group (3 constants, smallest blast radius)

- [ ] Add `emit_read_dir_entry`, `emit_stat_is_dir`; move the POSIX sequences verbatim
      into the two impls; delete the 3 constants and the 47-P branches at
      `fs_helpers_paths.rs:922`, `:1039`.
- [ ] Delete the corresponding poison values from `win_x86_64/code.rs`; leave
      `unreachable!("47-D owns this")`.

Acceptance: `scripts/artifact-gate.sh` 0 diffs on all four existing targets.
Commit: —

### Phase 3 — the termios group (8 constants, the dangerous one)

- [ ] Add `emit_set_raw_mode`; move the enter/exit sequence verbatim from
      `io_helpers.rs:866` into the two impls; delete the 8 constants.
- [ ] Delete the poison values; leave `unreachable!("47-E owns this")`.

Acceptance: 0 diffs on all four targets. This sequence is a read-modify-write of a struct
with per-field offsets — diff the `.ncode` for a `term::`-using fixture explicitly, not
just the aggregate gate.
Commit: —

### Phase 4 — the socket group (largest blast radius last)

- [ ] Add `emit_set_nonblocking` + `emit_classify_socket_error` per the Phase 1 split;
      lift the 4 non-portable constants; **keep the 6 portable ones**.
- [ ] Update `net/mod.rs`, `net/io.rs`, `net/poll.rs`, `tls/mod.rs`, `tls/openssl.rs`.
- [ ] Delete the poison values; leave `unreachable!("47-G owns this")`.

Acceptance: 0 diffs on all four targets, plus an explicit `.ncode` diff for a `net::`-
and a `tls::`-using fixture. Confirm no poison constant remains anywhere in
`win_x86_64/`.
Commit: —

## Validation Plan

- Tests: none new. This is a pure refactor; its correctness *is* byte-identity, and a
  unit test asserting "the method returns what the constant used to" would test the
  refactor against itself.
- Coverage check: **decisive here.** `linux-riscv64` has zero native goldens, so 0-diff
  is vacuous for it — and riscv64 compiles every line this sub-plan moves. Seed the
  goldens first.
- Runtime proof: none applicable; behavior is unchanged by construction, on every
  platform that has an implementation. Windows has none yet by design.
- Doc sync: `src/docs/spec/architecture/06_native.md` describes the `CodegenPlatform`
  seam and names it "implemented once per OS" — update the method inventory if it
  enumerates.
- Acceptance: the full suite plus `scripts/artifact-gate.sh` 0 diffs after **each**
  phase.

## Open Decisions

1. **The socket split** (§3.1) — keep 6 / lift 4, recommended. Settle it in Phase 1 with
   a sketch rather than by argument; the failure mode is discovering a seventh needed
   constant after the signature is committed.
2. **Whether `emit_set_raw_mode` takes the saved-state slot or owns the storage.**
   Recommended: caller owns the slot, matching today's shape — it keeps the move verbatim
   and therefore byte-identical, which is the whole acceptance criterion.
3. **Whether to fold `emit_is_terminal` into the raw-mode method.** Recommended no: it
   already exists, already works on all platforms, and merging it would churn a working
   method for symmetry.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The socket constants do not lift uniformly.** An earlier framing of this
  sub-plan (master §3.1, option (a)) treated all 21 constants as one group to raise.
  Measured: 6 of the 10 socket constants are genuinely portable to Winsock and lifting
  them would be churn; the other 4 are not constants at all on Windows
  (`O_NONBLOCK` → `ioctlsocket(FIONBIO)`, the three `E*` → `WSAGetLastError`). The split
  is now §3.1 and is Phase 1's job to confirm.
- 2026-07-20 — **This updates 2 implementations, not 4.** Linux is one
  `impl<A: LinuxArch>` shared across three arches (`linux_common/code.rs:302`), so the
  per-impl cost is half what the target count suggests.

## Summary

The engineering risk is that a "pure refactor" is not byte-neutral. Every mitigation here
points the same way: move sequences **verbatim** rather than rewriting them, one group per
phase, gate after each, and diff the specific fixture that exercises the group rather than
trusting the aggregate.

The design work is smaller than it looks but is concentrated in one place: the socket
group splits 6-keep/4-lift, and getting that wrong means either gratuitous churn in shared
net lowering or a Windows surface that still cannot be written. Phase 1 settles it with a
sketch before any production code.

What is left untouched: the ~125 POSIX symbol literals (E1/F1/G1 own those), every
existing target's emitted bytes, and the Windows implementations themselves — which E and
G write against the seam this sub-plan leaves them.
