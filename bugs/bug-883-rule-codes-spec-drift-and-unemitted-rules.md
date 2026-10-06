# bug-883: diagnostics/rule-codes spec drift (counts, a dangling "Code Collisions" section, stale shared-code prose) and seven registry rules that nothing emits but the spec presents as live

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: none (doc); optionally extend `rules::tests` with an "every non-reserved rule has an emitter" guard

**Owner decision (2026-10-05):** wire up all seven rules. Each condition (`BUILD_FAILED`,
`PACKAGE_VERSION_UNSUPPORTED`, `NATIVE_MANIFEST_INVALID`, `VERIFICATION_FAILED`,
`TARGET_UNSUPPORTED`, `LINK_FAILED`, `LOCKFILE_MISMATCH`) reports its registered code, with
a location where one exists, in place of today's uncoded `error:` lines. For example,
`--target bogus-arch` is `TARGET_UNSUPPORTED`, reported before "Building …" is printed. Each rule
gets a fixture that triggers it. Part A (counts, dangling section, shared-codes prose) is a
plain spec fix.

**Update (2026-10-05):** **Re-measured at HEAD 846aa55fd after plan-165 and plan-167
landed.** The counts drifted further. `RULES` now has **267** distinct codes (was 255 at
031f64353) and `2-203` has **141** (was 139). The spec still says "All 252 `RULES` entries"
and its `2-203` population row now says **140** (plan-165/167 edited it, but it is still one
short). Measured by extracting the codes inside `pub(super) const RULES` in
`src/rules/table.rs` at each revision. plan-165-C retired `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`,
which is still listed in the registry table. The fix must recount from the table at fix time,
not use the numbers in this doc.

Two classes of defect in `mfb spec diagnostics rule-codes` (and its package page).

**A. SPEC drift against `src/rules/table.rs:RULES`** (measured by parsing both with a script —
`/tmp/hardening/B05/cmp.py`; every row's code, name, severity and message otherwise match):

1. "All 252 `RULES` entries are listed" — the table has **255** entries (and the spec's own table
   lists 255 rows).
2. Subsystem population table: `2-203` "138" — actual **139**.
3. "Allocations are sparse: `2-203-0047`, `2-203-0057`, and `2-203-0134` have no registry entries" —
   `2-203-0141` and `2-203-0142` are also unallocated (codes run to `0144`).
4. Two references to a section *Code Collisions* that does not exist (the section is
   "Code Uniqueness"): §"The Rule Record" ("Different names currently allow duplicate codes to
   escape lookup validation (see *Code Collisions*)") and §`2-205` ("two independently-numbered
   blocks that **share** codes `0001` and `0002` (see *Code Collisions*)"). Both sentences are stale
   since bug-790: the package-metadata pair is now `2-205-0022`/`0023`, codes are unique, and
   `rules::tests::rule_codes_are_unique` enforces it.
5. `src/docs/spec/diagnostics/spec.md`: "A diagnostic's numeric display code is not necessarily
   unique." — false since bug-790 (same test).

**B. Rules with no emitter, presented as live (UNCLEAR — owner decides: wire up or mark reserved).**
The spec says "reserved entries describe their non-emission status"; these seven carry their live
message and no reserved note, yet `grep -rn '<NAME>' src | grep -v rules/table.rs | grep -v src/docs`
returns nothing for each: `2-200-0100 BUILD_FAILED`, `2-205-0022 PACKAGE_VERSION_UNSUPPORTED`,
`2-205-0023 NATIVE_MANIFEST_INVALID`, `3-302-0001 VERIFICATION_FAILED`, `3-304-0001 TARGET_UNSUPPORTED`,
`5-500-0001 LINK_FAILED`, `6-603-0001 LOCKFILE_MISMATCH`. Other spec pages lean on them as real:
`17_native-libraries.md` ("the build-time linker diagnostic `5-500-0001`/`LINK_FAILED`") and
`tooling/03_lockfile.md` ("the `6-603` `LOCKFILE_MISMATCH` diagnostic family"). The conditions they
name are reported today as unstructured `error:` lines with no code, e.g. (measured)
`mfb build --target bogus-arch` → `error: native output does not support bogus-arch yet` (after
printing `Building p1 (executable) for bogus-arch`), and a link-stage failure →
`error: native code internal relocation target 'a' is not defined`.

References:

- src/docs/spec/diagnostics/01_rule-codes.md, src/docs/spec/diagnostics/spec.md.
- src/rules/mod.rs `rule_codes_are_unique`, `rule_names_are_unique`; bug-790.
- Found during B05 hardening of diagnostics/rule-codes.

## Failing Reproduction

```
cd /Users/justinzaun/Development/mfb
python3 /tmp/hardening/B05/cmp.py          # table 255 vs "All 252"; per-prefix counts; gaps 47,57,134,141,142
grep -n "Code Collisions\|share\*\* codes" src/docs/spec/diagnostics/01_rule-codes.md
for n in BUILD_FAILED PACKAGE_VERSION_UNSUPPORTED NATIVE_MANIFEST_INVALID VERIFICATION_FAILED TARGET_UNSUPPORTED LINK_FAILED LOCKFILE_MISMATCH; do grep -rn "$n" src | grep -v rules/table.rs | grep -v src/docs; done   # no output
/tmp/hardening/mfb build --target bogus-arch <proj>
```

- Observed: as listed above.
- Expected: counts 255 / 139; gap list complete; no dangling section reference; no "shared codes" prose; the seven rules either emitted at their conditions or marked "reserved; not emitted".

## Root Cause

Prose not updated by bug-790 (unique codes) and by the additions of `2-203-0143`/`0144`; the seven
rules were registered ahead of emitters that were never written (orchestration paths `eprintln!`
directly, e.g. `src/cli/build/mod.rs`).

## Goal

- The rule-codes page matches `RULES` mechanically, and every row either has an emitter or says it is reserved.

### Non-goals (must NOT change)

- Codes/names of existing rules.

## Blast Radius

- `17_native-libraries.md` and `tooling/03_lockfile.md` citations of `LINK_FAILED`/`LOCKFILE_MISMATCH` — update together.
- Similar unstructured build errors observed: `error: the \`canvas\` package requires app mode (build with -app …)` (`src/cli/build/mod.rs`), the NIR-validator and relocation errors in sibling B05 reports.
