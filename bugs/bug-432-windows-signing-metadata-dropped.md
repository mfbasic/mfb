# bug-432: Windows PE linker omits the ELF/Mach-O executable metadata (signing_metadata + provenance marker)

Last updated: 2026-08-08
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness / Footgun

Status: Open
Regression Test: src/os/windows/link/tests.rs (new `signing_metadata_emits_mfbsign_section`, `provenance_marker_emitted_unconditionally`), src/os/linux/link/tests.rs, src/os/macos/link/tests.rs

The Windows PE linker emits only functional sections (`.text`/`.rdata`/`.data`/
`.idata`/`.rsrc`) and carries **none** of the executable metadata the ELF and Mach-O
linkers attach. Two distinct payloads are missing:

**Defect A — `signing_metadata` (conditional).** The CLI threads executable signing
metadata (the `mfb-signing-v1` JSON blob) into `EncodedImage.signing_metadata` for
**every** target, Windows included (`src/target/win_x86_64/mod.rs:279`). Linux/macOS
emit it as a dedicated section/segment; the Windows PE linker never reads the field,
so on a **signed** Windows build the metadata is **silently dropped** — the `.exe`
ships unsigned with no error.

**Defect B — the `MFBasic\0` provenance marker (unconditional).** Every executable
the ELF and Mach-O linkers emit carries an unconditional vendor note (plan-43) — the
`MFBasic\0` owner plus a versioned 16-byte descriptor from `src/os/note.rs` — so a
tool (or the runtime) can positively identify an mfb-produced binary and read the
compiler version. The Windows PE writer imports nothing from `note.rs` and emits no
marker, so **every** Windows `.exe` lacks it. This is broader than Defect A: it
affects every Windows build, not just signed ones.

The single correct behavior a fix produces: (A) a signed Windows build emits the
signing blob in an `.mfbsign` PE section (byte-verbatim, as Linux/macOS do), and —
per the directive that motivated this bug — **all three backends use the same
8-character section name `.mfbsign`** so a future single reader can locate the blob
by one name across every format; and (B) every Windows `.exe` unconditionally
carries the `MFBasic\0` provenance marker (same descriptor bytes as ELF/Mach-O) in a
dedicated PE section. A build that vendors/signs nothing keeps its functional
sections byte-identical to today apart from the newly-added unconditional marker
section.

References:

- **Defect A** data model + emission: `src/arch/image.rs:38` (`signing_metadata:
  Option<Vec<u8>>`), Linux `src/os/linux/link/elf.rs:387` (`append_elf_signing_section`),
  macOS `src/os/macos/link/commands.rs:155` (`mfb_sign_segment`). Format contract:
  plan-23 / plan-23-A (`mfb-signing-v1` JSON blob, Ed25519); the blob is built by
  `src/cli/build/signing.rs:176` (`executable_signing_metadata_json`) — the linker
  treats it as opaque bytes.
- **Defect B** data model + emission: `src/os/note.rs:14` (`MFB_NOTE_OWNER =
  b"MFBasic\0"`), `:42` (`mfb_note_descriptor()` → the 16-byte descriptor: `MFB1`
  magic, version 1, flags, compiler major/minor/patch). Linux `PT_NOTE`
  `src/os/linux/link/elf.rs:6` (`mfb_note_bytes`), `:20` (`note_program_header`),
  emitted by all three encoders (`elf.rs:112,192,373`); macOS `LC_NOTE`
  `src/os/macos/link/commands.rs:181` (`note_command`), `macho.rs:165`. Both are
  **unconditional** (plan-43 `planning/completed/plan-43-binary-magic-marker.md:6`).
- plan-47-C `planning/completed/plan-47-C-pe-coff-writer.md` deferred
  `signing_metadata` as out of scope for the PE writer (lines 507, 552); plan-43
  scoped the marker to ELF + macOS only (`:42`), predating the PE backend.
- Spec prose to update — Defect A section names → `.mfbsign`:
  `src/docs/spec/linker/08_linux-x86_64.md:152` (`.mfb_sign`),
  `src/docs/spec/linker/06_macos-aarch64.md:24,113` (`__MFB,__sign`), aarch64/riscv64
  siblings. Defect B "both formats" claim: `src/docs/spec/linker/13_provenance-marker.md:3,23`
  (says "both formats" / "Every executable either in-tree linker emits" — silently
  excludes the third, PE, emitter).
- Both defects are **write-only** (no in-repo reader parses either back out — the
  ELF/Mach-O tests only byte-scan for presence).
- Found during the spec work that added `src/docs/spec/linker/10_windows-x86_64.md`;
  see the sibling `bugs/bug-431-windows-vendored-native-libraries-nonfunctional.md`
  for the parallel `rpaths` drop.

## Failing Reproduction

`signing_metadata` is opaque bytes at the linker layer, so the reproduction is a
unit test on the Windows linker (there is no runtime verifier anywhere in-repo —
all three backends are write-only, so a byte-scan for the section is the check the
ELF/Mach-O tests already use).

```rust
// src/os/windows/link/tests.rs — add:
#[test]
fn signing_metadata_emits_mfbsign_section() {
    let mut img = image(vec![0xc3]); // ret
    img.signing_metadata = Some(b"{\"format\":\"mfb-signing-v1\"}".to_vec());
    let bytes = write_executable(&img, false, None, None).expect("link");
    // Expect a section named ".mfbsign" whose body contains the blob.
    assert!(section_named(&bytes, b".mfbsign").is_some());
}
```

- Observed: no `.mfbsign` section exists; `write_executable`
  (`src/os/windows/link/mod.rs:280`) never references `image.signing_metadata`, so
  the blob is dropped and the test fails.
- Expected: the section is present and carries the blob verbatim.

And Defect B — every Windows `.exe` lacks the marker (unconditional):

```rust
// src/os/windows/link/tests.rs — add:
#[test]
fn provenance_marker_emitted_unconditionally() {
    let bytes = write_executable(&image(vec![0xc3]), false, None, None).expect("link");
    // Even a bare console image must carry the MFBasic\0 provenance marker.
    assert!(find_bytes(&bytes, b"MFBasic\0").is_some());
}
```

- Observed: no `MFBasic\0` owner anywhere in the image; the PE writer imports
  nothing from `src/os/note.rs`. Fails.
- Expected: the marker is present in every Windows `.exe`, as on ELF/Mach-O.

Contrast (works today, becomes the regression guards):

- Linux `src/os/linux/link/tests.rs:294` byte-scans for `.mfb_sign` (signing) and
  `:1229` for the `MFBasic\0` note (provenance).
- macOS `src/os/macos/link/tests.rs:322` byte-scans for `__MFB`/`__sign` (signing)
  and `:391` for the note (provenance).

The signing scans pass with the *old* names today; after the rename they must
assert `.mfbsign`. The provenance scans already pass on Linux/macOS and are the
cross-platform guard the Windows path must join.

## Root Cause

Both defects are the same omission: `src/os/windows/link/mod.rs:write_executable`
consumes only `image.text`, `image.data`, `image.rodata_size`, `image.imports`,
`image.symbols`, `image.relocations`, and `image.entry`, and pushes only functional
sections.

**Defect A:** it neither emits nor rejects `signing_metadata` — the field is
populated on the in-memory image (`src/target/win_x86_64/mod.rs:279`) and then never
read. Linux/macOS branch on `image.signing_metadata.is_some()` and append a
section/segment (`elf.rs:387`, `macho.rs:120`). The name divergence is incidental
history: ELF uses `.mfb_sign` (9 chars, fine in an ELF string table), macOS uses
segment `__MFB` + section `__sign` (Mach-O convention). PE section-header names are a
fixed **8-byte** field — `src/os/windows/link/pe.rs:92` (`section_name`) silently
truncates to 8 — so `.mfb_sign` (9) would become `.mfb_sig`. Choosing the 8-char
`.mfbsign` for all three removes the truncation trap and unifies the locator name.

**Defect B:** the PE writer never imports `src/os/note.rs` and never emits the
`MFBasic\0` note. On ELF the note is an unconditional `PT_NOTE` in every encoder
(`elf.rs:112,192,373`); on Mach-O an unconditional `LC_NOTE` (`macho.rs:165`); PE has
**no** `PT_NOTE`/`LC_NOTE` equivalent, so the marker was simply never ported — plan-43
predated the PE backend and scoped itself to ELF + Mach-O (`plan-43:42`). The
descriptor bytes are already format-neutral (`note.rs:mfb_note_descriptor()` returns
a raw 16-byte `Vec`), so only a PE carrier is missing.

## Goal

Defect A (signing):
- A signed Windows build (`image.signing_metadata == Some(blob)`) emits an
  `.mfbsign` PE section, read-only initialized data (`SCN_RDATA`), whose body is
  the blob verbatim (no PE-specific header), placed so it shifts no functional RVA.
- Linux emits the same blob in a section named `.mfbsign` (renamed from
  `.mfb_sign`); macOS emits it in a Mach-O section named `.mfbsign` (renamed from
  `__sign`).
- An unsigned build carries no `.mfbsign` section.
- The three name-scan tests assert `.mfbsign`; the spec prose names `.mfbsign`.

Defect B (provenance marker):
- **Every** Windows `.exe` carries the `MFBasic\0` provenance marker
  unconditionally, with the same 16-byte descriptor bytes ELF/Mach-O emit
  (`note.rs:mfb_note_descriptor()`), in a dedicated read-only PE section.
- ELF/Mach-O provenance emission is unchanged (they keep `PT_NOTE`/`LC_NOTE`).

### Non-goals (must NOT change)

- **No Authenticode / OS-level signing.** This is MFBASIC's own `mfb-signing-v1`
  blob, not a PKCS#7 certificate. Do NOT route it through the PE Security data
  directory `[4]` — that slot expects a `WIN_CERTIFICATE` Windows itself parses,
  and using it would be malformed and would collide with any future real
  Authenticode support. OS-recognized signing is a separate, later feature.
- **No payload/descriptor format change.** Defect A keeps the exact `mfb-signing-v1`
  JSON bytes (`src/cli/build/signing.rs`), no length prefix/header (Linux/macOS store
  the raw blob — keep that). Defect B keeps the exact 16-byte descriptor from
  `src/os/note.rs`; do NOT fork a PE-specific descriptor — the single
  `mfb_note_descriptor()` stays the source of truth for all three formats.
- **Do NOT change how ELF/Mach-O carry the provenance marker.** They keep
  `PT_NOTE`/`LC_NOTE`; this bug only *adds* a PE carrier. There is no cross-format
  section-name unification for the marker (unlike the signing section) because
  ELF/Mach-O use note commands, not named sections.
- **Functional sections stay byte-identical.** Unsigned builds gain no `.mfbsign`;
  the new provenance section is the only unconditional addition to a Windows `.exe`,
  and `.text`/`.rdata`/`.data`/`.idata`/`.rsrc` bytes are otherwise unchanged. (The
  provenance section does grow `NumberOfSections`/`SizeOfImage` on Windows — an
  intended, documented change, not a regression.)
- **macOS signing section stays mapped** and covered by the ad-hoc code signature
  (its page hashes must cover it) — only the name changes, not the mapped status.
- Do NOT "fix" a Windows test by asserting a payload is dropped; the broken path is
  the missing emission, and each test must exercise the emitted section/marker.

## Blast Radius

Found by grep for `mfb_sign` / `__sign` / `signing_metadata` / `note` / `MFBasic`
across `src/`:

Defect A (signing):
- `src/os/windows/link/mod.rs:write_executable` — the drop; **fixed** (add the
  `.mfbsign` section).
- `src/os/linux/link/elf.rs:387` (`append_elf_signing_section`) and its shstrtab
  literal `b"\0.mfb_sign\0.shstrtab\0"` (`elf.rs:399`) — **renamed** to `.mfbsign`.
- `src/os/macos/link/commands.rs:155` (`mfb_sign_segment`) and `macho.rs:120,202` —
  **renamed** section name to `.mfbsign` (segment-name decision below).
- Signing name-scan tests: `src/os/linux/link/tests.rs:294`,
  `src/os/macos/link/tests.rs:322` — **updated** to `.mfbsign`.
- Spec prose naming the signing sections — `08_linux-x86_64.md:152`,
  `06_macos-aarch64.md:24,113`, aarch64/riscv64 siblings — **updated**.

Defect B (provenance marker):
- `src/os/windows/link/mod.rs:write_executable` — **fixed** (add an unconditional
  marker section importing `mfb_note_descriptor`/`MFB_NOTE_OWNER` from
  `src/os/note.rs`).
- `src/os/note.rs` — **consumed unchanged**: the shared descriptor is already
  format-neutral; the PE carrier reuses `mfb_note_descriptor()` verbatim. Do NOT edit
  the descriptor.
- `src/os/linux/link/elf.rs` `PT_NOTE` / `src/os/macos/link/commands.rs` `LC_NOTE` —
  **unaffected**: ELF/Mach-O keep their note mechanism.
- Provenance presence tests: `src/os/linux/link/tests.rs:1229`,
  `src/os/macos/link/tests.rs:391` — **joined** by a new Windows equivalent.
- Spec `13_provenance-marker.md:3,23` ("both formats") — **updated** to add a PE
  section (the third emitter); `10_windows-x86_64.md` gains a provenance note.

Shared:
- Acceptance goldens: **audit needed** in Phase 1. Signing sections shift a golden
  only if that fixture *signs* (likely none do). The provenance section is
  **unconditional**, so it shifts the byte output / `NumberOfSections` /
  `SizeOfImage` of **every Windows** acceptance fixture — the Windows golden delta is
  expected and must be regenerated; ELF/Mach-O goldens are untouched by Defect B.
- Repository-crate signing (`repository/src/crypto.rs`) — **unaffected**: package
  trust, a different subsystem, not executable sections.

## Fix Design

Both defects use the same vehicle: a **read-only PE section** (`SCN_RDATA`) pushed
into the section list in `write_executable` exactly like `.rsrc` (not the Security
directory, not an overlay — see rejected alternatives). Each is placed after the
functional sections at the next `SECTION_ALIGNMENT`/`FILE_ALIGNMENT` boundary, with
**no** data-directory entry. Neither need be mapped-and-signed on Windows; a plain
read-only section is simplest and is forward-compatible with future Authenticode
(which would hash it as part of the image).

- **Defect A — `.mfbsign`:** `has_sign = image.signing_metadata.is_some()`; body is
  the blob verbatim. Rename the ELF/Mach-O sections to `.mfbsign` too.
- **Defect B — `.mfbnote`:** **unconditional** (always pushed); body is
  `MFB_NOTE_OWNER` (`MFBasic\0`) followed by `mfb_note_descriptor()` — the same
  owner+descriptor framing the ELF note uses, so a reader locates `MFBasic\0` and
  reads the 16-byte descriptor in any of the three formats. `.mfbnote` is 8 chars
  (fits PE's field). Import the two symbols from `src/os/note.rs`.

Rejected alternatives (do not re-litigate):

- **Security data directory `[4]` / Authenticode** — wrong container for an Ed25519
  JSON blob; see Non-goals.
- **Trailing overlay** — the PE writer computes `total_file_size` from the section
  list and pads to it, so overlay bytes would sit outside the section table and need
  a bespoke locator. A section is self-locating and reuses the existing machinery.
  (plan-43 §3 rejected a `strip`-droppable ELF section for exactly this
  discoverability reason; on PE a real section in the table is the discoverable form.)
- **Rich header / `.rsrc` resource for the marker** — the Rich header has no room for
  a self-describing descriptor; `.rsrc` exists only in GUI builds, so it could not
  make the marker unconditional.

Name choices: `.mfbsign` / `.mfbnote` are each exactly 8 chars to fit PE's 8-byte
section-name field with no truncation; ELF/Mach-O section-name fields are wider so
`.mfbsign` fits them trivially.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [ ] Add `signing_metadata_emits_mfbsign_section` (+ a `None`-image assertion that
      no `.mfbsign` appears) and `provenance_marker_emitted_unconditionally` to
      `src/os/windows/link/tests.rs`. Confirm both fail.
- [ ] Audit acceptance/artifact-gate fixtures: (a) does any build sign? (Defect A
      golden impact.) (b) Confirm the Defect B provenance section shifts **every**
      Windows fixture (unconditional) and record the regeneration scope. Write both
      verdicts into Blast Radius.

Acceptance: both new Windows tests fail for the documented reasons; the golden-impact
verdicts are recorded.
Commit: —

### Phase 2 — the fix (both defects) + signing rename

- [ ] `src/os/windows/link/mod.rs`: emit the `.mfbsign` section from
      `image.signing_metadata` (mirror `.rsrc`; `SCN_RDATA`; no data directory;
      remove the silent drop), and **unconditionally** emit the `.mfbnote` section
      (`MFB_NOTE_OWNER` + `mfb_note_descriptor()` from `src/os/note.rs`).
- [ ] `src/os/linux/link/elf.rs`: rename `.mfb_sign` → `.mfbsign` (section name +
      shstrtab literal). Keep it non-alloc PROGBITS at EOF. (PT_NOTE unchanged.)
- [ ] `src/os/macos/link/{commands.rs,macho.rs}`: rename the signing section `__sign`
      → `.mfbsign` (keep it mapped). Segment-name decision: see Open Decisions.
      (LC_NOTE unchanged.)
- [ ] Update the signing name-scan tests to `.mfbsign`.

Acceptance: all four signing tests pass with `.mfbsign`; the Windows provenance test
passes; the ELF/Mach-O provenance tests still pass; nothing in Non-goals changed.
Commit: —

### Phase 3 — docs + regenerate goldens + full validation

- [ ] Docs: signing prose → `.mfbsign` (`08_linux-x86_64.md`, `06_macos-aarch64.md`,
      aarch64/riscv64 siblings); `13_provenance-marker.md` gains a "PE: `.mfbnote`
      section" arm and drops the "both formats" wording for "all three in-tree
      linkers"; `10_windows-x86_64.md` documents both new sections. Run the spec
      drift-guard tests.
- [ ] `scripts/artifact-gate.sh`: regenerate the **Windows** goldens (the unconditional
      `.mfbnote` shifts every Windows fixture); confirm the delta is exactly the
      section addition and that ELF/Mach-O goldens are untouched (unless a fixture
      signs).
- [ ] `cargo test --bin mfb` full suite.
- [ ] If a Windows/Wine runner is available, confirm a signed test `.exe` has a
      `.mfbsign` section and every `.exe` has `.mfbnote` carrying `MFBasic\0`
      (`dumpbin /headers` or a byte-scan).

Acceptance: full suite green; the Windows golden delta is exactly the two-section
addition; the reproductions pass.
Commit: —

## Validation Plan

- Regression test(s): `signing_metadata_emits_mfbsign_section` +
  `provenance_marker_emitted_unconditionally` (`src/os/windows/link/tests.rs`), the
  renamed ELF/Mach-O signing name-scan tests, and the existing ELF/Mach-O provenance
  presence tests (must stay green).
- Runtime proof: `dumpbin /headers` / byte-scan of a Windows `.exe` shows `.mfbnote`
  (with `MFBasic\0`) on every build and `.mfbsign` on a signed build; `readelf -S/-n`
  / `otool -l` show the renamed signing section and the unchanged PT_NOTE/LC_NOTE.
- Doc sync: signing spec pages (`08`/`06` + siblings), `13_provenance-marker.md`
  (add the PE arm, drop "both formats"), and `10_windows-x86_64.md` (Sections table
  gains `.mfbsign` + `.mfbnote`).
- Full suite: `cargo test --bin mfb`, `scripts/artifact-gate.sh` (Windows goldens
  regenerated for the unconditional `.mfbnote`; ELF/Mach-O unchanged).

## Open Decisions

- **macOS segment name.** The Mach-O section is `(segname, sectname)` =
  `(__MFB, __sign)`. Renaming the *section* to `.mfbsign` satisfies "same name" for
  a byte-scanning reader. Recommend: rename `sectname` → `.mfbsign`, keep `segname`
  `__MFB` (a segment is the natural grouping and `__`-prefixed names are the Mach-O
  convention). Alternative: set both to `.mfbsign`. Pick one and note it in the
  STATUS block. (§Fix Design)

## Summary

Two same-shaped omissions: the Windows PE writer carries none of the executable
metadata ELF/Mach-O attach. Both emissions are ~15-line copies of the `.rsrc`
section block. The engineering risk is in the **golden blast radius**, and it
differs by defect: the signing rename shifts bytes only on *signed* fixtures
(likely none — Phase 1 settles it), while the **unconditional** `.mfbnote` marker
shifts **every Windows** acceptance golden by exactly one section — an expected
regeneration, with ELF/Mach-O goldens untouched. The signing blob, the shared note
descriptor, the ELF/Mach-O note mechanisms, and functional-section output are all
left intact.
