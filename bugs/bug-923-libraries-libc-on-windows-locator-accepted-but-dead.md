# bug-923: A `libraries` locator with `libc` on `os: "windows"` passes validation but can never match

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Footgun

Status: Open
Coordinator note: repro measured by the review agent only (HEAD 28a15a153 build); the coordinator did not re-run it.
Regression Test: unit test beside `check_libraries` in src/manifest/mod.rs (new)

The spec says `libc` is "Linux only". The validator rejects `libc` on
`os: "macos"` with `PROJECT_JSON_LIBRARY_INVALID`, but accepts it on
`os: "windows"`. Windows target slots carry `libc: None`, so such a locator never
covers a Windows slot. It is a dead entry. The build-time
`NATIVE_LIBRARY_TARGET_UNCOVERED` warning names `windows/x86_64`, and a
`--target windows-x86_64` build fails with `NATIVE_LIBRARY_NO_MATCH` while the
locator sits in the manifest looking correct. The validator's own comment gives
the reason for making unknown axes hard errors: they "yield a dead entry".

**Correct behavior:** `libc` on any non-Linux `os` (macOS **and** Windows) is
`PROJECT_JSON_LIBRARY_INVALID` at manifest validation, with a message like the
macOS one.

References:

- `src/docs/spec/tooling/01_project-manifest.md` *Library Locator Entries* table: "`libc` … `glibc` or `musl`; omitted = wildcard. Linux only — macOS has no libc axis".
- Same file, *Diagnostic Codes*: `PROJECT_JSON_LIBRARY_INVALID` "… sets `libc` on macOS …".

## Failing Reproduction

Use `tests/rt-behavior/native/native-link-free-rt` copied to /tmp, with:

```json
"libraries": { "sqlite3": [
  { "os": "macos",   "type": "system", "source": "libsqlite3.dylib" },
  { "os": "windows", "type": "system", "libc": "musl", "source": "sqlite3.dll" } ] }
```

```
mfb build                         # exit 0, warns "no locator covering windows/x86_64"
mfb build --target windows-x86_64
```

- Observed: validation passes. The Windows build fails with
  `error[2-203-0118 NATIVE_LIBRARY_NO_MATCH] … The binding declares: … { os: windows, arch: any, libc: musl, type: system, source: "sqlite3.dll" }. Add a locator covering windows/x86_64.`
- Expected: `PROJECT_JSON_LIBRARY_INVALID` at manifest load: "sets `libc` on `os: \"windows\"` — Windows has no libc axis".

Contrast: the same locator without `libc` resolves for `windows-x86_64` (the
build gets past library resolution). `libc` on `macos` is rejected.

## Root Cause

`src/manifest/mod.rs:check_locator` only tests `if os == "macos"` before
accepting a `libc` token. `src/manifest/libraries.rs:locator_covers` assumes
"validation rejects `libc` there, so a Some(libc) locator can only be Linux". That
assumption is false for `windows`. `src/manifest/libraries.rs` builds Windows
slots with `libc: None` (the slot list: libc axis crossed for linux only).

## Goal

- Rejecting `libc` on every `os` whose slots carry no libc axis, decided from the slot list rather than a hard-coded `"macos"`.

### Non-goals (must NOT change)

- Linux `libc` handling, or the Linux `vendor` arch/libc requirement.

## Blast Radius

- `src/manifest/mod.rs:check_locator`: fixed.
- `src/manifest/libraries.rs:locator_covers`: its comment must be corrected.
- `.mfp` section-10 locators decoded from packages: verify the package-side reader applies the same rule.

## Validation Plan

- Unit test: `check_libraries` on a windows+libc locator returns `PROJECT_JSON_LIBRARY_INVALID`.
- Doc sync: the spec table's `libc` row ("Linux only — macOS and Windows have no libc axis") and the diagnostic row.
