# bug-936: wrong-arity `collections::merge`/`partition` diagnostics print the internal monomorph name `#collections_merge$String$Integer`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (diagnostic leaks compiler-internal naming)

Status: Open
Regression Test: tests/syntax/collections/merge-partition-arity-invalid (new; name suggested — compile-fail golden pinning the public name)

A wrong-arity call to `collections::merge` or `collections::partition` reports
`TYPE_CALL_ARITY_MISMATCH` naming the **monomorphized internal instance** — the untypeable `#`
sigil plus the `$`-mangled type suffix — instead of the public member:

```
Call to `#collections_merge$String$Integer` has 2 argument(s), expected 3 to 3.
Call to `#collections_partition$Integer` has 1 argument(s), expected 2 to 2.
```

`architecture internal-naming` makes this a hard rule: "The untypeable sigil must never leak into
user-facing messages. `display_name` maps a sigil name back to its readable `__` form". Here neither
`display_name` nor a public-name fallback runs, so the user sees a name they cannot write and a
`$String$Integer` suffix from a different naming domain. bug-443 fixed the same leak for other
source-companion members (it is FIXED, all 12 `*_invalid` fixtures pass); `merge` and `partition` are
residue. Sibling members in the same build print a public name (`collections::sort`,
`collections.reduceRight`, `collections.transform` — note these two spellings also disagree, `::`
vs `.`).

**Correct behavior:** the message names the public member as the sibling diagnostics do, e.g.
``Call to `collections::merge` has 2 argument(s), expected 3.``

References:

- `src/docs/spec/architecture/19_internal-naming.md` "Display name: diagnostics".
- bugs/completed/bug-443-source-companion-argmismatch-diagnostic.md (same leak, other members).
- Found in hardening batch B12 (architecture/19_internal-naming) while testing resource escape.

## Failing Reproduction

```
IMPORT io
IMPORT collections
FUNC main() AS Nothing
  LET m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  LET r = collections::merge(m, m)
END FUNC
```

`mfb build` (macOS aarch64, /tmp/hardening/mfb at HEAD):

- Observed: ``main.mfb:5 error[2-203-0022 TYPE_CALL_ARITY_MISMATCH]: ... Call to `#collections_merge$String$Integer` has 2 argument(s), expected 3 to 3.``
- Expected: the same rule and line, naming `collections::merge` (no `#`, no `$…` suffix).

Same leak: `LET r = collections::partition(xs)` with `xs AS List OF Integer` →
``Call to `#collections_partition$Integer` has 1 argument(s), expected 2 to 2.``; with a resource map
the suffix grows (`#collections_merge$String$RES$fs$File`).

Contrast (measured in the same sweep, one wrong-arity call each):

| call | named in the diagnostic |
| --- | --- |
| `collections::sort(xs, 1, 2)` | `collections::sort` ✓ |
| `collections::transform(xs)` | `collections.transform` ✓ (dotted) |
| `collections::reduceRight(xs, 0)` | `collections.reduceRight` ✓ (dotted) |
| `collections::merge(m, m)` | `#collections_merge$String$Integer` ✗ |
| `collections::partition(xs)` | `#collections_partition$Integer` ✗ |
| `collections::sortBy(xs)` / `groupBy(xs)` / `mapValues(m)` / `zip(xs)` | no arity error at all: `TYPE_CALL_ARGUMENT_MISMATCH` "cannot infer template argument(s) `U` … (they appear only in the return type)" — misleading, the call is simply missing its callback/second list |

## Root Cause

Hypothesis (not traced line by line): `merge` and `partition` are source-generic companions whose
monomorphizer instantiation succeeds on the supplied arguments (the template parameters are fully
determined by arg 0), so the call is rewritten to the concrete instance `#collections_<m>$<types>`
(`src/monomorph/helpers.rs:mangle_name` + `internal_name::internalize`) and the later arity check
(`TYPE_CALL_ARITY_MISMATCH` in the shape pass / IR verifier) renders the instance's name raw instead
of through `crate::internal_name::display_name` and the public-name mapping bug-443 added. The
`sortBy`/`groupBy`/`mapValues`/`zip` cases fail earlier, at instantiation (a template parameter only
inferable from the missing argument), which is why they get the "cannot infer" wording instead.
Confirm by grepping the `TYPE_CALL_ARITY_MISMATCH` emitters for the callee rendering.

## Goal

- Every wrong-arity call to a `collections` member names the public member, with one spelling.
- A missing trailing callback argument is reported as an arity mismatch, not as "cannot infer
  template argument(s)".

### Non-goals (must NOT change)

- Correct calls compile to the same instances (no change to `mangle_name` or the `_mfb_ifn_` symbols).

## Blast Radius

- `merge`, `partition` — observed.
- Any other source-generic `collections`/`json`/`regex`/`encoding` member whose template parameters are
  all fixed by the leading argument — latent, same hazard, not individually measured.
