# bug-697: post-monomorph diagnostics quote mangled generic type names (`Box$Integer`) instead of source spellings

Last updated: 2026-09-24
Effort: medium (1h–2h)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: `tests/syntax/monomorph/diagnostic-template-spelling-invalid` (new)

Any diagnostic raised after monomorphization names an instantiated user generic
by its **mangled symbol** — `Box$Integer`, `Opt$Integer`, `List OF Opt$Integer`
— not by what the user wrote, `Box OF Integer`. The `$` spelling is not valid
MFBASIC; a user cannot type it, search their source for it, or write the fix it
implies. Nothing miscompiles: every affected program is already being rejected,
and correctly. The cost is that the message is phrased in the compiler's
vocabulary rather than the user's.

**The single correct behavior a fix produces:** every user-visible diagnostic
renders an instantiated user generic in its template spelling —
`Box OF Integer`, `Opt OF Integer`, `List OF Opt OF Integer`,
`Holder OF Holder OF Integer` — whichever pass raises it. `-ir`/`-nir`/`.mfp`
artifacts and symbol names keep the mangled form.

References:

- `./mfb spec architecture monomorphization` — mangling (`mangle_name`) and the
  `type_instantiations` inverse map
- `./mfb spec language templates` §3 — templates are resolved to concrete
  declarations before IR
- Found while fixing bug-680 (`bugs/completed/bug-680-union-template-param-unusable.md`),
  whose exhaustiveness check reported `MATCH on UNION `Opt$Integer``. bug-680's
  own new diagnostics already use the template spelling
  (`Monomorphizer::union_pattern_type` via `template_view`), so the two now
  disagree within one compiler.

## Failing Reproduction

Measured at `e5f483655` with `CARGO_TARGET_DIR=/tmp/b697-target cargo build --release --bin mfb`.

```
mkdir -p /tmp/b697/a/src /tmp/b697/c/src
cat > /tmp/b697/a/project.json <<'EOF'
{"name":"bug697","version":"0.1.0","mfb":"1.0","kind":"executable",
 "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
 "entry":"main","targets":["native"]}
EOF
cp /tmp/b697/a/project.json /tmp/b697/c/project.json
cat > /tmp/b697/a/src/main.mfb <<'EOF'
TYPE Box OF T
  value AS T
END TYPE

FUNC main() AS Integer
  LET b AS Box OF Integer = Box[1]
  LET n AS Integer = b
  RETURN n
END FUNC
EOF
cat > /tmp/b697/c/src/main.mfb <<'EOF'
TYPE Some OF T
  value AS T
END TYPE

TYPE None
  value AS Nothing
END TYPE

UNION Opt OF T
  Some OF T
  None
END UNION

FUNC f(o AS Opt OF Integer) AS Integer
  MATCH o
    CASE Some(s)
      RETURN s.value
  END MATCH
  RETURN 0
END FUNC

FUNC main() AS Integer
  LET x AS Opt OF Integer = Some[1]
  LET y AS List OF Opt OF Integer = [x]
  LET z AS Integer = y
  RETURN f(x)
END FUNC
EOF
mfb build /tmp/b697/a
mfb build /tmp/b697/c
```

- Observed:

  ```
  /tmp/b697/a/src/main.mfb:7 error[2-203-0007 TYPE_BINDING_MISMATCH]: ...
                 Binding `n` has initializer type Box$Integer, expected Integer.
  /tmp/b697/c/src/main.mfb:15 error[2-203-0062 TYPE_MATCH_NOT_EXHAUSTIVE]: ...
                 MATCH on UNION `Opt$Integer` does not cover None; add unguarded CASE arms or CASE ELSE.
  /tmp/b697/c/src/main.mfb:25 error[2-203-0007 TYPE_BINDING_MISMATCH]: ...
                 Binding `z` has initializer type List OF Opt$Integer, expected Integer.
  ```

- Expected: `Box OF Integer`, `UNION `Opt OF Integer``, `List OF Opt OF Integer`.

Contrast cases, correct today:

| Case | Result |
| --- | --- |
| A generic *function* body error (`FUNC bad OF T(x AS T) AS Integer / RETURN x`, called `bad("s")`) | `RETURN value has type String, expected Integer.` — no mangled name; the message names the substituted type, not the function ✓ |
| Diagnostics raised inside the monomorphizer (`TYPE_CALL_ARGUMENT_MISMATCH` via `template_view`, `lower.rs` `actual_view`) | template spelling ✓ |
| bug-680's `CASE `Box` is not a member of UNION `Opt OF Integer`` (`Monomorphizer::union_pattern_type`) | template spelling ✓ |
| No golden pins a mangled name in a diagnostic: `grep -rhE '[A-Za-z]\$[A-Z][a-z]*`' tests --include=build.log` | no matches — so no golden asserts the wrong form today |

## Root Cause

Monomorphization replaces each user-generic instantiation with a nominal whose
name is the mangled symbol, `mangle_name(name, args)` →
`format!("{name}${suffix}")` (`src/monomorph/helpers.rs:mangle_name`), called
from `Monomorphizer::instantiate_type` (`src/monomorph/lower.rs:instantiate_type`).
From that point on, the concrete HIR and the IR contain only
`ParameterType::Named("Box$Integer")`.

The only way back to the source spelling is the monomorphizer's private
`type_instantiations: HashMap<ParameterType, (String, Vec<ParameterType>)>`
(`src/monomorph/mod.rs:Monomorphizer`), read by `Monomorphizer::template_view`
(`src/monomorph/lower.rs:template_view`). The doc on `template_view` records why
it must be a lookup: the mangling is lossy (every non-alphanumeric collapses to
`$`, and `unique_concrete_symbol` disambiguates collisions — bug-400), so a
mangled name cannot be parsed back.

`monomorphize_project` (`src/monomorph/mod.rs:monomorphize_project`) returns only
the concrete `HirProject`; `type_instantiations` is dropped with the
`Monomorphizer`. Every later pass that reports — the post-monomorph resolve
(`resolver::resolve_augmented`, `src/cli/build/mod.rs`), entry validation
(`manifest::entry::validate_entry_point`), and the IR verifier (`ir::verify`,
whose `Verifier::emit` in `src/ir/verify/mod.rs` stores a pre-formatted `detail`
string) — renders types with `ParameterType::name()`, which for a mangled
nominal is the mangled string. The contrast cases are immune only because they
run inside the monomorphizer, while the map is still alive.

## Goal

- The three reproduction diagnostics read `Box OF Integer`,
  `UNION `Opt OF Integer``, and `List OF Opt OF Integer`.
- A nested instantiation renders fully: `Holder OF Holder OF Integer`, never
  `Holder$Holder$Integer` or a half-translated mix.
- A mangle collision disambiguated by `unique_concrete_symbol` still renders the
  correct template spelling for each of the two instantiations.

### Non-goals (must NOT change)

- Mangled symbols themselves, `mangle_name`, `unique_concrete_symbol`, and every
  artifact that carries them: `-ast`/`-ir`/`-nir`/`-nplan`/`-ncode` dumps, the
  `.mfp` type table, native symbol names. Only diagnostic *text* changes.
- Rule codes, severities, line/column attribution, and which diagnostics fire.
- **Tempting wrong fix, forbidden:** demangling by parsing the `$` string
  (`Box$Integer` → split on `$`). The encoding is lossy by design
  (`Map OF K TO V`, `List OF RES File`, qualified `pkg.T` and collision suffixes
  all collapse), so a parser renders some types wrong. It must be a lookup
  against the monomorphizer's own map.
- Changing `ParameterType::name()` itself: IR dumps, symbol keys and package
  encoding call it and must keep the mangled form.

## Blast Radius

Searched with `grep -rn 'self.emit(' src/ir/verify/` (171 emit/report sites
across `types.rs`, `calls.rs`, `ops.rs`, `mod.rs`, `values.rs`, `resources.rs`,
`compat.rs`, `link.rs`, `matching.rs`) and by reading the build pipeline in
`src/cli/build/mod.rs` after `monomorphize_project`.

- `ir::verify` (every module above) — any `detail` that interpolates a type
  `name()` — fixed by this bug (reproduced: `TYPE_BINDING_MISMATCH`,
  `TYPE_MATCH_NOT_EXHAUSTIVE`).
- `resolver::resolve_augmented` (post-monomorph pass) — same hazard for any
  message naming a type; to be fixed by the same mechanism if it renders a
  mangled name (Phase 1 audit confirms or clears it).
- `manifest::entry::validate_entry_point` — names the entry's signature types;
  same audit.
- `ir::verify` run on an **imported `.mfp`** (package IR verified on load) — the
  mangled names there came from another compilation, so there is no
  `type_instantiations` for them; out of scope, and the fix must leave those
  names as they are rather than guess.
- Runtime text (`toString` of a record, panic/trace output) — unaffected by this
  bug's mechanism; the Phase 1 audit records whether any runtime string embeds a
  type name.

## Fix Design

Export the inverse map from monomorphization and apply it where diagnostic text
leaves the compiler, not at each of the ~171 format sites.

1. `monomorphize_project` also returns a `TemplateSpellings` table: for every
   mangled nominal, the rendered `template_view(...)` spelling (a string, fully
   recursive, so nested instantiations are already expanded).
2. Diagnostic text is rewritten through that table at the point where it is
   rendered (`rules::show_diagnostic` / `render_pending`, or one step earlier at
   the verifier/resolver `emit`), replacing each whole mangled token. A mangled
   name contains `$`, which no source identifier can, so a token match on
   `[A-Za-z0-9_.]+(\$[A-Za-z0-9_.]+)+` is unambiguous; replace longest-first so
   `Holder$Holder$Integer` is never partly rewritten as `Holder$Holder OF …`.
   Only exact table hits are replaced; an unknown `$` token (package IR) is left
   as is.

Rejected alternatives:

- *Per-site `display(type)` at all 171 emit sites.* Correct, but a large
  mechanical change that has to be remembered at every future site; the
  render-boundary rewrite covers new sites automatically.
- *Carrying a `source_name` on `HirTypeDecl` / IR types.* Touches the IR shape
  and, through it, `.mfp` encoding — a Non-goal.
- *Parsing the mangled string.* Forbidden above: lossy.

Expected output shift: none in existing goldens (the grep above found no pinned
mangled name in any `build.log`); only the new fixture.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/syntax/monomorph/diagnostic-template-spelling-invalid`
      (layout of `tests/syntax/monomorph/union-template-case-not-member-invalid`):
      the reproduction's `Box` binding mismatch, the `Opt` non-exhaustive MATCH,
      the `List OF Opt OF Integer` mismatch, and a nested
      `Holder OF Holder OF Integer` mismatch. Golden = the *expected* spellings;
      confirm it fails today on the `$` forms.
- [ ] Audit `resolve_augmented` and `validate_entry_point` for a reachable
      mangled name, and runtime strings for any embedded type name; write each
      verdict into Blast Radius.

Acceptance: the fixture fails only on the mangled spellings; every audited site
has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Return the spelling table from `monomorphize_project`, built from
      `type_instantiations` via `template_view`.
- [ ] Thread it to the diagnostic render boundary and rewrite whole mangled
      tokens, longest-first, exact hits only.
- [ ] Cover a `unique_concrete_symbol` collision pair with a unit test on the
      table.

Acceptance: the Phase 1 fixture passes; the contrast cases are unchanged;
`-ir` dumps of an unrelated generic fixture are byte-identical.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Confirm no pre-existing golden moves (expected: none).
- [ ] Full suite: `cargo test --no-fail-fast`.
- [ ] Re-run the reproduction.
- [ ] `./mfb spec architecture monomorphization`: document the spelling table
      and where it is applied.

Acceptance: full suite green; golden delta is exactly the new fixture; the
reproduction prints the template spellings.
Commit: —

## Validation Plan

- Regression test: `tests/syntax/monomorph/diagnostic-template-spelling-invalid`.
- Runtime proof: the reproduction above prints `Box OF Integer`,
  `Opt OF Integer`, `List OF Opt OF Integer`.
- Doc sync: `./mfb spec architecture monomorphization` (the table); no language
  spec change.
- Full suite: `cargo test --no-fail-fast`.

## Open Decisions

- **Where to rewrite.** Recommended: at the single render boundary
  (`rules::show_diagnostic`/`render_pending`), so every pass is covered.
  Alternative: at each pass's `emit`, which keeps `rules` unaware of
  monomorphization but needs the table threaded into three passes.

## Summary

A text-only fix at the diagnostic boundary, driven by a map the monomorphizer
already maintains and currently throws away. The risk is in the rewrite: it must
be an exact lookup, not a parse (the mangling is lossy), and it must not touch
IR, package, or symbol output. No existing golden is expected to move.
