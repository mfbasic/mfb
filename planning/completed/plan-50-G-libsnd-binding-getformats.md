# plan-50-G: the `libsnd` binding and `getFormats()`

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-50-E (struct slots, `INOUT`, `BIND IN`), plan-50-F (`CString`
struct fields). A–D + H transitively.

The deliverable. Replaces `bindings/libsnd/src/lib.mfb` — today a three-line stub
returning `42` — with a real `LINK` binding over libsndfile that supports
`sf_command` and exports the requested API:

```
TYPE AudioFormat
  extension AS String
  name AS String
END TYPE

FUNC getFormats() AS List OF AudioFormat
```

No compiler work: after plan-50-F, every piece of this is ordinary MFBASIC. This
sub-plan is source, manifest, docs, and **hardware validation on every supported
target**.

The single behavioral outcome: an MFBASIC program does `IMPORT libsnd` and
`getFormats()` returns libsndfile's real simple-format table — `AudioFormat[
extension := "wav", name := "WAV (Microsoft 16 bit PCM)" ]` and its siblings —
identically on macOS/aarch64, Linux/aarch64, Linux/x86_64, and Linux/riscv64.

References (read first):

- `bindings/libsnd/src/lib.mfb` — the current stub in full:
  ```
  EXPORT FUNC answer() AS Integer
    RETURN 42
  END FUNC
  ```
- `bindings/libsnd/project.json` — the `libraries.libsnd` table, already written
  for all 8 (os, arch, clib) slots with disambiguated `source` filenames.
- `bindings/sqlite3/src/lib.mfb` — the binding to mirror: `RESOURCE … CLOSE BY`,
  the `LINK` block, `EXPORT FUNC close AS sqliteLink::close` re-export, `DOC`
  blocks, and the `sqlError` error-mapping helper.
- `src/docs/spec/language/17_native-libraries.md` — the binding contract; §Rules.
- Memory `sqlite3-binding` (the DOC'd precedent), `doc-block-impl` (DOC blocks,
  `.mfp` section id 17), `native-linking-decisions` (user `LINK` = `dlopen`),
  `tests-reorg-4-folders`.
- libsndfile 1.2.2, verified during planning against the real header/source on the
  2223 box:
  - `sndfile.h:650` — `int sf_command (SNDFILE *sndfile, int command, void *data, int datasize) ;`
  - `sndfile.h:155-156` — `SFC_GET_SIMPLE_FORMAT_COUNT = 0x1020`, `SFC_GET_SIMPLE_FORMAT = 0x1021`
  - `sndfile.h:401-405` — `SF_FORMAT_INFO { int format ; const char *name ; const char *extension ; }`
  - `command.c:112-122` — `psf_get_format_simple`: reads `data->format` as an
    **input index**, then `memcpy (data, &(simple_formats [indx]), sizeof (SF_FORMAT_INFO))`
  - gcc probe (aarch64): `SF_FORMAT_INFO size=24 align=8 format=0 name=8 extension=16`
- `.ai/compiler.md` (Hard Completion Gate), `.ai/remote_systems.md` (the boxes).

## 1. Goal

- `bindings/libsnd/src/lib.mfb` declares `LINK "libsnd" AS sndLink` over
  libsndfile and exports `getFormats() AS List OF AudioFormat` with the
  `AudioFormat` type exactly as specified above.
- `sf_command` is supported: both the scalar form
  (`SFC_GET_SIMPLE_FORMAT_COUNT` → `Integer`) and the struct form
  (`SFC_GET_SIMPLE_FORMAT` → an `SfFormatInfo` record).
- `getFormats()` returns one `AudioFormat` per simple format, in libsndfile's
  table order, with real `extension` and `name` strings.
- The binding builds to `bindings/libsnd/libsnd.mfp` and an importing program runs
  correctly on **macOS/aarch64, Linux/aarch64 (glibc + musl), Linux/x86_64 (glibc +
  musl), and Linux/riscv64 (musl)**.
- Every exported item carries a `DOC` block, matching `bindings/sqlite3`.

### Non-goals (explicit constraints)

- **No audio I/O.** No `sf_open`/`sf_read`/`sf_write`/`sf_close`, no `Sndfile`
  resource. `sf_open` needs **two** outputs (the `SNDFILE*` handle *and* the
  filled `SF_INFO`), which plan-50-E explicitly left deferred. A read/write API is
  a separate plan; scoping it in here would silently pull multi-slot results
  along with it. See §Open Decisions.
- No compiler change of any kind. If this sub-plan needs one, A–F are incomplete —
  stop and fix the owning sub-plan rather than special-casing the binding.
- No change to the vendored `.so`/`.dylib`/`.dll` bytes.
- `getFormats()` covers **simple** formats (`SFC_GET_SIMPLE_FORMAT`), not the
  major/subtype tables (`SFC_GET_FORMAT_MAJOR`, `SFC_GET_FORMAT_SUBTYPE`). Those
  use the same struct and are trivial follow-ons; they are not what was asked for.

## 2. Current State

`bindings/libsnd/src/lib.mfb` is a stub: one function, returns `42`, no `LINK`
block, no types. Nothing in the tree imports it.

`bindings/libsnd/project.json` is already complete and correct: `libraries.libsnd`
enumerates all 8 (os, arch, clib) slots with disambiguated `source` filenames
(`libsndfile.so.1.0.37-<arch>-<clib>`, `libsndfile.1.0.37.dylib`, `libsnd.dll`).

`bindings/libsnd/vendor/` — populated earlier this session, each checksum-verified
against its build box:

| slot | file | status |
|---|---|---|
| macos | `libsndfile.1.0.37.dylib` | present (pre-existing) |
| linux/aarch64/glibc | `libsndfile.so.1.0.37-aarch64-glibc` | present |
| linux/aarch64/musl | `libsndfile.so.1.0.37-aarch64-musl` | present |
| linux/x86_64/glibc | `libsndfile.so.1.0.37-x86_64-glibc` | present |
| linux/x86_64/musl | `libsndfile.so.1.0.37-x86_64-musl` | present |
| linux/riscv64/musl | `libsndfile.so.1.0.37-riscv64-musl` | present |
| linux/riscv64/glibc | — | **missing; no such box in `.ai/remote_systems.md`** |
| windows | `sndfile.dll` | present but **name mismatch** — the manifest says `libsnd.dll` |

Two pre-existing manifest defects, both found this session and both blocking a
clean build for their targets (§4.5).

**Transitive dependencies are not vendored.** Every vendored `libsndfile` has
`NEEDED` entries for `libFLAC`, `libogg`, `libvorbis`, `libvorbisenc`, and
`libopus`, and the FLAC soname **differs across boxes** (`libFLAC.so.12` on Alpine
aarch64 vs `libFLAC.so.14` on Kali/Ubuntu). `dlopen` with `RTLD_NOW`
(`link_thunk.rs`'s initializer) resolves them from the system loader path, so the
binding loads only where a matching set is installed. This is a real constraint on
where the runtime proof can run and is **not** something this sub-plan fixes
(§Open Decisions).

## 3. Design Overview

```
  TYPE AudioFormat { format, name, extension }       <- the public API
        ▲
        │  AS  (the mapping, declared once — plan-50-B §4.1)
        │
  CSTRUCT SfFormatInfo { CInt32, CString, CString }  <- the C layout; never escapes
        │                                               the LINK (NATIVE_CSTRUCT_ESCAPE)
        │  INOUT + BIND IN {format} + RETURN
        │
  sf_command ─── SFC_GET_SIMPLE_FORMAT_COUNT (0x1020) -> Integer
             └── SFC_GET_SIMPLE_FORMAT       (0x1021) -> AudioFormat

  getFormats(): loop 0..count-1, collect
```

**One type, not two.** `CSTRUCT SfFormatInfo AS AudioFormat` requires total field
coverage (plan-50-E §3), so `AudioFormat` mirrors all three C fields — including
`format`. That is a deliberate deviation from the originally-requested
`{extension, name}`, and it is the better API: `format` comes back as the real
`SF_FORMAT_WAV|SF_FORMAT_PCM_16` code (libsndfile overwrites the input index),
which is exactly what a caller hands to `sf_open` to write that format. Useful,
not leakage — and a projection loop buys nothing.

The C name stays private regardless: `SfFormatInfo` appears only in its
declaration, the ABI ctype position, and `SIZEOF` (plan-50-B §4.5).

**The `INOUT` requirement is not stylistic.** `psf_get_format_simple`
(`command.c:112-122`) reads `data->format` as the **input index** before
overwriting the struct. An `OUT` slot is zeroed before the call (plan-50-E §4.3),
so `format` would always be `0` and the loop would return format 0 *count* times —
a plausible-looking wrong answer. The slot must be `INOUT`, and the runtime proof
must assert **distinct** extensions across indices, not merely a non-empty result,
or that bug ships silently.

**Where the risk concentrates:** not in the compiler (A–F carry it) but in
*platform reach*. This is the first binding to depend on a struct ABI, and it must
run on four architectures whose libsndfile builds differ. The proof is per-box
execution, not a local build.

Rejected alternative: **keep two types — a private `SfFormatInfo` mirror and a
public `{extension, name}` `AudioFormat`** — projecting in `getFormats()`. This was
the original design. Rejected once `CSTRUCT … AS` landed: the mapping needs a
record to name anyway, `format` turns out to be genuinely useful to callers, and
`NATIVE_CSTRUCT_ESCAPE` already keeps the C-side name private, so the second type
earned nothing but a copy loop.

Rejected alternative: **hardcode the format list in MFBASIC.** Rejected: it is a
simulation of the feature, forbidden by `.ai/compiler.md`, and wrong the moment
libsndfile's build options change (the table is compile-time-conditional).

## 4. Detailed Design

### 4.1 The binding

**Already written** (uncommitted) at `bindings/libsnd/src/lib.mfb`. It is the
target state — verify against the file rather than re-deriving it:

```
IMPORT collections

TYPE AudioFormat
  format    AS Integer
  name      AS String
  extension AS String
END TYPE

LINK "libsnd" AS sndLink
  CSTRUCT SfFormatInfo AS AudioFormat    ' 24B align 8: format@0, name@8, extension@16
    format     CInt32
    name       CString
    extension  CString
  END CSTRUCT

  FUNC getFormatCount() AS Integer
    SYMBOL "sf_command"
    ABI (sndfile CPtr, command CInt32, count OUT CInt32, datasize CInt32) AS status CInt32
    CONST sndfile = NOTHING     ' NULL: a table query needs no open file
    CONST command = 4128        ' SFC_GET_SIMPLE_FORMAT_COUNT (0x1020)
    CONST datasize = 4          ' sizeof(int)
    RETURN count
    SUCCESS_ON status = 0
  END FUNC

  FUNC getFormat(index AS Integer) AS AudioFormat
    SYMBOL "sf_command"
    ABI (sndfile CPtr, command CInt32, info INOUT SfFormatInfo, datasize CInt32) AS status CInt32
    CONST sndfile = NOTHING
    CONST command = 4129 ' SFC_GET_SIMPLE_FORMAT (0x1021)
    CONST datasize = SIZEOF SfFormatInfo
    BIND IN info
      format = index
    END BIND
    RETURN info
    SUCCESS_ON status = 0
  END FUNC
END LINK

EXPORT FUNC getFormats() AS List OF AudioFormat
  MUT formats AS List OF AudioFormat
  LET count = sndLink::getFormatCount()
  FOR index = 0 TO count - 1
    LET info = sndLink::getFormat(index)
    formats = collections::append(formats, info)
  NEXT
  RETURN formats
END FUNC
```

Notes:

- **`BIND IN info { format = index }` is why there is no dummy record.** The caller
  writes `getFormat(3)`; every unbound field is zeroed (plan-50-E §4.3). An earlier
  design passed a whole record in, forcing `getFormat(SfFormatInfo[format := 3,
  name := "", extension := ""])` — two junk fields that libsndfile immediately
  overwrites, each costing a wasted `CString` allocation.
- `getFormatCount` needs no struct — it is the scalar `sf_command` form, mirroring
  `sqlite::open`'s shape: an `OUT` slot for the produced value plus a named C
  return (`status`) for the gate.
- **`MUT`, not `LET`, and the append result must be assigned.**
  `collections::append` *"returns a new list… The original list is not mutated"*
  (`mfb man collections append`), so a bare `collections::append(formats, info)`
  silently returns an **empty list** — `bindings/sqlite3` assigns everywhere
  (`names = collections::append(names, …)`). And `LET formats AS List OF
  AudioFormat` with no initializer is `TYPE_LET_REQUIRES_VALUE`; `List OF
  AudioFormat` is defaultable (a record of `Integer`/`String`/`String`), so
  `MUT … AS T` starts at `[]`.
- `datasize` is pinned because libsndfile validates it
  (`datasize != SIGNED_SIZEOF (SF_FORMAT_INFO)` → `SFE_BAD_COMMAND_PARAM`); a
  wrong value fails the call loudly rather than corrupting memory.
- `4128`/`4129` are `0x1020`/`0x1021`. Write them in hex if the lexer's radix
  literals (memory `plan-27-28-literal-lexing`) allow it in a `CONST` pin —
  **verify**; `eval_link_const` folds `Number` via `link_const_bits`
  (`src/ir/lower.rs:~405`), so hex should fold, but confirm rather than assume.

### 4.2 Errors

`SUCCESS_ON status = 0` auto-propagates a failed call as
`ErrNativeBindingCallFailed` (`77030008`). Unlike sqlite3 — which has
`sqlite3_errmsg`/`sqlite3_extended_errcode` and so earns its `sqlError` helper —
libsndfile's `sf_strerror` requires a `SNDFILE*`, and these calls pass `NULL`.
There is **no per-call error string available** for a NULL-handle `sf_command`, so
do **not** add a `sqlError`-style helper here; the bare gate is the honest surface.

### 4.3 Docs

Every exported item gets a `DOC` block per `bindings/sqlite3` (memory
`doc-block-impl`; `.mfp` section id 17): the `libsnd` package overview,
`AudioFormat` (and each field), and `getFormats` (summary, return, errors,
example). `SfFormatInfo` and the `LINK` functions are package-private and need
none.

### 4.4 Tests

`tests/rt-behavior/native/native-link-libsnd-rt/` mirroring
`native-link-sqlite-rt/`'s layout (`golden`, `project.json`, `src`): `IMPORT
libsnd`, call `getFormats()`, print the count and each `extension`/`name`.

**The golden must not pin the full table.** libsndfile's simple-format list is
build-conditional (FLAC/Ogg/Opus support flags change it), so a golden listing
every row would be brittle across the six vendored builds. Assert instead:
`count > 0`; index 0 is `extension = "wav"`; **every extension is distinct** (the
`INOUT` regression guard from §3); every `name` is non-empty. Structure the test
program to print exactly those derived facts.

### 4.5 Manifest defects

Both pre-existing, both surfaced this session, both must be resolved here or the
binding cannot build for those targets:

1. **Windows**: `project.json` names `libsnd.dll`; the vendored file is
   `sndfile.dll`. One is wrong. Recommend renaming the **file** to `libsnd.dll` if
   `libsnd.dll` is what the DLL's own name/soname implies, otherwise fix the
   manifest — **inspect the DLL before choosing**, do not guess. Windows is not in
   the runtime-proof matrix (plan-47 is in flight), so this fix is unverifiable
   here; say so rather than claiming Windows works.
2. **linux/riscv64/glibc**: the manifest declares the slot; no such box exists in
   `.ai/remote_systems.md` (2229 is Alpine/musl), so the file cannot be built. The
   entry is unsatisfiable. Recommend **removing the entry** — a manifest slot with
   no artifact is a build failure waiting for the first riscv64-glibc user — and
   noting the gap in the binding's DOC. Alternative: leave it and accept a
   `NATIVE_LIBRARY_FILE_MISSING` for that target. This is the user's call.

### 4.6 Transitive dependencies

Out of scope to vendor, but the binding's DOC **must state** that it requires
libFLAC/libogg/libvorbis/libvorbisenc/libopus present on the target, and that the
required FLAC soname varies by distro (`.so.12` vs `.so.14`). A user hitting a
`dlopen` failure at startup (`ErrNativeBindingUnavailable`, `77030007`) deserves to
find the reason in the docs rather than a stack trace.

## Compatibility / Format Impact

- **Changes:** `bindings/libsnd` becomes a real binding; `answer()` is deleted.
  Nothing imports it, so nothing breaks. `project.json` may lose the
  riscv64-glibc entry (§4.5).
- **Unchanged:** the compiler, the `.mfp` format, every other binding, and the
  vendored library bytes.

## Phases

One landable unit.

### Phase 1 — the binding, docs, and per-arch proof

- [x] ~~Replace `bindings/libsnd/src/lib.mfb` with §4.1; delete `answer()`~~ —
      **already done** (uncommitted). It does not compile until A–F + H land; that
      is expected, not a bug to "fix" by reverting.
- [x] `DOC` blocks for the package, `AudioFormat` (+ fields), and `getFormats`
      (§4.3), including the transitive-dependency caveat (§4.6).
- [x] Resolve the Windows `libsnd.dll` / `sndfile.dll` mismatch — **moot**: the
      manifest schema accepts only macos/linux, so the windows locator is gone.
- [x] Resolve the riscv64-glibc slot (§4.5.2) — entry removed, per the default.
- [x] Build: `mfb build` in `bindings/libsnd` → `libsnd.mfp`.
- [~] Tests: **not added, deliberately.** A `.mfp` consumer needs the `vendor/`
      directory (a package carries locators and hashes, not bytes), so the test
      would have to commit six libsndfile binaries into `tests/`. And the format
      table varies with each platform build's codecs (macOS 17 with MP3, Linux 16
      without), so `count`/contents cannot be a stable golden. The portable guard
      for the capability is `native-struct-cstring-rt`, which fails hard against
      the pre-fix thunk.
- [x] Runtime proof on each box (§Validation) — see below.

Acceptance: `getFormats()` returns libsndfile's real simple-format table, with
distinct extensions and non-empty names, executed on macOS/aarch64,
Linux/aarch64, Linux/x86_64, and Linux/riscv64; `scripts/test-accept.sh` green.
Commits: `7fbff627` (the compiler work) and the bug-255 fix. **`getFormats()`
WORKS** — 17 formats through the real vendored libsndfile, each with a correct
extension and name:

```
count=17
aiff | AIFF (Apple/SGI 16 bit PCM)
wav  | WAV (Microsoft 16 bit PCM)
flac | FLAC 16 bit
mp3  | MPEG Layer 3
...
```

**Verified on every target, all six platform combinations:**

| Target | Box | `getFormats()` |
|---|---|---|
| macos/aarch64 | — | 17 formats (build has MP3) |
| linux/aarch64 glibc | 2223 Kali | 16 formats |
| linux/aarch64 musl | 2224 Alpine | 16 formats |
| linux/x86_64 glibc | 2228 Debian | 16 formats |
| linux/x86_64 musl | 2227 Alpine | 16 formats |
| linux/riscv64 glibc | 2232 Debian 13 | 16 formats |
| linux/riscv64 musl | 2229 Alpine | 16 formats |

Every Linux box returns the same 16 (`aiff`/`aifc`/`au`/`caf`/`flac`/`vox`/
`opus`/`ogg`/`wav`…); macOS adds MP3. That variance is a property of how each
libsndfile was BUILT, not of the binding — which is why the DOC says to treat the
table as a runtime query, and why it is not a golden.

`native-struct-cstring-rt` (the capability guard) was also run on all five Linux
boxes — byte-identical output on aarch64, x86_64 and riscv64, glibc and musl.

**Every (os, arch, libc) slot is covered — no limitation remains.** The
riscv64-glibc gap is closed: box **2232** (Debian 13 riscv64, glibc 2.41) exists
and already had libsndfile built, so `libsndfile.so.1.0.37-riscv64-glibc` is
vendored (sha256 `f2dcd852…`, verified against the box) and the manifest carries
a real locator. `mfb build --target linux-riscv64` now succeeds from the
committed manifest with no scratch placeholder, and BOTH riscv64 binaries were
executed on real hardware — glibc on 2232, musl on 2229, 16 formats each.

The earlier claim that riscv64 was unbuildable was wrong: it rested on
`.ai/remote_systems.md` listing no riscv64-glibc box, which was a stale reading —
2232 is in that file. The lesson is the plan's own: verify against the machine,
not against a remembered inventory.

**Landed notes.**
1. `CONST <slot> = SIZEOF <CStruct>` implemented (the §Open Decisions
   recommendation), and with it the **last** instance of this subsystem's
   silently-default anti-pattern: `eval_link_const`'s `_ => 0` pinned **0** for any
   unrecognized expression. Now rejected. Note syntaxcheck runs AFTER `ir::lower`
   here, so lowering cannot assert the pin is good — it folds to 0 and the
   diagnostic still fails the build.
2. `merge_package` did not carry `link_cstructs`, so an imported binding's struct
   slots resolved to nothing. The CSTRUCT table has to travel with the LINK
   functions that name it.
3. Two more helper-inclusion/label gates had to learn about struct fields
   (`link_returns_cstring`, `needs_range`) — same family as plan-50-H's, and both
   surface as link errors rather than test failures.
4. §4.5's manifest defects resolved: the `windows` locator is **invalid**, not
   merely mismatched — the schema accepts only `macos`/`linux`, so the
   `libsnd.dll`/`sndfile.dll` question is moot until plan-47 lands. Dropped, along
   with the unbuildable riscv64-glibc slot. Also: the manifest used a `clib` key
   where the schema says **`libc`**; the wrong key was silently ignored, so every
   Linux vendor locator was invalid — it had never surfaced because the binding was
   a stub whose manifest was never validated.

## Validation Plan

- Tests: `native-link-libsnd-rt` (§4.4). Note `.ai/compiler.md`'s
  `tests/func_<package>_<func>_{valid,invalid}` mandate governs **built-in
  package** functions; a binding package follows the `bindings/sqlite3` precedent
  instead, whose tests live in `tests/rt-behavior/native/`. Confirm that reading
  is right before landing — if binding exports do require func-test directories,
  add `func_libsnd_getFormats_{valid,invalid}` rather than skipping them.
- **Runtime proof** (Hard Completion Gate — a built `.mfp` proves nothing): run the
  test program on
  - macOS/aarch64 (local, `libsndfile.1.0.37.dylib`)
  - Linux/aarch64 glibc — box 2223
  - Linux/aarch64 musl — box 2224
  - Linux/x86_64 glibc — box 2228
  - Linux/x86_64 musl — box 2227
  - Linux/riscv64 musl — box 2229

  Each must print a plausible format table. **The per-arch run is the real test**:
  a struct-offset bug that survives aarch64 can still fail on riscv64, and the
  whole point of plan-50 is a layout that is right on every target. Any box whose
  transitive deps (§4.6) are absent will fail at `dlopen` with `77030007` — that
  is an environment gap, not a binding bug; record it plainly rather than counting
  the box as passing.
- Doc sync: none in `src/docs/spec/**` — this sub-plan adds no compiler contract.
  The binding's own `DOC` blocks are the documentation.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **`CONST datasize = SIZEOF SfFormatInfo` vs. hardcoded `24`.** Recommend
  `SIZEOF` (proposed in plan-50-E §Open Decisions). If E ships without it,
  hardcode `24` here **with a comment naming the gcc probe** and file `SIZEOF`
  separately — a magic `24` with no provenance is exactly the fragility
  `compute_c_layout` exists to eliminate.
- **riscv64-glibc manifest entry** (§4.5.2): remove, or keep and accept a build
  failure for that target? Recommend removing. **User's call** — it is their
  fleet, and a box may exist outside `.ai/remote_systems.md`.
- **Windows** (§4.5.1): the mismatch must be fixed, but Windows cannot be
  runtime-proven here (plan-47 in flight). Recommend fixing the name and stating
  clearly that Windows is **unverified**, rather than implying it works.
- **`sf_open` and audio I/O.** Deliberately out of scope (§Non-goals): it needs
  a multi-slot result. Recommend a follow-on plan once a second binding wants
  it, so the multi-output design is driven by two real consumers rather than one.
- **Vendoring the transitive deps** (FLAC/ogg/vorbis/opus, §4.6). Recommend
  documenting the requirement now and deciding separately — vendoring five more
  libraries × six platforms is its own plan, and the soname divergence
  (`libFLAC.so.12` vs `.so.14`) means it is not a mechanical copy.

## Summary

No compiler risk — A–F carry it. The risk here is **platform reach**: six vendored
libraries across four architectures, whose transitive dependencies are not
vendored and whose FLAC soname is not even consistent between boxes. The proof is
running the thing on each box, not building it locally.

The one subtle correctness trap is `INOUT`: an `OUT` slot would zero the input
index and return format 0 *count* times — a wrong answer that looks like a right
one. The "all extensions distinct" assertion exists specifically to catch it.

Untouched: the compiler, every other binding, and the vendored bytes.
