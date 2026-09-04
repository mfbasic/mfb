# audit-3 — Surface 1: untrusted `.mfp` decode + signature / IR verification

Part of `planning/goal-08-platform-security-review.md`. Finding prefix `PKG-`.

Untrusted party: the author of a `.mfp` artifact on the dependency path —
dropped in locally, or fetched/installed from a registry. They must not be able
to corrupt memory, crash, hang, or execute code in the compiler via a malformed
or hostile package, nor have unsigned / IR-tampered package contents accepted as
trusted.

**Verdict: hardened, one LOW.** Every audit-1/audit-2 finding on this surface is
re-verified fixed against current source. A 6,300-iteration mutation fuzz of the
decoder through a real `mfb build` produced zero panics, aborts, or hangs. The
one new finding is a time-of-check/time-of-use gap between the signature gate
and the reads that actually feed codegen.

## The four gate questions

### Q1 — Is a signature required and verified before a `.mfp`'s contents are used?

**Yes, on the build and test paths.** `build_project` calls
`verify_and_report_packages` at `src/cli/build/mod.rs:303`, and the comment there
names the finding it closes:

```
    // audit-1 PKG-01: verify every declared dependency's signature against a
    // project-pinned trust anchor before it is decoded, merged, or lowered, and
    // print a per-package verification report. A tampered signed dependency (or a
    // disallowed unsigned one) hard-fails the build with a non-zero exit.
    verify_and_report_packages(&options.location, &manifest, options.allow_unsigned)?;
```

It runs *before* the resolver, the shape pass, monomorph, and `merge_packages`.
Each declared dependency goes through `classify_installed_package`
(`src/cli/build/packages.rs:159`), which enforces the full plan-23 §3.5 chain
against the **project-pinned** `identKey` from `project.json` — never the
key embedded in the file being checked (`packages.rs:174-207`):

pinned server key → `verify_attestation` (`packages.rs:227`) → pinned ident →
`verify_proof` (`packages.rs:232`) → one-off signing key
`verify_package_signature` (`packages.rs:237`) → payload-hash weld
`verify_payload_hash` (`packages.rs:240`).

Any broken link yields `Tampered` and a fatal build. A signed package with no
pinned anchor is `Tampered` (`PACKAGE_IDENT_KEY_UNTRUSTED`). An unsigned package
is accepted only from a local source; unsigned + non-local requires `--unsigned`
(`packages.rs:108-118`). `mfb test` shares the path (`dispatch.rs:105` →
`build_project`).

**audit-1 PKG-01 is still fixed.** The locally-dropped unsigned case is
permitted without verification *by design* (local-dev policy), not a regression.

### Q2 — Is the decoded IR re-verified before monomorph/codegen?

**Yes.** `merge_packages` (`src/target/shared/nir/lower.rs:82`) runs
`crate::ir::verify_package` on each decoded package IR (`:90`) and
`crate::ir::verify_semantics` on the merged IR (`:108`) before `lower_module`.
`verify_semantics` is the same semantic checker source-lowered IR goes through:
it rebuilds a type environment and rejects member access on a primitive,
closure-capture indices past the slot count, call/constructor arity mismatch, a
union wrap naming a non-variant, an empty MATCH, and literal-range overflow.
**audit-1 PKG-02 is still fixed.**

### Q3 — Can a crafted `.mfp` panic / OOB / OOM / hang the decoder?

**Not found**, across code review and 6,300 mutated builds. The PKG-03..07
hardening from the prior audits is intact in current source:

| guard | location |
|---|---|
| IR-body recursion cap `MAX_DECODE_DEPTH` (`enter`/`leave`) | `src/ir/binary.rs:120-157` |
| type-graph `in_progress` cycle guard + `MAX_TYPE_GRAPH_DEPTH=256` | `src/binary_repr/reader.rs:718-737` |
| `bounded_capacity(count, remaining, min_elem)` on count-driven allocation | `src/binary_repr/util.rs:75-83` |
| `decode_vec` / `decode_vec_capped` | `src/ir/binary.rs:572-589, 792-806` |
| `checked_add`/`checked_mul`/`checked_usize` | `src/binary_repr/util.rs`, `reader.rs:392-401`, `src/ir/binary.rs:159-170` |
| duplicate-section rejection | `src/binary_repr/reader.rs:335-343` |

The single bare `Vec::with_capacity(count)` (`reader.rs:648`) is pre-bounded by
the `entries_end = 4 + count*20 <= bytes.len()` check immediately above it
(`reader.rs:637-647`).

**Fuzz evidence.** Corpus base
`tools/link-package-sources/collidera/collidera.mfp` (unsigned, so mutations
reach the decoder instead of being rejected at the signature gate), imported by
a scratch executable under `/tmp/`. Every run was a real
`./target/debug/mfb build` (debug asserts active). Panics detected by scanning
combined output for `panicked`/`RUST_BACKTRACE` and by any exit code outside
{0,1}; hangs by a 25 s timeout.

| harness | path exercised | runs | crashes | hangs |
|---|---|---|---|---|
| whole-file 1–4 byte flips, `-ast -ir` | outer container parse + type-export decode | 400 | 0 | 0 |
| inner MFPC payload flips, `-ast -ir` | `read_binary_repr_package` + section decoders | 400 | 0 | 0 |
| inner MFPC payload flips, full build | + IR-body decode + `verify_package`/`verify_semantics` | 3000 | 0 | 0 |
| 4-byte word → {0, MAX, …}, full build | biased at count/length/offset words (alloc + index paths) | 2500 | 0 | 0 |

Total 6,300 mutated builds, 0 panics / aborts / hangs. Random flips mostly fail
early length/hash/version checks; the word-corruption harness deliberately
maxes count/length/offset fields to reach the allocation and indexing paths,
and those held.

### Q4 — TOCTOU between verifying a package file and reading it?

**Yes — PKG-01 below.** LOW.

---

## PKG-01 — The signature gate and the codegen-feeding decode are separate, unsynchronised reads of the same path

- **Severity:** LOW
- **Location:**
  - `src/cli/build/packages.rs:159` — `classify_installed_package` → `std::fs::read(path)`: **the verified read**
  - `src/binary_repr/mod.rs:869` — `read_package_ir_with_identity` → `fs::read(path)`: **the read that feeds codegen**
  - `src/binary_repr/mod.rs:851` — `read_package_identity_id` → `fs::read(path)`
  - `src/binary_repr/reader.rs:256` — `read_package_binary_repr` → `fs::read(path)`: the resolver's type-export read
  - `src/manifest/package.rs:74` — a fourth read via `read_mfp_header`
- **Threat / impact:** integrity. A local actor with write access to the build
  tree's `packages/<name>.mfp` (or the `build/packages/` cache) *during* a
  `mfb build` / `mfb test` can have one set of bytes verified and a different
  set decoded, merged, and lowered into the victim's binary. The package author
  is **not** the threat here — they control the content at every read, so
  verification simply passes or fails on whatever is present.
- **Mechanism:** the `.mfp` is read from disk at least four independent times
  per build, and only one of those reads — `classify_installed_package`,
  inside `verify_and_report_packages` — performs the signature chain. The read
  that is actually lowered is a later, separate `fs::read` in
  `read_package_ir_with_identity`, called from `merge_packages`
  (`src/target/shared/nir/lower.rs:89`), which does only structural
  (`verify_package`) and semantic (`verify_semantics`) checks. The verified byte
  buffer in `classify_installed_package` is dropped at end of scope; nothing
  hands it to the decoder. So the bytes proven authentic are not the bytes
  compiled; they are bound only by the assumption that the file did not change
  in between. `merge_packages`' own comment states the reliance plainly —
  "`verify_package` re-states the package-format structural invariants …
  Decoded package IR is attacker controlled and never passed the source type
  checker" — i.e. it leans on the earlier pass for *authenticity*, not on
  re-verifying the bytes it holds.
- **Evidence:**
  - `grep -n 'fs::read' src/binary_repr/mod.rs src/binary_repr/reader.rs src/cli/build/packages.rs src/manifest/package.rs` → the five distinct read sites listed above.
  - `sed -n '845,875p' src/binary_repr/mod.rs` → `read_package_ir_with_identity` re-reads the path and calls only `mfp_binary_repr_payload` / `read_binary_repr_package` / `validate_container_manifest_identity`; no signature call.
  - `grep -rn 'verify_attestation\|verify_package_signature\|verify_proof\|verify_payload_hash' src/` → 16 hits in exactly three files: `src/cli/build/packages.rs` (the build gate), `src/cli/pkg.rs:1474-1504` (the separate `mfb pkg verify` command), and `src/target/package_mfp/mod.rs:413-475` (round-trip tests). **No signature call exists anywhere on the decode/merge/lower path.**
- **Reproduction:** not demonstrated as a live race — it needs a second local
  process winning a sub-second filesystem race against the build. Demonstrated
  *structurally*: the verified buffer and the lowered buffer are two independent
  `fs::read`s of the same path, with no shared cache and no re-verification on
  the second. Observed: signature verified over buffer A, code generated from
  buffer B. Expected: one read, verified and decoded.
- **MFB trigger program (spike):** **none possible.** The trigger is a second
  local process overwriting `packages/<name>.mfp` between two passes of the
  compiler; no `.mfb` source and no crafted package content can express it,
  because the defect is in *when* the file is read, not in what it contains. The
  structural evidence above stands in place of a spike.
- **Best fix:** read each dependency `.mfp` into memory **once**, verify the
  signature chain over that in-memory buffer, and thread the *same* buffer to
  the decoder — decode from bytes, never re-`fs::read` the path after
  verification. `binary_repr` already exposes byte-taking entry points
  (`package_info_from_mfp`, `mfp_binary_repr_payload`), so plumbing an
  `Arc<[u8]>` from the verify pass into `merge_packages` and the resolver closes
  the window with no format change. A cheaper alternative that also closes it:
  carry the verified `packageBinaryHash` forward and re-check it against the
  merge-time read.
- **Non-goals:** no `.mfp` wire-format change; keep the local-unsigned policy
  (unsigned *local* dependencies still build without `--unsigned`); do not
  weaken `verify_semantics` — it is the memory-safety net independent of this.
- **Prior art:** none. Searched `TOCTOU`, `toctou`, `time-of-check`, `re-read`,
  `read twice` across `planning/completed/audit-1-*`, `audit-2-*`, and `bugs/`.
  audit-2 recorded filesystem TOCTOUs only for `fs::` path operations
  (OS-03/OS-04), a different subsystem. Distinct from audit-1 PKG-01, which was
  "no verification at all" and is fixed.
- **Impact bound (why LOW, not higher):** an attacker with write access to the
  build tree can usually subvert the build more directly — edit the sources,
  swap the compiler binary, or rewrite `project.json`'s pinned `identKey`. This
  is defence-in-depth against a narrower attacker who can write only the
  `packages/` cache. Note also that the swapped-in content still has to pass
  `verify_semantics`, so there is no memory-safety escalation — only a signature
  bypass.

---

## Re-verified as still fixed (no finding)

| prior ID | claim | current evidence |
|---|---|---|
| audit-1 PKG-01 | no signature gate | fixed — `verify_and_report_packages` at `src/cli/build/mod.rs:303`, before any decode/merge/lower (Q1) |
| audit-1 PKG-02 | decoded IR trusted | fixed — `verify_package` + `verify_semantics` in `merge_packages` (Q2) |
| audit-1 PKG-03..07 | recursion / allocation / overflow / duplicate-section / usize-narrowing | all guards present (Q3 table) |
| audit-2 PKG-08 (bug-265) | Scalar const escapes the literal-range verifier | fixed — `check_const_literal`'s `Scalar` arm rejects `> 0x10FFFF` and surrogates `0xD800..=0xDFFF` with `TYPE_SCALAR_LITERAL_INVALID` (`src/ir/verify/values.rs:525-538`); `bugs/completed/bug-265-scalar-const-literal-range-verifier-gap.md` |
| bug-395 | foreign-owner path traversal | fixed — the re-exported foreign-type owner name, decoded verbatim from an untrusted `.mfp`, is re-validated with `validate_package_name` before the join (`src/binary_repr/mod.rs:728`); `resolved_package_file` likewise rejects a traversing name (`src/manifest/package.rs:270-273`) |

Two corrections to the sub-agent's raw notes, made while re-verifying:

- Its evidence line claimed the signature-chain grep "returns only
  `src/cli/build/packages.rs`". Measured, it also returns `src/cli/pkg.rs` and
  `src/target/package_mfp/mod.rs`. The *finding* is unaffected — neither is on
  the decode/merge/lower path — but the audit text above states the measured
  result rather than the overstated one.
- It reported bug-265 as still parked in `bugs/`. Measured
  (`find bugs -name 'bug-265-*'`), it is already in `bugs/completed/`. No
  housekeeping is needed.

## Coverage

Read in full: `src/binary_repr/reader.rs` (1443 lines), `src/binary_repr/util.rs`,
`src/binary_repr/mod.rs` (public API, section-id constants,
`resolve_package_type_exports`, `enqueue_referenced_types`,
`push_type_identifiers`), `src/ir/binary.rs:1-590`, `src/cli/build/packages.rs`,
`src/target/shared/nir/lower.rs:75-115`, `src/target/shared/lower.rs`,
`src/ir/verify/mod.rs:1-120`, `src/ir/verify/values.rs:445-560`,
`src/resolver/packages.rs`, `src/manifest/package.rs:230-410`,
`src/cli/dispatch.rs:80-245`.

Remaining `src/ir/binary.rs` op/value decoders were skimmed — they route through
the same `enter`/`leave` + `need` primitives audited above.

The files the first pass left thin — `src/cli/resolve.rs`,
`src/manifest/{entry,json_edit,mod}.rs`,
`src/binary_repr/{sections,builder,writer}.rs`, and
`src/target/shared/validate/` — were covered by a second pass commissioned
specifically to close them; its findings are folded into the "Second pass"
section below rather than a separate file, so this surface stays one document
as the work order specifies.

The Ed25519 primitives themselves live in the `repository/` crate and are
audited on Surface 6/8, not here; this pass confirmed the *compiler calls them*,
not that the primitive is sound.
