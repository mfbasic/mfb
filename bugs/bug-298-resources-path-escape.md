# bug-298: `mfb build` resource copying does not constrain `src` to the project tree (arbitrary-file read into build output); `dst` guard is Unix-only

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Security / Footgun

Status: Open
Regression Test: tests/ (new) — a `resources` entry with an absolute or `../` `src` (or a `\`/drive-prefix `dst`) is rejected

The `resources` manifest section (plan-55) validates a resource entry's `dst`
against escape (rejects absolute or `..`) but only checks `src` non-empty.
`resource_src_fixed_prefix` walks the glob-free leading components of `src` as a
directory (`walk_root = project_root.join(prefix)`); an absolute `src` makes
`Path::join` replace the base entirely, and a `../…` `src` walks above the project.
`copy_resources` then copies every glob-matched file into the (in-tree, validated)
`dst` — so the read side is unbounded while the write side is contained. Building an
untrusted or third-party project (CI, or a shared project later shipped as an
`.app`/AppDir bundle) can exfiltrate arbitrary readable files (e.g. `~/.ssh/*`) into
the distributable. Separately, the `dst` escape guard splits only on `/` and treats
only a leading `/` as absolute, so on a future Windows build host (plan-47) a `dst`
of `..\..\etc` or `C:\foo` escapes — inconsistent with `libraries::source_is_bare`,
which already rejects `\` and drive prefixes "so plan-47 does not inherit a hole".

The single correct behavior a fix produces: a resource `src` that is absolute or
whose fixed prefix escapes the project root is rejected at manifest-validation time,
and `dst` is validated against the same cross-platform rules as `source_is_bare`.

References:

- `planning/old-plans/plan-55-resources-and-resourcepath.md` §1 (dst must not escape;
  src is "project-relative" — intent never enforced).
- `src/manifest/libraries.rs:134,147` (`source_is_bare` — the cross-platform
  rejection precedent).
- Found during goal-06 review of `src/cli/build.rs` + `src/manifest/mod.rs`.

## Failing Reproduction

```
# project.json:  "resources":[{"src":"/tmp/mfbres/secret/*.conf","dst":"cfg/"}]
mfb build .
# also: "src":"../outside/*.conf"
```

- Observed: `build/cfg/leak.conf` contains the outside file's bytes; exit 0, no
  diagnostic.
- Expected: the entry is rejected ("resource src must be within the project").

## Root Cause

`src/manifest/mod.rs:438-454` (`validate_resources`) checks `src` only for
non-empty; `src/cli/build.rs:1692` (`resource_src_fixed_prefix`) / `:1734`
(`copy_resources`) then walk/copy from wherever `src` points. The `dst` guard at
`src/manifest/mod.rs:456-473` (`dst.starts_with('/') || dst.split('/').any(|c| c ==
"..")`) is Unix-path-only.

## Goal

- Reject a `src` that is absolute or whose fixed (glob-free) prefix contains a `..`
  component; defense-in-depth: canonicalize `walk_root` and require it to
  `starts_with` the canonicalized `project_root` before walking.
- Route `dst` (and `src`) through the same rejection rules as `source_is_bare`
  (reject `\` and a `X:` drive prefix in addition to `/`-prefix and `..`).

### Non-goals (must NOT change)

- Legitimate in-tree glob bundling of project assets.
- The validated in-tree `dst` output layout.

## Blast Radius

- `validate_resources` (`src` + cross-platform `dst`) — fixed here.
- `copy_resources` — add the canonicalize-and-`starts_with` defense-in-depth check.
- Share one path-component validator with `libraries::source_is_bare` to prevent
  future drift.

## Fix Design

Add `src` escape validation mirroring `dst`, plus the `source_is_bare`-style
backslash/drive-prefix rules to both `src` and `dst`; add a canonicalized
containment check in `copy_resources`. Rejected alternative: validating only at copy
time — validation belongs at manifest parse so the error is early and uniform.

## Phases

### Phase 1 — failing test
- [ ] Test absolute/`../` `src` exfiltrates today; `\`/drive `dst` slips through the
      Unix-only guard.
### Phase 2 — the fix
- [ ] Add `src` validation + cross-platform `dst` rules + copy-time containment.
### Phase 3 — validation
- [ ] Full suite green; legitimate in-tree resources still bundle.

## Validation Plan

- Regression: escape-`src` rejection; `\`/drive-`dst` rejection; in-tree contrast.
- Runtime proof: the repro no longer copies outside files.
- Doc sync: plan-55 / spec note that `src` must be project-relative.

## Summary

The resource copier trusts `src` and validates `dst` only for Unix paths; a build of
an untrusted project can exfiltrate local files. Mirroring the `dst`/`source_is_bare`
rules onto `src` (and cross-platform `dst`) closes it. Low risk, high value for the
untrusted-build story.
