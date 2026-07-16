# bug-248: a `mfb build -app` bundle omits `CFBundleVersion`/`CFBundleShortVersionString`, so App Store upload validation rejects it

Last updated: 2026-07-15
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness / Tooling

Status: Fixed
Regression Test: `src/os/macos/link/tests.rs:app_info_plist_publishes_manifest_version`

The `Info.plist` written for a macOS app-mode bundle never contained a version.
Apple treats both version keys as mandatory for a shipped app, so submitting the
bundle fails validation before upload:

```
ERROR: [altool.6000002704C0] The main Info.plist in 'hello_world.pkg' does not
contain CFBundleVersion. (12)
```

The bundle is otherwise well-formed and launches normally — macOS only requires
the version keys at submission time and for version display, so the gap surfaces
only when a user actually tries to ship, i.e. at the last step of the pipeline.
`CFBundleShortVersionString` was missing too; `altool` reports the two keys one
at a time, so fixing only `CFBundleVersion` would have produced the same error
again naming the other key.

## Failing Reproduction

```
$ mfb build -app examples/hello_world
$ plutil -p examples/hello_world/hello_world.app/Contents/Info.plist | grep -c CFBundleVersion
0
```

Then packaging the bundle (`productbuild --component hello_world.app /Applications
hello_world.pkg`) and submitting it via `altool`/Transporter fails with error 12.

- Observed (before fix): the plist carries `CFBundleName`, `CFBundleExecutable`,
  `CFBundleIdentifier`, `CFBundlePackageType`, `CFBundleIconFile`, and
  `NSPrincipalClass`, but no version key of any kind.
- Expected: `CFBundleShortVersionString` and `CFBundleVersion`, both carrying the
  manifest `version`.

## Root Cause

`app_info_plist` (`src/os/macos/link/mod.rs`) is a fixed `format!` template that
was only ever parameterized by the project *name*. The project *version* was
never threaded to the backend at all: `IrProject` carries `name` but no version,
and the `NativeBackend::write_executable` seam passed `app_icon` but nothing else
from the manifest. So the writer had no version to emit even though
`project.json` had required one all along (`validate_required_string` validates
`version` as a required, non-empty string).

## Fix

- `app_info_plist` takes the version and emits both `CFBundleShortVersionString`
  (release version) and `CFBundleVersion` (build version), XML-escaped through
  the existing `plist_escape` like the project name.
- Thread `app_version: Option<&str>` from the CLI to the backend alongside the
  existing `app_icon`, via `manifest::project_version`. The three Linux backends
  ignore it as they already ignore `app_icon` (no bundle format).
- The macOS backend errors on an app build with no version rather than inventing
  a default: the manifest requires a non-empty `version`, so `None` is a caller
  bug, and silently versioning a user's shipped app would be worse than failing.

Both keys carry the manifest `version` verbatim. A version that is not 1–3
period-separated integers is still App Store-invalid, but that is the user's
declared version string to correct, not something to rewrite behind their back.

## Validation

- `app_info_plist_publishes_manifest_version` — both keys present, carrying the
  manifest version.
- `app_info_plist_escapes_xml_metacharacters_in_version` — a version with XML
  metacharacters produces a well-formed plist.
- End-to-end: `mfb build -app examples/hello_world` emits both keys as `0.1.0`
  (from that project's `project.json`); `plutil -lint` passes and `defaults read`
  returns `0.1.0` for both. A `productbuild` package built from the bundle embeds
  both keys — the exact plist `altool` reads.
- Full unit suite: 2750 passed, with only the 3 failures already present at HEAD
  (`riscv64::select` ×2, `builtins::thread` ×1), all unrelated.

The inner Mach-O is untouched — this changes only the `Info.plist` sidecar, so no
golden re-sync was needed.

## Note on scope

`mfb` does not build the `.pkg` itself; the user packages the `.app` by hand. A
`.pkg` built before this fix still embeds the old plist and must be regenerated
from a freshly built bundle.
