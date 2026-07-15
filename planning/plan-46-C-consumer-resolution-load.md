# plan-46-C: consumer-side locator resolution + linker load

Last updated: 2026-07-14
Effort: medium (1h–2h)
Depends on: plan-46-B (which depends on plan-46-A)

Consumes the `NATIVE_LIBRARY_TABLE` (section id 10) at **executable** build
time: for the target being emitted `(os, arch, libc)`, resolve the
most-specific locator per logical library, and replace `link_thunk.rs`'s
hardcoded `library_filename()` soname guess with the resolved `source` string.
For `system` locators this is the full end-to-end path (author-declared exact
soname → `dlopen`). For `vendored` locators, the referenced file is verified
against the section-10 sha256 at build time; automated **distribution** of
vendored blobs is explicitly out of scope for this plan.

The single behavioral outcome: an executable importing the `sqlite3` binding
`dlopen`s the exact `source` the binding author declared for the build's
`(os,arch,libc)` — not a synthesized `libsqlite3.so.0` — and a target with no
matching locator fails the build with a precise diagnostic instead of silently
emitting a wrong soname.

References (read first):

- `src/target/shared/code/link_thunk.rs` — `library_filename` (lines 34-42, the
  guess to delete), `emit_link_support` (lines ~145-156, the sole caller that
  emits the `lib_symbol` cstring), `lower_link_initializer` (`_mfb_linker_init`,
  lines ~202-331, unchanged — it consumes whatever bytes sit in the cstring).
- `src/binary_repr/reader.rs` — where section 10 is decoded (plan-46-B) and how
  merged package data reaches codegen.
- `src/ir/package.rs:119` — how imported `link_functions` merge into the project
  (dedup by `(alias,name)`); the locator table merges alongside.
- `src/os/linux/flavor.rs` (`LinuxFlavor`), `src/os/linux/link/mod.rs:85`
  (`write_executable(flavor)` — the per-flavor output path that makes libc a
  build-time axis).
- `src/docs/spec/language/17_native-libraries.md:167-179` (loading model + the
  "platform-specific dependency" note to update).

## 1. Goal

- Building an executable for `(os,arch,libc)` that imports a binding with a
  section-10 table emits, for each logical library, the `source` string of the
  **most-specific matching locator**, and `_mfb_linker_init` `dlopen`s that
  exact string.
- No matching locator for the build's target → hard build error
  `NATIVE_LIBRARY_NO_MATCH` (not a runtime `ErrNativeBindingUnavailable`).
- Two equally-specific matching locators for one target → hard error
  `NATIVE_LIBRARY_AMBIGUOUS`.
- A `vendored` locator: the file at `source` (resolved on the consumer side) is
  read and its sha256 compared to the table's hash; mismatch →
  `NATIVE_LIBRARY_HASH_MISMATCH`, missing → `NATIVE_LIBRARY_FILE_MISSING`. On
  match, `dlopen` the resolved path.
- `library_filename()` is deleted; no code path synthesizes a soname anymore.

### Non-goals (explicit constraints)

- **Vendored distribution is out of scope**: nothing fetches/copies/embeds a
  vendored `.so` into the package or to the consumer. This plan only *verifies*
  a vendored file the consumer has placed locally. The registry/artifact story
  is a separate future plan.
- No change to `_mfb_linker_init`'s control flow, the resource model, or the
  `(alias,name,library,symbol,…)` trailer.
- No new manifest field on the *consumer* — the consumer needs no `libraries`
  section; it reads the imported binding's section 10.

## 2. Current State

`emit_link_support` (`src/target/shared/code/link_thunk.rs:~151`) collects each
distinct `IrLinkFunction.library` (declaration order, `library_index`) and, per
library, emits `cstring_object(lib_symbol(idx), library_filename(target,
logical))`. `library_filename` (lines 34-42) is the **only** producer of the
`dlopen` filename: `lib{logical}.dylib` on macos, else `lib{logical}.so.0`. This
is the string the plan replaces — it "misses many valid libraries" (unversioned
`.so`, `.so.3`, non-`lib`-prefixed, framework paths, per-arch/libc variants).

`lower_link_initializer` loads that cstring's address into the dlopen arg and
branches to `fail` on NULL; it needs **no change** — it already consumes
whatever bytes the constant holds.

Crucially, a single Linux build emits **both** libc flavors:
`write_executable(flavor)` (`src/os/linux/link/mod.rs:85`) writes
`<name>-glibc.out` and `<name>-musl.out`. `library_filename` ignores libc today
(same soname for both). Once locators can differ by libc, the emitted `source`
cstring may need to **differ per flavor output** — this is the plan's central
risk (§4.3).

Merged imported `link_functions` arrive via `src/ir/package.rs:119`; the
section-10 table (plan-46-B) is merged alongside, keyed by logical name.

## 3. Design Overview

Three pieces:

1. **Resolver** (new, e.g. `src/target/shared/code/link_locator.rs`): pure
   function `resolve(table, logical, os, arch, libc) -> Result<&Locator,
   ResolveErr>` implementing the most-specific-match rule (§4.1). No I/O.
2. **`emit_link_support` rewrite**: call the resolver instead of
   `library_filename`; emit the resolved `source` cstring; surface
   no-match/ambiguous as build diagnostics; delete `library_filename`.
3. **Vendored verify** (build-time, at the consumer executable link): for a
   resolved `vendored` locator, read `source` on the consumer, sha256, compare
   to the table hash; error on missing/mismatch.

Risk is **low and well-bounded**. The one axis worth confirming — Linux
per-libc-flavor emission — was checked against the pipeline and is **already
supported**: codegen runs once per flavor and emits a separate data image per
flavor (§4.3), so a libc-specific `source` lands in the right binary for free.
The remaining work there is pure plumbing (thread the flavor's libc into
`emit_link_support`). What real risk exists is in getting the match rule (§4.1)
and the vendored verify (§4.4) correct, not in codegen structure.

Rejected alternative: keep one shared cstring and pick glibc/musl at **runtime**
inside `_mfb_linker_init` via libc detection. Rejected — brittle runtime
detection for a fact fully known at build time, and unnecessary: the build
already forks per flavor with its own data image, so resolve at build time.

## 4. Detailed Design

### 4.1 Most-specific match rule

Target = concrete `(os, arch, libc)` (libc = the flavor being emitted; N/A for
macos). A locator **matches** when:
- `locator.os == target.os`, and
- `locator.arch` is `None` (any-arch) or `== target.arch`, and
- for linux: `effective_libc(locator) == target.libc`, where
  `effective_libc = locator.libc.unwrap_or(Glibc)` (plan-46-A default); for
  macos the libc axis is ignored.

Among matches, **specificity** = (arch specified? 1 : 0). Highest wins. A tie
(two matches of equal specificity — only possible with duplicate entries) →
`NATIVE_LIBRARY_AMBIGUOUS`. No match → `NATIVE_LIBRARY_NO_MATCH`. (libc is never
a wildcard — it always resolves to a concrete value via the default — so it does
not enter the specificity score.)

### 4.2 `emit_link_support` rewrite

Replace the `library_filename(platform.target(), library)` call with:
`resolve(&table, library, os, arch, libc)?` → emit
`cstring_object(lib_symbol(idx), locator.source)`. On `ResolveErr`, raise the
corresponding build diagnostic and abort (no cstring emitted). Delete
`library_filename`. `lower_link_initializer` is untouched.

For a `vendored` resolved locator, before emitting, run §4.4 verify; the emitted
`source` is the (consumer-resolved) path that `dlopen` will open.

### 4.3 Per-flavor emission (already supported — plumbing only)

**Confirmed against the pipeline: codegen already runs once per libc flavor.**
`write_executable` loops `for &flavor in &LinuxFlavor::ALL` and calls
`code::lower_module(&module, &native_plan, packages, flavor)` per flavor, then
encodes a **separate data image** per flavor
(`src/target/linux_x86_64/mod.rs:305-320`; linux-aarch64 is analogous). The
lowering is already flavor-parameterized — the two worlds emit **different
import library names** today (`libc.so.6` vs `libc.musl-x86_64.so.1`) via the
per-flavor `platform_imports` map. The native `LINK` soname is just one more
such per-flavor string, and it lands in the correct flavor's data image for
free — there is **no shared-codegen divergence problem**.

The only gap: `emit_link_support` (`src/target/shared/code/link_thunk.rs:136`)
currently receives just `platform` and reads `platform.target()` (`os-arch`,
**no libc**). So the work is to **thread the flavor's libc into
`emit_link_support`** (from the per-flavor `lower_module` call, through
`lower_module_for_platform` at `src/target/shared/code/mod.rs:1162`), then hand
it to `resolve`. This mirrors how `platform_imports` is already passed in
flavor-specific. macos is single-flavor — pass libc `None`.

When glibc and musl resolve to the same `source` (the common case — a `system`
soname, or the plan-46-A/B refinement where a `system` locator with no `libc`
covers both flavors), the two flavor images simply hold identical cstrings; when
they differ (a `vendored` locator with per-libc files), each flavor image holds
its own — automatically, because each flavor is already an independent codegen
pass.

### 4.4 Vendored verify (build-time, distribution out of scope)

For a resolved `vendored` locator at the **consumer** executable build:
- resolve `source` relative to the consumer project root (a project-relative
  path the consumer author placed);
- missing file → `NATIVE_LIBRARY_FILE_MISSING` (error, names the expected path);
- sha256(file) ≠ table hash → `NATIVE_LIBRARY_HASH_MISMATCH` (error, "wrong
  version");
- match → emit `source` as the `dlopen` string.

This makes manual placement a complete, verified behavior. Automated acquisition
(registry/artifact fetch) is a **separate future plan** and is called out as
deferred in the docs.

New diagnostics (`src/rules/table.rs`, next-free native codes, suggested
`2-203-0102`..`0105`): `NATIVE_LIBRARY_NO_MATCH`, `NATIVE_LIBRARY_AMBIGUOUS`,
`NATIVE_LIBRARY_FILE_MISSING`, `NATIVE_LIBRARY_HASH_MISMATCH` (all Error).

## Compatibility / Format Impact

- **Runtime/codegen behavior change:** the `dlopen` filename now comes from the
  binding's section-10 locator, not a synthesized soname. A binding that
  previously "happened to work" because `libX.so.0` matched will keep working
  only if its `libraries` section declares that soname (plan-46-B made the
  binding author responsible; that is the intended, non-backward-compatible
  shift — the plan-46 non-goal notes no back-compat is required).
- **No `.mfp` format change here** (section 10 defined in plan-46-B). No ABI or
  wire change. Consumer manifests unchanged.

## Phases

### Phase 1 — resolver + system-case emission

Full end-to-end for `system` locators; the common, lower-risk case.

- [ ] Add `src/target/shared/code/link_locator.rs` with the pure `resolve`
      function and `ResolveErr` per §4.1; unit tests covering wildcard-arch,
      libc default, no-match, ambiguous, macos-libc-ignored.
- [ ] Rewrite `emit_link_support` (`src/target/shared/code/link_thunk.rs`) to
      call `resolve` and emit `locator.source`; delete `library_filename`;
      surface `NATIVE_LIBRARY_NO_MATCH`/`AMBIGUOUS` as build diagnostics.
- [ ] Thread the flavor's libc from the per-flavor `lower_module` call into
      `emit_link_support` (via `lower_module_for_platform`,
      `src/target/shared/code/mod.rs:1162`) so `resolve` sees the correct libc
      (§4.3); pass `None` for macos.
- [ ] Add the two resolve diagnostics to `src/rules/table.rs`.
- [ ] Tests: golden acceptance — an executable importing a `system`-locator
      binding links and its `_mfb_linker_init` cstring holds the declared
      `source` (assert via the linked binary / a codegen dump); a target with no
      locator fails with `NATIVE_LIBRARY_NO_MATCH`. Run the **artifact/byte-diff
      gate** (`scripts/artifact-gate.sh`, `.ai/compiler.md`) — for bindings
      whose declared soname equals the old guess, codegen should be identical.

Acceptance: an executable `dlopen`s the author-declared `source` for its
`(os,arch,libc)`; unmatched target fails at build with `NATIVE_LIBRARY_NO_MATCH`;
`library_filename` no longer exists. Verified by golden + a runtime link of a
real `system` binding (e.g. sqlite3) on at least one target.
Commit: —

### Phase 2 — vendored verify

Build-time hash verification of a locally-placed vendored library; distribution
deferred.

- [ ] Implement §4.4 verify in the link-support path (read consumer-side
      `source`, sha256, compare to table hash).
- [ ] Add `NATIVE_LIBRARY_FILE_MISSING` / `NATIVE_LIBRARY_HASH_MISMATCH` rows.
- [ ] Tests: golden — a `vendored` binding + a correctly-placed fixture `.so`
      builds and verifies; a tampered/renamed file fails with
      `NATIVE_LIBRARY_HASH_MISMATCH`; an absent file fails with
      `NATIVE_LIBRARY_FILE_MISSING`.
- [ ] Doc: update `src/docs/spec/language/17_native-libraries.md` (line ~179
      platform-dependency note + the loading section: locators replace the
      synthesized soname; vendored verify semantics; distribution explicitly
      deferred), and `src/docs/man/link/package.md` (Loading + diagnostics).

Acceptance: a correctly-placed vendored file builds and loads; a wrong-hash or
missing file fails with the exact diagnostic. Verified by the three goldens.
Commit: —

## Validation Plan

- Tests: resolver unit tests (all match/tie/no-match branches); golden builds
  for system-resolve, no-match error, vendored verify pass/mismatch/missing.
- Runtime proof: build + run an executable that calls the `sqlite3` binding on a
  real target and confirm it opens the author-declared library (system case);
  for vendored, place a fixture `.so`, build, run, and confirm load + a
  hash-mismatch failure when the file is altered.
- Doc sync: `src/docs/spec/language/17_native-libraries.md`,
  `src/docs/man/link/package.md`; `.ai/specifications.md` and `.ai/compiler.md`
  obligations (codegen change → runtime completion gate + byte-diff gate).
- Acceptance: `scripts/test-accept.sh` green; `scripts/artifact-gate.sh` clean
  (execution-free codegen gate) since `link_thunk` is on the codegen path.

## Open Decisions

- Consumer-side vendored `source` resolution root — consumer project root
  (recommended) vs a dedicated `libs/` cache dir. Affects only where the file is
  expected; pick project-root for symmetry with the binding author's layout.

## Summary

Turns the author-declared locator table into real load behavior and kills the
soname guess. The feared Linux per-libc-flavor emission turned out to be a
non-issue — codegen already runs per flavor with its own data image, so it's
just plumbing the libc into `emit_link_support`. What's left is a pure resolver
(§4.1) + a build-time sha256 check (§4.4), both straightforward. Vendored
*distribution* is deliberately left to a later plan — this plan makes manual
placement a verified, first-class path.
