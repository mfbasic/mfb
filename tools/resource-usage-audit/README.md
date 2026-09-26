# tools/resource-usage-audit

Regression check for bug-714 (`bugs/completed/bug-714-dungeon-ships-unused-unlicensed-tiles.md`):
an example's `resources` block in `project.json` bundles files by glob, so
nothing stops it from shipping files its own source never loads (as
`examples/dungeon` did — 288 tiles shipped, 28 used).

- **audit.py** — for one or more example directories (or `--all`, every
  `examples/*` with a `resources` block), statically walks the example's
  `.mfb` source from its `project.json` `entry` function, symbolically
  resolving every `os::appResourcePath(...)` call site to the dst-relative
  resource name(s) it can reference (see the module docstring in
  `audit.py` for the exact method: literal strings, `&`-concatenation,
  `FOR EACH`/`LET` bindings, and one level of user-defined-function
  parameter binding; anything it cannot resolve becomes a wildcard rather
  than a false "unreferenced" — a passing run is real signal, a failing one
  worth checking by hand). Reports, per example:
  - shipped files matched by no reference (unused — the bug-714 shape);
  - exact (non-wildcard) references matched by no shipped file (a typo'd
    or since-renamed resource name).

  Exits non-zero if any checked example has either.

Usage:

```
python3 tools/resource-usage-audit/audit.py --all
python3 tools/resource-usage-audit/audit.py examples/dungeon
```
