# audit-3 — Surface 7: custom linker & emitted-binary hardening (Mach-O / ELF / PE / AppImage)

Part of `planning/goal-08-platform-security-review.md`. Finding prefix `LNK-`.
Untrusted party: an attacker exploiting an emitted binary at runtime (a missing
mitigation is a real finding even without a companion in-program bug), plus — new
this pass — anyone who supplies the *project* being built.

**Verdict: 2 HIGH · 3 MEDIUM · 5 LOW · 2 NTH.** The prior-audit Linux/macOS
hardening gaps are largely fixed (see re-verified table). The two HIGHs are
LNK-12 (an arbitrary-path executable write from an untrusted `project.json`,
**reproduced live**) and LNK-13 (the never-before-audited Windows PE target ships
with **no ASLR at all**). The custom linker is *not* an untrusted-input parser
(clean verdict, item 5).

## HIGH

### LNK-12 — `project.json` `name` path-joined unsanitized → arbitrary 0755 executable write → **bug-503**

Lead-reproduced: a project named `../../../../../../tmp/lnk12-pwn/evil` makes
`mfb build` write a 0755 Mach-O to `/tmp/lnk12-pwn/evil.out`, outside the project.
The name reaches `out_dir.join(...)` / `bin_dir.join(project_name)` with no
component check (`src/os/linux/link/mod.rs:74`, `linux/appdir.rs:70`,
`macos/link/mod.rs:86`), then `chmod 0755`. `validate_package_name`
(`src/manifest/package.rs:58`) exists but has no `src/os/**` caller. Building an
untrusted project must not write outside it.

### LNK-13 — Windows PE has no ASLR → **bug-504**

`IMAGE_FILE_RELOCS_STRIPPED` set (`pe.rs:189`), no `.reloc`, `DllCharacteristics
= 0x8100` (no `DYNAMIC_BASE`/`HIGH_ENTROPY_VA`, `pe.rs:221`), fixed
`IMAGE_BASE = 0x140000000` (`pe.rs:15`) — all lead-verified in current source.
Every emitted Windows binary loads at the same address on every machine. The
Windows analog of bug-186 (non-PIE Linux, fixed); the PE target had no prior
audit.

## MEDIUM

- **LNK-14** — PE has no `IMAGE_LOAD_CONFIG_DIRECTORY` → no CFG, no `/GS` security
  cookie, no unwind data (`pe.rs:243-259`). Companion to LNK-13 on the same
  target; larger fix (a load-config + GuardCF table).
- **LNK-15** — ~633 KB of immutable tables (`types.rs:161-166`,
  `data_objects.rs:948`) still sit in the writable segment on all three platforms;
  bug-187 moved only String literals to `__DATA_CONST`. A partial re-open of
  audit-2 LNK-08 / bug-187.
- **LNK-16** — Windows vendored-`LINK` DLL path built with unbounded `lstrcatA`
  into a 1024-byte global (`src/target/win_x86_64/code.rs:928-944`); overruns into
  the adjacent `GetProcAddress` name strings. A memory-safety nit on the Windows
  dynamic-link path.

## LOW

- **LNK-17** — the `LINK` function-pointer table and its name strings are
  permanently writable, contra the module doc (`link_thunk.rs:13-16,57-69`).
- **LNK-18** — macOS: no hardened runtime (`CS_RUNTIME`), `LC_BUILD_VERSION.sdk=0`
  disables library validation, `__DATA_CONST maxprot=0x3`
  (`macos/link/commands.rs:628,62`). Lead-confirmed via `codesign -dvvv` (ad-hoc,
  no runtime flag) and `otool -l` (`__DATA_CONST initprot 0x3`) on a freshly built
  binary.
- **LNK-19** — no stack canaries on any target (absent by construction).
- **LNK-20** — no `GNU_PROPERTY` note → no BTI/PAC/IBT (`elf.rs`).
- **LNK-22** — AppImage squashfs copies host modes verbatim (keeps setuid /
  world-write) (`linux/appimage/mod.rs:284-288`).

## NTH

- **LNK-21** — `.desktop` `Exec=` does not escape `%` (`;`/newline are handled)
  (`linux/appdir.rs:170-176`).

## Item 5 — the linker as a parser: clean

`EncodedImage` is never deserialized; a `.mfp` carries IR, not native object code;
the one untrusted string the linker consumes (a native-lib `source` name) is
validated on both the producer and reader sides (its only gap is the missing
length bound, LNK-16). So all linker *input* is first-party output — a legitimate
"clean" verdict, recorded so a later audit does not re-derive it.

## Lead-measured hardening (macOS aarch64, fresh hello-world)

`otool -hv`: `PIE` set. Segment `initprot`: `__TEXT` r-x (0x5), `__DATA_CONST`
rw- (0x3), `__DATA` rw-, `__LINKEDIT` r-- — **no RWX segment**.
`codesign -dvvv`: ad-hoc, linker-signed, `LC_CODE_SIGNATURE` present, no
hardened-runtime flag (→ LNK-18). Confirms audit-2 LNK-01/bug-187 fixed on macOS.

## Prior findings re-verified from fresh binaries

Fixed: LNK-01 (bug-186 non-PIE Linux), LNK-02 (GNU_STACK), LNK-03 (RELRO, both
platforms), LNK-04, LNK-06, LNK-11. Still open: LNK-05/07/09/10 (→ LNK-20 /
noted / LNK-19 / LNK-18). LNK-08 **partial** → re-opened as LNK-15.

## Bug docs filed

bug-503 (LNK-12), bug-504 (LNK-13). LNK-14/15/16 are recorded for follow-up
(LNK-14 is a companion to bug-504 on the same target; LNK-16 is a small memory
nit).

## Coverage

Read: `src/os/{linux,macos,windows}/link/**` header/segment writers,
`src/os/linux/{appdir,appimage}/`, `src/codegen/link/{locator,thunk}/`,
`src/os/macos/link/commands.rs`. Emitted binaries inspected with `otool -hv`/`-l`
and `codesign` (macOS, native) and a python PE-header parse (Windows output).

Gaps: `src/os/windows/link/rsrc.rs`'s `VS_VERSIONINFO` `u16` length arithmetic not
audited; `src/os/{linux,macos,windows}/object.rs` grepped, not read (deprioritised
once item 5's first-party verdict held); Windows binaries were header-parsed, not
executed (needs box 2230).
