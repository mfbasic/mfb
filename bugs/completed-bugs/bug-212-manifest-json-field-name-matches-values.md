# bug-212: manifest field-name locator matches string values as if they were keys

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness

Status: Fixed (2026-07-15) — json_field_name_position now confirms key position before matching: a string token is treated as a field name only when the next non-whitespace byte after its closing quote is `:`, so a string *value* equal to a field name no longer latches. Regression test: json_scanning_primitives asserts `"packages"` resolves to the real key when a `"source": "packages"` value precedes it.

`json_field_name_position` (`src/manifest/package.rs:712-724`) scans every JSON
string token — keys and values alike — and returns the first whose contents
equal the needle, so a string *value* equal to a field name is matched as if it
were the key. `json_array_bounds` / `project_json_with_updated_ident_key` then
latch onto the value token, fail the subsequent `:` lookup, and return a spurious
"could not locate / malformed" error during `pkg add` / ident-key rewrite (no
corruption, just a false failure).

Trigger: a `project.json` with a value string literally equal to `"packages"` /
`"identKey"` appearing before the real key (e.g. `"source": "packages"`), then
`mfb pkg add`.

Fix: only treat a string token as a field name when the next non-whitespace,
non-string byte is `:` (confirm key position before matching).
