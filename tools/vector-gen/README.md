# tools/vector-gen

The built-in `vector` package's ~170 overloads follow strict per-(element-type,
dimension) patterns, so their MFBASIC bodies are generated rather than hand-written.

- **gen_vector_package.py** — prints the package source (the nine vector records and
  every geometry/utility/2D function). The committed source of truth is the set of
  `BODY*` consts in `src/codegen/builtins/vector/{func_,helper_}*.rs`; when semantics
  change, edit the generator, re-run it, and copy the changed FUNC bodies into their
  consts.
- **check_vector_bodies.py** — runs the generator and compares each generated FUNC
  body with its `BODY*` const; exits non-zero on drift. `scripts/check-generated.sh`
  (CI) runs it.
