### 1. `List OF Byte` is advertised as an `expectEqual` operand but cannot be compared
UNIT:      man-pkg:testing
PAGE:      package-wide (overview, expectEqual, expectNEqual)
CATEGORY:  consistency
CLAIM:     "the operands must be comparable and printable (a number, Boolean, String, Byte, Scalar, or List OF Byte)" (overview); "Both operands must be printable — a number, Boolean, String, Byte, Scalar, or List OF Byte" (expectEqual, expectNEqual)
VERDICT:   misleading
EVIDENCE:  Probe `/tmp/plan-125-scratch/B-recheck/man-pkg-testing/p1` with `LET a AS List OF Byte = [1, 2]` / `LET b AS List OF Byte = [1, 2]` / `expectEqual(a, b)`; `mfb test` printed `error[2-208-0003 TESTING_EXPECT_INCOMPARABLE]: expectEqual/expectNEqual operands must be comparable with "=" ; got List OF Byte and List OF Byte` and `error[2-203-0061 TYPE_REQUIRES_COMPARABLE]`. The same list is repeated verbatim on three pages (`grep -n "List OF Byte" src/codegen/builtins/testing/*.rs`: mod.rs:112, func_expect_equal.rs:19, func_expect_n_equal.rs:18). The parenthetical names the *printable* set, but a developer reads it as the set of values the assertion accepts, and `List OF Byte` fails the comparable half on every one of those pages. Money and Fixed via `expectEqual` pass (same probe), so "a number" is fine.
SUGGESTED: On all three pages, drop `List OF Byte` from the list, or split the two requirements so neither can be misread: "the operands must be comparable with `=` (a number, Boolean, String, Byte, or Scalar — not a List, record, or Map) and printable, because the failure message has to show them."

### 2. Nothing says a `TCASE` must sit inside a `TGROUP`
UNIT:      man-pkg:testing
PAGE:      package-wide (overview)
CATEGORY:  coverage
CLAIM:     "Inside it, TGROUP groups cases and TCASE holds one case's statements; both take a string description ... and groups nest as deeply as you like"
VERDICT:   missing
EVIDENCE:  Probe `p4`: a `TCASE` written directly under `TESTING` fails with `error[1-102-0010 MFB_PARSE_TESTING_EXPECTED_TGROUP]: a TESTING block may contain only TGROUP groups` (and a `FUNC` inside the block is rejected the same way). The overview's only example happens to nest the case in a group, but no sentence states the rule, and the spec's grammar (`TESTING NEWLINE Group* END TESTING`) is the only place it is written. A developer writing their first one-case test is the person most likely to omit the group.
SUGGESTED: Add after the structure sentence: "A `TESTING` block holds only `TGROUP`s; every `TCASE` lives inside a group, and nothing else — no `FUNC` or `LET` — may appear directly under `TESTING`. Helpers go beside the block as ordinary top-level declarations."

### 3. Assertions cannot be factored into a helper `FUNC`
UNIT:      man-pkg:testing
PAGE:      package-wide (overview)
CATEGORY:  coverage
CLAIM:     "An assertion is only valid inside a TCASE body; using one anywhere else is a compile error."
VERDICT:   missing
EVIDENCE:  Probe `p3`: `FUNC check(a AS Integer) AS Nothing` containing `expectEqual(a, 1)`, called from a `TCASE`, is rejected at compile time with `error[2-208-0001 TESTING_EXPECT_OUTSIDE_TCASE]`. The sentence is literally true, but the one placement a developer actually reaches for — a shared assertion helper called from several cases — is not called out, and the rule as read ("inside a TCASE body") invites the interpretation that a call made *from* a TCASE body counts. Twelve function pages each repeat "valid only inside a TCASE body" and none closes this gap either.
SUGGESTED: Extend the overview sentence: "...using one anywhere else — including inside a `FUNC` that a case calls — is a compile error. Put shared setup in a helper that returns a value and assert on it in each case."

### 4. Whether `mfb test` runs `main` is never stated
UNIT:      man-pkg:testing
PAGE:      package-wide (overview)
CATEGORY:  coverage
CLAIM:     "mfb test [path] builds the project with its TESTING blocks and runs them"
VERDICT:   missing
EVIDENCE:  Probe `p5` has `io::print("MAIN RAN")` in `main`; `mfb test` output contains only the case tree and summary, never that line. Every example on the twelve function pages carries a `main` that prints something, which makes the question unavoidable for a reader copying one, and the only answer is in the spec ("replaces the normal entry point with a synthesized driver").
SUGGESTED: Add one clause to the overview: "The program's own `main` is not run; only the cases are."
