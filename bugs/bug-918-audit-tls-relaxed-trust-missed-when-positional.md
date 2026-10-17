# bug-918: `mfb audit` misses `AUDIT-TLS-RELAXED-TRUST` when `allowSelfSigned` is passed positionally

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: tests/cli/audit-tls-relaxed-trust-positional (new; name suggested)

## Cause

- Rule: an argument binds to a parameter by name OR by position (`mfb spec language` call binding); audit rows keyed on a parameter must bind arguments the way the compiler does.
- Where: `src/audit/collect/source.rs:relaxes_certificate_trust` matches only `CallArg::Named { name: "allowSelfSigned" }`; `CallArg::Positional(_) => false`.
- Fix: bind the call's arguments to the parameter names of each `tls::connect` overload from the registry descriptor (the compiler's own parameter table), then test the argument bound to `allowSelfSigned` for a literal `TRUE`; no hard-coded slot numbers. Deletes the hard-wired positional arm and its comment.

`mfb audit` reports a `tls::connect` call that hard-codes `allowSelfSigned := TRUE`
as the `AUDIT-TLS-RELAXED-TRUST` warning. The same relaxation written as a plain
positional argument, `tls::connect("h", 443, 1000, "h", TRUE)`, compiles to the
same call and the same weakened trust, but audit reports nothing for it. The
warning is the audit's only signal that transport trust was lowered, so anyone can
get past it by dropping the parameter name. That is the silent under-report that
the code comment on `relaxes_certificate_trust` calls "the bad direction".

**Correct behavior:** a literal `TRUE` bound to the `allowSelfSigned` parameter
raises `AUDIT-TLS-RELAXED-TRUST` whether it is passed by name or by position. That
means the 5th positional argument of the host/port overload and the 4th of the
`net::Address` overload.

References:

- `src/docs/spec/tooling/04_audit-format.md` finding catalogue: "`AUDIT-TLS-RELAXED-TRUST` | network | warning | a `tls::connect` call passes `allowSelfSigned := TRUE`, so a chain whose root is not in the host trust store is accepted".
- bug-477 (the original finding that added this audit row).
- Found by hardening batch B13 (tooling audit-format).

## Failing Reproduction

```
mkdir -p /tmp/t/src && cd /tmp/t
printf '{"name":"t","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src"}]}' > project.json
cat > src/main.mfb <<'EOF'
IMPORT tls

FUNC main() AS Nothing
  RES a = tls::connect("example.com", 443, 1000, "example.com", TRUE)
  RES b = tls::connect("example.com", 443, 1000, "example.com", allowSelfSigned := TRUE)
END FUNC
EOF
mfb build      # Wrote executable to ./build/t.out  (both forms are accepted)
mfb audit --format json | python3 -c "import json,sys;print([(f['code'],f['location']) for f in json.load(sys.stdin)['findings'] if 'TLS' in f['code']])"
```

- Observed: `[('AUDIT-TLS-RELAXED-TRUST', {'path': 'src/main.mfb', 'line': 5})]`. Only the named call on line 5 is reported. The positional call on line 4 has no finding, and the text summary says `warnings: 1`.
- Expected: two findings, at lines 4 and 5.

Contrast: the named form is reported correctly, and so is `allowSelfSigned := FALSE` (no finding, correctly).

## Root Cause

`src/audit/collect/source.rs:relaxes_certificate_trust` only matches
`CallArg::Named { name: "allowSelfSigned", value: Expression::Boolean(true) }`.
The positional arm is hard-wired: `CallArg::Positional(_) => false`. Its own
comment names the positions ("The 5th positional of the host/port form, 4th of
the `Address` form") but does not check them. The doc comment's justification,
"the argument is matched by NAME, which is how it binds in each", is not true
for the language: a positional argument binds to the same parameter.

## Goal

- A literal `TRUE` in the `allowSelfSigned` slot raises the warning by name or by position, for both `tls::connect` overloads.

### Non-goals (must NOT change)

- Non-literal values stay unreported, as the comment documents.
- The finding's severity, category and message text stay as they are.

## Blast Radius

- `src/audit/collect/source.rs:relaxes_certificate_trust`: fixed by this bug.
- Other audit rows keyed on a named argument: grep `CallArg::Named` in `src/audit/` (only this site found).

## Fix Design

Resolve the positional index per overload: host/port form = 5 positional
arguments with a Boolean last, `Address` form = 4. Either check the arity and the
argument's static type, or reuse the builtin's per-position parameter-name table
(the shape pass's `CalleeParams::Builtin*` alias tables in `src/ir/shape.rs`), so
the audit maps a position to `allowSelfSigned` the same way the compiler does.

## Validation Plan

- Regression test: an audit golden whose source has both spellings, expecting two findings.
- Doc sync: the catalogue row could say "binds `TRUE` to `allowSelfSigned` (by name or position)".

## Summary

This is a one-function fix. The risk is choosing the slot index per overload,
which the compiler's own parameter tables already answer.
