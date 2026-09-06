# bug-550: every `debug_assert!` in the tree is decorative — CI builds release, so none of the 55 ever runs

Last updated: 2026-09-05
Effort: small for the CI job; medium if load-bearing assertions are promoted
Severity: MEDIUM (missing gate; it has already cost one miscompile)
Class: Missing gate / test infrastructure

Status: Open — needs a decision on WHICH of the two fixes (see "The decision")

## The finding

    $ grep -rn 'debug_assert' src/ | wc -l
    55
    $ grep -nE 'cargo (test|build)' .github/workflows/coverage.yml
    50:  run: cargo build --release --bin mfb
    168: run: cargo test --release --workspace --no-fail-fast
    173: run: cargo test --release --workspace --target x86_64-unknown-linux-musl --no-fail-fast

`coverage.yml` is the only workflow. Every job builds and tests **release**, and
`debug_assertions` is off in release, so all 55 assertions are compiled out on
every platform CI covers. They are documentation that looks like enforcement.

This is the same class as the four dead-enforcement findings of 2026-09-05
(bug-470's lock covering 3 of 11 writers; `curve448_secret_paths_are_branch_free`
covering one of two curve fields; `test-winapp.sh` covering one of two halves of
itself; bug-491's pin check whose callers discarded its `Result`) — with the
sharpest variant: here the check is *written and correct* and simply never runs.

## It has already cost a miscompile

`strings::left`/`right`/`padLeft`/`padRight` raised `ErrInvalidArgument` while
declaring `errors: vec![]`. `inline_builtin_is_infallible` therefore proved them
infallible, the compiler emitted `TYPE_INLINE_TRAP_DEAD_HANDLER`, **deleted the
live handler**, and the program aborted with `7-705-0002` instead of recovering.

The invariant that would have caught it is encoded: `raise_error_bare` is meant
to assert that a raised error appears in the member's `errors:` list. That
assertion is a `debug_assert!`. It has never executed in CI, so four members
carried the wrong declaration until an inline-`TRAP` fixture found them by
behaviour (fixed 2026-09-05, `6d9f4b79a`).

## Reproduction

Not a runtime bug; the reproduction is the two commands above. To confirm an
individual assertion is inert, build release and exercise the path it guards —
e.g. before `6d9f4b79a`, a `strings::padLeft` with a negative width raised
without tripping the declaration assertion.

## The decision

Two fixes, and they are not exclusive:

1. **Add a debug-assertions CI job.** Cheapest, and it turns all 55 on at once.
   Cost: a second `cargo test` matrix entry; the suite is already the long pole in
   CI, and a debug build of this compiler is substantially slower than release,
   so this is a real wall-clock decision rather than a free one. It also risks a
   burst of pre-existing failures the first time it runs — which is information,
   but has to be budgeted for.
2. **Promote the load-bearing ones to real `assert!`.** Targeted: the ones that
   encode a compiler invariant whose violation is a miscompile (the
   `raise_error_bare` declaration check is the proven example) become
   unconditional. Cost: a release-mode branch on a hot path for some of them, and
   someone has to classify all 55.

A reasonable split is (2) for the handful that guard miscompiles and (1) for
everything else, but that is a judgement about CI budget and hot-path cost, so it
is recorded here rather than assumed.

## Blast radius

All 55 sites; `src/optimizer/`, `src/ir/`, `src/codegen/`, `src/target/` and the
canvas backends each hold some. Turning them on for the first time will surface
whatever has rotted behind them — expect the first run to be informative rather
than green.

References: `.github/workflows/coverage.yml` (the only workflow);
`src/codegen/builtins/mod.rs:inline_builtin_is_infallible`;
`bugs/completed/bug-533-*` and the `6d9f4b79a` commit (the two most recent
dead-handler miscompiles); `.ai/testing-gates.md`.

Found while landing `6d9f4b79a` — the fix's root-cause analysis named the
`debug_assert!` as the reason the invariant was unenforced, which generalizes
past that bug.
