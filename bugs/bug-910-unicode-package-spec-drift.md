# bug-910: unicode + package spec drift — stale fold/runtime "differences", `author` called informational though trust-bound, path-component claim overstated, lenient flags undocumented

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec accuracy)

Status: Open
Regression Test: none (doc-only; verify by rendering `mfb spec unicode` / `mfb spec package`)

**Owner decision (2026-10-05):** for the two open items. (1) **Function kind ≠ 1 is rejected (CODE change).** The package
verifier rejects a function-table entry with an unknown kind as an invalid package, with
the documented package-invalid diagnostic, before any consumer reads it. (2) **A newer
format major gets its own message (CODE change).** A package whose format major is newer
than this compiler supports says the package needs a newer compiler. An older major keeps
"predates … must be rebuilt". The other four items are plain spec fixes.

Six spec claims in the B10 topics disagree with measured behavior. In each case the code
looks deliberate and the spec text is what is wrong (items 4 and 6 are UNCLEAR: the owner
must decide which side moves).

References: `src/docs/spec/unicode/spec.md`, `src/docs/spec/unicode/01_tables-and-algorithms.md`,
`src/docs/spec/package/01_container-format.md`, `src/docs/spec/package/07_functions.md`,
`src/docs/spec/package/11_doc-section.md`, `src/docs/spec/package/12_verifier-rules.md`.

## Items

### 1. SPEC — unicode: claims normalization and Indic fold/runtime differences that no longer exist

- `unicode/spec.md` (Unicode Runtime Model, line 16): "Their current normalization and Indic
  grapheme-context differences are specified in `tables-and-algorithms`."
- `unicode/01_tables-and-algorithms.md` (Runtime algorithm: grapheme segmentation, line 298):
  "based on UAX #29, with the Indic context limitation described below".
- But the same topic later says both agree: "Runtime and folded normalization agree" and a
  leading linker gives "two clusters, as on the folded Rust path".
- Measured: runtime `normalizeNfc` matches all 100,170 NormalizationTest 17.0.0 NFC cases
  and all 99,825 16.0.0 cases (`python3 /tmp/hardening/B10/ntest.py ucd17` → `bad 0`);
  runtime graphemes pass all 766 GraphemeBreakTest 17.0.0 cases incl. every GB9c case
  (`UCD=ucd17 python3 /tmp/hardening/B10/gtest.py` → `bad 0`). Fix: drop both sentences
  (the remaining real divergence is the separate unassigned-code-point bug).

### 2. SPEC — container-format: `author` is "Informational" but the trust chain binds it

- `package/01_container-format.md` header-fields table: "`author` — Informational author string."
  `package/12_verifier-rules.md` step 2/3 lists the attestation/proof fields that "pin this
  exact package" without saying what `owner` is compared to.
- Code: `repository/src/package.rs:expect_signed_owner` requires the signed `owner` of both
  proof and attestation to equal the header `author`, case-folded (plan-168-B).
- Measured with a self-made key chain (`/tmp/hardening/B10/signed.py`, `m17.py`, `m19.py`):
  flipping one byte of `author` (`ada`→`adb`) after signing →
  `[Tampered] PACKAGE_ATTESTATION_INVALID (attestation owner does not match the package author)`;
  attestation `owner: "ADA"` with author `ada` → `[Verified]`. Fix: document `author` as
  the publisher identity bound by proof/attestation `owner` (case-folded).

### 3. SPEC — container-format: path-component `name` rule "enforced ... when its header is read" is only true for one reader

- `package/01_container-format.md`: "`name` is additionally constrained to a single safe
  path component ... enforced both when a package is built and when its header is read, so
  a crafted name such as `../../x` cannot escape".
- Measured: a package whose header + manifest name is `../x` (`/tmp/hardening/B10/evil.mfp`,
  built by `m3.py`) placed at `packages/geom.mfp` is classified `uses geom - [Unsigned]` by
  the build gate (`repository/src/package.rs:parse_mfp_package` does not apply
  `validate_path_component_name`), and the import reader (`src/binary_repr/reader.rs:mfp_binary_repr_payload`)
  decodes it; the build only fails later on identity/type errors. `mfb pkg add file://…/evil.mfp`
  does reject it (`package name ../x is not a valid path component`) via
  `src/manifest/package.rs:read_mfp_header`. No escape is possible because consumers name the
  install path from their own manifest, but the sentence should say the rule is enforced
  at build and at `pkg add`/install, not by every header reader.

### 4. UNCLEAR — functions: "exported precisely when it is kind 1" vs reader accepting kind 2

- `package/07_functions.md`: "A function is exported precisely when it is kind `1` and the
  private bit is clear (`is_exported_function`)".
- Measured (`/tmp/hardening/B10/m7.py`, "add kind=2"): patching exported `add`'s kind to `2`
  is accepted; the consumer still calls `geom::add` and builds/runs. The reader neither
  rejects unknown function kinds nor drops the export. Either reject kinds ≠ 1 or document
  that the reader ignores `kind`.

### 5. SPEC — doc-section: `hasPackage (0 or 1)` — any non-zero byte is accepted as 1

- `package/11_doc-section.md`: "`u8 hasPackage (0 or 1)`".
- Code `wire/src/docs.rs:read_doc_table` tests `has_package == 0`; measured: DOC with
  `hasPackage = 2` builds (`m21.py`, "build DOC hasPackage=2 rc=0"). The `internal` byte is
  read the same way (`!= 0`). Document "non-zero = present" or reject >1.

### 6. UNCLEAR — container-format: MFPC major-version message says "predates" for a newer major

- `package/01_container-format.md` quotes the message
  `unsupported MFPC major version <n> (expected 2); this package predates the structured Binary Representation format and must be rebuilt`.
- Measured with `bcMajor = 3` (`m4.py`): the same "predates ... must be rebuilt" text is printed
  for a package from a *newer* format. Message (and quoted spec) should distinguish n < 2 from n > 2.

## Goal

- Each quoted sentence above is corrected (or, for 4 and 6, the owner decides and code or text follows).
