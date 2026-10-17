# plan-176-R: Builtins code, part A (audio, big, canvas geometry and helpers)

Last updated: 2026-10-17
Effort: large (3h–1d)
Depends on: plan-176-Q

R splits ten over-budget code files under `src/codegen/builtins/`:

- two audio emitters (`gen_macos_stream.rs`, and the `include!` fragment `gen_windows_io.rs`);
- the `big::Int` emitter `gen_big.rs`;
- the two native canvas geometry builders `func_geo_build.rs` and `func_geo_cache.rs`;
- five canvas MFBASIC-helper files (`helper_curves.rs`, `helper_geometry.rs`, `helper_items.rs`,
  `helper_particles.rs`, `helper_picture.rs`), which are mostly raw-string MFBASIC consts.

Every split is pure code motion. Most become `foo/mod.rs` plus parts. The `include!` fragment
instead gains sibling fragments. `helper_geometry`'s one 954-LOC `GEO_HEADER` string becomes
`concat!` of two macro pieces, which produces the same `&'static str`.

Single outcome: after R lands, `cargo test --test file_size_budget` shows no row for any of the
ten R paths or their parts, and `tests/guards/file_size_ceilings.txt` has no `ceiling` row for
any of them. R adds no `exempt` row.

References:

- `planning/plan-176-A-file-size-guard.md`: §2 counting rules, §Decisions, and the
  `.ai/build-tooling.md` section "Splitting an over-budget file (plan-176)".
- `.ai/canvas-threading.md`: names `helper_items.rs`, `helper_geometry.rs`, `func_geo_cache.rs`,
  `helper_particles.rs`, `helper_curves.rs` and `helper_picture.rs`. The sweeps below repoint each.
- `.ai/testing-gates.md` (artifact-gate, "Splits must sweep man AND spec citations"; line 582 names
  `helper_geometry.rs`).
- `.ai/codegen-invariants.md` (registry helpers, raw-string helper bodies).
- `.ai/build-tooling.md` (rustfmt policy; `#[rustfmt::skip]` on raw-string consts stays).
- Spec topics whose citations move: `src/docs/spec/app/06_canvas.md` (56 citations) and
  `src/docs/spec/stdlib/19_big.md` (2). `src/docs/spec/stdlib/11_audio.md:157` names
  `gen_windows_io.rs:emit_exclusive_submit`, which keeps its path and symbol.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-Q archived | `ls planning/plan-176-Q-* planning/completed/plan-176-Q-*` → only `planning/completed/` matches | NOT MET (2026-10-17) |
| plan-176-D's canvas test splits have landed (D precedes Q) | `ls tests/canvas/rt_canvas_curves/driver.rs tests/canvas/rt_canvas_picture_fit/driver.rs tests/canvas/rt_canvas_particles/model.rs` → all exist | NOT MET (2026-10-17) |
| The guard exists | `cargo test --test file_size_budget` passes | NOT MET (2026-10-17) |
| R's files and every file R's sweeps edit have no uncommitted changes | `git status --porcelain -- src/codegen/builtins/audio src/codegen/builtins/big src/codegen/builtins/canvas src/codegen/runtime/canvas/curve_shader.rs src/codegen/runtime/canvas/mod.rs src/docs/spec/app/06_canvas.md src/docs/spec/stdlib/19_big.md tests/canvas tests/guards scripts/test-canvas-vulkan.sh .ai` → empty | NOT MET (2026-10-17: ` M src/docs/spec/app/06_canvas.md`, ` M tests/canvas/rt_canvas_group_ownership.rs`) |

Everything below assumes these rows hold.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify. A
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites**, not only
> the one that blocked you.

## 1. Goal

- Every file the ten splits produce holds ≤ 1000 code LOC and ≤ 250 test LOC. The target is
  ≤ 900 code per part (A's convention). The largest part is `helper_picture/item.rs` at 757.
- Native output and the text of every registered MFBASIC helper are byte-identical. Proof:
  `sh scripts/artifact-gate.sh target/release/mfb all` passes with no golden touched, and the R-BID
  checksums (§3.2) are equal before and after every phase.
- Every spec citation, `.ai/` reference, Rust comment, test doc, script message, `include_str!`
  and source-reading test path that named a moved file now names the file that holds the item.

### Non-goals (explicit constraints)

- No behavior change. The only non-motion edits are:
  - visibility words, and `mod`/`use`/re-export lines;
  - the two `include!` lines in `audio/gen_windows.rs`;
  - the `GEO_HEADER` `concat!` rewrite (§4.6);
  - the `include_str!` path in `runtime/canvas/curve_shader.rs`;
  - the file arguments of the source-reading canvas tests;
  - comment, doc and message text.
- The text inside every raw-string MFBASIC const is moved byte-for-byte. Not one character inside
  `r#"…"#` changes, including the `'` comments there that mention `helper_*.rs` basenames. Shader
  sources (`src/codegen/runtime/canvas/shaders/*.frag`, `*.tpl`) are not edited.
- Each raw-string const keeps its declaration line verbatim, including its visibility word, where
  a source-reading test matches the opener. Examples: `pub(super) const CURVE_BASE: &str =`
  followed by a line starting `r#"`, and `pub(super) const MODEL: &str =` likewise.
- Every crate path callers use today keeps resolving, through re-exports. Examples:
  `audio::gen_macos_stream::lower_close_input`, `big::gen_big::emit_load_int`,
  `canvas::func_geo_build::{record_type, …}`, and `func_geo_cache::register`.
- No `[[test]]` target renamed, no test fn renamed, no test module path changed (the inline
  `mod tests` modules stay in the host `mod.rs`).

## 2. Current State

Sizes from `/tmp/loc/groups.json`, as measured by the planning census
(`python3 -I /tmp/loc/census.py`; after A, `python3 -I tools/loc-census/census.py --file <path>`)
on the working tree of `17a9b5799`:

| File | Kind | Code LOC | Inline test LOC | `#[test]` |
|---|---|---|---|---|
| `src/codegen/builtins/audio/gen_macos_stream.rs` | code | 1055 | 0 | 0 |
| `src/codegen/builtins/audio/gen_windows_io.rs` (`include!` fragment) | code | 1060 | 0 | 0 |
| `src/codegen/builtins/big/gen_big.rs` | code | 2148 | 0 | 0 |
| `src/codegen/builtins/canvas/func_geo_build.rs` | code | 3111 | 66 | 1 |
| `src/codegen/builtins/canvas/func_geo_cache.rs` | code | 1647 | 15 | 1 |
| `src/codegen/builtins/canvas/helper_curves.rs` | code | 1916 (`python3 -I /tmp/loc/span.py …/helper_curves.rs` → `(1916, 95)`) | 95 | 3 |
| `src/codegen/builtins/canvas/helper_geometry.rs` | code | 2210 | 2 (`mod tests;` → existing `helper_geometry/tests/mod.rs`, 4 tests) | 0 |
| `src/codegen/builtins/canvas/helper_items.rs` | code | 1073 | 19 | 1 |
| `src/codegen/builtins/canvas/helper_particles.rs` | code | 1445 | 16 (`HOOKS` + `mod tests;` → existing `helper_particles/tests/mod.rs`, 5 tests) | 0 |
| `src/codegen/builtins/canvas/helper_picture.rs` | code | 1216 | 51 | 2 |

(`grep -c '#\[test\]' <file>` for the last column.) Line numbers below are as of `17a9b5799`.
No earlier letter edits these files, and item names are authoritative if lines drift. Per-part LOC
= `python3 -I /tmp/loc/span.py <file> <a> <b>` summed over the part's ranges (after A:
`tools/loc-census/census.py --file <file> --lines A B`). "+N" means the `use`/`mod`/wrapper lines
the new file adds.

### Verified properties

- **A parent's private imports reach the parts.** Probe `/tmp/loc/probe_qr`, run with
  `cargo build` and `cargo test`. `use super::*;` in a part sees an import that `mod.rs` never
  uses itself: no warning, and the program prints `7`. An explicit `use super::{only_b_uses};` in a
  child test module names a private parent import: `test result: ok. 1 passed`, no warning. That
  second case is the shape of `helper_picture`'s `use super::{ABSENT, HOOKS, PICTURE}`.
- **A `macro_rules!` raw string defined in one file and invoked by path from its parent's `concat!`
  yields the identical `&'static str`.** Probe `/tmp/loc/probe_mac`: `p1.rs`/`p2.rs` each hold
  `macro_rules! geo_pN { () => { r#"…"# }; } pub(super) use geo_pN;`, and `mod.rs` holds
  `#[rustfmt::skip] pub(crate) const H: &str = concat!(p1::geo_p1!(), p2::geo_p2!());`.
  `assert_eq!(H, "ab\ncd\n")` passed and printed `6`. `cargo build` and `cargo clippy` printed no warning.
  Precedent in tree: `helper_render.rs` `render_loop_head!`/`render_loop_tail!` +
  `const RENDER_LOOP: &str = concat!(…)` (`grep -n 'macro_rules\|concat!' src/codegen/builtins/canvas/helper_render.rs`).
- **Byte-identity coverage of canvas and big in the committed goldens is thin.**
  Only 34 fixture directories have native `.ncodesum` goldens
  (`find tests -name '*.ncodesum' | sed 's|/golden/.*||' | sort -u | wc -l` → 34), and one of them
  uses `canvas::`. None uses `big::`. `tests/byte-identity/` has an `audio` selector but no `canvas`
  or `big` selector (`ls tests/byte-identity`). That is why §3.2 adds a direct before/after
  checksum check (R-BID) for the canvas and big phases.
- **R-BID is deterministic and builds.** `tests/canvas/scenes/curves.mfb` in a scaffolded project:
  `mfb build --app --ncode` twice gave the same sha256 (`4bf16adf…`), at 11.8 s per build. A
  `--target linux-x86_64` build succeeded in 39 s. The nine other chosen scenes each built with
  `--app --ncode --target macos-aarch64` (probe output: `canvas_geo_native_matrix ok …` through
  `gradients_on_black ok …`). The `/tmp` big program in §3.2 built in 0.14 s.
- **Source-reading tests and includes that name R files**
  (`grep -rn -E "<basenames>" src tests scripts .ai tools Cargo.toml`, full list in §4):
  - `src/codegen/runtime/canvas/curve_shader.rs:145`
    `include_str!("../../builtins/canvas/helper_curves.rs")` finds `LET __CANVAS_CURVE_ARC_STEPS`
    (line 94, in `CURVE_BASE`), `LET __CANVAS_CURVE_DASH_ROW` (96, `CURVE_BASE`) and
    `LET __CANVAS_PIECE_STEPS` (1269, `CURVE_NEAREST`). All three go to `helper_curves/pieces.rs`.
  - `rt_canvas_curves`'s `chunk("helper_curves.rs", "pub(super) const CURVE_X: &str =\nr#\"")` calls
    (`CURVE_BASE`, `CURVE_REDUCE`, `CURVE_VALID`, `CURVE_CHAIN`, `CURVE_NEAREST`). D moved them into
    `tests/canvas/rt_canvas_curves/driver.rs`.
  - `rt_canvas_particles`'s `helper_source("helper_particles.rs", "pub(super) const MODEL: &str =\nr#\"")`
    (D moved it into `tests/canvas/rt_canvas_particles/model.rs`).
  - `rt_canvas_picture_fit`'s `chunk("helper_picture.rs", "PICTURE_GEOMETRY")` (D moved it into
    `tests/canvas/rt_canvas_picture_fit/driver.rs`).
  - All three test helpers read `CARGO_MANIFEST_DIR/src/codegen/builtins/canvas/<arg>` at run time,
    so the argument `"helper_curves/pieces.rs"` works unchanged.
  - The vector body checker (`tools/vector-gen/check_vector_bodies.py`) globs only
    `src/codegen/builtins/vector/{func,helper}_*.rs`, which contains no R file.
- **No guard allowlist or coverage exception names an R file.**
  `grep -rnE "gen_macos_stream|gen_windows_io|gen_big|func_geo_build|func_geo_cache|helper_curves|helper_geometry|helper_items|helper_particles|helper_picture" tests/guards scripts/coverage-exceptions.txt`
  → no hit. One guard keys on a file NAME:
  `architecture_guards.rs::no_cross_member_os_seam_dispatcher` checks `name.starts_with("func_")`
  for a `body:` + `_native(` registration. Inside `func_geo_build/` and `func_geo_cache/` the file
  names are `mod.rs` and part names. plan-176-O Phase 1 already widened the rule to any file below
  a `func_*` directory, so both modules stay in the check. §4.11 proves it on R's split.
- **Guard scan slices.** `architecture_guards.rs::code_above_tests` truncates at the first
  `#[cfg(test)]`. In `helper_curves.rs` that marker is at line 71
  (`grep -nF '#[cfg(test)]' src/codegen/builtins/canvas/helper_curves.rs | head -1`). So lines
  72–2171 are unscanned today, and the parts (lines 74–1829) become scanned. Scanning that text for
  every pattern those guards forbid gave no output:
  `sed -n 72,2171p …/helper_curves.rs | grep -nE '"%[vf][0-9]|_os_seam|Reg::physical\(|\.physical_field\(|Operand::phys\(|format!\("%[vf]|"(x|w|d|s|v|q)[0-9]+"|"r[abcd]x"|"r[sd]i"|"r[bs]p"'`.
  So no new finding appears. In every other R file the first `#[cfg(test)]` comes after all moved
  code (func_geo_build 3636, func_geo_cache 1918, helper_geometry 2341, helper_items 1135,
  helper_particles 1502, helper_picture 1283; the audio and big files have none).
- **No R file is host-cfg-gated.** `audio/mod.rs:53,55` declare `gen_macos_stream` and
  `gen_windows` unconditionally. These are target emitters compiled on every host, so the host
  `cargo check` covers them and no remote box is needed.
- **Moving Rust code cannot change emission through source locations** (A §2 Verified).

## 3. Design Overview

### 3.1 Conventions

- `foo.rs` → `foo/mod.rs` + parts. `mod.rs` keeps the original `use` block. Each part starts with
  `use super::*;`. A part that holds only a raw-string const needs no `use` line, and a macro part
  needs none either. Prune a `use` only when `cargo check --all-targets` reports it unused.
- A part item that `mod.rs` or a sibling uses gets `pub(super)`. A struct moved into a part and
  named from another file gets `pub(super)` on the struct and on every field. That avoids E0616
  field-privacy errors and `private_interfaces` warnings, and it adds no warning because every field
  is read today. Siblings reach each other's items through `mod.rs`: `mod.rs` imports each part's
  `pub(super)` names (`use paint::{…};`, etc.), and siblings see those imports through
  `use super::*;` (§2 Verified, probe_qr).
- Exception for the `include!` fragment: `gen_windows_io.rs` is spliced textually into
  `gen_windows.rs` (`grep -n 'include!' src/codegen/builtins/audio/gen_windows.rs` → lines 293–295).
  It follows the sibling-fragment precedent (`gen_windows_open.rs`, `gen_windows_devices.rs`), not
  `mod.rs`.
- Citations, `.ai/` text, Rust comments, test docs and script messages that name a moved file are
  repointed in the same commit. The new name is the part holding the named item, or `foo/` when the
  text names the module. Raw-string bodies, shader sources and `bugs/`/`planning/` history are not
  edited (§1).

### 3.2 Gates

Gate class: provably-neutral code motion, so **byte-identity is the gate**. A diff means
root-cause it: objdump or diff ONE fixture, fix the motion error, re-run. A diff is never a stop.

- audio: `sh scripts/artifact-gate.sh target/release/mfb audio` (macos-aarch64, windows-x86_64 and
  the three Linux targets' `.ncodesum`; `ls tests/byte-identity/audio/golden`).
- big and canvas: **R-BID**, a `/tmp`-only before/after checksum. It is a one-off probe for this
  letter, so it goes in `/tmp` and not in `scripts/` (AGENTS.md). Run it with the pre-phase binary
  into `before.txt`. Do the phase. Then `cargo build --release`, run it into `after.txt`, and
  `diff /tmp/r-bid/before.txt /tmp/r-bid/after.txt` → no output.

  ```sh
  # R-BID: run from the repo root; $1 = before|after. Est. 9 min (curves.mfb measured 11.8 s macos-aarch64 + 39 s linux-x86_64; big 0.14 s).
  mkdir -p /tmp/r-bid/proj
  for s in curves curve_pictures_on_black particles_grouped_on_black tilemap picture_shapes_on_black \
           polylines_on_black joins_on_black dashes_on_black gradients_on_black canvas_geo_native_matrix; do
    d=/tmp/r-bid/proj/$s; mkdir -p $d/src; cp tests/canvas/scenes/$s.mfb $d/src/main.mfb
    printf '{ "name": "%s", "version": "0.1.0", "mfb": "1.0", "kind": "executable",\n  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],\n  "entry": "main", "targets": ["native"] }\n' $s > $d/project.json
    for t in macos-aarch64 linux-x86_64; do
      target/release/mfb build --app --ncode --target $t $d >/dev/null && echo "$s $t $(shasum -a 256 < $d/$s.ncode)"
    done
  done > /tmp/r-bid/$1.txt
  d=/tmp/r-bid/proj/bigops; mkdir -p $d/src   # src/main.mfb = the program below
  printf '{ "name": "bigops", "version": "0.1.0", "mfb": "1.0", "kind": "executable",\n  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],\n  "entry": "main", "targets": ["native"] }\n' > $d/project.json
  for t in macos-aarch64 linux-x86_64 windows-x86_64; do
    target/release/mfb build --ncode --target $t $d >/dev/null && echo "bigops $t $(shasum -a 256 < $d/bigops.ncode)"
  done >> /tmp/r-bid/$1.txt
  wc -l < /tmp/r-bid/$1.txt   # → 23
  ```

  The scenes cover curves, curve pictures, particles, tiles, picture shapes, polylines, joins,
  dashes, gradients and the native geometry matrix. Together they cover every R canvas file's helper
  text and the native `func_geo_*` emitters. `bigops/src/main.mfb` reaches every `gen_big` part
  (add/sub, mul/fold, shift, to-string, div-mod, gcd/modPow):

  ```
  IMPORT big
  IMPORT io

  FUNC main() AS Integer
    LET a AS big::Int = big::parse("123456789012345678901234567890")
    LET b AS big::Int = big::fromInteger(-987654321)
    LET q AS big::DivResult = big::divMod(a, b)
    io::print(big::toString(big::add(a, b)))
    io::print(big::toString(big::subtract(a, b)))
    io::print(big::toString(big::multiply(a, b)))
    io::print(big::toString(big::pow(a, 3)))
    io::print(big::toString(big::remainder(a, b)))
    io::print(big::toString(big::divide(a, b)))
    io::print(big::toString(big::shiftLeft(a, 70)))
    io::print(big::toString(big::shiftRight(a, 3)))
    io::print(big::toRadixString(big::factorial(30), 16))
    io::print(big::toString(big::sum([a, b])))
    io::print(big::toString(big::product([a, b])))
    io::print(big::toString(big::gcd(a, b)))
    io::print(big::toString(big::modPow(a, big::fromInteger(65537), big::abs(b))))
    io::print(toString(big::compare(a, b)))
    RETURN 0
  END FUNC
  ```

  (Built in planning: `mfb build --ncode` → `Wrote native code plan to …/bigops.ncode`.)
- Letter end, once: `sh scripts/artifact-gate.sh target/release/mfb all`.

Where risk concentrates:

- `func_geo_build`: 7 parts, 51 new `pub(super)` items plus 7 widened to `canvas` reach, structs crossing files, and items whose
  reach must stay the whole `canvas` module.
- `helper_geometry`: the `GEO_HEADER` macro split, the only non-motion code edit.

These go last. The raw-string helper files are lowest risk because their consts move verbatim and
R-BID catches any byte slip.

Rejected alternatives:

- *`GEO_HEADER` moved whole into `helper_geometry/header.rs` (956 LOC).* It would pass the 1000
  rule but miss the 900 part target, and it leaves one file that cannot grow at all. The `concat!`
  macro form has an in-tree precedent and is proven identical (probe_mac).
- *Splitting `GEO_HEADER` into two registered helpers.* That changes helper names and order, so it
  is not code motion.
- *Converting `gen_windows_io.rs` to a module.* It shares `gen_windows`'s namespace textually, with
  no `use` lines. A module would need imports and visibility edits throughout. The sibling
  `include!` fragments are the existing pattern.
- *Changing the canvas test helpers to search a directory.* That is a test logic change. Repointing
  the file argument is enough.

## 4. Detailed Design

### 4.1 `audio/gen_macos_stream.rs` (1055)

The four `pub(crate)` fns do not call each other (xref: no cross-group use).

- `audio/gen_macos_stream/mod.rs` (545 + 3): lines 1–600 (docs, the 11 `use` lines,
  `lower_open_output`, `lower_close_output`). Adds `mod input;` and
  `pub(crate) use input::{lower_close_input, lower_open_input};`.
- `audio/gen_macos_stream/input.rs` (510 + 1): 602–1142 (`use super::*;`, `lower_open_input`,
  `lower_close_input`).
- Visibility edits: 0. Callers `audio/gen_shared.rs:290,297`, `audio/func_close.rs:33,39`,
  `audio/status_tests.rs:59` and `codegen/resource/tests/mod.rs:194,199` resolve through the
  re-export (`grep -rn "gen_macos_stream::" src`).
- Citations 0. Other refs 0.

### 4.2 `audio/gen_windows_io.rs` (1060, `include!` fragment)

- `audio/gen_windows_io.rs` (517): keeps 1–198 (header comment; shared helpers `emit_copy_bytes`,
  `emit_s16_to_f32`, `emit_f32_to_s16`, `emit_wait_event`, `emit_render_wait_timeout`, `SubmitFail`
  + impl) and 200–597 (the write side `emit_exclusive_submit`, `F_OFF`, `C_OFF`,
  `emit_write_fill`, `lower_write`).
- `audio/gen_windows_read.rs` (385): 599–1010 (`emit_read_fill`, `lower_read`), headed by a `//`
  comment line naming it the read side of `gen_windows`, spliced by `include!`. Fragments carry
  `//` comments, not `//!`, as in `gen_windows_open.rs`.
- `audio/gen_windows_query.rs` (158): 1011–1188 (`emit_unstarted_exclusive_output_ready`,
  `lower_query`), with the same style of header.
- `audio/gen_windows.rs`: after `include!("gen_windows_io.rs");` add
  `include!("gen_windows_read.rs");` and `include!("gen_windows_query.rs");`, before
  `include!("gen_windows_devices.rs");`. Textual order is otherwise unchanged.
- Visibility edits: 0 (one namespace). Cross uses: `F_OFF`/`C_OFF` (write → read) and
  `emit_wait_event` (→ read, query).
- Citation `stdlib/11_audio.md:157` `[[…/gen_windows_io.rs:emit_exclusive_submit]]` stays valid.

### 4.3 `big/gen_big.rs` (2148)

- `big/gen_big/mod.rs` (467 + 13 + 50 + 15): 1–598 (`INT_*` consts, `IntSlots`, `ResultSlots`,
  `emit_spill_args`, `emit_load_int`, `emit_alloc_magnitude`, `WordSlots`, `emit_word_view`,
  `emit_word_at`, `emit_word_or_zero`, `emit_copy_words`, `emit_build_int`,
  `emit_compare_magnitude`, `emit_compare_int`), 2440–2456 (`emit_copy_int`), 2517–2577
  (`emit_trim`, `emit_release_int`, `emit_int_from_integer`). Adds
  `mod add_sub; mod mul; mod shift; mod to_string; mod div_mod;` and the 5 re-export lines (10
  names: every name `big/func_*.rs` imports from `gen_big` that moves, found by flattening each
  `use super::gen_big::{…}`):
  `pub(crate) use add_sub::emit_add_int;`
  `pub(crate) use mul::{emit_fold_list, emit_mul_int};`
  `pub(crate) use shift::{emit_reject_negative, emit_shift_left_magnitude, emit_shift_right_magnitude};`
  `pub(crate) use to_string::emit_int_to_string;`
  `pub(crate) use div_mod::{emit_build_div_result, emit_div_mod_int, emit_rem_int};`.
  `func_parse.rs:267`'s `super::gen_big::ResultSlots` stays in mod.rs.
- `big/gen_big/add_sub.rs` (245): 599–884 (`emit_add_magnitude`, `emit_sub_magnitude`, `emit_add_int`).
- `big/gen_big/mul.rs` (304): 885–1243 (`emit_assign_word_roles`, `emit_mul_magnitude`,
  `emit_mul_int`, `emit_fold_list`).
- `big/gen_big/shift.rs` (156): 1244–1419 (`emit_reject_negative`, `emit_shift_left_magnitude`,
  `emit_shift_right_magnitude`).
- `big/gen_big/to_string.rs` (245): 1420–1707 (`emit_int_to_string`).
- `big/gen_big/div_mod.rs` (618 + 54): 1708–2439 and 2457–2516 (`DivModSlots`, `emit_move_slot`,
  `emit_adopt_result`, `emit_store_constant`, `emit_div_mod_magnitude` (501, the largest item),
  `emit_div_mod_int`, `emit_rem_int`, `emit_build_div_result`).
- Visibility edits: 1. `emit_move_slot` (div_mod.rs) becomes `pub(super)` because `mul.rs` uses it,
  and `mod.rs` adds `use div_mod::emit_move_slot;` so `mul` reaches it through `use super::*;`.
  `mul` reaches `emit_add_int` through the mod.rs re-export.
- Citations (2, `grep -rnoE "\[\[src/codegen/builtins/big/gen_big.rs:[^]]+\]\]" src/docs/spec`):
  `stdlib/19_big.md:44` `emit_load_int` and `:45` `emit_build_int` → `big/gen_big/mod.rs`.
- Prose: `src/codegen/builtins/big/mod.rs:6,94,424` (`gen_big.rs` → `gen_big/`).

### 4.4 Canvas raw-string helpers: `helper_items.rs`, `helper_picture.rs`, `helper_particles.rs`

**`helper_items.rs` (1073 + 19 test).**
- `canvas/helper_items/mod.rs` (25 code + 19 test): 1–63 (docs, `use`, `STROKE_HALF`), 1129–1134
  (`register`), inline tests stay (they read `STROKE_HALF` via `use super::*`). Adds
  `mod geo_read; mod draw_geometry; use geo_read::GEO_READ; use draw_geometry::DRAW_GEOMETRY;`.
- `canvas/helper_items/geo_read.rs` (424): 64–489 (`GEO_READ`; holds `__canvas_geoByte` l.71 and
  `__canvas_drawBound` l.193).
- `canvas/helper_items/draw_geometry.rs` (628): 490–1128 (`DRAW_GEOMETRY`).
- Visibility: `GEO_READ`, `DRAW_GEOMETRY` → `pub(super)`. Nothing reads their opener text.
- Citations (7, `06_canvas.md`): `GEO_READ` (:979, :1033, :1134, :1529, :1541) → `geo_read.rs`;
  `DRAW_GEOMETRY` (:980, :1135) → `draw_geometry.rs`.
- Prose: `.ai/canvas-threading.md:698` (`helper_items.rs:__canvas_drawBound` →
  `helper_items/geo_read.rs:__canvas_drawBound`); `canvas/helper_clamp_byte.rs:13`
  (`helper_items.rs`'s `__canvas_geoByte` → `helper_items/geo_read.rs`'s); `canvas/helper_hit.rs:5`,
  `canvas/func_handle_bridge.rs:148`, `canvas/helper_draw.rs:12` (`helper_items.rs` → `helper_items/`).
  `canvas/helper_items.rs:4`'s `//!` mention of `helper_geometry.rs` moves with mod.rs and is
  repointed in Phase 4.

**`helper_picture.rs` (1216 + 51 test).**
- `canvas/helper_picture/mod.rs` (78 code + 51 test): 1–31 (`use`, `CONSTRUCTED`), 1237–1318
  (`ABSENT`, `#[cfg(test)] HOOKS`, `BODY`, `register`), 1319–1368 inline tests stay (they use
  `use super::{ABSENT, HOOKS, PICTURE}`, and `PICTURE` resolves through mod.rs's private import per
  §2 Verified). Adds `mod geometry; mod item; use geometry::PICTURE_GEOMETRY; use item::PICTURE;`.
- `canvas/helper_picture/geometry.rs` (397): 32–464 (`PICTURE_GEOMETRY`, already `pub(super)`;
  declaration unchanged).
- `canvas/helper_picture/item.rs` (757): 465–1236 (`PICTURE`).
- Visibility: `PICTURE` → `pub(super)`.
- Source-reading test: in `tests/canvas/rt_canvas_picture_fit/driver.rs`,
  `chunk("helper_picture.rs", "PICTURE_GEOMETRY")` → `chunk("helper_picture/geometry.rs", "PICTURE_GEOMETRY")`.
  The test doc `//! (`src/codegen/builtins/canvas/helper_picture.rs:PICTURE_GEOMETRY`)` (root
  `tests/canvas/rt_canvas_picture_fit.rs`) → `…/helper_picture/geometry.rs:PICTURE_GEOMETRY`.
- Citations (4, `06_canvas.md`): `PICTURE` (:977, :2435, :2448) → `item.rs`; `PICTURE_GEOMETRY`
  (:2480) → `geometry.rs`.
- Prose: `.ai/canvas-threading.md:1626` (`helper_picture.rs` → `helper_picture/`).

**`helper_particles.rs` (1445 + 16 test).**
- `canvas/helper_particles/mod.rs` (88): 1–18 (`use`, `CONSTRUCTED`), 1454–1541 (`ABSENT`,
  `#[cfg(test)] HOOKS`, static `BODY`, `register`, `mod tests;`). Adds
  `mod model; mod expand; mod damage; mod hooks;`
  `use model::MODEL; use expand::{DRAW, EXPAND}; use damage::DAMAGE; use hooks::HOOKS_BODY;`
  (`BODY`'s `format!` captures all five; the existing `helper_particles/tests/mod.rs` reads
  `MODEL, EXPAND, ABSENT, HOOKS, BODY` through `use super::*`).
- `canvas/helper_particles/model.rs` (402): 19–438 (`MODEL`, keeps `pub(super)` and its declaration line).
- `canvas/helper_particles/expand.rs` (471): 439–927 (`EXPAND`, `DRAW`; holds
  `__canvas_particleCompose` l.759, `__canvas_particleItem` l.813, `__canvas_particleRecord` l.907).
- `canvas/helper_particles/damage.rs` (252): 928–1197 (`DAMAGE`).
- `canvas/helper_particles/hooks.rs` (254): 1198–1453 (`HOOKS_BODY`).
- Visibility: `EXPAND`, `DRAW`, `DAMAGE`, `HOOKS_BODY` → `pub(super)`.
- Source-reading test: in `tests/canvas/rt_canvas_particles/model.rs`,
  `helper_source("helper_particles.rs", "pub(super) const MODEL: &str =\nr#\"")` →
  `helper_source("helper_particles/model.rs", …)`. Its comment `(`helper_particles.rs:MODEL`)`
  (`grep -rn 'helper_particles.rs:MODEL' tests/canvas`) → `helper_particles/model.rs:MODEL`.
- Script message: `scripts/test-canvas-vulkan.sh:818`
  `src/codegen/builtins/canvas/helper_particles.rs (__canvas_particleCompose, __canvas_particleItem)`
  → `src/codegen/builtins/canvas/helper_particles/expand.rs (…)`.
- Citations (8, `06_canvas.md`): `MODEL` (:1656, :1723) → `model.rs`; `EXPAND` (:1685) and `DRAW`
  (:1745, :1802) → `expand.rs`; `DAMAGE` (:1837) → `damage.rs`; `HOOKS_BODY` (:1650) → `hooks.rs`;
  `ABSENT` (:1866) → `mod.rs`.
- Prose: `.ai/canvas-threading.md:1393` (`helper_particles.rs:__canvas_particleRecord` →
  `helper_particles/expand.rs:__canvas_particleRecord`); `canvas/helper_damage.rs:101`
  (`///`, `helper_particles.rs:DAMAGE` → `helper_particles/damage.rs:DAMAGE`).

### 4.5 `helper_curves.rs` (1916 + 95 test)

Every big const is already `pub(super)`. In a part, `pub(super)` means visible in `helper_curves`,
which is exactly where they are used (`grep -rnwE "CURVE_(BASE|REDUCE|VALID|CHAIN|BUILD|NEAREST|DRAW)" src`
→ hits only in `helper_curves.rs`). So **every declaration line stays byte-identical**, which the
`rt_canvas_curves` opener match needs.

- `canvas/helper_curves/mod.rs` (228 code + 95 test): 1–73 (docs, `use`, `NAMED`,
  `#[cfg(test)] use …CURVE_PIECE_SLOTS as PIECE_FLOATS`), 1830–2073 (`CALLED`, `CURVE_FUNCS`,
  `FUNCS_ABSENT`, `#[cfg(test)] HOOKS`, `ABSENT`, statics `BODY`/`BASE`/`FUNCS`, `register`),
  2074–2171 inline tests stay. Adds `mod pieces; mod reduce; mod build; mod draw;` and
  `use pieces::{CURVE_BASE, CURVE_NEAREST, CURVE_VALID}; use reduce::{CURVE_CHAIN, CURVE_REDUCE}; use build::CURVE_BUILD; use draw::CURVE_DRAW;`
  (the statics' `format!` implicit captures and the tests' `use super::{…}` resolve through these).
- `canvas/helper_curves/pieces.rs` (482): 74–275 (`CURVE_BASE`), 699–741 (`CURVE_VALID`),
  1226–1484 (`CURVE_NEAREST`). Grouping these three puts the three `LET` constants
  `curve_shader.rs` reads in one file, along with `__canvas_cvFlatten` (l.145), `__canvas_cvRefine`
  (l.233) and `__CANVAS_CURVE_PIECE` (l.83).
- `canvas/helper_curves/reduce.rs` (565): 276–698 (`CURVE_REDUCE`), 742–893 (`CURVE_CHAIN`).
- `canvas/helper_curves/build.rs` (319): 894–1225 (`CURVE_BUILD`).
- `canvas/helper_curves/draw.rs` (342): 1485–1829 (`CURVE_DRAW`).
- Visibility edits: 0.
- `src/codegen/runtime/canvas/curve_shader.rs:145`:
  `include_str!("../../builtins/canvas/helper_curves.rs")` →
  `include_str!("../../builtins/canvas/helper_curves/pieces.rs")`; its `//!` line 5 → `helper_curves/`.
- Source-reading test: in `tests/canvas/rt_canvas_curves/driver.rs`, the `chunk` file argument for
  `CURVE_BASE`, `CURVE_VALID`, `CURVE_NEAREST` → `"helper_curves/pieces.rs"`, and for `CURVE_REDUCE`,
  `CURVE_CHAIN` → `"helper_curves/reduce.rs"`. The opener literals do not change. The root doc
  `tests/canvas/rt_canvas_curves.rs:5` (`src/codegen/builtins/canvas/helper_curves.rs:CURVE_BASE`, …)
  → name `helper_curves/pieces.rs` and `helper_curves/reduce.rs` for the respective consts.
- Citations (11, `06_canvas.md`): `CURVE_VALID` (:1252), `CURVE_BASE` (:1291), `CURVE_NEAREST`
  (:1339) → `pieces.rs`; `CURVE_CHAIN` (:1259), `CURVE_REDUCE` (:1290) → `reduce.rs`; `CURVE_BUILD`
  (:1306, :2493) → `build.rs`; `CURVE_DRAW` (:1370, :1940) → `draw.rs`; `ABSENT` (:1377),
  `CURVE_FUNCS` (:1441) → `mod.rs`.
- Prose: `canvas/func_flatten.rs:5` (`helper_curves.rs:__canvas_cvFlatten` → `helper_curves/pieces.rs:…`)
  and `:7` (`helper_curves.rs:CURVE_FUNCS` → `helper_curves/mod.rs:CURVE_FUNCS`);
  `canvas/func_nurbs_to_beziers.rs:5` (`__canvas_cvRefine` → `helper_curves/pieces.rs`) and `:10`
  (`CURVE_FUNCS` → `helper_curves/mod.rs`); `src/codegen/runtime/canvas/mod.rs:1105`
  (`__CANVAS_CURVE_PIECE` in `builtins/canvas/helper_curves.rs` → `…/helper_curves/pieces.rs`);
  `.ai/canvas-threading.md:1614` (→ `helper_curves/`).

### 4.6 `helper_geometry.rs` (2210; tests in the existing `helper_geometry/tests/mod.rs`)

`GEO_HEADER` is one 954-LOC raw string (lines 256–1208, the `#[rustfmt::skip] const GEO_HEADER:
&str =` at 255, registered as the single helper `canvas_geoHeader`). It is split by the
`helper_render.rs` precedent.

- `canvas/helper_geometry/mod.rs` (120): 1–148 (docs, `use`, `GEO_LAYOUT`, which holds
  `LET __CANVAS_GEO_HEADER` at l.53); the doc comment 249–254 and `#[rustfmt::skip]` above the
  rewritten
  `const GEO_HEADER: &str = concat!(header_paint::geo_header_paint!(), header_kinds::geo_header_kinds!());`;
  2326–2342 (`register`, `#[cfg(test)] mod tests;`, still resolving to `helper_geometry/tests/mod.rs`).
  Adds `mod header_paint; mod header_kinds; mod tail; mod cache; mod verify;` and
  `use tail::GEO_TAIL; use cache::{GEO_CACHE, GEO_CACHE_STATE, GEO_HASH}; use verify::{GEO_VERIFY, GEO_VERIFY_DEBUG};`.
- `canvas/helper_geometry/header_paint.rs` (487 + 3):
  `macro_rules! geo_header_paint { () => { r#"<the text after r#" on line 256, through the end of line 741 including its newline>"# }; }`
  then `pub(super) use geo_header_paint;`. This is the blank/capTag/paint/gradient/clip header
  builders through `__canvas_withClipEdges` (`__canvas_capTag` at l.287).
- `canvas/helper_geometry/header_kinds.rs` (467 + 3): the same shape, holding the text from the
  start of line 742 (`FUNC __canvas_strokeAsFill`) through `END FUNC` on line 1208, up to but not
  including its `"#;`. That is `strokeAsFill`, the bounds header and the per-kind header builders
  (`__canvas_polylineHalf` l.1120, `__canvas_polylineHeader` l.1145).
- `canvas/helper_geometry/tail.rs` (319): 1210–1536 (`GEO_TAIL`; `__canvas_polylineTail` l.1263,
  `__canvas_textGlyphRun` l.1402).
- `canvas/helper_geometry/cache.rs` (72 + 476): 149–248 and 1537–2025 (`GEO_CACHE_STATE`,
  `GEO_HASH`, `GEO_CACHE`).
- `canvas/helper_geometry/verify.rs` (277): 2026–2325 (`GEO_VERIFY`, `GEO_VERIFY_DEBUG`).
- Visibility: `GEO_TAIL`, `GEO_CACHE_STATE`, `GEO_HASH`, `GEO_CACHE`, `GEO_VERIFY`,
  `GEO_VERIFY_DEBUG` → `pub(super)` (6), plus the 2 macro `pub(super) use` lines.
- Identity: R-BID (every scene registers `canvas_geoHeader`). The probe in §2 shows `concat!` of the
  two macro literals equals the joined text.
- Citations (16, `06_canvas.md`): `GEO_HEADER` (:202, :915, :978, :1032, :1067, :1133) and
  `GEO_LAYOUT` (:832, :936, :1297) → `mod.rs` (where `GEO_HEADER` is defined); `GEO_TAIL` (:263,
  :833), `__canvas_polylineTail` (:872), `__canvas_textGlyphRun` (:2712) → `tail.rs`;
  `__canvas_capTag` (:696) → `header_paint.rs`; `__canvas_polylineHalf` (:873),
  `__canvas_polylineHeader` (:860) → `header_kinds.rs`.
- Prose: `src/codegen/runtime/canvas/mod.rs:1040` (`helper_geometry.rs`'s → `helper_geometry/`'s)
  and `:1262` (`__CANVAS_GEO_HEADER` (`helper_geometry.rs`) → `helper_geometry/mod.rs`);
  `canvas/helper_items/mod.rs` `//!` line 4 (→ `helper_geometry/`); `rt_canvas_group_ownership.rs`'s
  `//!` mention (D's root; `grep -rn 'helper_geometry.rs' tests/canvas`) and the
  `rt_canvas_rasteriser` doc mention (D moved it; same grep) → `helper_geometry/`;
  `.ai/canvas-threading.md:1356` and `.ai/testing-gates.md:582` → `helper_geometry/`. The `'` lines
  inside `helper_geometry`'s own raw strings (old l.144, 169, 690, 1067, 1563, 1581, 1702, 2013,
  2019, 2121, 2174) are not edited (§1).

### 4.7 `func_geo_cache.rs` (1647 + 15 test)

- `canvas/func_geo_cache/mod.rs` (233 + 114 + 4 code, 15 test): 1–308 (uses incl.
  `use super::func_geo_build::{…}`, `SLOT_WORDS`, `MIN_BUCKETS`, `PICTURE_KIND_BITS`,
  `TILE_KIND_BITS`, `int_list`, `float_list`, `Globals` + impl, `global`, `gload`, `gstore`, `gadd`,
  `imm`, `word_addr`, `load_word`, `alloc_words`, `free_block`, `emit_reserve`, `emit_push`,
  `emit_set_count`); 1800–1916 (`function`, `param`, `register`); inline test
  `image_kind_bits_are_their_kinds_as_floats` stays (it uses `super::{PICTURE_KIND_BITS, TILE_KIND_BITS}`).
  Adds `mod table; mod resolve; mod layout; mod snapshot;` and
  `use table::{emit_add_slot, emit_find, emit_stamp, finish, lower_geo_begin_frame, lower_geo_find, lower_geo_forget, lower_geo_insert};`
  `use resolve::{emit_hash_step_zero, lower_scene_resolve};`
  `use layout::{lower_scene_draws_flat, lower_scene_layout};`
  `use snapshot::{lower_scene_snapshot, lower_snapshot_hashes, lower_snapshot_items, lower_snapshot_layers};`.
- `canvas/func_geo_cache/table.rs` (433): 309–789 (`emit_bucket`, `emit_find`, `emit_put`,
  `emit_rebuild`, `emit_index`, `emit_stamp`, `emit_add_slot`, `finish`, `lower_geo_find`,
  `lower_geo_insert`, `lower_geo_forget`, `lower_geo_begin_frame`).
- `canvas/func_geo_cache/resolve.rs` (315): 790–1144 (`lower_scene_resolve` (231), `emit_resolved`,
  `emit_hit`, `emit_hash_step_zero`).
- `canvas/func_geo_cache/layout.rs` (439): 1145–1649 (`lower_scene_layout`, `geo_int`,
  `emit_block_instances`, `emit_block_visible`, `lower_scene_draws_flat`).
- `canvas/func_geo_cache/snapshot.rs` (117): 1650–1799 (`lower_scene_snapshot`, `snapshot_copy`,
  `lower_snapshot_items`, `lower_snapshot_layers`, `lower_snapshot_hashes`).
- Visibility: the 16 names in the `use` lines above → `pub(super)`. `Globals` stays in mod.rs, so
  its private fields are visible to the parts. No outside caller other than
  `func_geo_cache::register` (`canvas/mod.rs:2476`).
- Citations (2, `06_canvas.md`): `emit_block_visible` (:927) → `layout.rs`; `lower_geo_begin_frame`
  (:1808) → `table.rs`.
- Prose: `canvas/helper_geometry/mod.rs`'s `///` mention (old l.157, moved with `GEO_CACHE_STATE`'s
  doc into `helper_geometry/cache.rs`): `func_geo_cache.rs` → `func_geo_cache/`;
  `.ai/canvas-threading.md:1357` → `func_geo_cache/`. Old l.169 is inside a raw string and is not
  edited.

### 4.8 `func_geo_build.rs` (3111 + 66 test)

- `canvas/func_geo_build/mod.rs` (606 code + 66 test): 1–101 (uses, `GEO_HEADER`/`KIND_*`/`SLOT_*`/
  `IDENTITY_ONE_BITS`/`EDGE_FLOATS` consts); 363–467 (`fconst`, `fload`, `fbin`, `fadd`, `fsub`,
  `fdiv`, `fmin`, `fmax`, `fselect`, `branch_if_not_positive`, `put`, `put_const`, `put_field`);
  2009–2043 (`Kind` + impl, `put_empty`); 3139–3634 (`native_kinds`, `emit_record_size`,
  `emit_fill_record`, `lower_geo_build`, `lower_geo_same`, `register`); 3635–3705 the inline
  `mod tests` stays (`the_geometry_constants_match_the_mfbasic_source`; it reads only consts that
  stay here). Adds `mod record; mod paint; mod joins; mod dashes; mod fills; mod polyline;`, the
  outside re-export
  `pub(super) use record::{emit_field_block, emit_list_element, emit_list_view, record_fields, record_type, variant};`,
  and one private `use <part>::{…};` per part listing that part's `pub(super)` names (below).
- `canvas/func_geo_build/record.rs` (221): 102–362 (`record_type`, `record_fields`, `field`,
  `scalar_offset`, `emit_field_block`, `variant`, `enum_ordinal`, `ListView`, `emit_list_view`,
  `emit_list_element`, `reserve_float_list`).
- `canvas/func_geo_build/paint.rs` (653): 468–1220 (`PaintView`, `build_offset`, `moved_point`,
  `placed_point`, `emit_build_offset`, `emit_stroke_half`, `put_color`, `emit_transparency_step`,
  `emit_fade_alpha`, `ClipShape`, `emit_clip_shape`, `emit_clip_rings`, `emit_clip_point_count`,
  `emit_clip_size`, `emit_clip_edges`, `emit_paint_header` (151), `put_bounds`, `emit_cap_tag`,
  `cap_pad`, `dash_cap_reach`, `stroke_pad`, `polyline_pad`, `emit_paint_supported`).
- `canvas/func_geo_build/joins.rs` (424): 1221–1712 (`JOIN_FLOATS`, `JoinStyle`,
  `emit_join_style`, `fmul`, `fmadd`, `branch_unless_live`, `emit_store_join`, `emit_join_record`,
  `emit_ring_joins`, `branch_unless_joined`, `emit_join_reach`, `emit_put_joins`,
  `emit_add_join_floats`, `emit_rect_joined`, `emit_rect_dash_size`, `emit_rect_sides`).
- `canvas/func_geo_build/dashes.rs` (258): 1713–2008 (`DASH_EDGE_FLOATS`, `DashPattern`,
  `emit_dash_pattern`, `emit_add_dash_floats`, `emit_dash_head`, `emit_dash_phase`,
  `emit_ring_dash_data`, `emit_put_dashes`, `emit_dash_tail`, `emit_record_end`).
- `canvas/func_geo_build/fills.rs` (587): 2044–2706 (`fill_rect`, `RectInputs`, `emit_rect_inputs`,
  `fill_circle`, `fill_line`, `ring_point_offsets`, `fill_polygon` (263)).
- `canvas/func_geo_build/polyline.rs` (377): 2707–3138 (`PolylineScan`, `fused_len_sq`,
  `emit_polyline_scan`, `emit_polyline_tail_size`, `fill_polyline`).
- Outside reach (must equal today's `pub(super)` from `func_geo_build.rs` = the `canvas` module):
  `func_geo_cache` imports `{emit_field_block, emit_fill_record, emit_list_element, emit_list_view, emit_record_size, record_type}`;
  `func_item_hash.rs:43` imports `{emit_field_block, emit_list_element, emit_list_view, record_fields, record_type, variant}`
  and reads `view.element` and `view.fixed_width` of a `ListView` (`func_item_hash.rs:228–261`).
  `canvas/gen_present.rs:47` and `canvas/mod.rs:1532` name `func_geo_build::record_type`/`::variant`
  in comments, and those paths stay valid through the re-export. In `record.rs` these become
  `pub(in crate::codegen::builtins::canvas)`: the six fns `record_type`, `record_fields`,
  `emit_field_block`, `variant`, `emit_list_view`, `emit_list_element`, and the struct `ListView`
  with all 5 of its fields (`entries`, `data`, `element`, `fixed_width`, `pointer_payload`).
  `emit_record_size`/`emit_fill_record` stay in mod.rs as `pub(super)`. `field`, `scalar_offset` and
  `enum_ordinal` stay `pub(super)` (= `func_geo_build`), which is enough
  (`grep -nwE "scalar_offset|enum_ordinal|field\(" func_geo_cache.rs func_item_hash.rs` → none).
- New `pub(super)`, by part (xref):
  - record: `reserve_float_list` (1).
  - paint: `PaintView` (+ all its fields), `build_offset`, `moved_point`, `placed_point`,
    `emit_build_offset`, `emit_stroke_half`, `emit_transparency_step`, `emit_fade_alpha`,
    `emit_clip_size`, `emit_clip_edges`, `emit_paint_header`, `put_bounds`, `emit_cap_tag`,
    `cap_pad`, `dash_cap_reach`, `stroke_pad`, `polyline_pad`, `emit_paint_supported` (18).
  - joins: `JOIN_FLOATS`, `JoinStyle` (+ all its fields; `paint` names it), `emit_join_style`,
    `fmul`, `fmadd`, `emit_ring_joins`, `branch_unless_joined`, `emit_join_reach`,
    `emit_put_joins`, `emit_add_join_floats`, `emit_rect_joined`, `emit_rect_dash_size`,
    `emit_rect_sides` (13).
  - dashes: `DASH_EDGE_FLOATS`, `DashPattern` (+ all its fields; joins/fills/mod/polyline receive
    it as `pattern`), `emit_dash_pattern`, `emit_add_dash_floats`, `emit_ring_dash_data`,
    `emit_dash_tail`, `emit_record_end` (7).
  - fills: `fill_rect`, `RectInputs` (+ all its fields), `emit_rect_inputs`, `fill_circle`,
    `fill_line`, `ring_point_offsets`, `fill_polygon` (7).
  - polyline: `PolylineScan` (+ all 8 fields; mod.rs reads `scan.count` and `scan.half`),
    `fused_len_sq`, `emit_polyline_scan`, `emit_polyline_tail_size`, `fill_polyline` (5).
- Citations (8, `06_canvas.md`): `placed_point` (:254), `emit_clip_edges` (:1035) → `paint.rs`;
  `fill_polygon` (:264, :834), `emit_rect_inputs` (:306) → `fills.rs`; `emit_polyline_scan` (:265)
  → `polyline.rs`; `emit_join_record` (:735) → `joins.rs`; `emit_dash_tail` (:795) → `dashes.rs`.
- Prose: `canvas/helper_joins.rs:49` (`//!`, `func_geo_build.rs` → `func_geo_build/`); the
  `rt_canvas_rasteriser` doc mention (D moved it; `grep -rn 'func_geo_build.rs' tests/canvas`) and the
  `rt_canvas_geo_native.rs:122` assertion-message text (`func_geo_build.rs` → `func_geo_build/`).
  The `'` lines inside `helper_geometry`'s raw strings (old l.1067, 1581) are not edited.

### 4.9 Sizes after the split

`python3 -I tools/loc-census/census.py --file <new path>` → code ≤ 900, test ≤ 250 for every file
above. Largest: `helper_picture/item.rs` 757 and `func_geo_build/paint.rs` 653 (planning sums
plus their header lines).

### 4.10 Spec citation total

`grep -rnoE "\[\[src/codegen/builtins/(audio/gen_macos_stream|big/gen_big|canvas/(func_geo_build|func_geo_cache|helper_curves|helper_geometry|helper_items|helper_particles|helper_picture))\.rs:[^]]+\]\]" src/docs/spec | wc -l`
→ 58 (0 + 2 + 8 + 2 + 11 + 16 + 7 + 8 + 4). All are repointed. The `gen_windows_io.rs` citation (1)
stays.

### 4.11 `tests/guards/architecture_guards.rs`: the `func_` rule covers R's parts

plan-176-O Phase 1 changed `no_cross_member_os_seam_dispatcher` (and the overflow-label scan) so a
file also qualifies when an ancestor directory under the scan root starts with `func_`/`builder_`/
`gen_`. The AGENTS.md justification is recorded there. R edits no guard. Phase 5 proves the rule
fires on a part file inside `func_geo_cache/`. `func_geo_cache` is the first `func_*` directory in
the tree: `find src/codegen/builtins -type d -name 'func_*'` → none today.

## Phases

> **NOTE: keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes**: never in a
>   batch at the end of a phase, never "I'll update the plan after". If it is a
>   separate step it will be skipped, and the list becomes a lie.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather than
>   deleting it. A vanished task is indistinguishable from a forgotten one.
> - Fill the phase's `Commit:` line with the hash(es) the moment it lands.
> - Add any task you discover you needed; an unlisted task that got done is a
>   task the next reader will do twice.
>
> **An unticked box means NOT DONE.** Someone reading this file, including the
> person who has to decide whether to stop, is entitled to trust that.

Every phase uses **the R gate**:

1. The phase's byte-identity check (named per phase; §3.2), with `cargo build --release` first
   (est. 4 min).
2. `cargo test -p mfb --bin mfb <module>` for the phase's modules → all pass. The whole-binary
   count `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` equals the count
   before the phase (est. 3 min).
3. `cargo check --all-targets 2>&1 | grep -c '^warning'` equals the count before the phase (est. 2 min).
4. `cargo test -p mfb --bin mfb spec_citations_resolve` → passes (est. 1 min).
5. `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings` → pass (est. 1 min).
6. The phase's sweep grep → only the not-edited raw-string/shader/`bugs/`/`planning/` hits listed for it.
7. `cargo test --test file_size_budget 2>&1 | grep -E '<phase paths>'` → only `STALE` rows for the
   old `foo.rs` entries and no `NEW`/`GREW`. Then `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`
   removes them; commit `tests/guards/file_size_ceilings.txt` with the phase (est. 1 min).

Before each phase that uses R-BID, run R-BID `before` with the binary built from the phase's
starting commit.

### Phase 1: audio

- [ ] `src/codegen/builtins/audio/gen_macos_stream.rs` → `gen_macos_stream/{mod,input}.rs` per §4.1.
- [ ] `src/codegen/builtins/audio/gen_windows_io.rs` → keep + new `gen_windows_read.rs`,
      `gen_windows_query.rs`, and the two `include!` lines in `gen_windows.rs` per §4.2.

Acceptance: both audio emitters are split, with byte-identical native code on all five targets.
  Check: the R gate with `sh scripts/artifact-gate.sh target/release/mfb audio` → pass, no golden
  change (est. 1 min). Modules: `codegen::builtins::audio` and `codegen::resource::tests`. Sweep:
  `grep -rn "gen_macos_stream\.rs" src tests scripts .ai tools` → no hit. File-size paths:
  `gen_macos_stream|gen_windows_io|gen_windows_read|gen_windows_query` (`gen_windows_io.rs` shows a
  `TIGHTEN` row because its path stays; tighten removes it) (est. 15 min).
Commit: —

### Phase 2: big

- [ ] `src/codegen/builtins/big/gen_big.rs` → `gen_big/{mod,add_sub,mul,shift,to_string,div_mod}.rs`
      per §4.3, with the 5 re-export lines and the `emit_move_slot` edit.
- [ ] Repoint the 2 `19_big.md` citations, and `big/mod.rs:6,94,424`.

Acceptance: big is split, with byte-identical native code for a program that reaches every part.
  Check: the R gate with R-BID (`diff /tmp/r-bid/before.txt /tmp/r-bid/after.txt` → no output),
  plus `cargo test --test rt_big_int` → all pass (est. 5 min). Module: `codegen::builtins::big`.
  Sweep: `grep -rn "gen_big\.rs" src tests scripts .ai tools` → no hit. File-size path: `gen_big`
  (est. 20 min).
Commit: —

### Phase 3: canvas raw-string helpers (items, picture, particles, curves)

- [ ] `helper_items.rs` → `helper_items/{mod,geo_read,draw_geometry}.rs` per §4.4.
- [ ] `helper_picture.rs` → `helper_picture/{mod,geometry,item}.rs` per §4.4, and the
      `rt_canvas_picture_fit/driver.rs` `chunk` argument + root doc.
- [ ] `helper_particles.rs` → `helper_particles/{mod,model,expand,damage,hooks}.rs` per §4.4, the
      `rt_canvas_particles/model.rs` `helper_source` argument + comment, and the
      `scripts/test-canvas-vulkan.sh:818` message.
- [ ] `helper_curves.rs` → `helper_curves/{mod,pieces,reduce,build,draw}.rs` per §4.5, the
      `curve_shader.rs` `include_str!`, and the `rt_canvas_curves/driver.rs` `chunk` arguments + root doc.
- [ ] Repoint the 7 + 4 + 8 + 11 `06_canvas.md` citations, and the prose listed in §4.4–4.5.

Acceptance: the four helper files are split, and the helper text is byte-identical. The three
source-reading tests still find every const they lift.
  Check: the R gate with R-BID, plus
  `cargo test --test rt_canvas_curves --test rt_canvas_particles --test rt_canvas_picture_fit` → all
  pass (est. 10 min). Module: `codegen::builtins::canvas` (the 3 + 1 + 2 inline tests and the 5 in
  `helper_particles/tests`). Sweep:
  `grep -rnE "helper_(items|picture|particles|curves)\.rs" src tests scripts .ai tools` → only `'`
  lines inside raw-string consts (`helper_geometry.rs`, `helper_render.rs`, `helper_autopaint.rs`,
  `helper_dashes.rs`, `helper_draw.rs:135`, `helper_damage.rs:122`, `helper_picture/item.rs`,
  `helper_curves/build.rs`) and the shader files `mfb_canvas.frag`, `curves.tpl`. File-size paths:
  `helper_(items|picture|particles|curves)` (est. 30 min).
Commit: —

### Phase 4: helper_geometry

- [ ] `helper_geometry.rs` → `helper_geometry/{mod,header_paint,header_kinds,tail,cache,verify}.rs`
      per §4.6, including the `GEO_HEADER` `concat!` of `geo_header_paint!()` + `geo_header_kinds!()`.
      The existing `helper_geometry/tests/mod.rs` is unchanged.
- [ ] Repoint the 16 `06_canvas.md` citations, and the prose listed in §4.6 (including
      `helper_items/mod.rs` line 4).

Acceptance: the file is split. `GEO_HEADER` and every other geometry helper produce identical text
and native code.
  Check: the R gate with R-BID, plus `cargo test -p mfb --bin mfb helper_geometry::tests` → 4 passed,
  plus `cargo test --test rt_canvas_rasteriser` → all pass (est. 10 min). Sweep:
  `grep -rn "helper_geometry\.rs" src tests scripts .ai tools` → only `'` lines inside raw-string
  consts. File-size path: `helper_geometry` (est. 25 min).
Commit: —

### Phase 5: func_geo_cache

- [ ] `func_geo_cache.rs` → `func_geo_cache/{mod,table,resolve,layout,snapshot}.rs` per §4.7,
      with the 16 `pub(super)` edits and mod.rs's 4 `use` lines.
- [ ] Repoint the 2 `06_canvas.md` citations, and the prose in §4.7.

Acceptance: the native geometry cache is split, with byte-identical native code. The `func_`
guard covers files inside `func_*/` directories.
  Check: the R gate with R-BID, plus `cargo test --test rt_canvas_geo_native` → all pass (est. 5 min).
  Guard proof: add the scratch line `const _PROBE: &str = "body: x_native(";` to `func_geo_cache/table.rs`, run
  `cargo test --test architecture_guards no_cross_member_os_seam_dispatcher` → fails naming
  `func_geo_cache/table.rs`, revert the line, rerun → passes (est. 2 min). Sweep:
  `grep -rn "func_geo_cache\.rs" src tests scripts .ai tools` → only the raw-string `'` line in
  `helper_geometry/cache.rs`. File-size path: `func_geo_cache` (est. 25 min).
Commit: —

### Phase 6: func_geo_build

- [ ] `func_geo_build.rs` → `func_geo_build/{mod,record,paint,joins,dashes,fills,polyline}.rs`
      per §4.8, with the `pub(in crate::codegen::builtins::canvas)` reach for the six record fns
      and `ListView` (+ 5 fields), the 51 new `pub(super)` items (+ their structs' fields), the
      outside re-export, and one `use <part>::{…};` per part in mod.rs.
- [ ] Repoint the 8 `06_canvas.md` citations, and the prose in §4.8.

Acceptance: the native geometry builder is split, with byte-identical native code. `func_geo_cache`
and `func_item_hash` compile unchanged.
  Check: the R gate with R-BID, plus `cargo test --test rt_canvas_geo_native --test rt_canvas_rasteriser`
  → all pass, plus `cargo test -p mfb --bin mfb func_geo_build::tests` → 1 passed (est. 10 min).
  Sweep: `grep -rn "func_geo_build\.rs" src tests scripts .ai tools` → only the `'` lines inside
  `helper_geometry`'s raw strings. File-size path: `func_geo_build` (est. 30 min).
Commit: —

### Phase 7: letter gate

- [ ] `sh scripts/artifact-gate.sh target/release/mfb all` on the Phase 6 binary (est. 15–20 min).
      This is justified once per letter: code motion in codegen can change emission for any fixture
      on any target, and only `all` covers every fixture's goldens on all five targets.
- [ ] `cargo test --test file_size_budget 2>&1 | grep -E 'src/codegen/builtins/(audio|big|canvas)/'`
      → no row for an R file, and
      `grep -E 'gen_macos_stream|gen_windows_io|gen_big|func_geo_(build|cache)|helper_(curves|geometry|items|particles|picture)' tests/guards/file_size_ceilings.txt`
      → no `ceiling` line.

Acceptance: the whole letter is byte-identical on every target and the budget is clean for R's files.
  Check: the two commands above. The first prints a pass and the greps print no output.
Commit: —

## Validation Plan

- Byte-identity:
  - `audio` artifact gate (Phase 1);
  - R-BID before/after over 10 canvas scenes and the big program on two to three targets
    (Phases 2–6), which covers native emitters and helper text that the committed goldens barely
    reach (§2);
  - `all` once (Phase 7).
- Unit tests: `cargo test -p mfb --bin mfb codegen::builtins::{audio,big,canvas}` per phase. The
  whole-binary test count is unchanged.
- Integration tests:
  - the three source-reading canvas suites (`rt_canvas_curves`, `rt_canvas_particles`,
    `rt_canvas_picture_fit`), which prove the repointed paths still lift every const;
  - `rt_canvas_geo_native` and `rt_canvas_rasteriser` (native builder vs MFBASIC oracle);
  - `rt_big_int`.
- Guards: architecture_guards (with the §4.11 rule shown to fire on a part), no_type_strings,
  no_operator_strings per phase.
- Doc sync: `spec_citations_resolve` per phase (58 citations repointed). No `mfb man` topic changes.
- Full workspace suite: not in R. It runs once, in plan-176-V's Validation Plan.

## Decisions

- Rule: code ≤ 1000, test ≤ 250 non-comment non-blank LOC; scope `.rs`/`.mfb` under `src/
  packages/ examples/ repository/ tests/`. Owner, 2026-10-14.
- Inline test code counts against the 250 test budget. Over 250 it moves out. Owner, 2026-10-14.
  (R's inline modules are all ≤ 95 and stay.)
- Generated files are exempt. The guard warns with a banner and never fails. A new module is
  always an acceptable split. Owner, 2026-10-14.
- A single function over the code limit is decomposed by extract-method only in letters S–V. R has
  none (largest fn: `emit_div_mod_magnitude`, 501). Owner, 2026-10-14.
- `GEO_HEADER` is split into `concat!` pieces held in two `macro_rules!` parts (precedent:
  `helper_render.rs` `RENDER_LOOP`). It is neither moved whole nor exempted. Author decision recorded
  in the plan-176 sub-plan brief.
- The `func_` directory rule that keeps R's parts inside `no_cross_member_os_seam_dispatcher` is
  owned by plan-176-O Phase 1. R proves it on a part (§4.11). Author.
- `gen_windows_io.rs` splits into sibling `include!` fragments, not a module (§3 rejected). Author.
- Moved structs that are named across files get `pub(super)` on every field, and `ListView` keeps its
  old `canvas`-wide reach through `pub(in crate::codegen::builtins::canvas)`. Author.
- R-BID is a `/tmp` one-off and is not added to `scripts/` (AGENTS.md: a one-off probe for one plan
  lives in `/tmp`). Author.
- Prose is repointed in Rust comments, tests, scripts and `.ai/`. Text inside raw-string MFBASIC
  consts and shader sources is not edited (byte-identity of helper text). Author.
- R adds no exemption. Author.

## Corrections

## Summary

R is ten pure moves across three builtin packages, with one textual-fragment split and one
`concat!` rewrite of a single 954-LOC string. The committed goldens barely exercise canvas and big,
so R adds a direct before/after `.ncode` checksum over ten canvas scenes and a big program, on top
of the `audio` selector and one final `all` run.

The reference hazards are the real work:

- 58 spec citations;
- an `include_str!` in the Vulkan curve shader generator;
- three tests that lift helper source by file name and opener text;
- a script failure message;
- a guard rule keyed on the `func_` file-name prefix.

`func_geo_build`'s cross-file visibility is the riskiest step, so it goes last.
