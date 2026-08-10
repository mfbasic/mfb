# Build, formatting & vendor toolchain

Project-specific mechanics for formatting (rustfmt), linting (clippy), Linux-box builds/testing, and rebuilding vendored libraries in the MFB compiler.

## rustfmt policy: keep the tree current with the tool

The accepted policy is to keep the tree current with the locally-shipped rustfmt rather than pin to whatever older version last formatted it ("if we don't keep up with the tool, it becomes a mess"). `cargo fmt --all` + commit is the accepted path. `main` was reformatted with **rustfmt 1.9.0-stable (2026-05-25)**. Use the project's pinned toolchain: `rustup run 1.96.0 cargo fmt --all`.

`cargo fmt --check` prints one `Diff in …` line **PER HUNK**, so its count runs ~5× the real file count — it counts hunks, not files; don't panic at a large number.

When you touch Rust, run `cargo fmt` normally and commit the result — do NOT hand-match an older style. Whole-tree churn from a rustfmt upgrade is fine to commit on `main` when the user asks.

Two enduring gotchas:
- rustfmt does **not** follow `include!`. The `tls/schannel_{impl,io,read_close,server}.rs` files are `include!`d into `schannel.rs`, so `cargo fmt` skips them entirely — they are hand-formatted in a compact multi-arg-on-one-line style. Running `rustfmt <file>` directly on one of them expands those calls (wrong); keep new edits in the compact neighbor style.
- `cargo fmt -- <files>` does not scope (see the module-tree recursion section below).

Revert fmt churn with `git restore .` only inside an isolated worktree, never the shared tree.

## rustfmt recurses the module tree

There is **no `[workspace]` table** in the root `Cargo.toml` — `mfb_repository` is a plain path dependency — so root `cargo fmt` does NOT reach it. `repository/` **needs its own pass**: `cargo fmt && (cd repository && cargo fmt)`.

**`cargo fmt -- <files>` does NOT scope formatting to those files** — it formats the entire workspace (the `--` args are rustfmt options). For targeted formatting use `rustfmt --edition 2021 <files>` directly. To prove a diff is fmt-only: `diff <(git show HEAD:$f | rustfmt --edition 2021) $f`.

Why this recursion matters: `rustfmt <file>` follows `mod` declarations and formats every module reachable from it, so `rustfmt src/main.rs` = format the entire crate. When the tree is not fmt-clean, tidying your own new code can silently reformat many unrelated files.

If you ever contaminate a dirty tree, do NOT blanket `git checkout`. Prove each file is only your reformatting first: clone HEAD to /tmp, run the same rustfmt there, `cmp` each file, and restore only exact matches by explicit file list.

### Zero blanket dead-code allows
All eight file-level `#![allow(dead_code)]` attributes were removed (they had covered 2,634 lines) and the tree builds warning-free; `cargo check --all-targets` is clean. That state is the regression guard: a new dead item is reported the moment it appears. The rule is written into `AGENTS.md`.

If an item must stay without a reader, give it a **targeted** `#[allow(dead_code)]` (or `#[cfg(test)]`) plus a comment naming what makes it load-bearing — a spec `[[path:symbol]]` anchor, a layout slot, an integrity guard. Never justify with "consumed by a later phase": a dozen such promises had their phases land by another route or be dropped, and three attributes were outright false (the suppressed item had 3–76 references).

Two traps:
- `src/target/shared/code/simd_kernel_coeffs.rs` is **generated** by `tools/math-kernels/gen_coeffs.py`. Deleting a dead coefficient block there is undone by the next regeneration — use a targeted allow instead.
- A doc comment claiming an item is used is not evidence. Re-grep before acting.

## clippy --fix trims double-double constants

`cargo clippy --fix --all-targets` rewrites every full-precision `hi` half in `src/target/shared/code/builder_pow.rs` and `builder_simd_float_math.rs` to the shortest literal that round-trips a lone `f64` (e.g. `6.931_471_805_599_452_862_27e-01` → `6.931_471_805_599_453e-1`). Those extra digits are exactly what the paired `lo` tail recombines against, so the "fix" silently degrades `pow`/`exp`/`log`/`sin`/`cos` accuracy.

Why: `--fix` applies `excessive_precision` mechanically; it cannot read the prose saying the precision is deliberate.

**How to apply:** add `#![allow(clippy::excessive_precision)]` and `#![allow(clippy::approx_constant)]` to both files **before** running `--fix`, never after. Both allows are in the tree — re-check the two files' diffs after any `--fix` run.

Related trap: `approx_constant` is **deny-by-default** (clippy correctness group), so a missing allow makes `cargo clippy --all-targets` exit non-zero, not merely warn.

## Linux boxes may lack a Rust toolchain

Only some Linux boxes have a system `cargo`/`rustc`; others have none. Where a box does have a system cargo (e.g. an Alpine x86_64/musl box with `/usr/bin/cargo` and no rustup), the coverage-CI test suite CAN be reproduced natively — but **remove `rust-toolchain.toml` first** (it pins 1.96.0 and with no rustup to honor it cargo would otherwise error). Where no toolchain exists, prove Linux behavior by **cross-compiling here and shipping** the executable.

The working cross-ship pattern (`scripts/linux-runtime-proof.sh`): cross-build each fixture here, `scp` the executable, run it on the box, diff against the committed `golden/build.log`. Two traps:
- **Fixtures address data files by REPO-ROOT-relative path**, because `test-accept.sh` runs every binary with the repo root as cwd. Ship the `tests/` tree and run from it, or every fs fixture silently reports "not found" and still exits 0 — looks like a real regression but is pure harness error.
- **The `[exit N]` marker is not always on its own line.** It's appended with `echo`, so a program whose last write lacks a trailing newline (every `term::` fixture) yields `...[0m[exit 0]` on one line. Compare the golden tail verbatim.

Single-core boxes exist — keep JOBS low there.

Some boxes have known pre-existing failures unrelated to your change (e.g. a glibc 2.42 box: 21 `rt-behavior` fixtures — the `resources/*` cluster plus fs tempfile/buffered ones — segfault at teardown after printing correct output; proven pre-existing since byte-identical binaries from the pre-refactor compiler fail identically).

### linux-artifact-baseline.sh: RELEASE binary + JOBS=10, always
It cross-builds ~1014 fixtures × 3 Linux targets. With a **debug** `mfb` and no parallelism it produces roughly **6 manifest lines per minute** — a ~66-hour run that never finishes inside a session (measured, not estimated). Release + `JOBS=10` finishes the full corpus in minutes:
```
JOBS=10 scripts/linux-artifact-baseline.sh target/release/mfb capture <manifest>
```

To baseline against pre-change output, build the old compiler in a **separate worktree** (`git worktree add /tmp/base <commit>`) rather than stashing — this repo forbids tree-wide git restore and other clients share the tree.

Corollary easy to miss: the baseline builds **console mode only**. Linux **app-mode** codegen is not covered by it and has no committed goldens — diff it separately with `--app` if a change touches the app path.

## Linux vendor locator needs arch + libc (emits BOTH libc flavors)

Any test/fixture that renders its `libraries` `vendor` locator from `BuildTarget::host()` os/arch with **no `libc`** passes on the macOS dev host (a valid single-target locator) but breaks on Linux CI. On Linux `os="linux"`, and a Linux `vendor` locator that omits `libc` is a hard `PROJECT_JSON_LIBRARY_INVALID` (rule 2-200-0014, validation at `src/manifest/mod.rs` ~1049) — a glibc `.so` cannot double as musl. The build fails at validation, panicking a "should succeed" expect. Classic macOS-authored fixture that never ran green on Linux.

**Non-obvious domain invariant:** a Linux native **console** build emits BOTH libc flavors (glibc + musl) from one invocation (`native_libs.rs` `emitted_link_targets`); each resolves the linked library independently and **hard-errors on a no-match**, and both blobs share the one flat `build/vendor/` (`vendor_output_dirs`, whose comment notes filenames must be unique so a glibc blob and a musl blob never collide). So a single logical library needs **two** vendor locators (glibc + musl) with **distinct source filenames**, and **two** copies land in `build/vendor/`. macOS has no libc axis → one locator, one copy.

**How to apply:** any test/fixture built from `BuildTarget::host()` os/arch is untested on the other OSes; the coverage gate runs on Linux, so verify the Linux shape. Cross-check: run the built `mfb` on a linux-only-locator project.json on macOS — validation is host-independent, so acceptance shows as "no PROJECT_JSON_LIBRARY_INVALID, only advisory NATIVE_LIBRARY_TARGET_UNCOVERED".

## libsnd vendor rebuild mechanics

Rebuilding a `bindings/libsnd/vendor/` library for a Linux box:

- **USE `bindings/libsnd/buildLibsndfile.sh` verbatim** — it is standalone and downloads/builds ogg, vorbis, flac, opus, and libsndfile itself into `$(pwd)/build/`. Never hand-build or reuse a pre-extracted libsndfile source tree; the old per-box manual builds are obsolete. The user wants the script to be the single source of truth for vendor libs.
- Output lands at `~/build/output/lib/libsndfile.so.1.0.37` on the box. Script-built libs are ~2.3MB (static codecs, unstripped) vs the old ~800KB manual ones — the size jump is expected. Vendor naming, when asked to import one: `vendor/libsndfile.so.1.0.37-<arch>-<libc>`.
- **A clang box quirk is durable:** on a box where gcc 15.3 + binutils 2.40 (2023) mismatch — gcc emits `.base64` pseudo-ops the old `as` rejects (vorbis fails) — run the script with `CC=clang CXX=clang++` (clang 21's integrated assembler); no script change needed.
- FLAC 1.5.0 cmake hard-errors without pandoc; the script passes `-DINSTALL_MANPAGES=OFF` to avoid that.
- **How to apply:** scp the script to the box HOME dir and run it there — nothing more:
  ```
  scp -P <port> buildLibsndfile.sh test@127.0.0.1:~/ && ssh -p <port> test@127.0.0.1 'cd ~ && bash buildLibsndfile.sh'
  ```
  (prefix `CC=clang CXX=clang++` where needed). Output stays on the box at `~/build/output/lib/`. Do NOT invent extra steps: no scratch subdirectories, and do NOT copy the lib back into `vendor/` unless explicitly asked — the user retrieves it themselves. Keep box home directories clean; old build residue in `~` made it look like the script wasn't used.
