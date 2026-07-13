# bug-169 — byte-at-a-time type-name scanners (`split_top_level_to` / `split_map_body`) panic on a non-ASCII byte in a `.mfp`-decoded type string

Last updated: 2026-07-12
Severity: LOW — latent; unreachable from source (ASCII identifiers) but reachable via a crafted/decoded package type-name string.
Class: Memory-safety (slice at non-char-boundary → panic).
Status: Open

## Finding

Four copies of the same routine scan a `Map`/`MapEntry` type-name body one **byte**
at a time (`index += 1`), then test `body[index..].starts_with(" TO ")` (and
slice at `index`). If `index` lands on a UTF-8 continuation byte, the slice
panics (not a char boundary). Source identifiers are ASCII
(`src/lexer.rs:183,1117`) so it is safe from user code, but the same routines
parse `.mfp`-package-metadata type strings, which are not guaranteed ASCII:

- `src/resolver/resolution.rs:1392` (`split_top_level_to`)
- `src/monomorph/helpers.rs:246` (`split_top_level_to_str`)
- `src/syntaxcheck/inference.rs:1447` (`split_top_level_to`)
- `src/syntaxcheck/types.rs:464` (`split_map_body`; also `owns_a_to_separator`
  at :437)

That the authors added a `prev >= 0x80` guard in `type_owns_a_to_separator`
implies non-ASCII bytes here were anticipated.

## Trigger

A `Map OF <non-ASCII-key> TO Integer` / `MapEntry OF …` type string decoded from a
package (e.g. `install_package_type_info`/`collect_package_functions` →
`parse_type`, or `infer_member_access` → `split_top_level_to`) whose key/value
type name carries a non-ASCII byte → panic (compiler abort) instead of a decode
error.

## Fix

Advance by the current char's UTF-8 width (or iterate `char_indices()`), or guard
each slice with `body.is_char_boundary(index)`; alternatively reject non-ASCII
type-name bytes at `.mfp` decode. Apply the same fix at all four sites.
