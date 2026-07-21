# plan-60-B: Confirmation prompt and the resolve-first apply pipeline

Last updated: 2026-07-21
Effort: small (< 1h)
Depends on: plan-60-A
Produces:
- `cli::confirm(question: &str, assume_yes: bool) -> Result<bool, String>` — the
  interactive yes/no prompt with a non-interactive guard. Consumed by E and F.
- `cli::resolve::apply_manifest_change(project_dir: &Path, new_contents: &str) -> Result<(), String>`
  — the resolve-first mutate→resolve→write→install pipeline. Consumed by C, E, F.
- The zero-dependency lockfile policy (§4.3), which F depends on to remove the
  last package without erroring.

Three commands in this feature mutate `project.json` and must then reconcile the
lock and the `packages/` directory: `add` (C), `update` (E) and `remove` (F). Two
of them must ask the user a yes/no question first. This letter builds both shared
primitives once, before any consumer exists.

**Behavioral outcome:** `apply_manifest_change` leaves `project.json` untouched on
disk when resolution fails, and leaves `project.json`, `mfb.lock` and `packages/`
mutually consistent when it succeeds. `confirm` returns the user's answer on a
TTY, returns `true` immediately when `assume_yes` is set, and errors rather than
hanging or guessing when stdin is not a terminal.

This letter is separate from its first consumer on purpose. `confirm` is used by
E and F; `apply_manifest_change` by C, E and F. Bundling a shared surface with
whichever letter happens to need it first would force the later letters to depend
on an unrelated command's letter, and would mean re-cutting the split the moment
the second consumer arrives.

References:

- `src/cli/resolve.rs` — `resolve()` at `:155`, `install()` at `:89`,
  `write_lock()` at `:529`, `read_lock()` at `:604`
- `src/cli/repo.rs:97-103` — the only existing interactive stdin read in the tree
- `src/audit/collect/mod.rs:87` — `project_hash`, the lock/manifest consistency key

## Prerequisites

See plan-60-A for the plan-wide prerequisite gate. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-60-A complete (publisher commands moved, `run_pkg_command` trimmed) | `sed -n '/pub(crate) fn run_pkg_command/,/^}/p' src/cli/pkg.rs \| grep -c 'publish_package_project\|transfer_offer\|transfer_accept\|set_release_state\|check_abi'` → 0 | **MET** (2026-07-21, → 0). plan-60-A archived to `planning/old-plans/`. |
| plan-60-A's move is behaviorally live (the stronger check) | `mfb pkg publish alice .` → exit 2 naming `mfb repo publish`; `mfb repo publish --help` surface present in `REPO_HELP` | **MET** (2026-07-21) — asserted by `cli::pkg::tests::run_pkg_rejects_the_moved_publisher_commands` and `help_lists_each_moved_command_under_repo_only` |

If plan-60-A is not complete, this plan cannot start, full stop.

> **Corrected 2026-07-21 (see Corrections).** The original check was
> `grep -c '"publish"' src/cli/pkg.rs` → 0. That command does not measure the
> stated condition: after plan-60-A it returns **3**, all legitimate — one is the
> moved-command guard plan-60-A deliberately added at `src/cli/pkg.rs:95`, two are
> test assertion lists naming the moved commands. Taken at face value it would
> have blocked this letter on a gate that plan-60-A *passing* is what trips. The
> replacement greps only inside `run_pkg_command`'s body for calls to the five
> implementations, which is the condition the row actually claims.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop.

## 1. Goal

- `apply_manifest_change` performs the full mutate→resolve→persist→install
  sequence such that a resolution failure writes nothing to disk.
- `confirm` provides a single prompt implementation with a `--yes` bypass and an
  explicit non-interactive failure.
- Removing every registry dependency is representable without a resolver error.

### Non-goals (explicit constraints)

- **No new CLI commands or flags.** This letter adds no dispatch arms. `--yes` is
  *parsed* by E and F; B only accepts the resulting boolean.
- **No change to `resolve()`'s selection algorithm.** The fixpoint, the ABI
  superset rule, pin handling and diamond-conflict detection are untouched.
- **No change to `mfb.lock`'s format or `lockfileVersion`.**
- **No change to existing `mfb pkg update` / `mfb pkg install` behavior.** This
  letter adds a function; it does not rewire the existing callers. C, E and F do
  that.

## 2. Current State

`update()` (`src/cli/resolve.rs:72`) is the closest thing to the pipeline this
letter extracts, but it reads the manifest from disk and therefore cannot
resolve against a *proposed* manifest:

```
read_manifest → read_lock (for the diff) → resolve → print_lock_diff
              → write_lock → install
```

`add_package_from_registry` (`src/cli/pkg.rs:613`) does the opposite of
resolve-first: it picks a version with `select_index_version` (`:639`), downloads
and installs the blob (`:661`), and only then rewrites `project.json` (`:676`).
It never touches `mfb.lock`.

The one interactive read in the tree is the machine-pairing code at
`src/cli/repo.rs:97-103`: a bare `println!` prompt plus `stdin().lock().read_line`,
with no TTY check and no non-interactive escape. There is no reusable confirm
helper — verified by
`grep -rn 'stdin()\|read_line\|fn confirm\|prompt' src/cli/`, which returns only
that site plus two unrelated doc-comment matches.

### Measured populations

| What | Count | Command |
|---|---|---|
| Interactive stdin reads in `src/cli/` | 1 | `grep -rn 'stdin()' src/cli/ \| wc -l` → 1 (`repo.rs:99`) |
| Reusable confirm/prompt helpers in `src/` | 0 | `grep -rn 'fn confirm\|fn prompt' src/ \| wc -l` → 0 |
| Callers of `resolve::install` today | 1 | `grep -rn 'resolve::install\|install(project_dir)' src/cli/ \| wc -l` → 1 (`resolve.rs:83`, inside `update`) |
| Callers of `resolve::resolve` today | 1 | `grep -rn 'resolve(&manifest)' src/cli/ \| wc -l` → 1 (`resolve.rs:75`) |

### Verified properties

- **`resolve()` computes the lock's `projectHash` from the manifest it is
  handed**, not from disk (`src/cli/resolve.rs:314`,
  `project_hash: crate::audit::project_hash(manifest)`). Read the function body.
  This is what makes resolve-first possible: resolving against a proposed
  in-memory manifest yields a lock whose `projectHash` already matches the
  `project.json` we are about to write.
- **`install()` re-reads `project.json` from disk and compares hashes**
  (`src/cli/resolve.rs:90-101`). Read the body. Therefore `project.json` must be
  written *before* `install()` runs — the pipeline order in §4.2 is forced, not
  stylistic.
- **`resolve()` hard-errors when the manifest declares no registry dependencies**:
  `src/cli/resolve.rs:179-181` returns `"project.json declares no registry
  dependencies to resolve"` when `registry_deps.is_empty()`. Read the body. This
  is the reason §4.3 exists — without a zero-dependency path, F's `remove` of the
  last package would fail.
- **An empty `Lock` cannot be written and then installed.** `resolve()` derives
  `repo_fingerprint` from the first node's index (`src/cli/resolve.rs:308-312`),
  defaulting to `""` when there are no nodes; `install()` then compares that
  against the pinned server key and errors (`src/cli/resolve.rs:104-110`). Read
  both. So "write an empty lock" is not a viable zero-dependency policy — see
  §4.3 and the Open Decision.
- **`std::io::IsTerminal` is available.** Stable since Rust 1.70; toolchain is
  1.96.0 (`rustc --version` → `rustc 1.96.0 (ac68faa20 2026-05-25)`). It is not
  currently imported anywhere in `src/` (`grep -rn 'IsTerminal' src/` → 0 hits),
  so this letter adds the first use.

## 3. Design Overview

Two independent pieces, neither of which changes observable behavior on its own.

**Design uncertainty: concentrated in §4.3**, the zero-dependency policy. It is
the only decision here that is not forced by the code, and it is the one F
depends on. It is settled in this letter — before F is written — precisely so F
does not have to invent it mid-flight.

**Correctness risk: concentrated in the failure ordering of §4.2.** The pipeline
touches three pieces of on-disk state in sequence, and a failure between any two
leaves a partial state. The design accepts one such window deliberately (§4.2)
and states what recovers from it.

**Rejected alternative: mutate `project.json` first and roll back on failure.**
Rejected because rollback is unreliable in exactly the cases that matter — a
crash, a signal, or a failure *during* the rollback write leaves the manifest
corrupt with no record of the intended state. Resolving against an in-memory
manifest makes the failure case a no-op instead of a repair.

**Rejected alternative: make `confirm` return `false` on a non-TTY.** Rejected:
a silent "no" makes `mfb pkg remove x` in CI appear to succeed while doing
nothing. An explicit error is the only answer that cannot be mistaken for
success.

## 4. Detailed Design

### 4.1 `confirm`

```
pub(crate) fn confirm(question: &str, assume_yes: bool) -> Result<bool, String>
```

- `assume_yes == true` → return `Ok(true)` without printing a prompt or reading
  stdin. The caller has already been told what will happen by its own output.
- stdin is not a terminal (`std::io::stdin().is_terminal() == false`) → return
  `Err`, with a message naming the flag that would have bypassed the prompt:
  `"refusing to prompt for confirmation in a non-interactive session; pass --yes
  to proceed"`.
- Otherwise print `<question> [y/N]: `, read one line, and return `true` only for
  a case-insensitive `y` or `yes` after trimming. **Default is no** — an empty
  line, EOF, or anything unrecognized is `false`.

Placement: `src/cli/mod.rs`, alongside the other cross-command helpers
(`install_verified_package` at `:61`, `install_vendor_file` at `:91`). It is not
package-specific and both consumers live outside `pkg.rs`'s natural scope.

Prompt text is supplied by the caller, so E and F each phrase their own question
and print their own consequence list before calling.

### 4.2 `apply_manifest_change`

```
pub(crate) fn apply_manifest_change(project_dir: &Path, new_contents: &str) -> Result<(), String>
```

Callers construct the *complete proposed `project.json` text* and hand it over.
They do not hand over a mutation closure — the existing manifest editors
(`project_json_with_package` at `src/manifest/package.rs:547`,
`project_json_with_updated_ident_key` at `:644`) already work by surgical string
edit to preserve formatting and comments, and this signature keeps that property.

Sequence:

1. Parse `new_contents` with `parse_project_json`; run `validate_packages_array`.
   Failure → `Err`, nothing written.
2. If the parsed manifest declares **zero** registry dependencies, take the §4.3
   path and return.
3. `resolve(&manifest)` → `Lock`. **Failure → `Err`, nothing written.** This is
   the resolve-first guarantee: a diamond conflict, an unpublished anchor version,
   or a non-converging graph leaves the project exactly as it was.
4. `print_lock_diff(previous.as_ref(), &lock)` — read the previous lock with
   `read_lock` before step 3 so the diff is available.
5. Write `project.json` = `new_contents`.
6. `write_lock(project_dir, &lock)`.
7. `install(project_dir)`.

**The accepted failure window is between steps 5 and 7.** If `install` fails
(network, a blob that does not verify), `project.json` and `mfb.lock` are already
written and mutually consistent — `projectHash` matches, because step 3 computed
it from the same manifest text. Only `packages/` is incomplete, and `mfb pkg
install` recovers it. This is strictly better than the alternative, where a
partially-populated `packages/` is paired with a manifest that never mentioned
the new dependency.

Callers print their own success line (`Added …`, `Updated …`, `Removed …`) after
`apply_manifest_change` returns.

### 4.3 The zero-dependency path

When the proposed manifest declares no registry dependencies, `resolve()` cannot
be called (verified in §2 — it errors), and a synthesized empty `Lock` cannot be
installed (also verified — `repo_fingerprint` would be empty and `install`
rejects it).

**Policy: write `project.json`, then delete `mfb.lock` if it exists, and skip
resolve and install.** A project with no registry dependencies has nothing to
lock; an absent lock is the same state a freshly-`mfb init`-ed project is in, and
`mfb pkg install` already reports that state correctly — `"no mfb.lock; run
\`mfb pkg update\` to resolve dependencies first"` (`src/cli/resolve.rs:92`).

Deleting `packages/` contents is **not** part of this path — that is F's cleanup
task, which knows which package names it removed.

## Compatibility / Format Impact

None. This letter adds two internal functions and changes no command, no output,
no file format. The zero-dependency policy in §4.3 becomes observable only when F
lands the `remove` command that can reach it.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes. An unticked box means NOT DONE.

### Phase 1 — `confirm`

- [x] Add `pub(crate) fn confirm(question: &str, assume_yes: bool) -> Result<bool, String>`
      to `src/cli/mod.rs`, implementing §4.1: `assume_yes` short-circuit, non-TTY
      error, `[y/N]` prompt defaulting to no.
- [x] ~~Import `std::io::IsTerminal` (first use in the tree).~~ — imported, but
      **not** the first use: `src/cli/spec.rs:2` and `src/cli/man.rs:1` already
      use it (Corrections #2). Imported function-locally, matching neither
      file's module-level style but keeping the trait out of `mod.rs`'s scope,
      where it is needed by exactly one function.
- [x] Tests in `src/cli/mod.rs`: `assume_yes == true` returns `Ok(true)` without
      reading stdin; answer parsing accepts `y`/`Y`/`yes`/`YES` and rejects
      empty, `n`, and arbitrary text. Factor the answer parsing into a separate
      pure function (e.g. `answer_is_yes(&str) -> bool`) so it is unit-testable
      without a TTY — the TTY-gated wrapper is then a two-line shell around it.
      Done: `answer_is_yes_only_for_an_explicit_yes` covers 8 yes-forms and 14
      no-forms, including near-misses (`yep`, `yess`, `y e s`) and plausible
      affirmatives that are not the documented answer (`sure`, `1`, `true`).
- [x] **Added task:** a third test, `confirm_refuses_to_prompt_without_a_terminal`,
      pinning that a non-interactive session errors rather than silently
      answering no, and that the message names `--yes`.
- [x] **Added task** (Corrections #3): targeted `#[allow(dead_code)]` on both
      functions, each commented with the letter that must delete it. Needed
      because B lands its primitives before any consumer exists, and the plan
      requires a warning-free build.

Acceptance: `cargo test --bin mfb` passes, including a test that
`answer_is_yes("")` is `false` (the default-no property) and that `confirm` with
`assume_yes` never touches stdin. **VERIFIED** — 3146 passed / 0 failed (from
3143), 0 build warnings.

The `assume_yes` test is **A/B-verified, not assumed**: deleting the
short-circuit makes it fail. That works because the suite's stdin is not a
terminal, so without the short-circuit `confirm` returns the non-TTY `Err` — which
is what makes `Ok(true)` a genuine proof that the short-circuit precedes any I/O,
rather than a tautology.
Commit: —

### Phase 2 — `apply_manifest_change`

- [ ] Add `pub(crate) fn apply_manifest_change(project_dir: &Path, new_contents: &str) -> Result<(), String>`
      to `src/cli/resolve.rs`, implementing §4.2's seven steps in order.
- [ ] Implement the §4.3 zero-dependency path: write `project.json`, remove
      `mfb.lock` if present, skip resolve and install, return `Ok`.
- [ ] Do **not** rewire `update()` (`src/cli/resolve.rs:72`) to use it. `update`'s
      bare form re-resolves the *on-disk* manifest, which is a different
      operation; letter E decides its final shape.
- [ ] Add a targeted `#[allow(dead_code)]` **only if** `cargo check --all-targets`
      reports the new function as unused before C lands, with a comment naming
      plan-60-C as the consumer — and delete the attribute in C. Per AGENTS.md,
      "consumed by a later phase" attributes rot; if C is landing in the same
      session, prefer landing B and C together over adding the attribute.
- [ ] Tests in `src/cli/resolve.rs`: the zero-dependency path writes the manifest
      and removes an existing `mfb.lock` (use a `tempfile::tempdir` project, as
      the existing lock round-trip test at `:1026` does). The resolve-failure
      path cannot be unit-tested without a registry — it is covered by the
      acceptance test in Phase 3.

Acceptance: `cargo test --bin mfb` passes; a unit test proves that a manifest with
an empty `packages` array results in `project.json` written and `mfb.lock` absent.
Commit: —

### Phase 3 — Acceptance coverage for the resolve-first guarantee

The property that makes this letter worth having cannot be proven by unit tests —
it needs a registry that can produce a resolution failure.

- [ ] Add a test to `tests/repo_acceptance.rs` that drives
      `apply_manifest_change`'s failure path end to end: construct a proposed
      manifest naming a version that is not published, invoke it via whichever
      command reaches it (this test lands with C if no command reaches it yet —
      note that dependency here and in C's plan rather than leaving the test
      unwritten).
- [ ] Assert that after the failure, `project.json` is **byte-identical** to its
      pre-invocation contents and `mfb.lock` is unchanged.

Acceptance: `cargo test --test repo_acceptance` passes, and the new test fails if
step 5 of §4.2 is moved before step 3 — verify this by temporarily reordering and
confirming the test goes red, then restore. A test that cannot fail is not
coverage.
Commit: —

## Validation Plan

- **Tests:** `answer_is_yes` parsing table (`src/cli/mod.rs`); `assume_yes`
  short-circuit; zero-dependency manifest handling (`src/cli/resolve.rs`);
  resolve-first atomicity (`tests/repo_acceptance.rs`).
- **Coverage check:** `confirm`'s TTY branch is not reachable from the test suite
  — that is why §4.1 splits the pure parsing out. Confirm `answer_is_yes` and the
  `assume_yes` path are in the denominator; do **not** mark the whole function
  `coverage:off`, which would hide the parts that are testable.
- **Runtime proof:** with a package project and the acceptance-harness registry,
  edit `project.json` to name an unpublished version, run the command that calls
  `apply_manifest_change`, and confirm with `git diff` that `project.json` and
  `mfb.lock` are untouched.
- **Doc sync:** none — this letter adds no observable contract. §4.3's policy
  becomes documentable when F lands the command that reaches it.
- **Acceptance:** `cargo build && cargo test --bin mfb && cargo test --test repo_acceptance`,
  **and `scripts/test-accept.sh target/debug/mfb target/accept-actual`** — the
  project gate required by `.ai/compiler.md:67` for any change that can affect
  generated diagnostics, which CLI output is. The three cargo commands alone do
  **not** see the goldens that embed CLI output: in plan-60-A they missed a
  `USAGE` golden entirely (plan-60-A Corrections #8). Any letter here that
  changes `USAGE`/`PKG_HELP`/`REPO_HELP` or a command's printed output must
  expect `tests/syntax/packages/audit-usage/golden/audit_usage.audit` to move,
  and must run the four-question check in AGENTS.md before regenerating it.

## Open Decisions

- **Zero-dependency policy: delete `mfb.lock`, or keep a lock with an empty
  `packages` array?** Recommended: **delete**. Verified in §2 that an empty lock's
  `repoFingerprint` would be `""` and `install` rejects it, so keeping one
  requires either special-casing `install` or synthesizing a fingerprint from a
  registry the project no longer depends on. Deleting reuses a state the tooling
  already handles correctly. The cost is that `mfb pkg install` then says "no
  mfb.lock; run `mfb pkg update`" rather than "nothing to install" — which is
  slightly misleading prose that F should improve when it can distinguish the two
  cases. (§4.3)

## Corrections

**#1 — The Prerequisites check measured the wrong thing and would have
false-blocked this letter.** (Found at the gate, 2026-07-21.) The row asserted
"plan-60-A complete (`run_pkg_command` trimmed)" but checked it with
`grep -c '"publish"' src/cli/pkg.rs` → 0. After plan-60-A that command returns
**3**, and all three are correct:

- `src/cli/pkg.rs:95` — the moved-command guard plan-60-A's Open Decision
  resolved to add, which emits `mfb pkg publish has moved to mfb repo publish`.
  This exists *because* A succeeded.
- `src/cli/pkg.rs:1873`, `:1921` — assertion lists in the two tests that pin the
  moved commands' rejection and the exactly-one-parent help rule.

So the check is inverted with respect to its own claim: plan-60-A passing is
exactly what makes it fail. A gate row is a stop condition, so taken literally it
would have halted plan-60 at the letter after the one that satisfied it.
Replaced with a check scoped to `run_pkg_command`'s body (0 calls to the five
implementations remain), plus a behavioral second row. Both MET.

**Worth noting for the remaining letters:** C/D/E/F may carry similarly-shaped
"grep for a string" gates. A grep over a whole file cannot distinguish dispatch
from a test that asserts the dispatch is gone — check the construct, not the
spelling.

**#2 — `IsTerminal` is not "first use in the tree".** (Found in Phase 1,
2026-07-21.) Phase 1's task says so, but `src/cli/spec.rs:2` and
`src/cli/man.rs:1` already import it. Harmless to the design — noted only because
the claim implied there was no house style to follow, and there is one.

**#3 — B's primitives cannot land warning-free without a dead-code attribute.**
(Found in Phase 1, 2026-07-21.) Phase 2 anticipates this for
`apply_manifest_change` but Phase 1 does not for `confirm`, and it applies
equally: `confirm`/`answer_is_yes` have no consumer until plan-60-E. Both carry a
targeted `#[allow(dead_code)]` naming the letter that must delete it (E), per the
plan's own guidance for the Phase 2 case. Note the attribute is longer-lived than
Phase 2's: C consumes `apply_manifest_change`, but nothing consumes `confirm`
until E.

## Summary

The real engineering content is §4.2's ordering and the failure window it
accepts, and §4.3's zero-dependency policy — both forced by properties of
`resolve()` and `install()` that were read, not assumed (§2, Verified
properties). Everything else is a small prompt helper.

Untouched: the selection algorithm, the lockfile format, and every existing
command's behavior. This letter is inert until C consumes it.
