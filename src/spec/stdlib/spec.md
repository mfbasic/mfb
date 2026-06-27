# Standard Package Semantics

The semantic and algorithmic *models* of the standard packages that are
implemented as injected MFBASIC source (plus a few Rust seam helpers): the regex
engine, the date/time model, the CSV dialect, the JSON data model, the HTTP
client, the URL model, and the `math::` PCG64 RNG. These are the parts a faithful
reimplementation needs that the per-function API reference does not capture.

The per-function API of each package — signatures, parameters, return types,
errors — is owned by `./mfb man <package>` (e.g. `./mfb man regex`). This package
specifies the *behavior behind* that API. How these packages are injected into a
build is `./mfb spec architecture frontend`; their source-package mechanics are
`./mfb spec architecture monomorphization` (generic instantiation) and the
built-in injection chain.

## Reading order

- `regex` — pattern grammar, the CPS backtracking matcher and its leftmost-first
  preference order, supported syntax, flags, and scalar-based matching.
- `datetime` — the Instant/Duration/Date/Time model, civil-calendar math, the OS
  clock/zone seam, and parse/format rules.
- `csv` — the RFC-4180-aligned dialect (delimiters, quoting, record separators).
- `json` — the `Json` union data model and parse/stringify behavior.
- `http` — the HTTP/1.1 client model (the `Response` record, header handling,
  caps, transport selection).
- `url` — the URL parsing/rendering model (`net::Url`).
- `math-rng` — the PCG64 algorithm, seeding, and `math::seed` semantics.

## See Also

* ./mfb man regex — and the other per-package API references
* ./mfb spec architecture frontend — how these source packages are injected
* ./mfb spec memory arenas — where the per-arena RNG stream state lives
