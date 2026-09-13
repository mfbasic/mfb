### 1. Public names contradict the rendered declarations
UNIT:      man-pkg:testing
PAGE:      package-wide
CATEGORY:  consistency
CLAIM:     “These are unqualified global builtins: you write them as bare names (`expectEqual(actual, expected)`), never `testing::expectEqual`.”
VERDICT:   inconsistent
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man testing` prints that sentence, but its Functions table lists `testing::expectEqual`; `mfb man testing expectEqual | sed -n '1,34p'` renders the declaration as ``testing::expectEqual(actual AS T, expected AS T) AS Nothing``.
SUGGESTED: Render these global builtins under their callable names—`expectEqual`, etc.—in the Functions table and Declaration. Keep `testing` only as the documentation grouping.

### 2. The unit omits the developer-facing test-framework guide
UNIT:      man-pkg:testing
PAGE:      package-wide
CATEGORY:  coverage
CLAIM:     The unit tells readers assertions belong in a `TCASE` and that `mfb test` runs tests, but never explains how to form a `TESTING` block, nest `TGROUP`s, supply required descriptions, invoke `mfb test [path]`, interpret its exit status, or discover `--coverage`.
VERDICT:   missing
EVIDENCE:  `rg -l -i 'TESTING|TGROUP|TCASE' src/docs/man` finds only tour pages, not a test-framework guide; `mfb man testing --all | rg -n 'Structure|nested|description|mfb test \\[|--coverage|exits non-zero|TGROUP'` finds only example snippets. The missing developer behavior is implemented and documented only in `src/docs/spec/language/22_test-framework.md` (“Structure”, “Running”, and “Coverage”), while the overview directs the terminal user to `mfb spec language test-framework`.
SUGGESTED: Add a concise developer-facing test-framework section to the testing overview (or a discoverable narrative man topic): minimal `TESTING`/`TGROUP`/`TCASE` shape, string descriptions, nested groups, `mfb test [path]`, non-zero on failure, and `--coverage`. Replace the spec-only referral with that developer documentation.