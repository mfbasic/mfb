# bug-321: the three Linux backends are a ~90% copy-paste triple — 5,250 lines carrying roughly one backend's worth of information

Last updated: 2026-07-19
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup / duplication)

Status: Blocked (no Linux box reachable; see BLOCKED)
Regression Test: none new — the guarantee is **byte-identical generated output**, enforced by `scripts/artifact-gate.sh` plus `scripts/test-accept.sh` (see Validation Plan for the Linux-coverage caveat)

`src/target/linux_aarch64/`, `src/target/linux_riscv64/`, and
`src/target/linux_x86_64/` are three near-verbatim copies of the same backend.
Measured on this worktree: `plan.rs` shares 443 of 458 lines aarch64↔riscv64 and
the *only* semantic difference between those two files is **two string
literals**; `code.rs` shares 699 of 774 lines, with 48 of 64 `CodegenPlatform`
methods byte-for-byte identical (460 lines); `mod.rs` triples a 150-entry
capability array that is set-identical in all three and five dump-writer
functions that differ only by a wrapper name. No user-visible defect follows
from this — every backend compiles correctly today. The cost is maintenance
arithmetic: any Linux ABI fact, import rule, or runtime-call registration has to
be edited in three places, and the copies have already drifted in comment
content (finding #4 below: the x86 copy silently dropped every explanatory
comment on its socket/errno constants).

The single correct outcome a fix produces: the Linux-invariant material lives in
one `src/target/linux_common/` module, each of the three backends retains only
its genuinely arch-specific overrides, and **every artifact the compiler emits
for every Linux target is byte-identical before and after**.

References:

- `spec/architecture/06_native.md` (backend/platform seam), `spec/linker/07_linux-aarch64.md`,
  `spec/linker/08_linux-x86_64.md`, `spec/linker/09_linux-riscv64.md`.
- Cleanup review, Agent 10 — backend targets, findings #1–#5 (`/tmp/cleanup-findings/index.md:428-432`).
  Findings #6–#26 from the same agent (spec drift, GTK layout staleness, app-runtime
  duplication) are **out of scope here** and should be filed separately.
- Related: bug-223 (the riscv64 app-mode rejection this refactor must not weaken).

## BLOCKED (2026-07-19)

Phase 1 cannot start: every Linux box in `.ai/remote_systems.md` refuses
connections (2222 ArchLinux, 2228 Ubuntu x86_64, 2232 Debian riscv64 all
`Connection refused` — the VMs are not running).

This blocks the whole bug rather than just delaying it, because Phases 1 and 6
*are* the correctness argument. As this document's own Validation Plan
establishes, the repo commits **zero** Linux artifact goldens
(`find tests -path "*/golden/*" -name "*linux*"` → 0), and
`scripts/artifact-gate.sh` derives its target from `uname`, so running it on
macOS proves only that `src/target/shared/` was undisturbed. There is no local
gate that can detect a byte-level change to any of the three Linux backends.

Proceeding without the baseline would mean performing a 5,250-line
three-backend refactor whose sole acceptance criterion — byte-identical output
on 3 targets × 2 libc flavors — cannot be evaluated. The specific hazard this
document already identifies makes that unacceptable rather than merely
unverified: a `linux_common` trait carrying default app-mode bodies is exactly
the mechanism that would hand riscv64 working-looking implementations for the
nine `unimplemented!` hard-stops that were deliberately left unported
(bug-223), and the guard that would catch it
(`linux_riscv64/mod.rs` `lower_validated_module`) is per-backend and cannot be
hoisted.

Resume when the boxes are up. Nothing in this document needs revising first;
the audit below was re-confirmed against the tree at HEAD.

## Current State

This is a cleanup bug, so there is no failing run. What follows are the
measurements, with the commands as run in the worktree root.

### Baseline sizes

```
$ wc -l src/target/linux_{aarch64,riscv64,x86_64}/*.rs
   774 src/target/linux_aarch64/code.rs
   466 src/target/linux_aarch64/mod.rs
   458 src/target/linux_aarch64/plan.rs
   764 src/target/linux_riscv64/code.rs
   459 src/target/linux_riscv64/mod.rs
   488 src/target/linux_riscv64/plan.rs
   821 src/target/linux_x86_64/code.rs
   494 src/target/linux_x86_64/mod.rs
   526 src/target/linux_x86_64/plan.rs
  5250 total
```

### #1 — `plan.rs` is a 3-way copy; aarch64↔riscv64 differ by exactly two string literals

```
$ comm -12 <(sort src/target/linux_aarch64/plan.rs) <(sort src/target/linux_riscv64/plan.rs) | wc -l
443          # of 458 lines
$ comm -12 <(sort src/target/linux_aarch64/plan.rs) <(sort src/target/linux_x86_64/plan.rs) | wc -l
372
```

`diff src/target/linux_aarch64/plan.rs src/target/linux_riscv64/plan.rs` produces
six hunks. Four are reworded comments and two are appended `#[test]` functions.
The only **semantic** differences are:

- `linux_aarch64/plan.rs:18` `"libc.musl-aarch64.so.1"` vs `linux_riscv64/plan.rs:18` `"libc.musl-riscv64.so.1"`
- `linux_aarch64/plan.rs:40` `"linux-aarch64"` vs `linux_riscv64/plan.rs:40` `"linux-riscv64"`

`runtime_imports` (`linux_aarch64/plan.rs:81`, `linux_riscv64/plan.rs:81`,
`linux_x86_64/plan.rs:87`) is 322 lines on aarch64 and 325 on x86 — the bulk of
each file, written out three times. aarch64 and riscv64 have 45 call arms each;
x86 has 47.

x86's divergence is real but narrow, and lies along exactly two axes: `write` is
a raw syscall on x86 so it is never imported, and `emit_random_bytes` uses raw
`getrandom` so `getentropy` is never imported. After stripping comments, the
shared arms that materially differ are six — the `io.print|io.write|io.printError|io.writeError`
arm, `io.input`, `term.on`, `term.off`, `term.sync`, and the `fs.*` arm — plus one
x86-only arm covering eight zero-import `term.*` calls. (The review recorded
"~12 of ~45 arms"; the measured figure is **6 shared arms plus 1 x86-only arm**.
The x86 module doc states the policy at `linux_x86_64/plan.rs:1-3`.)

### #2 — `code.rs` is a 3-way copy: 699 of 774 lines, 48 of 64 methods byte-identical

```
$ comm -12 <(sort src/target/linux_aarch64/code.rs) <(sort src/target/linux_riscv64/code.rs) | wc -l
699          # of 774 lines
$ comm -12 <(sort src/target/linux_aarch64/code.rs) <(sort src/target/linux_x86_64/code.rs) | wc -l
622
```

All three implement the same 64-method `CodegenPlatform` surface (x86 omits only
`libc`). Comparing method bodies pairwise:

| Comparison | byte-identical methods | lines |
| --- | --- | --- |
| aarch64 ↔ riscv64 | 48 / 64 | 460 |
| aarch64 ↔ x86_64 (raw) | 14 / 64 | — |
| aarch64 ↔ x86_64 (after normalizing the one call idiom + whitespace) | 27 / 64 | 259 |

The 16 methods that differ aarch64↔riscv64 are `arch`, `backend`, `target`,
`o_nonblock`, `emit_environ_pointer`, `emit_program_exit`, `emit_sync_file`,
`emit_variadic_call`, and the eight app-mode methods (which on riscv64 are the
hard-stops described below).

**Correction to the review.** Finding #2 states "only the `abi` import differs."
It does not: all three files import the *same* module —
`linux_aarch64/code.rs:4`, `linux_riscv64/code.rs:4`, and
`linux_x86_64/code.rs:19` are each verbatim `use crate::arch::aarch64::abi;`.
(That shared shim is itself Agent 11 finding #8.) The real textual axis is that
aarch64 and riscv64 route libc calls through a file-local free function
`emit_linux_c_call` (`linux_aarch64/code.rs:726-746`, wrapped by the trait method
at `:385-394`), whereas x86 inlines the same body directly into its trait method
at `linux_x86_64/code.rs:521-545`. Normalizing that one idiom nearly doubles the
identical-method count, as the table shows.

### #3 — `emit_temp_directory` is an 81-line verbatim triple

`linux_aarch64/code.rs:511-591`, `linux_riscv64/code.rs:529-609`,
`linux_x86_64/code.rs:629-709` — 81 lines each.

```
$ diff <(sed -n '511,591p' src/target/linux_aarch64/code.rs) \
       <(sed -n '529,609p' src/target/linux_riscv64/code.rs)
             # no output — byte-for-byte identical

$ diff <(sed -n '511,591p' src/target/linux_aarch64/code.rs) \
       <(sed -n '629,709p' src/target/linux_x86_64/code.rs)
30c30
<         emit_linux_c_call(from, "getenv", platform_imports, instructions, relocations)?;
---
>         self.emit_libc_call("getenv", from, platform_imports, instructions, relocations)?;
```

Exactly one differing line across three copies, and that line is the idiom from
#2. Deduplicating removes ~162 lines.

### #4 — 21 Linux-ABI constant methods return identical values in all three backends

Extracting every single-expression `fn name(&self)` from the three files and
comparing values with trailing comments stripped yields **21** methods with
identical values everywhere:

`termios_size` (60), `termios_lflag_offset` (12), `termios_lflag_width` (4),
`termios_cc_offset` (17), `termios_echo_flag` (8), `termios_icanon_flag` (2),
`termios_vmin_index` (6), `termios_vtime_index` (5) —
`linux_aarch64/code.rs:77-107`, `linux_riscv64/code.rs:76-…`, `linux_x86_64/code.rs:90-…`;
`dirent_name_offset` (19) and `dirent_name_length_offset` (0) —
`linux_aarch64/code.rs:632-638`, `linux_riscv64/code.rs:650`, `linux_x86_64/code.rs:750`;
`addrinfo_addr_offset` (24), `sol_socket` ("1"), `so_reuseaddr` ("2"),
`so_rcvtimeo` ("20"), `so_sndtimeo` ("21"), `eagain` ("11"), `emsgsize` ("90"),
`o_nonblock` ("2048"), `einprogress` ("115"), `so_error` ("4") —
`linux_aarch64/code.rs:683-723`, `linux_riscv64/code.rs:707-739`,
`linux_x86_64/code.rs:794-818`; and `entry_args_in_registers` (false).

The x86 copy lost the explanatory comments the other two carry — e.g.
`linux_aarch64/code.rs:705-707` reads `"11" // EAGAIN on Linux` while
`linux_x86_64/code.rs:806-808` reads bare `"11"`. This is the drift already
present, and it is exactly the kind that a single shared definition prevents.

**Correction to the review.** Finding #4 says "none arch-dependent." One nearby
sibling method **is**, and it is the trap for this phase: `stat_mode_offset` is
`16` on aarch64 (`linux_aarch64/code.rs:313-315`) and riscv64
(`linux_riscv64/code.rs:315-317`) but `24` on x86-64
(`linux_x86_64/code.rs:435-438`, whose comment says so). It sits inside the same
run of one-line constants and must **not** be swept into the shared set.

### #5 — `mod.rs`: 150-entry capability array tripled; five dump-writers effectively identical

The `runtime_calls` array in `capabilities()` —
`linux_aarch64/mod.rs:33-184`, `linux_riscv64/mod.rs:40-191`,
`linux_x86_64/mod.rs:40-191`:

```
$ # extract quoted entries from each region, sort, compare
aarch64: 150 entries    riscv64: 150 entries    x86_64: 150 entries
diff aarch64 riscv64 -> identical set
diff aarch64 x86_64  -> identical set
```

(The review said 145; the measured count is **150**.) The riscv64 and x86_64
regions are byte-identical to each other. The only textual difference against
aarch64 is that its 12-entry `thread.*` block sits at a different position in the
list — which is what makes the raw `diff` look like a 24-line divergence when the
information content is zero.

The five diagnostic dump-writers (`write_nir`, `write_native_plan`,
`write_native_object_plan`, `write_native_code_plan`, `write_mir`) at
`linux_aarch64/mod.rs:362-440`, `linux_riscv64/mod.rs:343-424`,
`linux_x86_64/mod.rs:378-459` diff in only two ways: a 3-line comment present in
the riscv64/x86 copies, and `plan::lower_module(...)` vs `plan_lower(...)`.
`plan_lower` is a pure one-line forwarder
(`linux_riscv64/mod.rs:426-433`, `linux_x86_64/mod.rs:461-468`) whose doc comment
claims the backend "reuses the AArch64 backend's `plan` lowering verbatim" — it
does not; `plan` there resolves to that backend's own `pub(crate) mod plan`
(declared at `:11` in each `mod.rs`), a 488- and 526-line module respectively.
Deleting the wrapper collapses the last difference between the three sets of
dump-writers.

## Root Cause

Successive plans added Linux backends by copying the previous one wholesale
rather than by extracting a shared layer:

- `linux_aarch64` was the original Linux backend.
- `linux_x86_64` was added by plan-00-H (`linux_x86_64/code.rs:1-14` module doc:
  "the same ones the AArch64 backend uses").
- `linux_riscv64` was added by plan-99 (`linux_riscv64/mod.rs:200-202` cites it).

Each copy started as a byte-identical fork and was then edited only where the ISA
genuinely forced a change. Because the copy was total rather than selective, the
Linux-invariant 90% — kernel/libc struct offsets, socket and errno constants,
libc import policy, the runtime-call capability list, the diagnostic dump
plumbing — was duplicated along with the ~10% that is actually per-arch. Nothing
in the codebase distinguishes the two categories, so a reader cannot tell whether
a given constant is a shared Linux fact or a deliberate per-arch value without
diffing all three files. `stat_mode_offset` (#4) is exactly the case where that
distinction matters and is invisible.

The one structural attempt at sharing that exists — `plan_lower` — is a no-op
wrapper whose doc comment asserts the opposite of what it does, which suggests
the sharing intent was present but never carried through.

## Goal

- Linux-invariant backend material exists in exactly one place under
  `src/target/linux_common/`; each of `linux_aarch64`, `linux_riscv64`,
  `linux_x86_64` retains only its genuine per-arch overrides.
- The generated output for every Linux target — `.nir`, `.nplan`, `.nobj`,
  `.ncode`, `.mir`, and the linked executables for both glibc and musl flavors —
  is **byte-identical to the pre-refactor output**.
- `stat_mode_offset` remains per-arch (16 / 16 / 24) and is documented as such at
  its new home.
- The three defense layers guarding riscv64 app mode (see Non-goals) all survive.

### Non-goals (must NOT change)

- **Any byte of generated output.** This refactor is a pure code reorganization.
  If any artifact changes, the refactor is wrong — do not regenerate goldens to
  make a diff go away.
- **The riscv64 app-mode rejection, at all three layers.** The review flagged
  this and it is the highest-risk part of the change. The layers are:
  1. Nine `unimplemented!("rv64 app mode not ported (plan-05 is aarch64/x86-64 only)")`
     panics in `linux_riscv64/code.rs` at `:124, :149, :189, :201, :209, :217, :227, :235, :245`.
     (The review said eight; there are **nine**.)
  2. `supports_app_mode() -> false` at `linux_riscv64/mod.rs:199-204`.
  3. The `NativeBuildMode::Console`-only guard in `lower_validated_module` at
     `linux_riscv64/mod.rs:444-454`, which returns a clean `Err` — its own comment
     cites bug-223 and explains that it exists precisely so a non-CLI/API caller
     cannot reach the panic.
  A `linux_common` trait with default app-mode bodies will make it *easy* to give
  riscv64 working-looking defaults. Do not. If the shared trait supplies default
  app-mode method bodies, riscv64 must override all nine to keep panicking, and
  layers 2 and 3 must remain riscv64-local. Note that the aarch64 and x86_64
  `lower_validated_module`s accept `LinuxApp` (`linux_aarch64/mod.rs:451-461`) —
  so this guard is genuinely per-backend and cannot be hoisted.
- **The x86-64 raw-syscall policy.** `write`, `exit_group`, `getrandom`,
  `mmap`/`munmap`, and `_exit` are raw syscalls on x86 and must stay uncalled
  through libc; the corresponding import arms must stay empty
  (`linux_x86_64/plan.rs:1-3`, and its own test module at `:431-526`).
- **`stat_mode_offset`.** Not a shared constant.
- **The musl/glibc dual-flavor output.** Console builds still emit
  `-glibc.out` + `-musl.out`; app mode stays glibc-only
  (`linux_aarch64/mod.rs:303-307`).
- **The `runtime_calls` array contents.** Reordering to a single canonical list is
  fine (the sets are already identical and the consumer is
  `validate::validate_capabilities`), but no entry may be added or dropped.

## Blast Radius

Every site below was found by reading the three backends and their consumers, not
from memory.

Fixed by this bug:

- `src/target/linux_aarch64/plan.rs`, `linux_riscv64/plan.rs`, `linux_x86_64/plan.rs`
  — the `runtime_imports` triple and the `libc()`/`target()` per-arch literals.
- `src/target/linux_aarch64/code.rs`, `linux_riscv64/code.rs`, `linux_x86_64/code.rs`
  — the 21 constant methods, `emit_temp_directory`, and the 48/64 identical
  `CodegenPlatform` bodies.
- `src/target/linux_aarch64/mod.rs`, `linux_riscv64/mod.rs`, `linux_x86_64/mod.rs`
  — the `runtime_calls` array, the five dump-writers, and the `plan_lower` wrappers.

Latent, same hazard, out of scope:

- `src/target/macos_aarch64/` — implements the same `CodegenPlatform` /
  `NativePlanPlatform` traits with genuinely different ABI values (Darwin struct
  layouts, different errno numbering). It shares the *shape* but not the *facts*,
  so folding it into `linux_common` would be wrong by construction. Out of scope.
- `src/target/linux_gtk/` — shared by the aarch64 and x86_64 app runtimes already;
  its own duplication against `macos_aarch64/app/` is Agent 10 findings #13–#17,
  a separate document.
- `src/arch/{aarch64,x86_64,riscv64}/` — the encoder-level triplication (Agent 11
  findings #3, #4, #11, #12) is the same disease one layer down but a disjoint
  file set. Separate document.

Unaffected:

- `src/target/shared/` — already the single copy of the target-neutral pipeline;
  this refactor adds a Linux-specific layer beneath it, changing nothing there.
- `src/target.rs` backend registry — `linux_common` is not a `NativeBackend`; the
  three registered backends keep their identities, targets, and capability
  reporting.
- `src/os/linux/` — object/ELF emission, already shared by all three.

## Fix Design

Add `src/target/linux_common/` with three modules mirroring the current file
split, and reduce each backend to its deltas:

- `linux_common/plan.rs` — the `runtime_imports` body, parameterized by a small
  `LinuxSyscallPolicy` (or equivalent) describing which primitives the backend
  raw-syscalls. On aarch64/riscv64 the policy is "nothing is raw"; on x86 it is
  `{write, getrandom}` for import purposes. Per-arch inputs reduce to the musl
  soname and the target name.
- `linux_common/code.rs` — a trait (or a `LinuxCodegen` blanket impl) supplying
  default bodies for the 48 shared methods and the 21 constants, over a single
  `emit_libc_call` seam. Unifying `emit_linux_c_call` / the inlined x86 body onto
  one seam is a prerequisite and is itself output-neutral (the two bodies are
  identical — compare `linux_aarch64/code.rs:726-746` against
  `linux_x86_64/code.rs:521-545`).
- `linux_common/mod.rs` — one `LINUX_RUNTIME_CALLS` const and one generic set of
  dump-writers parameterized by the backend's `plan::lower_module`.

Sequencing matters: the `emit_libc_call` unification comes first, because it is
what makes the x86 copy comparable to the other two at all (it lifts
byte-identical methods from 14 to 27).

**Rejected alternatives.**

- *Make riscv64 and x86_64 delegate to `linux_aarch64` directly.* This is the
  status quo's implicit shape (`use crate::arch::aarch64::abi` in all three,
  `linux_aarch64/code.rs:4` / `linux_riscv64/code.rs:4` / `linux_x86_64/code.rs:19`)
  and it is the thing to undo, not extend: it makes two shipping ISAs formally
  depend on a third's module. A peer `linux_common` is the correct direction.
- *Collapse the three backends into one parameterized backend.* Rejected: the
  three are distinct `NativeBackend` registrations with different capabilities
  (riscv64 has no app mode) and different encoders; the registry seam is load-bearing.
- *Generate the shared code with a macro.* Rejected: macros would preserve the
  duplication in expanded form while making the per-arch deltas harder to see,
  which is the actual problem.
- *Fix the drift (restore x86's lost comments) and leave the structure alone.*
  Rejected: that treats the symptom. The comments were lost because there are
  three copies.

Expected output shift: **none**. That is the acceptance criterion, not a hope.

## Phases

### Phase 1 — capture the byte-identical baseline (no code change)

Because the repo has **no Linux artifact goldens** (see Validation Plan), the
baseline must be captured manually before anything moves.

- [ ] On each Linux box in the matrix (linux-aarch64, linux-x86_64, linux-riscv64),
      for both glibc and musl flavors, build every `tests/**/project.json` fixture
      with `-nir -nplan -nobj -ncode -mir` and archive the outputs, plus the
      `sha256` of every linked executable.
- [ ] Record the archive location and hashes in this file.
- [ ] Confirm the blast-radius audit above against the tree at the base commit.

Acceptance: a reproducible, hashed baseline exists for all three Linux targets ×
both flavors; the audit list carries a verdict per site.
Commit: —

### Phase 2 — unify the libc-call seam (output-neutral, no `linux_common` yet)

- [ ] Make x86's `emit_libc_call` (`linux_x86_64/code.rs:521-545`) call a shared
      helper with the identical body used by the other two
      (`linux_aarch64/code.rs:726-746`).
- [ ] Delete the `plan_lower` wrappers (`linux_riscv64/mod.rs:426-433`,
      `linux_x86_64/mod.rs:461-468`) and their false doc comments; call
      `plan::lower_module` directly as aarch64 does.

Acceptance: Phase 1 baseline reproduces bit-for-bit on all three targets.
Commit: —

### Phase 3 — extract `linux_common/plan.rs`

- [ ] Move `runtime_imports` into `linux_common`, parameterized by the syscall
      policy; leave `libc()` and `target()` per-backend.
- [ ] Keep the existing per-backend `#[test]`s where they are; they now guard the
      shared implementation from three angles (notably x86's
      `linux_x86_64/plan.rs:514` "must not import libc write on x86 (raw syscall)" and riscv64's
      `create_temp_file_imports_getentropy` at `linux_riscv64/plan.rs:474`).

Acceptance: baseline reproduces; all three backends' plan tests pass unchanged.
Commit: —

### Phase 4 — extract `linux_common/code.rs`

- [ ] Move the 21 shared constants. **Leave `stat_mode_offset` per-backend** and
      add a comment at each site saying why.
- [ ] Move `emit_temp_directory` and the 48 shared method bodies.
- [ ] Re-assert all three riscv64 app-mode defense layers: nine overriding
      `unimplemented!` bodies, `supports_app_mode() == false`, and the
      `lower_validated_module` `LinuxApp` rejection. Add a comment at the
      `linux_common` trait declaration stating that app-mode defaults must not be
      inherited by riscv64.

Acceptance: baseline reproduces; a `-app` build targeting linux-riscv64 still
returns the clean bug-223 error (not a panic), and the CLI still rejects it.
Commit: —

### Phase 5 — extract `linux_common/mod.rs` (capability array + dump-writers)

- [ ] One `LINUX_RUNTIME_CALLS` const, one canonical ordering, referenced by all
      three `capabilities()`.
- [ ] One generic set of the five dump-writers.

Acceptance: baseline reproduces; `validate_capabilities` behavior unchanged.
Commit: —

### Phase 6 — full validation

- [ ] Re-run the Phase 1 capture on all three Linux targets × both flavors and
      diff against the archived baseline. **Any non-empty diff blocks the change.**
- [ ] `scripts/artifact-gate.sh` and `scripts/test-accept.sh` green on macOS
      (guards against collateral damage to the shared layer) and on each Linux box.
- [ ] `cargo test`, `cargo fmt --check`, `cargo clippy`.

Acceptance: zero artifact bytes changed; full suite green on every box.
Commit: —

## Validation Plan

- **Regression test(s):** none added. The guarantee is byte-identical output; a
  new unit test would assert less than the artifact diff already does. If a test
  is wanted, the cheapest useful one is a `linux_common` test asserting
  `stat_mode_offset` is 16/16/24 across the three backends — the one constant this
  refactor could plausibly homogenize by accident.
- **Runtime proof:** the full acceptance suite (462 `.run` goldens) executed on
  each Linux box, both flavors, plus a `-app` attempt on linux-riscv64 confirming
  the clean error.
- **Artifact gate — read this caveat.** `scripts/artifact-gate.sh:8-9` derives its
  target from `uname`, and the repo currently commits native-artifact goldens for
  **one target only**:

  ```
  $ find tests -path "*/golden/*" \( -name "*.ncode" -o -name "*.mir" -o -name "*.nir" -o -name "*.nobj" \) \
      | grep -o 'macos-aarch64\|linux-aarch64\|linux-x86_64\|linux-riscv64' | sort | uniq -c
    47 macos-aarch64
  $ find tests -path "*/golden/*" -name "*linux*" | wc -l
    0
  ```

  So the artifact gate **does not cover the Linux backends at all**. Running it on
  macOS proves only that `src/target/shared/` was not disturbed. The byte-identity
  guarantee for Linux must come from the manual Phase 1 / Phase 6 self-diff on
  real Linux hosts. `scripts/test-accept.sh:37-40` honors `MFB_TARGET` for
  cross-targeting, but with no Linux goldens committed there is nothing for it to
  compare artifacts against.
- **Doc sync:** none required by this refactor. Note separately that Agent 10
  findings #6, #7, #11 record real spec drift in
  `spec/linker/07_linux-aarch64.md`, `08_linux-x86_64.md`, and
  `spec/linker/03_import-selection.md` — those are pre-existing and belong in
  their own document, not this one.
- **Full suite:** `cargo test`; `scripts/artifact-gate.sh <mfb>`;
  `scripts/test-accept.sh <mfb>` on macOS and on each Linux box.

## Open Decisions

- **Shared-code mechanism** — a `LinuxCodegen` trait with default method bodies
  (recommended: keeps each backend's `impl CodegenPlatform` visible and makes a
  per-arch override an explicit, greppable act) vs. a `LinuxCommon` struct that
  the backends delegate to field-wise. The trait is preferred because the riscv64
  app-mode overrides must be conspicuous.
- **`runtime_calls` canonical ordering** — adopt the riscv64/x86_64 ordering (they
  are already byte-identical to each other, so two of three files stay untouched)
  vs. the aarch64 ordering. Recommended: the riscv64/x86_64 ordering. Order is not
  semantically significant (the consumer is a membership check), but confirm that
  before committing.
- **Whether to fold `stat_mode_offset` into a documented per-arch override list**
  alongside `arch`/`backend`/`target`/`o_nonblock`, making the per-arch surface a
  single named group rather than scattered exceptions. Recommended: yes.

## Summary

The engineering risk is not in the extraction — the code is provably identical,
and every step is mechanically checkable. It is concentrated in two places.
First, **verifying byte-identity on Linux**, which the repo's normal gates cannot
do because zero Linux artifact goldens are committed; Phases 1 and 6 exist
entirely to substitute a manual, hashed self-diff for the gate that does not
exist. Second, the **riscv64 app-mode hard-stops**: a shared trait with default
app-mode bodies is exactly the mechanism that would silently hand riscv64 working
implementations for a path that was deliberately left unported, and the guard
that catches it (`linux_riscv64/mod.rs:444-454`) is per-backend and cannot be
hoisted.

Two review figures were corrected against the source: `runtime_calls` has 150
entries (not 145), and there are nine riscv64 `unimplemented!` sites (not eight).
Two review claims were found false and are documented above: the three `code.rs`
files import the *same* `abi` module, not different ones; and `stat_mode_offset`
is genuinely arch-dependent, sitting inside the run of otherwise-identical
constants.

Left untouched: `src/target/shared/`, `src/target/macos_aarch64/`,
`src/os/linux/`, the backend registry, the dual-flavor output contract, and the
x86-64 raw-syscall policy.
