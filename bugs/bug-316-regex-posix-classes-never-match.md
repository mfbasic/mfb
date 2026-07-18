# bug-316: regex POSIX classes `[:alnum:] [:word:] [:xdigit:] [:blank:] [:graph:] [:print:]` silently never match

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/ (new) — `regex::match("a5", "^[[:alnum:]]+$")` is TRUE

`__regex_posixProp` maps these six POSIX class names to sentinel tokens
(`"posixAlnum"`, `"posixWord"`, `"posixXdigit"`, `"posixBlank"`, `"posixGraph"`,
`"posixPrint"`), but `__regex_propTest` has no case for any `posix*` token, so they
fall through to `__regex_scriptTest`, which returns FALSE. The class parses as valid
(non-empty prop name), so no error is raised — a valid-looking pattern silently
matches nothing. Silently-wrong-match is a worse failure mode than a parse error.

The single correct behavior a fix produces: the six POSIX classes match their
intended character sets (or, if unsupported, raise a parse error rather than silently
matching nothing).

References:

- `mfb spec stdlib regex` documents that these "effectively never match a scalar"
  (docs match behavior — but silent empty-match is still a footgun).
- Found during goal-06 review of `src/builtins/regex_package.mfb`.

## Failing Reproduction

```
' regex::match("a5", "^[[:alnum:]]+$")  -> FALSE
' regex::match("a",  "^[[:word:]]+$")   -> FALSE
' regex::match("f",  "^[[:xdigit:]]+$") -> FALSE
' regex::match(" ",  "^[[:blank:]]+$")  -> FALSE
```

- Observed: all FALSE (the classes never match).
- Expected: TRUE (`[[:digit:]]` → Nd already works).

## Root Cause

`src/builtins/regex_package.mfb:1040-1081` (`__regex_posixProp`) emits `posix*`
sentinel tokens that `__regex_propTest` (`:467-500`) has no case for, so they fall
through to `__regex_scriptTest` → FALSE.

## Goal

- Implement the six `posix*` tokens in `__regex_propTest` (alnum = L*/Nl/Nd; word =
  alnum + Pc; xdigit = [0-9A-Fa-f]; blank = \t + Zs; graph/print via category), or
  make the unimplemented names a parse error (`FAIL 77050003`).

### Non-goals (must NOT change)

- The working `[[:digit:]]` (Nd) and other property classes.
- The regex public API.

## Blast Radius

- `__regex_propTest` (add the six cases) and `__regex_posixProp` — fixed here.
- All `regex::` entry points using character classes benefit.

## Fix Design

Add the six token cases to `__regex_propTest` mapping to the correct category
predicates. If full support is deferred for some, make those a parse error so callers
see the gap. Rejected: leaving them silently matching nothing — a correctness footgun.

## Phases

### Phase 1 — failing test
- [ ] Tests for the four repro classes (FALSE today).
### Phase 2 — the fix
- [ ] Implement the six POSIX classes (or error on unsupported).
### Phase 3 — validation
- [ ] Full regex suite green; the classes match correctly.

## Validation Plan

- Regression: POSIX-class match tests.
- Doc sync: update the regex spec/man to reflect the now-working classes.

## Summary

Six POSIX character classes parse but silently match nothing due to a missing
`propTest` case; implementing them (or erroring) fixes a silent-wrong-result footgun.
