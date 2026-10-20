# Changelog

All notable changes to the `mustache` package. The package follows
[Semantic Versioning](https://semver.org/): from 1.0.0 on, a breaking change to
the exported API, or to which templates render and which error a refused template
raises, requires a new major version.

## 1.0.0

The first stable release. It supersedes the unreleased 0.1.0 working version, which
was never published. The decisions below were made in the course of hardening the
package for release.

### Changed

- `render` and `renderWith` take an optional `maxBytes` output limit, which
  defaults to 16 777 216 bytes. It is the same parameter, and the same default
  rule, as `compress::inflate`. A negative `maxBytes` raises
  `errorCode::ErrInvalidArgument`.
- Every failure is a registry error code. The package's own error code, which
  matched a generator-9 integer (`93120001`), is gone. A template or output past
  its size limit raises `errorCode::ErrTooLarge` (`77050027`), which is what the
  registry documents for a result larger than the caller's limit.
- Context flattening. The context is turned once into a flat table, and rendering
  walks it by index. A section over a list no longer copies its root context for
  every item. A 20 000-item list that took 32 s to render takes about 0.6 s for the
  whole program, which also builds and parses its input.
- Set-delimiter, dotted-name and interpolation behaviour is unchanged. The oracle
  (`oracle/diff.mjs`, all modes) agrees before and after the change.

### Added

- `maxBytes` on `render` and `renderWith`.
- `tools/mustache-bench` (in the repository's `tools/`), which measures render time
  against the size of the context.
- `check-doc-examples.sh` now fails when it finds no examples, and no longer prints a
  stray count line.

### Removed

- `mustache::ErrorLimit`. Match `errorCode::ErrTooLarge` instead.

### Known limitation

- Scaling is not yet linear at large sizes, and the cause is in the language
  runtime, not in this package. Flattening a 256 000-item list takes about 2 s. In
  a scratch build with the arena's two large-block flush paths disabled (not a
  shipped change), the same flatten takes 0.36 s, and at 64 000 items the two
  builds take 0.25 s and 0.09 s. The flush sweeps every parked small block, and it
  is triggered by frees of large blocks alone. The fix belongs in the arena
  allocator, which every compiled program uses, so it is a separate runtime change.
