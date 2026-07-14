# bug-226: monomorph symbol mangling is lossy (collision) + return-type-only type params fail silently

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness / footgun

Status: Open

Two related monomorphization gaps:

- `mangle_name` (`src/monomorph/helpers.rs:371-378`, `sanitize_type_name`
  `:456-467`) builds the concrete-function/type symbol by `$`-joining
  `sanitize_type_name(arg)`, which replaces every non-alphanumeric with `$` — a
  lossy encoding. Two distinct type-argument tuples of the same arity that differ
  only in characters that sanitize to `$` (spaces/commas/parens, e.g.
  function-typed args) collapse to the same symbol; the `concrete_functions`/
  `concrete_types` maps are keyed by that symbol, so the second instantiation
  overwrites the first and both call sites are rewritten to one shared,
  possibly-wrong symbol. Latent. Fix: use an injective encoding (length-prefix or
  escape `$`), or key the maps by the unambiguous `name<args>` dedup string.
- `instantiate_function` (`src/monomorph/lower.rs:521-525`) returns `None` with
  no diagnostic when a template type-param cannot be inferred from the arguments
  (appears only in the return type) and never consults `expected_type`, so e.g.
  `FUNC make OF T() AS T` called as `LET x AS Integer = make()` is left as bare
  `make` and later fails with a confusing "unknown function". Fix: report a
  "cannot infer template argument" diagnostic, or thread `expected_type` into the
  return-type unification.
