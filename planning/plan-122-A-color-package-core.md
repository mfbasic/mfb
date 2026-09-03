# plan-122-A: The `color` builtin package — core value type and constructors

Last updated: 2026-09-02
Overall Effort: huge (> 3d)
Effort: large (3h–1d)
Depends on: nothing

MFBASIC has three unrelated notions of a colour today: `canvas::Color` (a 4-`Byte`
RGBA record), `term::TermColor` (a 3-`Byte` RGB record the runtime allocates), and
an `astrings` foreground/background attribute carrying a packed `0xRRGGBB`
`Integer` with no type at all. Nothing converts between them. plan-122 replaces all
three with one `color::Color`.

This sub-plan builds the **package** and nothing else: registration on the
clean-room registry, the `Color` record, the clamping constructors, the packed-
integer bridge, hex parsing and rendering, and the `toString` override. **No
consumer changes** — `canvas::Color`, `term::TermColor` and `astrings::foreground`
are untouched and still work exactly as they do today, so this letter lands with a
green `cargo test` and zero golden churn outside the new fixtures.

Behavioral outcome: a program can `IMPORT color`, build a `color::Color` from
components / a packed integer / a hex string, render it back to hex, and read its
four `Byte` channels — proved by an rt-behavior fixture and `mfb man color`.

References:

- `.ai/resources-packages.md` §"New builtin-package registration seams" — the
  authoritative seam checklist. **Two of its bullets are stale** (§7 task): there is
  no `src/builtins/<pkg>.rs` and no `descriptor.rs` (`ls src/builtins` →
  `No such file or directory`); those seams moved into `src/codegen/builtins/` and
  `src/codegen/registry/`.
- `.ai/resources-packages.md` §".mfb source-language authoring gotchas" — record
  literals are positional `Type[...]`, record fields are not assignable, reserved
  words cannot be field or parameter names. Every one of these bites when writing a
  pure-source package.
- `.ai/man-content.md` — the man-page content standard every new member must meet.
- `.ai/testing-gates.md` — artifact gate, acceptance golden harness, byte-identity.
- `mfb spec` §18 (builtin packages), §14 (value/copy semantics), §4.2 (records).
- `src/codegen/builtins/encoding/` — the closest existing model: a **pure-source**
  package, every member `Body::mfb`, one `func_*.rs` per member plus `helper_*.rs`
  chunks.
- `src/codegen/builtins/vector/mod.rs` — the `add_constant` record-constant
  precedent (`vector::zeroFloat3`) that plan-122-C's named colours reuse.

## Prerequisites

These are a precondition on the whole plan-122 feature, not a dependency to
negotiate. Sub-plans B–F point here.

| Must be true | Command | Status |
|---|---|---|
| A release binary matching `main` exists (tests run the RELEASE `mfb`) | `cargo build --release && ls -l target/release/mfb` | MET (built 2026-09-02, 32551936 bytes) |
| Working tree is clean enough to attribute golden churn | `git status --porcelain` → only the 5 known `examples/browser` edits | NOT MET — 5 modified `examples/browser/**/*.mfb` files predate this plan; commit or stash them first, or golden attribution in D/E/F is unreadable |
| The registry qualifies cross-package value-type leaves automatically | `grep -n "fn qualify_value_type_references" src/codegen/registry/mod.rs` → 1665 | MET |
| Rule code / error code for a malformed hex string already exists | `grep -rn "ErrInvalidFormat" src/codegen/builtins/errorcode/mod.rs` → `77050003` | MET — **no new `data_objects.rs` row is needed** (see `new-error-in-a-package-needs-a-data-object-row`) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

Everything below is written against the world where these hold.

## 1. Goal

- A `color` builtin package exists and is importable from any build (not
  `--app`-gated), exporting the record `color::Color` with `Byte` fields
  `red`/`green`/`blue`/`alpha`.
- `color::rgb`, `color::rgba`, `color::gray`, `color::withAlpha`, `color::invert`,
  `color::toPacked`, `color::fromPacked`, `color::fromHex`, `color::toHex`,
  `color::toHexAlpha` all work, are documented on `mfb man color`, and every man
  example compiles and runs.
- `toString(c)` on a `color::Color` renders `#rrggbbaa`.
- `color::fromHex` accepts `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` and the same
  four without the leading `#`, case-insensitively, and raises `ErrInvalidFormat`
  on anything else.

### Non-goals (explicit constraints)

- **No consumer changes in this letter.** `canvas::Color`, `canvas::rgb`,
  `canvas::rgba`, `term::TermColor`, `astrings::foreground`/`background` keep
  their current signatures and behavior. Their removal is D, E and F.
- **No colour-space maths.** `brighten`/`darken`/`luminance`/HSL are B. Nothing in
  A converts between sRGB and linear light, and A does **not** move canvas's sRGB
  table.
- **No named colours.** `fromName`/`nameOf` and the CSS table are C.
- **No new error code.** `fromHex` raises the existing `ErrInvalidFormat`
  (`77050003`).
- **No transcendentals anywhere in `color`.** canvas's software rasteriser is the
  oracle its GPU backends are compared against and must produce identical bytes on
  every target (`src/codegen/builtins/canvas/helper_color.rs:1-11`); from B onward
  canvas calls into `color`, so the whole package inherits that rule. IEEE `+ - * /
  sqrt` only — no `pow`, no trig.
- **`Color` is a plain value record, not a resource and not read-only.** Unlike
  `term::TermColor` (which the runtime allocates, so `is_read_only_record` refuses
  construction — `src/codegen/builtins/term/mod.rs:100-108`), a program may build a
  `color::Color` with a record literal and `WITH`-update one.

## 2. Current State

### The three colour representations

| Where | Shape | Built by | Cited |
|---|---|---|---|
| `canvas` | record `Color { red, green, blue, alpha AS Byte }` | `canvas::rgb`/`rgba`, clamping through `__canvas_clampByte` | `src/codegen/builtins/canvas/mod.rs:183`, `func_rgb.rs:36`, `func_rgba.rs:52`, `helper_clamp_byte.rs:13` |
| `term` | record `TermColor { r, g, b AS Byte }`, **runtime-allocated, read-only** | never by a program; only read back from `term::getForeground`/`getBackground` | `src/codegen/builtins/term/mod.rs:89-95`, `:190`; hand-emitted at `src/codegen/term/core/term.rs:2069` |
| `astrings` | no type — a packed `0xRRGGBB` `Integer` in `AttrNumber.value` | `astrings::foreground(r,g,b)` / `background` via `__astrings_packColor` | `src/codegen/builtins/astrings/func_foreground.rs:38`, `helper_pack_color.rs:16`, `mod.rs:170` |

The `term`↔`astrings` bridge is where the absence of a shared type is most
visible: it re-implements channel extraction by hand
(`__term_colorR`/`G`/`B`, `src/codegen/builtins/term/helper_astrings_bridge.rs:97-107`)
and uses `-1` as an "unset" sentinel that is only sound because a packed colour is
always in `0..0xFFFFFF`.

### The seams a new package must touch

Measured by reading each site, not from the doc:

| Seam | File:line | What |
|---|---|---|
| module decl | `src/codegen/builtins/mod.rs:5-35` | `pub(crate) mod color;` in the sorted list |
| import gate | `src/codegen/builtins/mod.rs:50` | `is_builtin_import` match arm |
| argument checking | `src/codegen/builtins/mod.rs:451` | `ARGUMENT_CHECKED_PACKAGES` — **without a row here an arity/type mistake in a `color::` call degrades to a bare `TYPE_UNKNOWN_VALUE` with no diagnostic** |
| test census | `src/codegen/builtins/mod.rs:935` | `ALL_BUILTIN_PACKAGES` + the second list in `is_builtin_import_cases` |
| registry build | `src/codegen/registry/mod.rs:1937` | `crate::codegen::builtins::color::register(&mut r);` in `build()` |
| resolver type seed | `src/resolver/mod.rs:20-58` | `BUILTIN_TYPES` needs the **package-qualified** `color.Color` id, never the bare leaf (bug-484 comment at `:33-40`) |
| §18 package list | `src/docs/spec/language/18_builtin-functions.md:46` | pinned by `spec_section_18_package_list_matches_is_builtin_import` (`src/codegen/builtins/mod.rs:967`) — the test fails if the sentence and `is_builtin_import` disagree |

`mfb man` needs **no** list update: the package index is derived from the registry
(`src/cli/man.rs` resolves by name; there is no hand-written package table).

### Measured populations

| What | Count | Command |
|---|---|---|
| `canvas::rgb(` occurrences, whole tree | 141 | `grep -rn 'canvas::rgb(' --include='*.mfb' --include='*.rs' --include='*.md' . \| wc -l` |
| — of those, in `src/` / `tests/` / `examples/` | 19 / 110 / 8 | `for d in src tests examples; do grep -rn 'canvas::rgb(' $d \| wc -l; done` |
| `canvas::rgba(` occurrences | 14 (2 src / 11 tests / 0 examples) | same, `canvas::rgba(` |
| `canvas::Color` occurrences | 57 (24 src / 27 tests / 5 examples) | same, `canvas::Color` |
| `TermColor` occurrences (all spellings incl. `TERM_COLOR_*`) | 109 (38 src / 63 tests / 6 planning / 2 bugs) | `for d in src tests planning bugs; do grep -rn 'term::TermColor\|term\.TermColor\|TERM_COLOR' $d \| wc -l; done` |
| `astrings::foreground(`/`background(` occurrences | 24 | `grep -rn 'astrings::foreground(\|astrings::background(' --include='*.mfb' --include='*.rs' --include='*.md' . \| wc -l` |
| `.mfb` fixtures importing `term` | 45 | `grep -rl '^IMPORT term' --include='*.mfb' tests/ examples/ \| wc -l` |
| `.mfb` fixtures importing `astrings` | 15 | `grep -rl '^IMPORT astrings' --include='*.mfb' tests/ examples/ \| wc -l` |
| golden fixture dirs under `tests/rt-behavior/term` + `tests/syntax/term` | 14 + 23 | `ls -d tests/rt-behavior/term/*/ \| wc -l; ls -d tests/syntax/term/*/ \| wc -l` |
| golden fixture dirs under `tests/rt-behavior/astrings` + `tests/syntax/astrings` | 9 + 3 | same for `astrings` |
| `.ncodesum` files under `tests/byte-identity/term` | 5 | `find tests/byte-identity/term -name '*.ncodesum' \| wc -l` |
| `.ncodesum` files, whole byte-identity tree | 133 | `find tests/byte-identity -name '*.ncodesum' \| wc -l` |
| Rust test files naming the colour surface | 10 | `grep -rln 'canvas::rgb\|canvas::Color\|term::TermColor\|astrings::foreground\|astrings::background' tests/*.rs \| wc -l` |
| example `.mfb` files naming it | 3 | same over `examples/` |
| spec/man docs naming it | 5 | same over `src/docs/` |

### Verified properties

- **Cross-package value types work, and imports are NOT transitive.** Verified by
  building a probe: a project with `IMPORT udp` (no `IMPORT net`) doing
  `LET addr = udp::localAddress(sock)` then `addr.host` fails with
  `2-203-0043 TYPE_UNKNOWN_VALUE`: *"Field `host` belongs to `net::net.Address`,
  whose fields are not visible in this file. Imports are not transitive and a
  package cannot re-export another's types, so add `IMPORT net`."*
  **Consequence for plan-122: every program that reads a channel off a
  `color::Color` must `IMPORT color`, no matter which package handed it over.**
  This is why "keep `canvas::rgb` as an alias" would not have spared callers
  anything, and why removing the old spellings outright is the right call.
- **That diagnostic renders a doubled qualifier** — `net::net.Address`, because
  `src/ir/shape.rs:1868` interpolates `{owner}::{type_name}` while `type_name` is
  already package-qualified. It is a live cosmetic bug that plan-122 will make far
  more visible (every colour consumer will hit it during D/E/F). Pinned by 5
  goldens under `tests/syntax/tcp/`. **Fixed out-of-band before this plan starts —
  see §7.**
- **A pure-source package's entire companion is compiled into every importing
  binary, used or not.** Measured by building `IMPORT io` + `IMPORT <pkg>` with no
  call to `<pkg>` and comparing `build/*.out` sizes against the 66,596-byte
  `IMPORT io` baseline:

  | package | bytes | delta | companion |
  |---|---|---|---|
  | `bits`, `math`, `collections`, `strings`, `os`, `money`, `term`, `tcp`, `tls` | 66,596 | **0** | empty or type-declarations-only |
  | `net`, `udp` | 182,180 | +115,584 | pure-source |
  | `datetime` | 281,412 | +214,816 | pure-source |
  | `vector` | 396,836 | +330,240 | pure-source |
  | `encoding` | 495,908 | +429,312 | pure-source |
  | `csv` | 545,444 | +478,848 | pure-source |
  | `json` | 644,516 | +577,920 | pure-source |
  | `regex` | 1,007,780 | +941,184 | pure-source |
  | `astrings` | 1,139,876 | +1,073,280 | pure-source |
  | `crypto` | 1,404,180 | +1,337,584 | pure-source |

  Command (per package): `mfb init /tmp/szp; printf 'IMPORT io\nIMPORT <pkg>\n\nFUNC main AS Integer\n  io::print("hi")\n  RETURN 0\nEND FUNC\n' > /tmp/szp/src/main.mfb; (cd /tmp/szp && mfb build); stat -f%z /tmp/szp/build/szp.out`
- **The trigger is a non-empty companion, not `add_imports`.** `tcp` declares
  `add_imports(vec!["net"])` (`src/codegen/builtins/tcp/mod.rs:172`) yet costs 0
  bytes, because it declares no records/enums/helpers and so renders an empty
  companion. `udp` declares the same import **plus** one record
  (`src/codegen/builtins/udp/mod.rs:143,147`) and therefore pays net's full
  115,584 bytes. `term` declares records and enums but **no** `add_imports`, and
  costs 0.
  **Consequence for plan-122-F: `term` must NOT gain `add_imports(["color"])`.** A
  native (`Body::abi_function`) member can name `color.Color` in its signature
  through a qualified type-id constant — exactly how `tcp::localAddress` returns
  `net.Address` (`src/codegen/builtins/tcp/mod.rs:103-112`, `func_local_address.rs:71`)
  — without dragging the companion in. That keeps every TUI binary at its current
  size.
- **A member may mix a native and a source overload.** `canvas::getSize` has a
  `Body::abi_function` overload taking an `Image` and a `Body::mfb` overload taking
  nothing (`src/codegen/builtins/canvas/func_get_size.rs:125-148`). This is the
  precedent F uses for `term::setForeground(color::Color)` alongside the existing
  three-`Byte` native form.
- **The whole companion lands in an importer's `.ir`.** A small `astrings`
  fixture's golden is 5,450 lines and contains every `#astrings_*` member including
  ones the fixture never calls
  (`grep -o '"name": *"[^"]*"' tests/rt-behavior/astrings/attribute-model-rt/golden/attribute_model_rt.ir | sort -u`).
  So D/E/F churn `.ir`/`.ast` goldens for **every** importer, not just the fixtures
  they edit.
- **UNVERIFIED — the byte cost of `color` itself.** It cannot be measured before
  the package exists. Phase 6 measures it and records the number; it is an input to
  C's go/no-go on the CSS table, not a blocker for A.

## 3. Design Overview

`color` is a **pure-source** package modelled on `encoding`: one `func_*.rs` per
public member carrying `INTRO`/`DESC`/`EX` prose and a `Body::mfb` body, plus
`helper_*.rs` chunks for the private `__color_*` FUNCs, assembled by
`RegistryPackage::get_mfb`.

Three pieces, layered:

1. **The record and the clamp.** `Color` plus `__color_clampByte` — a byte-for-byte
   move of `__canvas_clampByte` (`src/codegen/builtins/canvas/helper_clamp_byte.rs:13`)
   under the new name. Clamping rather than raising is a contract inherited from
   canvas and documented there: colours are computed, and a value one past an end is
   a rounding artefact, not a bug.
2. **The packed-integer bridge.** `toPacked`/`fromPacked` over `0xAARRGGBB`. This is
   not a convenience: `astrings` stores fg/bg as a packed `Integer` today, so E is
   written against these two functions rather than re-deriving shifts, and the term
   bridge's hand-rolled `__term_colorR/G/B` collapse into `color::fromPacked`.
3. **Text form.** `fromHex`/`toHex`/`toHexAlpha` and the `toString` override.

**Where correctness risk concentrates:** `fromHex`, because it is the only member
with a parse and therefore the only one with an error path. Its risk is contained —
it touches nothing outside the package — so it is scheduled early rather than last.

**Where design uncertainty concentrates:** the package-registration seams. A missed
seam does not fail loudly; `ARGUMENT_CHECKED_PACKAGES` in particular degrades
silently to "no diagnostic at all". Phase 1 therefore lands a package with **one**
trivial member and proves every seam works before any real content is written —
the cheapest experiment that could falsify the approach.

**Byte-identity is NOT this plan's gate.** plan-122 changes behavior and API by
design. A is *expected* to be byte-identical everywhere (it adds a package nobody
imports yet), and a diff in A is a bug to root-cause. From B onward the expected
diffs are named per letter. Byte-identity is a verification method here, never a
premise: a failing `.ncodesum` means *objdump one fixture, find the cause, fix it*
— never "the design is dead".

### Rejected alternatives

- **Keep `canvas::rgb`/`rgba` as forwarding aliases.** Rejected: the measured
  non-transitive-import probe shows a caller must `IMPORT color` to touch a channel
  anyway, so an alias buys no source compatibility while leaving two names for one
  concept. (User decision, 2026-09-02.)
- **Make `color` a native package.** Rejected: every member is pure arithmetic on a
  4-`Byte` record, and a pure-source package is a fraction of the work and reviewable
  as MFBASIC. The binary-size cost is real (see the table above) but it is a
  pre-existing compiler gap that affects nine other packages equally.
- **Put the colour maths in `math`.** Rejected: `math` has an empty companion and
  costs importers 0 bytes today; adding pure-source members to it would put that
  cost on every numeric program.
- **Two packages (`color` core + `color.names`).** Rejected: the user asked for one
  colour system, and MFBASIC has no sub-package mechanism.

## 4. The `Color` record

```
TYPE Color
  red AS Byte
  green AS Byte
  blue AS Byte
  alpha AS Byte
END TYPE
```

Identical in field names, order and types to today's `canvas::Color`
(`src/codegen/builtins/canvas/mod.rs:183-210`), which is what lets D be a rename
rather than a reshape. `alpha` is **straight, not premultiplied**: `0` fully
transparent, `255` fully opaque, and red/green/blue are unaffected by it. The
all-zero value is fully transparent — the property canvas's `Paint` relies on for
"an unset channel is a no-op" (`src/codegen/builtins/canvas/helper_paint_defaults.rs:15`).

## 5. Member surface (this letter)

| Member | Signature | Notes |
|---|---|---|
| `rgb` | `(red, green, blue AS Integer) AS Color` | alpha 255; components clamped |
| `rgba` | `(red, green, blue, alpha AS Integer) AS Color` | components clamped |
| `gray` | `(level AS Integer) AS Color` | `rgb(level, level, level)` |
| `withAlpha` | `(base AS Color, alpha AS Integer) AS Color` | rgb preserved, alpha clamped |
| `invert` | `(base AS Color) AS Color` | `255 - channel`; alpha preserved |
| `toPacked` | `(base AS Color) AS Integer` | `0xAARRGGBB`, always `0..0xFFFFFFFF` |
| `fromPacked` | `(value AS Integer) AS Color` | inverse; low 32 bits only |
| `fromHex` | `(text AS String) AS Color` | see §6; raises `ErrInvalidFormat` |
| `toHex` | `(base AS Color) AS String` | always `#rrggbb`, alpha dropped |
| `toHexAlpha` | `(base AS Color) AS String` | always `#rrggbbaa` |

Parameters are `Integer`, not `Byte`, for the same reason `canvas::rgba`'s are
(`src/codegen/builtins/canvas/func_rgba.rs:81-85`): declaring them `Byte` pushes an
out-of-range value into a conversion error at the call site, which is the opposite
of the clamping contract.

`toHex` and `toHexAlpha` are two members rather than one alpha-sensitive member so
the output width is a property of the call, not of the data — a caller writing a
fixed-width field never has to branch.

`toString(color::Color)` is registered with `add_override`
(`src/codegen/registry/mod.rs:1285`) and renders `#rrggbbaa` — the lossless form,
since `toString` is what a debugging `io::print` reaches for. The `net::Url`
renderer is the precedent (`registry::general_override_target`).

## 6. `fromHex` grammar

Accepted, case-insensitively, with an optional leading `#`:

| Form | Length after `#` | Expansion |
|---|---|---|
| `rgb` | 3 | each digit doubled: `f0a` → `ff00aa`, alpha 255 |
| `rgba` | 4 | each digit doubled, 4th is alpha |
| `rrggbb` | 6 | alpha 255 |
| `rrggbbaa` | 8 | as written |

Anything else — an unsupported length, a non-hex character, an empty string, a
second `#` — raises `ErrInvalidFormat` (`77050003`) with a message naming the
offending input.

Implementation: strip the optional `#` with `strings::stripPrefix`, upper-case
with `strings::upper`, walk `strings::toBytes` and map each byte through a private
`__color_hexValue` returning `-1` for a non-digit. **`encoding::hexDecode` is
deliberately not used**: it would make `color` import `encoding`, whose companion
costs an importer 429,312 bytes (measured, §2), and it does not handle the 3/4-digit
short forms or the `#`.

`toHex`/`toHexAlpha` emit **lowercase** digits, matching `encoding::hexEncode`'s
convention, so `fromHex(toHex(c))` round-trips and `toHex` output compares equal
across two programs.

## Compatibility / Format Impact

Nothing observable changes in this letter. `color` is a new package no shipped
program imports.

What A does establish, and D–F depend on:

- **`color.Color` is the package-qualified type id** — the spelling that goes in
  `resolver::BUILTIN_TYPES` and in every cross-package descriptor reference. The
  bare leaf `Color` must never appear in either place (bug-484:
  `src/resolver/mod.rs:33-40`).
- **`0xAARRGGBB` is the canonical packed order**, alpha in the high byte. E widens
  the astrings attribute payload to this layout.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; use `- [~]` for partially done and say what remains;
> mark a task moot with `- [x] ~~text~~ — moot: <evidence>`; fill each `Commit:`
> line the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Registration skeleton (falsify the seams first)

Land a `color` package with exactly one member, and prove every registration seam
works before writing any real content. Nothing here is throwaway — the member
survives; only its siblings are missing.

- [ ] Create `src/codegen/builtins/color/mod.rs`: `RegistryPackage::new("color", MODULE_INTRO, MODULE_DESC)`,
      the `Color` record via `add_record` (§4), `add_imports(vec!["color", "strings"])`
      (a package reaches its own internal members through the qualified spelling,
      as `astrings` does — `src/codegen/builtins/astrings/mod.rs:135`).
- [ ] Add `pub(crate) const COLOR_TYPE: &str = "Color";` and
      `pub(crate) const COLOR_TYPE_ID: &str = "color.Color";` to that `mod.rs`,
      mirroring `term::TERM_COLOR_TYPE`/`TERM_COLOR_TYPE_ID`
      (`src/codegen/builtins/term/mod.rs:89-95`). D/E/F reference these, never a
      string literal.
- [ ] Add `src/codegen/builtins/color/func_rgba.rs` and `helper_clamp_byte.rs`
      (`__color_clampByte`, the body of `canvas/helper_clamp_byte.rs:13` renamed).
- [ ] Wire the seams, in this order: `pub(crate) mod color;`
      (`src/codegen/builtins/mod.rs:5-35`); `"color"` in `is_builtin_import`
      (`:50`); `"color"` in `ARGUMENT_CHECKED_PACKAGES` (`:451`); `"color"` in
      `ALL_BUILTIN_PACKAGES` (`:935`) and in the `is_builtin_import_cases` list;
      `color::register(&mut r)` in `registry::build()`
      (`src/codegen/registry/mod.rs:1937`); `builtins::color::COLOR_TYPE_ID` in
      `resolver::BUILTIN_TYPES` (`src/resolver/mod.rs:20-58`).
- [ ] Add `` `color` `` to the §18 package sentence
      (`src/docs/spec/language/18_builtin-functions.md:46`) — the
      `spec_section_18_package_list_matches_is_builtin_import` test fails until it
      matches `is_builtin_import` exactly.
- [ ] Tests: new `tests/rt-behavior/color/func_color_rgba_valid/` fixture with all
      **four** goldens (`build.log`, `.ast`, `.ir`, `.run` — a missing one prints
      `unexpected actual` with no `mismatch:` line and only a FULL `test-accept.sh`
      run sees it).
- [ ] Prove the argument checker is live: a `tests/syntax/color/` fixture calling
      `color::rgba(1, 2)` must produce `TYPE_CALL_ARITY_MISMATCH`, **not** a bare
      `TYPE_UNKNOWN_VALUE`. If it produces the latter, the
      `ARGUMENT_CHECKED_PACKAGES` row did not take.

Acceptance: `cargo test --no-fail-fast` green; `mfb man color` renders the package
page and `mfb man color rgba` the member page; the syntax fixture reports
`TYPE_CALL_ARITY_MISMATCH`; `./scripts/test-accept.sh` green with the new fixtures
counted in its `N ran` line (watch the count — a silently skipped fixture is the
`test-accept-acceptance-eof-subtests-preexisting` trap).
Commit: —

### Phase 2 — Constructors

- [ ] `func_rgb.rs`, `func_gray.rs`, `func_with_alpha.rs`, `func_invert.rs` — each
      a `Body::mfb` member with full `INTRO`/`DESC`/`EX` prose per `.ai/man-content.md`.
- [ ] Tests: extend the rt-behavior fixture; assert clamping at both ends
      (`rgba(300, -20, 128, 255)` → `255, 0, 128, 255`) and that `invert` preserves
      alpha.

Acceptance: `mfb man color --all` shows every member with a non-empty
intro/description/example; `scripts/man-run-examples.sh color --run` compiles and
runs every example on the page with zero failures.
Commit: —

### Phase 3 — Packed-integer bridge

- [ ] `func_to_packed.rs`, `func_from_packed.rs` over `0xAARRGGBB`, using
      `bits::sl`/`sr`/`band`/`bor` (add `"bits"` to `add_imports`).
- [ ] Tests: round-trip `fromPacked(toPacked(c)) = c` across the corners
      (all-zero, all-255, one channel at a time) and pin the byte order explicitly —
      `toPacked(rgba(0x12, 0x34, 0x56, 0x78)) = 0x78123456`.

Acceptance: the round-trip and byte-order assertions pass in the rt-behavior
fixture, and the byte-order test names the constant literally rather than
recomputing it (so a shift-direction inversion cannot pass).
Commit: —

### Phase 4 — Hex parse and render

The one member with an error path.

- [ ] `helper_hex_value.rs` (`__color_hexValue(byte) AS Integer`, `-1` on a
      non-digit) and `helper_hex_pair.rs` if the digit-doubling is worth its own FUNC.
- [ ] `func_from_hex.rs` implementing §6, raising
      `FAIL error(77050003, "…")` on every rejected input.
- [ ] `func_to_hex.rs`, `func_to_hex_alpha.rs` — lowercase output.
- [ ] Tests: a `tests/rt-behavior/color/` fixture covering **all four accepted
      lengths, both with and without `#`, in mixed case** (8 accepting cases), plus
      a TRAP-ing fixture for each rejection class: bad length (5), non-hex char,
      empty string, doubled `#`. Round-trip `fromHex(toHexAlpha(c)) = c` over the
      corner colours.

Acceptance: every accepted form yields the documented channels and every rejected
form raises `ErrInvalidFormat` — asserted positively, not by "did not crash". The
negative cases must assert the raised code, since a fixture that merely fails to
produce output passes vacuously.
Commit: —

### Phase 5 — `toString` override

- [ ] Register a `toString` override for `color.Color` via `add_override`
      (`src/codegen/registry/mod.rs:1285`), rendering `#rrggbbaa`, modelled on the
      `net::Url` renderer reached through `registry::general_override_target`.
- [ ] Tests: `io::print(toString(color::rgba(255, 0, 0, 128)))` prints `#ff000080`.

Acceptance: the rt-behavior `.run` golden shows `#ff000080`; `toString` on an
unrelated record is unaffected (pin one, per
`shortest-fix-disables-an-adjacent-guarantee` — every RED test needs a partner
pinning what must not change).
Commit: —

### Phase 6 — Documentation, census and the cost measurement

- [ ] `MODULE_INTRO`/`MODULE_DESC` on the package written to the `.ai/man-content.md`
      standard: what a colour is here, the straight-alpha rule, the clamping rule,
      the `0xAARRGGBB` packed order, and that `Color` is an ordinary value record a
      program may build and `WITH`-update.
- [ ] `scripts/man-census.sh --fill color` → 100% fill on every column.
- [ ] `scripts/man-census.sh --memory-scope color` and `--scope color` → **0**
      unclassified hits. No `copy`/`free`/`heap`/`borrow` vocabulary; the permitted
      words are copy, mutate, value, alias.
- [ ] Add a `color` row to `src/docs/spec/stdlib/` as `18_color.md`, and its entry
      in `src/docs/spec/stdlib/spec.md`.
- [ ] **Measure and record the package's unused-import byte cost** with the
      Verified-properties command from §2, and write the number into this file's
      Corrections section. This is C's input.

Acceptance: `man-census.sh --fill color` reports no empty cells;
`--memory-scope`/`--scope` report 0; `scripts/man-run-examples.sh color --run` is
green; the measured byte cost is recorded here.
Commit: —

### Phase 7 — Out-of-band: the `{owner}::{type_name}` doubled qualifier

Not colour work, but plan-122 makes this diagnostic fire at every consumer during
D–F, so it is fixed before those letters make its goldens harder to read.

- [ ] `src/ir/shape.rs:1868` — render the **leaf**, not the qualified name, so the
      message reads ``belongs to `net::Address` `` instead of
      ``belongs to `net::net.Address` ``. The suggested `IMPORT {owner}` line is
      already correct.
- [ ] Regenerate the 5 goldens that pin the old text
      (`grep -rln 'Imports are not transitive' tests/` → 5 files under
      `tests/syntax/tcp/`). The refusal behavior is unchanged; only the rendered
      name is corrected, so this is a corrected line, not a re-baseline.

Acceptance: the 5 `tests/syntax/tcp/**/build.log` goldens show `net::Address`; the
diagnostic still fires (the fixtures still fail to build) and its rule code is
unchanged.
Commit: —

## Validation Plan

- **Tests:** `tests/rt-behavior/color/**` (positive behavior, four goldens each),
  `tests/syntax/color/**` (arity/type diagnostics and every `fromHex` rejection
  class). Negative cases assert the raised error **code**, never merely absence of
  output.
- **Coverage check:** `scripts/coverage.sh --bin mfb` — confirm
  `src/codegen/builtins/color/` is in the denominator. A green suite that never
  reached the new module proves nothing (`codegen-cover-fixture-may-not-cover-your-member`).
- **Runtime proof:** a scratch project that does
  `io::print(color::toHexAlpha(color::fromHex("#f0a")))` → `#ff00aaff`, built and
  run, not just compiled.
- **Doc sync:** §18 package sentence; new `src/docs/spec/stdlib/18_color.md` +
  `spec.md` index; `.ai/resources-packages.md` §"New builtin-package registration
  seams" — correct the two stale bullets (`src/builtins/<pkg>.rs`, `descriptor.rs`)
  found in §2, and add the measured "a pure-source companion is compiled into every
  importer" finding with its numbers.
- **Acceptance:** `cargo test --no-fail-fast` (never fail-fast — a failing
  `golden.rs` skips every later `rt_*`), `./scripts/test-accept.sh` **full run**
  (watch the `N ran` count), `scripts/artifact-gate.sh`, and
  `cargo check --all-targets` at the **end** as well as the start (only
  `--all-targets` sees test-target warnings).
- **Formatting:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **`color::gray` vs `color::grey`.** Recommend `gray` only, matching the CSS
  keyword `gray` that C's table will carry as canonical. (§5)
- **Whether `toPacked` should have an RGB-only sibling.** Recommend not: E needs
  `0xAARRGGBB` and a caller wanting 24 bits writes `bits::band(toPacked(c), 16777215)`.
  Revisit if E's astrings body ends up needing it twice. (§5)

## Corrections

_(filled in during execution)_

## Summary

The engineering risk in A is not the arithmetic — it is the seven registration
seams, of which `ARGUMENT_CHECKED_PACKAGES` fails silently and `BUILTIN_TYPES`
fails only for a bare-leaf spelling. Phase 1 exists to make all seven fail loudly
before any content is written.

Untouched by this letter: canvas, term, astrings, every existing golden, and the
whole question of colour-space maths.
