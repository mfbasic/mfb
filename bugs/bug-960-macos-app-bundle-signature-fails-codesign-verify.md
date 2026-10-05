# bug-960: `mfb build --app` bundles fail `codesign --verify` ("code has no resources but signature indicates they must be present")

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness (owner-confirmed: bundles must be signable and verify)

Status: Open
Regression Test: tests/<app-bundle gate> running `codesign -v build/<name>.app` (macOS host only)

**Owner decision (2026-10-05):** `.app` bundles must be signable and must pass verification. `mfb build --app`
produces a bundle whose signature is a proper bundle signature: the code directory
covers the bundle's resources (`_CodeSignature/CodeResources`) and `Info.plist`, so that
`codesign --verify --deep --strict <name>.app` succeeds. The bundle can also be re-signed
with a real identity (`codesign --force --sign <identity> <name>.app`, and the
`mfb build --sign` path if it applies to bundles) and still verify. Console outputs keep
passing as today.

The macOS linker gives every executable an ad-hoc, linker-signed CodeDirectory with no special
slots (`nspecial=0`). That is a valid signature for the console `build/<project>.out`
(`codesign -v` passes, measured). But once the same Mach-O is written to
`build/<project>.app/Contents/MacOS/<project>` it is the main executable of a bundle, and
`codesign` then requires the CodeDirectory to bind `Info.plist` and a `_CodeSignature/CodeResources`
seal. Both the bundle and the inner executable fail verification:

```
codesign -v app1/build/app1.app
app1/build/app1.app: code has no resources but signature indicates they must be present   (rc=1)
codesign -dvvv app1/build/app1.app  ->  Info.plist=not bound, Sealed Resources=none
```

The bundle still launches locally (measured: `open app1.app` rc=0, executable exits 0), so this
is not a load failure today. It is classed UNCLEAR because the spec never states what a bundle's
signature must satisfy: `mfb spec linker macos-aarch64` says "arm64 macOS refuses to run an
unsigned binary, so the linker always emits an ad-hoc code signature" and, for signed builds,
"so the ad-hoc signature covers the sealed blob and `codesign -v` passes" — a reader expects
`codesign -v` to pass for every output, and it does not for `.app`. Consequences (hypotheses,
not measured): `spctl --assess` fails (measured: `spctl -a -vv app1.app` → `invalid API object
reference`), and TCC/LaunchServices treat the bundle as having an invalid signature, which
matters because the generated Info.plist carries `NSDocumentsFolderUsageDescription` (a TCC prompt).

Owner decision needed: either (a) sign bundles as bundles (bind Info.plist in special slot -1,
emit `Contents/_CodeSignature/CodeResources` and hash it in slot -3, `Info.plist` hash), or
(b) document that `--app` bundles are intentionally not bundle-signed and that `codesign -f -s -
<name>.app` must be run before distribution (measured: re-signing with
`codesign -f -s - resigned.app` makes `codesign -v` pass).

References:

- `mfb spec linker macos-aarch64` ("Ad-hoc code signing", "App-mode bundle", "Bundle
  generation contract")

## Failing Reproduction

```
cd /tmp && /tmp/hardening/mfb-head init app1 && /tmp/hardening/mfb-head build -q --app app1
codesign -v app1/build/app1.app; echo rc=$?
codesign -v app1/build/app1.app/Contents/MacOS/app1; echo rc=$?
```

- Observed: both print `code has no resources but signature indicates they must be present`, rc=1.
- Expected (per (a)): both rc=0.

Contrast: console `build/app1.out` → `codesign -v` rc=0; same bundle after
`codesign -f -s -` → rc=0.

## Root Cause

`src/os/macos/link/commands.rs:code_signature` builds a single CodeDirectory (identifier
`mfb.<project>`, `nspecial = 0`, `flags = adhoc|linker-signed`) over the Mach-O alone;
`src/os/macos/link/mod.rs:write_app_bundle` writes that same bytes into the bundle and adds
`Info.plist`/`AppIcon.icns` afterwards, so nothing binds them and no `CodeResources` exists.

## Goal

- Owner picks (a) or (b); for (a) `codesign -v` passes on the `.app` and on the inner executable.

### Non-goals (must NOT change)

- Console-output signature bytes and the two-pass size-settle encode.

## Blast Radius

- `write_app_bundle` (+ vendored `Contents/Frameworks/*` libraries, which a bundle seal must
  also cover) — fixed by this bug.
- Console `.out` — unaffected (measured valid).
