# bug-454: `os.resourcePath` (and its exe-path acquisition) is unimplemented on windows-x86_64 — valid cross-builds are rejected

Last updated: 2026-09-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (portable API missing on one target)

Status: Fixed
Regression Test: `tests/codegen_win64_resource_path.rs` (three cases: the
windows-x86_64 cross-build, the acquisition-frame addressing invariant, and the
`\`-vs-`/` separator split), plus the positive pin
`tests/rt-behavior/os/func_os_resourcePath_reads_resource` (reads a real resource
through the returned path) and the new
`tests/byte-identity/os/golden/os_codegen_cover_rt.windows-x86_64.ncodesum`.
Runtime proof on box 2230 recorded under "Resolution" below.

A project using `os::resourcePath` builds for macOS and Linux but is rejected
when cross-compiled to `windows-x86_64`: the capabilities gate reports the
runtime call as unsupported, so `examples/tls-server` (and any user of the
API) cannot target Windows at all. plan-55-B implemented the call on
macOS (`src/target/macos_aarch64/plan.rs:266`, exe-path acquisition) and Linux
(`src/target/linux_common/plan.rs:223`, `readlink("/proc/self/exe")`), and the
Win64 twin was never added. **The single correct behavior a fix produces:
`os.resourcePath` (and `os.executablePath`, which shares the acquisition)
lowers on windows-x86_64 via `GetModuleFileNameW`, and the cross-build
succeeds with the same semantics as the other targets.**

References:

- plan-55-B (the `os.executablePath`/`os.resourcePath` design and its per-OS
  acquisition table).
- Memory note `adding-a-call-to-an-existing-native-pkg.md` — the per-target
  `SUPPORTED_RUNTIME_CALLS` gate this trips.
- Found during the optimizer worktree's all-targets examples verification
  (2026-08-24).

## Failing Reproduction

> **Repro updated 2026-08-31.** The original command named
> `examples/tls-server`, which no longer exists — it was replaced by
> `examples/network-server` (`cb44b95b8`), and that example does not call
> `os::resourcePath`, so the old command now *passes* for the wrong reason.
> Use the self-contained project below instead.

```
mkdir -p /tmp/r454/src /tmp/r454/resources && : > /tmp/r454/resources/data.txt
cat > /tmp/r454/project.json <<'EOF'
{ "name": "r454", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [ { "root": "src", "role": "main", "include": ["**/*.mfb"] } ],
  "entry": "main", "targets": ["native"] }
EOF
cat > /tmp/r454/src/main.mfb <<'EOF'
IMPORT os
IMPORT io

FUNC main AS Integer
  io::print(os::resourcePath("data.txt"))
  RETURN 0
END FUNC
EOF
target/release/mfb build --target windows-x86_64 -q /tmp/r454
```

- Observed: `error: native backend does not support runtime call
  'os.resourcePath'`, exit 1.
- Expected: a windows-x86_64 executable, as produced for
  macos-aarch64 / linux-x86_64 / linux-aarch64 / linux-riscv64.

**Re-verified 2026-08-31** at `ba1c1750b` with a freshly built
`target/release/mfb`: the project above builds for `native` (macos-aarch64) and
`linux-x86_64` and fails for `windows-x86_64` with exactly the error above.

**Partially fixed since this was filed — scope is now narrower.** The sibling
call `os.executablePath`, which this document treats as sharing the acquisition,
**has** landed on Windows: it is in the win64 supported list
(`src/target/win_x86_64/mod.rs:55`) and has a lowering
(`src/target/win_x86_64/plan.rs:229`). Only `os.resourcePath` is missing —
`grep -n 'os.resourcePath' src/target/win_x86_64/mod.rs` returns nothing, while
`src/target/linux_common/mod.rs:98` and `src/target/macos_aarch64/mod.rs:83`
both list it.

**Correction, same day — that last sentence was wrong, and the scope is larger.**
An earlier revision of this note said the fix "routes `os.resourcePath` to the
exe-path acquisition Windows already has". Reading the code rather than the
import table: Windows *does* acquire an executable path, but in a shape
`resourcePath` cannot consume, and two further things are missing.

1. **The acquisition is a whole-function lowering, not a reusable fragment.**
   `resourcePath` gets its path from `emit_executable_path_into`
   (`src/codegen/builtins/os/gen_paths.rs:19`), a raw-**byte**-buffer helper
   whose `PlatformFamily::Windows` arm deliberately returns `Err`:

   > Windows acquires its executable path through the UTF-16 wide-string helper
   > (`lower_os_wide_string_windows`), not this raw-buffer routine, so
   > `lower_executable_path` early-returns for Windows before reaching here.

   `lower_os_wide_string_windows` returns `(instructions, relocations,
   stack_size)` for an entire function producing a finished `String`
   (`func_executable_path.rs:28-30`). It cannot be spliced into the middle of
   `lower_resource_path`, which needs the raw bytes *before* its strip/suffix
   arithmetic. So the work is a new Windows arm in `emit_executable_path_into`
   (`GetModuleFileNameW` → `WideCharToMultiByte` into a frame byte buffer,
   returning `(buf, Some(count))` as the Linux arm does) — genuinely new code,
   not a routing change.

2. **The path-separator arithmetic is POSIX-only, and nothing in this document
   mentions it.** `lower_resource_path` hardcodes `/` in both places it inspects
   the path: the `.`/`..` component validation
   (`func_resource_path.rs:77`, `compare_immediate(&scan_byte, "47")`) and the
   backward scan for the `strip`-th separator
   (`func_resource_path.rs:166`, same constant). `GetModuleFileNameW` returns
   `C:\path\to\app.exe`. A backward scan for `/` in that string finds **no**
   separator, so the strip loop does not merely mis-count — it never terminates
   on a match and the base-path computation is wrong however good the
   acquisition is. Both sites need to accept `\` (92) on Windows.

3. **The `resource_base_offset` table must be checked for Windows.** It maps
   build mode to `(components-to-strip, suffix)` and is documented as needing to
   stay "in lockstep with plan-55-A's `resource_output_dir`". Whether the
   Windows output layout has the same depth as the POSIX one is unverified here;
   if it differs, the table needs a Windows row, and a wrong row is silent — it
   yields a plausible path that does not exist.

**Corrections, 2026-09-05 (while fixing).** Two of the three numbered claims
above are wrong, and the third is right but understated.

- **Item 1 is half wrong. The acquisition IS a reusable fragment**, and writing
  a second `GetModuleFileNameW` call site would have been duplication. The
  whole-function wrapper is `lower_os_wide_string_windows`
  (`os/gen_introspect.rs`), but what it wraps is
  `CodegenPlatform::emit_os_wide_string` (`engine/types/types.rs`; Win64 impl in
  `win_x86_64/code.rs`) — a *fragment* that runs
  `GetModuleFileNameW(NULL, wide, 2048)`, marshals UTF-16→UTF-8 into an arena
  buffer and leaves a NUL-terminated C-string pointer in `return_register()`, 0
  on failure. That is byte-for-byte the shape the **macOS** arm of
  `emit_executable_path_into` returns (`(buf, None)`), so the Windows arm is
  eight instructions that call the existing fragment, not a new Win32 call —
  and `(buf, Some(count))` (the Linux shape the item recommends) would have been
  wrong.
- **Item 3 is wrong: the Windows row was already there.**
  `resource_base_offset` (`os/gen_paths.rs`) has mapped
  `NativeBuildMode::WindowsApp` onto the `Console` arm — `(1, "")` — since
  plan-66-I/J, with a comment saying why (a Windows app `.exe` sits in `build/`
  beside its resources; there is no bundle). What was missing was only a *test*:
  `base_offset_per_build_mode` asserted the other three modes and never named
  `WindowsApp`, which is how this document could believe the row was absent. The
  assertion is now there, and the row is also proven at runtime — a
  `--app --target windows-x86_64` build printed
  `path=C:\mfb454app/greeting.txt` and read the file back.
- **Item 2 is right, and was the actual runtime symptom.** It is also
  understated: the `.`/`..` *validation* site needs `\` too, for a different and
  security-relevant reason (see "Resolution").

Consequence for planning: the `Effort: medium (1h–2h)` header predates all of
this and is likely optimistic. Treat the acquisition, the separator handling and
the base-offset table as three separate pieces, each with its own fixture.
Verification also needs a real Windows host or the PE rig — a green cross-build
proves only that codegen emitted something ([[windows-box-has-no-test-script]]
in the operator notes; no test on the Windows box runs a binary).

Expect this to drift the ~24 windows `.ncodesum` goldens, per the standing note
that a `win_x86_64` code change ripples every io-importer; re-sync them all with
`regen-ncodesum.sh` rather than by hand.

| Environment | Details | Result |
| --- | --- | --- |
| windows-x86_64 | any project calling `os::resourcePath` | fails ✗ |
| macos-aarch64 | same source (plan-55-B lowering) | works ✓ |
| linux-* | same source (`/proc/self/exe` lowering) | works ✓ |

## Root Cause

The Win64 target never received plan-55-B: `src/target/win_x86_64` has no arm
for `os.resourcePath`/exe-path acquisition in its plan lowering, so the call
never enters its supported-runtime-calls set and
`src/target/shared/validate/capabilities.rs` rejects the build up front (the
gate is doing its job; the lowering is what is missing). The macOS and Linux
implementations live at `src/target/macos_aarch64/plan.rs:266` and
`src/target/linux_common/plan.rs:223` respectively; the Windows equivalent of
their acquisition step is `GetModuleFileNameW` (kernel32), with the
resource-directory derivation shared with the other targets.

## Goal

- `mfb build --target windows-x86_64 /tmp/r454` (the Failing Reproduction
  project) succeeds, and
  `os::resourcePath` on a Windows host returns the executable-adjacent
  resource path with the same joining semantics as macOS/Linux.

### Non-goals (must NOT change)

- macOS/Linux lowerings and their goldens — byte-untouched.
- The capabilities gate itself — it must keep rejecting genuinely-unsupported
  calls; do NOT "fix" this by whitelisting the call without a lowering.
- No UTF-16 shortcuts: `GetModuleFileNameW` returns UTF-16 — conversion must
  go through the runtime's existing UTF-16→UTF-8 path (see the Win console
  handling in `.ai/arch-abi.md`), not a lossy byte cast.

## Blast Radius

- `src/target/win_x86_64` plan lowering — fixed by this bug
  (`os.resourcePath` + `os.executablePath` twin arm).
- Other `os.*` calls on Win64 — audit in Phase 1: diff the macOS/Linux
  supported sets against Win64's and list any additional missing calls; each
  becomes either in-scope here (same acquisition) or its own bug.
- The capabilities gate (`validate/capabilities.rs`) — unaffected (data-driven
  by the per-target sets).

## Fix Design

Add the plan-55-B arm to the Win64 plan lowering: acquire the module path via
`GetModuleFileNameW` (growing buffer loop per the API contract), convert
UTF-16→UTF-8 through the runtime's existing helper, then reuse the shared
resource-path derivation. Register the call(s) in the target's supported set.
Risk concentrates in the wide-string conversion and long-path (`\\?\`)
handling; PE import bookkeeping for kernel32 already exists.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Fixture cross-built to windows-x86_64 in a test
      asserting today's rejection message.
      (`tests/codegen_win64_resource_path.rs`; all three cases RED at
      `c88230dfe` with `native backend does not support runtime call
      'os.resourcePath'`.)
- [x] Audit: diff Win64's supported `os.*` set against macOS/Linux; verdict
      per missing call. **`os.resourcePath` was the only one** — see the audit
      table under "Resolution".

Acceptance: test fails with the documented error; audit table filled in.
Commit: bug-454 (this change)

### Phase 2 — the fix

- [x] Win64 plan arm for `os.executablePath`/`os.resourcePath`
      (`GetModuleFileNameW` + UTF-16→UTF-8 + shared derivation); add to the
      supported set. Implemented as a Windows arm of the SHARED
      `emit_executable_path_into` reusing `emit_os_wide_string` —
      `os.executablePath`'s own Windows lowering is untouched, so its Windows
      bytes are unchanged.
- [x] Separator arithmetic (item 2), both sites, Windows-only.

Acceptance: the cross-build succeeds; the Phase 1 test flips to asserting an
artifact.
Commit: bug-454 (this change)

### Phase 3 — regenerate expected outputs + full validation

- [x] `artifact-gate.sh <mfb> all` — **1933 golden(s), 0 diff(s)** (baseline for
      this tree: 1930/0). The `win64-change-ripples-all-io-importers` ripple did
      NOT happen and was not supposed to: nothing in `win_x86_64/code.rs`
      changed, only the capability list and one import-table arm. All 144
      `.ncodesum` goldens were regenerated (`regen-ncodesum.sh`, under bash) and
      **not one existing sum moved** — which is also the byte-untouched proof for
      macOS/Linux, since `tests/byte-identity/os` exercises `os::resourcePath` on
      all four POSIX targets.
- [x] `cargo test --release --no-fail-fast`; full `test-accept.sh`
      (**1413 ran**, baseline 1412 + the new positive-pin fixture).
- [x] Runtime proof on box 2230 — literal output under "Resolution", including a
      negative control.

Acceptance: suite green; golden delta is exactly the windows-x86_64
resourcePath users; Windows-host run correct.
Commit: bug-454 (this change)

## Validation Plan

- Regression test: the Phase 1 cross-build test (fails today → asserts
  success + artifact after).
- Runtime proof: Windows-host execution printing the path.
- Doc sync: `mfb man os resourcePath` platform notes if they enumerate
  targets; `.ai/arch-abi.md` Windows section.
- Full suite: `cargo test --no-fail-fast`, `artifact-gate.sh all`,
  `test-accept.sh`.

## Open Decisions

- Buffer strategy for `GetModuleFileNameW` (fixed MAX_PATH vs. grow-on-
  truncation). Recommended: grow-on-truncation loop — long paths are real on
  modern Windows.

## Resolution (2026-09-05)

### Audit — Win64's runtime-call set vs macOS/Linux

Parsed all three `RUNTIME_CALLS`/`runtime_calls` arrays and set-differenced them.

| set | count | verdict |
| --- | --- | --- |
| `macos-aarch64` | 241 | — |
| `linux-*` (shared `linux_common`) | 241 | identical to macOS (`mac △ lin = ∅`) |
| `windows-x86_64` (before) | 250 | `(mac ∪ lin) − win = {os.resourcePath}` — **one** gap |
| `windows-x86_64` (after) | 251 | `(mac ∪ lin) − win = ∅` |

So there was exactly one missing call and it is this bug; nothing spun out. The
ten `win`-only entries (`thread.emit`, `process.receiveFrom`, …) are code-layer
alias spellings the other backends never see as a `RuntimeCall` name.

**Consequence for `tests/rt_trapped_call_capability_gate.rs`.** That test used
`os.resourcePath` on `windows-x86_64` as its "genuinely unsupported call"
vehicle, and its own header says to re-point it when the vehicle gains an
implementation. There is nothing left to re-point at — a registry sweep over
every member that is neither `is_native_direct_call` nor `helper_for_call`-less
found six candidates (`audio.close`, `net.parseQuery`, `net.percentDecode`,
`net.toUrl`, `thread.accept`, `thread.transfer`) and **all six build for
`windows-x86_64` today**; they never reach the gate under those names. So the
rejection case moved to
`validate::tests::a_trapped_runtime_call_is_capability_checked_like_a_bare_one`,
which builds the capability set by hand and cannot go vacuous. The two cases that
still need a real build stayed in the integration test.

### The fix, in four pieces

1. **Acquisition.** `emit_executable_path_into` gained a
   `PlatformFamily::Windows` arm that calls the existing
   `platform.emit_os_wide_string("executablePath", …)` fragment and returns
   `(buf, None)` — the macOS shape (NUL-terminated, no count), so the caller's
   existing NUL scan applies unchanged. No second `GetModuleFileNameW` site.
2. **Separators.** The backward scan over the OS-produced executable path
   compares against `92` on Windows and `47` elsewhere — the same split
   `fs::isWithin`'s `within_sep` makes for `realpath`-produced bytes. The
   `.`/`..` component validation, which walks the CALLER's relative argument,
   accepts **both** `47` and `92` on Windows: `\` is a directory separator to
   every Win32 API, so `..\secret` navigates out of the base exactly as
   `../secret` does, and refusing it rejects strictly more traversal and nothing
   valid (a Windows filename cannot contain `\`).
3. **Join byte — deliberately NOT platform-dependent.** The base and `relative`
   are joined with `/` on every target, matching `fs::pathJoin`'s unconditional
   `SEP = 47` (`fs/gen_path_builder.rs`), which is `/` on Windows too. Portable
   MFB path strings are `/`-delimited everywhere; only OS-produced bytes carry
   the native separator. Win32 accepts `/` in every path it parses. The payoff is
   that `strings::endsWith(p, "/song.ogg")` holds on every target, which is what
   the fixtures assert.
4. **Registration.** `os.resourcePath` added to `win_x86_64`'s `RUNTIME_CALLS`
   and folded into `os.executablePath`'s import arm (`GetModuleFileNameW`,
   `WideCharToMultiByte`).

Also fixed in passing: `os::resourcePath` and `os::executablePath` both declared
`errors: vec![]` while raising `ErrUnsupported`/`ErrInvalidPath`, so their
rendered man pages had no Errors section. `raise_error_into` runs no declaration
check and the static `every_raise_error_site_is_declared_in_its_descriptor` scan
only reads literal `raise_error("id", "Err…")` sites, so nothing caught it. **It
is not a miscompile** — verified: an inline `os::resourcePath(x) TRAP(e)` catches
`77030002` correctly, because `inline_builtin_is_infallible` keys on
`native_bare_target`, which is `None` for an `abi_function` member.

### The frame hazard, and why no code was added for it

`emit_os_wide_string` brackets its body with `subtract_stack(0x60)` …
`add_stack(0x60)`, x86-64 spill slots are `[rsp + offset]`, and
`adjust_stack_instruction_offsets` leaves accesses inside such a window
unshifted — so a spill written before the `sub_sp` and reloaded inside it would
be read `0x60` bytes off. `lower_resource_path` holds its `String` argument live
across the acquisition, which is exactly the shape that could trip it.

Measured on the emitted plan rather than reasoned about: in the shipped
windows-x86_64 `runtime.os.resourcePath` the allocator uses **seven** spill slots
(offsets 4176…4272 of a 4296-byte frame), and **not one of them is touched inside
the window** — every access there is the hook's own `32/40/48/56/72/80`, all
`< 0x60`. The window contains no allocator-visible vreg operand at all, so the
allocator has nothing to place in it. Adding explicit park/reload slots would
therefore have been dead code; the invariant is pinned by a test on the emitted
plan instead
(`nothing_addresses_outside_the_windows_acquisition_frame`), which goes red if a
future change ever does place a spill there.

### Runtime proof — box 2230 (Win11 x86_64)

Console build, `resources: [{ "src": "data/greeting.txt", "dst": "./" }]`, `.exe`
and resource shipped to `C:\mfb454\`:

```
exe=C:\mfb454\res454.exe
path=C:\mfb454/greeting.txt
read=hello from a windows resource
endsWith=TRUE
dotdot=77030002
backslashdotdot=77030002
nested=TRUE
rc=0
```

`read=` is the end-to-end assertion: the returned path was handed to
`fs::readText` and opened. `backslashdotdot` is the Windows-only validation
hardening (`..\secret`); without it that line reads `0` and the traversal is
accepted.

`--app` (`NativeBuildMode::WindowsApp`, the row item 3 claimed was missing):

```
path=C:\mfb454app/greeting.txt
read=app-mode resource
rc=0
```

**Negative control.** With the backward scan reverted to the POSIX `47`,
rebuilt, and re-run on the same box:

```
exe=C:\mfb454\res454.exe
Error: 7-705-0007
Operation is not supported by the implementation or platform.
rc=255
```

— `ErrUnsupported`, exactly as predicted for a `/` scan over `C:\mfb454\res454.exe`.
The cross-build and the artifact gate are both green in that state, which is the
point: only the box can see it.

## Summary

A contained per-target feature gap: the risk is UTF-16 conversion and the
windows `.ncodesum` ripple (known from memory to hit every io-importing
fixture), not the design — macOS/Linux define the semantics to copy.
