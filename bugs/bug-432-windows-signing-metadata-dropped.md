# bug-432: Windows PE linker silently drops executable signing_metadata

Last updated: 2026-08-08
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness / Footgun

Status: Open
Regression Test: src/os/windows/link/tests.rs (new `signing_metadata_emits_mfbsign_section`), src/os/linux/link/tests.rs, src/os/macos/link/tests.rs

The CLI threads executable signing metadata (the `mfb-signing-v1` JSON blob) into
`EncodedImage.signing_metadata` for **every** target, Windows included
(`src/target/win_x86_64/mod.rs:279`). The Linux and macOS linkers emit it as a
dedicated section/segment; the Windows PE linker never reads the field, so on a
signed Windows build the metadata is **silently dropped** — the `.exe` ships
unsigned with no error. A user who runs `mfb build` with signing configured gets a
provenance-bearing binary on Linux/macOS and a bare one on Windows, with nothing to
tell them the difference.

The single correct behavior a fix produces: a signed Windows build emits the blob
in an `.mfbsign` PE section (byte-verbatim, as Linux/macOS do), and — per the
directive that motivated this bug — **all three backends use the same 8-character
section name `.mfbsign`** so a future single reader can locate the blob by one name
across every format. An unsigned build (the common case) stays byte-identical to
today on all three platforms.

References:

- Data model + emission recipes: `src/arch/image.rs:38` (`signing_metadata:
  Option<Vec<u8>>`), Linux `src/os/linux/link/elf.rs:387` (`append_elf_signing_section`),
  macOS `src/os/macos/link/commands.rs:155` (`mfb_sign_segment`).
- Format contract: plan-23 / plan-23-A (`mfb-signing-v1` JSON blob, Ed25519 trust
  model); the blob is built by `src/cli/build/signing.rs:176`
  (`executable_signing_metadata_json`) — the linker treats it as opaque bytes.
- plan-47-C `planning/completed/plan-47-C-pe-coff-writer.md` deferred
  `signing_metadata` as out of scope for the PE writer (lines 507, 552).
- Spec prose that names the current sections (must be updated to `.mfbsign`):
  `src/docs/spec/linker/08_linux-x86_64.md:152` (`.mfb_sign`),
  `src/docs/spec/linker/06_macos-aarch64.md:24,113` (`__MFB,__sign`), and the
  aarch64/riscv64 siblings that share the ELF text.
- Found during the spec work that added `src/docs/spec/linker/10_windows-x86_64.md`.
  Sibling bugs from the same Windows-PE metadata gap:
  `bugs/bug-431-windows-vendored-native-libraries-nonfunctional.md` (the parallel
  `rpaths` drop) and `bugs/bug-433-windows-provenance-marker-missing.md` (the
  unconditional `MFBasic\0` marker the PE writer also omits — same `.rsrc`-style
  section vehicle, so land them together).

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

Contrast (works today, becomes the rename regression guards):

- Linux `src/os/linux/link/tests.rs:294` byte-scans the output for `.mfb_sign`.
- macOS `src/os/macos/link/tests.rs:322` byte-scans for `__MFB` / `__sign`.

Both pass today with the *old* names; after the rename they must assert `.mfbsign`.

## Root Cause

`src/os/windows/link/mod.rs:write_executable` consumes only `image.text`,
`image.data`, `image.rodata_size`, `image.imports`, `image.symbols`,
`image.relocations`, and `image.entry`. It has explicit rejections for unsupported
relocations (`mod.rs:265`) but neither emits nor rejects `signing_metadata` — the
field is populated on the in-memory image (`src/target/win_x86_64/mod.rs:279`) and
then never read. The Linux/macOS linkers, by contrast, branch on
`image.signing_metadata.is_some()` and append a section/segment
(`elf.rs:387`, `macho.rs:120`).

The name divergence is incidental history: ELF uses `.mfb_sign` (9 chars, fine in
an ELF string table), macOS uses segment `__MFB` + section `__sign` (Mach-O
convention). PE section-header names are a fixed **8-byte** field —
`src/os/windows/link/pe.rs:92` (`section_name`) silently truncates to 8 — so
`.mfb_sign` (9) would become `.mfb_sig`. Choosing the 8-char `.mfbsign` for all
three removes the truncation trap and unifies the locator name.

## Goal

- A signed Windows build (`image.signing_metadata == Some(blob)`) emits an
  `.mfbsign` PE section, read-only initialized data (`SCN_RDATA`), whose body is
  the blob verbatim (no PE-specific header), placed last so it shifts no other RVA.
- Linux emits the same blob in a section named `.mfbsign` (renamed from
  `.mfb_sign`); macOS emits it in a Mach-O section named `.mfbsign` (renamed from
  `__sign`).
- An unsigned build is byte-identical to today on all three platforms.
- The three name-scan tests assert `.mfbsign`; the spec prose names `.mfbsign`.

### Non-goals (must NOT change)

- **No Authenticode / OS-level signing.** This is MFBASIC's own `mfb-signing-v1`
  blob, not a PKCS#7 certificate. Do NOT route it through the PE Security data
  directory `[4]` — that slot expects a `WIN_CERTIFICATE` Windows itself parses,
  and using it would be malformed and would collide with any future real
  Authenticode support. OS-recognized signing is a separate, later feature.
- **No blob format change.** The bytes stay the exact `mfb-signing-v1` JSON the CLI
  produces (`src/cli/build/signing.rs`); the linker keeps treating them as opaque.
  No length prefix / header is added on any platform (Linux/macOS store the raw
  blob today — keep that).
- **Unsigned builds stay byte-identical** on every platform (the console-vs-signed
  invariant the acceptance goldens depend on).
- **macOS section stays mapped** and covered by the ad-hoc code signature (its page
  hashes must cover it) — only the name changes, not the segment's mapped status.
- Do NOT "fix" the Windows test by asserting the field is dropped; the broken path
  is the dropped emission, and the test must exercise the emitted section.

## Blast Radius

Found by grep for `mfb_sign` / `__sign` / `signing_metadata` across `src/`:

- `src/os/windows/link/mod.rs:write_executable` — the drop; **fixed by this bug**
  (add the `.mfbsign` section).
- `src/os/linux/link/elf.rs:387` (`append_elf_signing_section`) and its shstrtab
  literal `b"\0.mfb_sign\0.shstrtab\0"` (`elf.rs:399`) — **renamed** to `.mfbsign`.
- `src/os/macos/link/commands.rs:155` (`mfb_sign_segment`) and `macho.rs:120,202` —
  **renamed** section name to `.mfbsign` (segment-name decision below).
- Name-scan tests: `src/os/linux/link/tests.rs:294`,
  `src/os/macos/link/tests.rs:322` — **updated** to `.mfbsign`.
- Spec prose naming the sections — `src/docs/spec/linker/08_linux-x86_64.md:152`,
  `06_macos-aarch64.md:24,113`, and the aarch64/riscv64 ELF siblings — **updated**.
- Acceptance goldens: **audit needed** — do any acceptance/artifact-gate fixtures
  build with signing enabled? If none sign, the ELF/Mach-O byte output is unchanged
  for every golden (the signing section only appears on a signed build), so the
  rename shifts no golden. If any golden signs, its bytes shift by exactly the name
  delta. Record the verdict in Phase 1.
- `src/os/note.rs` (the `MFBasic\0` `LC_NOTE`/`PT_NOTE` provenance marker) —
  **out of scope here**: separate, unconditional payload tracked in bug-433, but it
  shares this bug's PE-section vehicle, so the two should land in one pass.
- Repository-crate signing (`repository/src/crypto.rs`) — **unaffected**: package
  trust, a different subsystem, not executable sections.

## Fix Design

A read-only PE section is the right analog (not the Security directory, not an
overlay — see Non-goals and the rejected alternatives). Mirror the existing `.rsrc`
wiring in `write_executable`: compute `has_sign = image.signing_metadata.is_some()`,
add it to `section_count`, place it last (after `.idata`/`.rsrc`, at the next
`SECTION_ALIGNMENT`/`FILE_ALIGNMENT` boundary), push a `Section { name:
section_name(".mfbsign"), characteristics: SCN_RDATA, .. }`, and set **no** data
directory. Unlike macOS the section need not be mapped-and-signed; a plain
read-only section is simplest and is forward-compatible with future Authenticode
(which would just hash it as part of the image).

Rejected alternatives (do not re-litigate):

- **Security data directory `[4]` / Authenticode** — wrong container for an Ed25519
  JSON blob; see Non-goals.
- **Trailing overlay** (mirroring ELF's non-alloc, EOF-appended section) — the PE
  writer computes `total_file_size` from the section list and pads to it, so overlay
  bytes would sit outside the section table and need a bespoke locator. A section is
  self-locating and reuses the existing machinery.

Name unification: `.mfbsign` (exactly 8 chars) is chosen to fit PE's 8-byte section
name field with no truncation. ELF/Mach-O section-name fields are wider, so `.mfbsign`
fits them trivially.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `signing_metadata_emits_mfbsign_section` to `src/os/windows/link/tests.rs`
      (above), plus a `None`-image byte-identity assertion. Confirm the first fails.
- [ ] Audit acceptance/artifact-gate fixtures for any signed build; write the
      verdict (does any golden carry a signing section?) into Blast Radius.

Acceptance: the new Windows test fails for the documented reason; the golden-impact
verdict is recorded.
Commit: —

### Phase 2 — the fix + rename

- [ ] `src/os/windows/link/mod.rs`: emit the `.mfbsign` section from
      `image.signing_metadata` (mirror the `.rsrc` block; `SCN_RDATA`; last; no data
      directory). Remove the silent drop.
- [ ] `src/os/linux/link/elf.rs`: rename `.mfb_sign` → `.mfbsign` (section name +
      the shstrtab literal). Keep it non-alloc PROGBITS at EOF.
- [ ] `src/os/macos/link/{commands.rs,macho.rs}`: rename the section name `__sign`
      → `.mfbsign` (keep it mapped). Segment-name decision: see Open Decisions.
- [ ] Update the three name-scan tests to `.mfbsign`.

Acceptance: all four backends' signing tests pass with `.mfbsign`; unsigned builds
unchanged; nothing in Non-goals changed.
Commit: —

### Phase 3 — docs + full validation

- [ ] Update spec prose to `.mfbsign` (`08_linux-x86_64.md`, `06_macos-aarch64.md`,
      the aarch64/riscv64 siblings); add the `.mfbsign` section to the new
      `10_windows-x86_64.md`. Run the spec drift-guard tests.
- [ ] `cargo test --bin mfb` full suite; `scripts/artifact-gate.sh` — confirm the
      golden delta is exactly the intended change (nothing if no golden signs).
- [ ] If a Windows/Wine runner is available, confirm a signed test `.exe` contains a
      `.mfbsign` section (`dumpbin /headers` or a byte-scan).

Acceptance: full suite green; golden deltas exactly the intended change; the
Windows reproduction passes.
Commit: —

## Validation Plan

- Regression test(s): `signing_metadata_emits_mfbsign_section`
  (`src/os/windows/link/tests.rs`) + the renamed ELF/Mach-O name-scan tests.
- Runtime proof: byte-scan / `dumpbin /headers` of a signed test `.exe` shows the
  `.mfbsign` section carrying the blob; ELF `readelf -S` / Mach-O `otool -l` show
  the renamed section on a signed build.
- Doc sync: the three (four incl. Windows) linker spec pages; the `10_windows-x86_64.md`
  Sections table gains `.mfbsign`.
- Full suite: `cargo test --bin mfb`, `scripts/artifact-gate.sh`.

## Open Decisions

- **macOS segment name.** The Mach-O section is `(segname, sectname)` =
  `(__MFB, __sign)`. Renaming the *section* to `.mfbsign` satisfies "same name" for
  a byte-scanning reader. Recommend: rename `sectname` → `.mfbsign`, keep `segname`
  `__MFB` (a segment is the natural grouping and `__`-prefixed names are the Mach-O
  convention). Alternative: set both to `.mfbsign`. Pick one and note it in the
  STATUS block. (§Fix Design)

## Summary

The engineering risk is almost entirely in the **rename's golden blast radius**:
the Windows emission is a ~15-line copy of the `.rsrc` block, but changing the ELF
and Mach-O section names shifts bytes on any *signed* acceptance fixture. The
Phase 1 audit settles whether that set is empty (likely — unsigned builds are the
norm), which determines whether Phase 3 regenerates any golden at all. The blob
format, the provenance marker, and unsigned-build output are all untouched.
