# bug-433: Windows PE executables carry no MFBasic provenance marker

Last updated: 2026-08-08
Effort: small (<1h)
Severity: LOW
Class: Correctness / Footgun

Status: Open
Regression Test: src/os/windows/link/tests.rs (new `provenance_marker_emitted_unconditionally`)

Every executable the ELF and Mach-O linkers emit carries an **unconditional** vendor
note (plan-43): the `MFBasic\0` owner plus a versioned 16-byte descriptor from
`src/os/note.rs`, so a tool (or the runtime) can positively identify an mfb-produced
binary and read the compiler version. The Windows PE writer imports nothing from
`note.rs` and emits no marker, so **every** Windows `.exe` is unmarked. Unlike its
sibling bug-432 (signing metadata, which only affects *signed* builds), this affects
every Windows build.

The single correct behavior a fix produces: every Windows `.exe` unconditionally
carries the `MFBasic\0` provenance marker, with the exact same 16-byte descriptor
bytes ELF/Mach-O emit, in a dedicated read-only PE section. ELF/Mach-O emission is
unchanged.

References:

- Marker byte format: `src/os/note.rs:14` (`MFB_NOTE_OWNER = b"MFBasic\0"`), `:42`
  (`mfb_note_descriptor()` → 16 bytes: `MFB1` magic, version 1, flags 0, compiler
  major/minor/patch, pad). The descriptor is already format-neutral (a raw `Vec<u8>`;
  each format frames it).
- Unconditional ELF `PT_NOTE`: `src/os/linux/link/elf.rs:6` (`mfb_note_bytes`), `:20`
  (`note_program_header`), emitted by all three encoders
  (`elf.rs:112,192,373`). Unconditional Mach-O `LC_NOTE`:
  `src/os/macos/link/commands.rs:181` (`note_command`), `macho.rs:165`.
- plan-43 `planning/completed/plan-43-binary-magic-marker.md:6` ("Embed an
  unconditional `MFBasic\0` provenance marker in **every** executable the built-in
  linker emits"); its scope statement (`:42`) names only Linux + macOS — the plan
  predates the PE backend.
- Stale spec: `src/docs/spec/linker/13_provenance-marker.md:3` ("Every executable
  either in-tree linker emits carries a vendor note … emitted for **both formats**")
  and `:23` ("**Both formats** carry the same 16-byte descriptor") — "both formats"
  silently excludes the third in-tree emitter, PE.
- Split out of bug-432 (they share the PE-section vehicle — land together). See also
  `bugs/bug-431-windows-vendored-native-libraries-nonfunctional.md`.

## Failing Reproduction

The marker is write-only (no in-repo reader parses it back — the ELF/Mach-O tests
only byte-scan for presence), so the reproduction is a byte-scan unit test:

```rust
// src/os/windows/link/tests.rs — add:
#[test]
fn provenance_marker_emitted_unconditionally() {
    // Even a bare console image must carry the MFBasic\0 provenance marker.
    let bytes = write_executable(&image(vec![0xc3]), false, None, None).expect("link");
    assert!(find_bytes(&bytes, b"MFBasic\0").is_some());
}
```

- Observed: no `MFBasic\0` owner anywhere in the image; the PE writer never imports
  `src/os/note.rs`. Fails.
- Expected: the marker is present in every Windows `.exe`, as on ELF/Mach-O.

Contrast (works today, the cross-platform guard the Windows path must join):

- Linux `src/os/linux/link/tests.rs:1229` byte-scans for the `MFBasic\0` `PT_NOTE`.
- macOS `src/os/macos/link/tests.rs:391` byte-scans for the `LC_NOTE`.

## Root Cause

`src/os/windows/link/mod.rs:write_executable` pushes only functional sections
(`.text`/`.rdata`/`.data`/`.idata`/`.rsrc`) and never imports `src/os/note.rs`. On
ELF the note is an unconditional `PT_NOTE` in every encoder
(`elf.rs:112,192,373`); on Mach-O an unconditional `LC_NOTE` (`macho.rs:165`). PE has
**no** `PT_NOTE`/`LC_NOTE` equivalent, so the marker was never ported — plan-43
scoped itself to ELF + Mach-O (`plan-43:42`), predating the PE backend. Because
`mfb_note_descriptor()` already returns a raw, format-neutral 16-byte `Vec`, only a
PE carrier is missing — no descriptor work is needed.

## Goal

- Every Windows `.exe` unconditionally carries the `MFBasic\0` provenance marker in a
  dedicated read-only PE section, with the same 16-byte descriptor bytes ELF/Mach-O
  emit (`note.rs:mfb_note_descriptor()`).
- ELF/Mach-O emission is byte-for-byte unchanged (they keep `PT_NOTE`/`LC_NOTE`).

### Non-goals (must NOT change)

- **No descriptor format change and no PE-specific descriptor.** The single
  `mfb_note_descriptor()` in `src/os/note.rs` stays the source of truth for all three
  formats; the PE carrier wraps its exact bytes (owner + descriptor).
- **Do NOT change how ELF/Mach-O carry the marker.** They keep `PT_NOTE`/`LC_NOTE`;
  this bug only *adds* a PE carrier. There is no cross-format section-name
  unification (ELF/Mach-O use note commands, not named sections), so nothing is
  renamed — contrast bug-432, which does rename the signing section.
- **Keep it unconditional** — the marker must appear in every Windows `.exe`
  (console and GUI, signed or not), matching ELF/Mach-O. Do not gate it.
- Do NOT "fix" the test by asserting the marker is absent; the broken path is the
  missing emission, and the test must exercise the emitted marker.

## Blast Radius

Found by grep for `note` / `MFBasic` / `mfb_note` across `src/`:

- `src/os/windows/link/mod.rs:write_executable` — **fixed by this bug** (add an
  unconditional marker section importing `mfb_note_descriptor`/`MFB_NOTE_OWNER`).
- `src/os/note.rs` — **consumed unchanged**; do not edit the descriptor.
- `src/os/linux/link/elf.rs` `PT_NOTE` / `src/os/macos/link/commands.rs` `LC_NOTE` —
  **unaffected** (ELF/Mach-O keep their note mechanism).
- Provenance presence tests: `src/os/linux/link/tests.rs:1229`,
  `src/os/macos/link/tests.rs:391` — **joined** by the new Windows equivalent; both
  must stay green.
- Spec: `src/docs/spec/linker/13_provenance-marker.md:3,23` ("both formats") —
  **updated** to add the PE arm and drop the two-format wording;
  `10_windows-x86_64.md` gains a provenance-section note.
- **Acceptance goldens:** the marker is **unconditional**, so it shifts the byte
  output / `NumberOfSections` / `SizeOfImage` of **every Windows** acceptance
  fixture — an expected, intended regeneration. ELF/Mach-O goldens are untouched.
- bug-432 (signing) — shares the PE-section vehicle; land in one pass to regenerate
  the Windows goldens once.

## Fix Design

A read-only PE section is the right vehicle — mirror the existing `.rsrc` wiring in
`write_executable`: **unconditionally** push a `Section { name:
section_name(".mfbnote"), characteristics: SCN_RDATA, .. }` after the functional
sections, at the next `SECTION_ALIGNMENT`/`FILE_ALIGNMENT` boundary, with **no**
data-directory entry. The body is `MFB_NOTE_OWNER` (`MFBasic\0`) followed by
`mfb_note_descriptor()` — the same owner+descriptor framing the ELF note uses, so a
reader can locate `MFBasic\0` and read the 16-byte descriptor in any of the three
formats. `.mfbnote` is exactly 8 chars, fitting PE's 8-byte section-name field with
no truncation (`src/os/windows/link/pe.rs:92`).

Rejected alternatives (do not re-litigate):

- **Rich header** — undocumented, toolchain-specific, no room for a self-describing
  descriptor.
- **`.rsrc` resource** — `.rsrc` exists only in GUI builds
  (`link/mod.rs:299`), so it could not make the marker unconditional.
- **Overlay past `SizeOfImage`** — droppable and not a real section; plan-43 §3
  rejected a `strip`-droppable carrier for exactly this discoverability reason. A
  section in the table is the discoverable form.
- **Security data directory / Authenticode** — that is for signing, not provenance;
  wrong container.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `provenance_marker_emitted_unconditionally` to
      `src/os/windows/link/tests.rs`; confirm it fails.
- [ ] Confirm the unconditional golden impact: the `.mfbnote` section shifts every
      Windows acceptance fixture; record the regeneration scope.

Acceptance: the new Windows test fails for the documented reason; the golden scope is
recorded.
Commit: —

### Phase 2 — the fix

- [ ] `src/os/windows/link/mod.rs`: unconditionally emit the `.mfbnote` section
      (`MFB_NOTE_OWNER` + `mfb_note_descriptor()` from `src/os/note.rs`; `SCN_RDATA`;
      no data directory), mirroring the `.rsrc` block.

Acceptance: the Windows provenance test passes; the ELF/Mach-O provenance tests still
pass; ELF/Mach-O output is byte-identical.
Commit: —

### Phase 3 — docs + regenerate Windows goldens + full validation

- [ ] `13_provenance-marker.md`: add a "PE: `.mfbnote` section" arm; replace "both
      formats" / "either in-tree linker" with "all three in-tree linkers".
      `10_windows-x86_64.md`: document the `.mfbnote` section. Run the spec
      drift-guard tests.
- [ ] `scripts/artifact-gate.sh`: regenerate the Windows goldens; confirm the delta
      is exactly the one added section and ELF/Mach-O goldens are untouched.
- [ ] `cargo test --bin mfb` full suite.
- [ ] If a Windows/Wine runner is available, confirm a bare `.exe` carries `.mfbnote`
      with `MFBasic\0` (`dumpbin /headers` or a byte-scan).

Acceptance: full suite green; the Windows golden delta is exactly the section
addition; the reproduction passes.
Commit: —

## Validation Plan

- Regression test(s): `provenance_marker_emitted_unconditionally`
  (`src/os/windows/link/tests.rs`); the existing ELF/Mach-O presence tests stay green.
- Runtime proof: `dumpbin /headers` / byte-scan of any Windows `.exe` shows
  `.mfbnote` carrying `MFBasic\0`; `readelf -n` / `otool -l` still show the unchanged
  PT_NOTE/LC_NOTE.
- Doc sync: `13_provenance-marker.md` (PE arm) + `10_windows-x86_64.md` (Sections
  table gains `.mfbnote`).
- Full suite: `cargo test --bin mfb`, `scripts/artifact-gate.sh` (Windows goldens
  regenerated; ELF/Mach-O unchanged).

## Summary

A one-section addition — a ~15-line copy of the `.rsrc` block wrapping the shared
`note.rs` bytes — but **unconditional**, so the real work is regenerating every
Windows acceptance golden for the added section. The shared descriptor and the
ELF/Mach-O note mechanisms are untouched. Best landed together with bug-432 so the
Windows goldens regenerate once for both new sections.
