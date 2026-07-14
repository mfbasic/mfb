# bug-215: .mfp decode hardening — understated bounded_capacity min_elem + one unchecked function-id add

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: security / footgun

Status: Open

Two minor robustness gaps on the untrusted `.mfp` path (the module is otherwise
well hardened — every other count→alloc and offset uses checked bounds):

- `bounded_capacity` is called with an understated `min_elem`, weakening the
  PKG-05 pre-allocation bound: `src/binary_repr/reader.rs:831`
  (`read_function_table`, passes `min_elem=4` though a function entry occupies
  ≥52 wire bytes) and `:76` (`read_doc_table`, `min_elem=2` though a doc decl is
  ~40+ bytes). A crafted section of size S with a huge `count` reserves ~S/4
  (resp. S/2) `Function`/`DeclDocEntry` slots (~88 B each) before per-element
  cursor reads fail — ~13× the true S/52 cap the comment claims. Fix: pass the
  real minimum on-wire entry size (52 / ~40) as `min_elem`.
- `src/binary_repr/writer.rs:68` (`external_function_metadata`) uses an unchecked
  `next_function_id + export.function_id` on a decoded, attacker-influenced id,
  while the sibling two lines later uses `checked_add`. Latent (needs ~4B
  functions), but inconsistent. Fix: use `checked_add`.
