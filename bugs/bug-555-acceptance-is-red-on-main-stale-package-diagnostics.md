# bug-555: acceptance has been RED on main since `8f0ebfeb8` — two package diagnostics changed and their goldens did not

Last updated: 2026-09-05
Effort: small (decide which side is right, then one golden edit or one diagnostic fix)
Severity: MEDIUM (a red harness that every later branch inherits, and whose obvious "fix" is to re-baseline)
Class: Stale golden / diagnostic regression

Status: Open — **needs the `8f0ebfeb8` author's call** on which side is right

## The finding

`bash scripts/test-accept.sh <mfb> <scratch>` on main reports

    acceptance tests failed: 4 mismatch(es)
      mismatch: syntax/packages/package-comparable-import-invalid/build.log
      mismatch: syntax/packages/package-unknown-member-invalid/build.log

The two `build.log` goldens and their `.testrun` siblings are stale relative to
`8f0ebfeb8` ("fix(front end): a qualified imported package TYPE named a type
nothing else knew"), which changed how a qualified imported package type
resolves — and with it, the diagnostic for one that does not exist.

## The diverging line

`package-unknown-member-invalid/src/main.mfb:15`:

    golden:  error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: identifier could not be resolved
                 Package `package_import_as` does not export `NoSuchType`.

    actual:  error[2-201-0015 SYMBOL_UNKNOWN_TYPE]: type name could not be resolved
                 Type `NoSuchType` is not a built-in or top-level project type.

Line 14 of the same fixture still matches, so this is one specific resolution
path, not a wholesale change.

## Why this is filed and NOT re-baselined

AGENTS.md's four-question gate: the golden wins unless proven wrong, and **the
new message is worse**. The old one names the package and says it does not export
the symbol; the new one is generic and does not mention the import at all. For a
developer who mistyped a type in a qualified package reference, the golden's
wording is the actionable one.

But whether the new resolution path is *correct* — and the message merely an
unintended consequence — is `8f0ebfeb8`'s intent to state, not mine to guess.
Both outcomes are plausible:

- if the new path is right, the fix is to restore the specific message on it and
  regenerate the two goldens;
- if it is wrong, `SYMBOL_UNKNOWN_TYPE` is firing where the package-aware check
  should still have run.

Either way the goldens must not simply be re-baselined to the generic message —
that would silently ratify a diagnostic regression, which is the exact failure
mode the four-question gate exists to prevent.

## Reproduction

Independent of any in-flight branch — reproduced on a worktree whose only change
is the riscv64 linker (`bug-552`), which cannot touch front-end resolution:

    $ ./target/release/mfb build tests/syntax/packages/package-unknown-member-invalid
    …:15 error[2-201-0015 SYMBOL_UNKNOWN_TYPE]: type name could not be resolved
    $ grep SYMBOL_UNKNOWN …/golden/build.log
    …:15 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: identifier could not be resolved

## Why it matters beyond the two fixtures

**Every branch cut from main inherits a red acceptance run.** Four mismatches that
are not yours are exactly the noise that trains a reader to skim the harness's
output — and the harness is the only instrument that compares `build.log` and
`.run` goldens at all (`cargo test` does not run it, and the artifact gate is
execution-free). A standing red run is how a real regression gets waved through.

Two sessions have now had to prove these four are not theirs before landing
unrelated work.

References: `8f0ebfeb8`; `tests/syntax/packages/package-unknown-member-invalid/`,
`tests/syntax/packages/package-comparable-import-invalid/`; AGENTS.md
"Never edit a test/golden to pass"; `.ai/testing-gates.md` (the acceptance
harness is not in `cargo test`).

Possibly related, both peer-filed and both about imported package symbols:
**bug-551** (an exported package constant does not resolve for an importer) and
**bug-554** (an imported union/enum loses its members). Worth reading together —
they may share `8f0ebfeb8`'s resolution path.
