# plan-12-C: Coverage — Tier-C OS, Tier-D CLI/repo, gate

Last updated: 2026-07-01
Effort: large

Part **C** of plan-12 (Built-in Unit Tests + Coverage). Covers the OS object/link emitters and the
CLI + repository crate, then flips the enforcement gate. Shared design lives in the overview:
[plan-12-unit-tests.md](plan-12-unit-tests.md).

- **Depends on:** plan-12-A (tooling) and plan-12-B (the gate is meaningful only once the bulk is
  covered).
- **Design:** overview "Testing strategy by tier" (Tiers C/D), "Verification & acceptance criteria".

## Phases

### Phase C1 — Tier C OS writers

- [ ] Cover `os/macos/object.rs`, the full Linux backend, and extend `os/macos/link.rs`.
- [ ] Run `cargo llvm-cov --no-report` on macOS to capture platform-gated arms; merge with the Linux CI profile and gate on the unioned report.

Acceptance: the Tier-C writers ≥95% in the unioned macOS+Linux report (syscall/UI lines on the documented exception list).
Commit: —

### Phase C2 — Tier D CLI + repository crate

- [ ] Cover manifest parsing/validation in `main.rs` (after the optional `cli/`+`manifest/` split).
- [ ] Fill `repository/src/{package,main,lib}.rs` gaps.

Acceptance: the CLI + repository-crate files ≥95% in both reports; suite green.
Commit: —

### Phase C3 — Gate

- [ ] Flip `--fail-under-lines` to 95 and enable per-file enforcement in `coverage-check.sh`.
- [ ] Document the justified-exception list.
- [ ] Require the `coverage` CI job to pass on PRs.

Acceptance: the project-complete criteria hold — every in-scope file ≥95% in the `cargo llvm-cov` report on both macOS and Linux CI (same tool ⇒ identical per-platform numbers, union covers platform-gated arms), the CI `coverage` job gates PRs at `--fail-under-lines 95` with per-file check, and every excluded region carries an inline reason.
Commit: —
