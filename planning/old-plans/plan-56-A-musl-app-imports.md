# plan-56-A: Flavor-correct GTK app-mode imports

Last updated: 2026-07-18
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)
Depends on: nothing

A Linux app-mode build declares its C-library dependencies from a hardcoded
glibc table: `src/target/linux_gtk/mod.rs:231-232` pins `LIBC = "libc.so.6"` and
`LIBPTHREAD = "libpthread.so.0"` regardless of the libc flavor being built. The
console path solved this long ago — `src/target/linux_x86_64/plan.rs:24-34` maps
the flavor to `libc.so.6` or `libc.musl-x86_64.so.1`, and on musl puts pthread in
libc — but the GTK module never got the same treatment, because plan-05 declared
app mode glibc-only and nothing forced the issue.

This sub-plan makes the app-mode import surface flavor-correct, so a musl app
build declares only musl-world libraries. It is a **prerequisite** for plan-56-B
(which starts actually emitting musl AppImages) and lands first because it is
artifact-neutral today: app mode is still glibc-only until B, so nothing
observable changes and the whole sub-plan is provable by unit test.

The behavioral outcome: `app_mode_imports(LinuxFlavor::Musl)` yields imports
naming `libc.musl-<arch>.so.1` and **no** `libc.so.6` / `libpthread.so.0`.

References (read first):

- `src/target/linux_gtk/mod.rs:225-260` — the hardcoded library-name block and
  `lib_for`, the two tables this sub-plan collapses into one.
- `src/target/linux_gtk/mod.rs:684-780` — `app_mode_imports`, the import list.
- `src/target/linux_x86_64/plan.rs:19-42` — `Platform { flavor }` with
  `libc()` / `libpthread()` / `libc_import()`: the mapping to reuse.
- `src/target/linux_aarch64/plan.rs:14-27` — the aarch64 equivalent
  (`libc.musl-aarch64.so.1`).
- `src/target/shared/code/mod.rs:457-461` — where `platform_imports` (the
  symbol → library map) is built from the native plan.
- `src/target/linux_gtk/mod.rs:290-305` — `call_external`, which currently calls
  `lib_for` and already receives the map it should use instead.

## 1. Goal

- `app_mode_imports(flavor)` returns imports whose `library` field is
  `libc.so.6` for `Glibc` and `libc.musl-<arch>.so.1` for `Musl`, with pthread
  symbols following the same rule the console path uses (`libpthread.so.0` on
  glibc, libc itself on musl).
- The emitted relocation `library` labels agree with those imports **by
  construction**, not by a second table that has to be kept in sync.
- A `Musl` app-mode plan contains zero occurrences of `libc.so.6` and
  `libpthread.so.0`.

### Non-goals (explicit constraints)

- **No artifact change.** App mode is still glibc-only after this sub-plan
  (plan-56-B flips that), so every emitted binary — console and app, both
  arches — must be byte-identical to before. This is a refactor with a widened
  parameter, and the `macos-aarch64` app goldens and `scripts/artifact-gate.sh`
  must confirm it.
- **No change to the GTK/GLib library names.** `libgtk-4.so.1`,
  `libgobject-2.0.so.0`, `libglib-2.0.so.0`, `libgio-2.0.so.0`, `libcairo.so.2`
  are the same on both libc worlds; only the C-library names are flavored.
- **No macOS change.** `macos_aarch64/plan.rs:app_mode_imports` takes no flavor
  and must keep its current signature behavior (the trait default gains a
  parameter it ignores).
- **No riscv64 app mode.** `linux_riscv64` rejects app mode outright
  (plan-51-A §3.3: no GTK entry ported, no upstream AppImage runtime). Nothing
  here changes that.
- **No new symbols.** The import *set* is unchanged; only the `library` each
  symbol is attributed to becomes flavor-dependent.

## 2. Current State

### 2.1 Two tables that must agree, and only one is checked

`src/target/linux_gtk/mod.rs` carries the library names as module constants:

```rust
// --- Library names (app mode is glibc-only, plan-05 §1.1) ---
const LIBC: &str = "libc.so.6";
const LIBPTHREAD: &str = "libpthread.so.0";
```

They are consumed by **two** independent places:

1. `app_mode_imports()` (`:684`) — builds the `PlatformImport` list that becomes
   the plan's `platform_imports`, and from there the ELF's `DT_NEEDED`.
2. `lib_for(symbol)` (`:238`) — maps a symbol to its library for the *relocation*
   record, whose doc comment already admits the hazard: "*Library that exports
   `symbol`, **matching `app_mode_imports`***".

That is a duplicated table with a documented obligation to stay in sync — the
same shape as the `.dynstr` bug plan-46-D §1 records, where two derivations of one
value drifted and the failure was invisible to every header assertion. §4.2
removes the duplication rather than flavoring it twice.

### 2.2 The console path already has the mapping

`src/target/linux_x86_64/plan.rs:19-42`:

```rust
struct Platform { flavor: LinuxFlavor }
impl Platform {
    fn libc(&self) -> &'static str {
        match self.flavor {
            LinuxFlavor::Glibc => "libc.so.6",
            LinuxFlavor::Musl => "libc.musl-x86_64.so.1",
        }
    }
    fn libpthread(&self) -> &'static str {
        // On musl and modern glibc, pthread lives in libc.
        self.libc()
    }
}
```

aarch64 is the same with `libc.musl-aarch64.so.1`, except its glibc arm keeps
`libpthread.so.0`. **`Platform` is the receiver of `app_mode_imports()`**
(`linux_x86_64/plan.rs:413`, `linux_aarch64/plan.rs:404`), so the flavor is
already in scope at the exact call site — the mapping needs threading, not
inventing.

### 2.3 The symbol → library map is already flavor-correct downstream

`src/target/shared/code/mod.rs:457-461` builds `platform_imports` directly from
`native_plan.platform_imports`:

```rust
let platform_imports = native_plan.platform_imports.iter()
    .map(|import| (import.symbol.clone(), import.library.clone()))
    .collect::<HashMap<_, _>>();
```

So once `app_mode_imports` is flavor-correct, that map is too — automatically.
And `linux_gtk`'s asm builders already *receive* it and ignore it
(`mod.rs:405,437`: `_platform_imports: &HashMap<String, String>`). The correct
source of truth is already plumbed to the place that needs it.

### 2.4 Why nobody noticed

⚠️ **musl's loader silently absorbs the glibc compat names.** Verified
empirically on **both** musl boxes — 2227 (Alpine x86_64) and 2224 (Alpine
aarch64) — each with `gcompat` absent and no `/lib/libc.so.6` on disk. A musl app
binary whose `DT_NEEDED` is:

```text
libc.musl-<arch>.so.1   libgio-2.0.so.0   libgtk-4.so.1   libglib-2.0.so.0
libgobject-2.0.so.0     libcairo.so.2     libpthread.so.0   libc.so.6
```

loads and reaches GTK anyway. `ldd` reports
`libpthread.so.0 => /lib/ld-musl-<arch>.so.1` and does not list `libc.so.6` at
all; musl also provides `__libc_start_main` as a compat symbol. The obvious
explanation — that `gcompat` was covering for it — was tested and **disproved**:
the box was stock Alpine at the time.

This is the single most important fact in plan-56: **a wrongly-linked musl binary
runs fine and looks correct.** Launching it proves nothing. The only detector is
inspecting `DT_NEEDED` directly, which is why that assertion — not a smoke test —
is this sub-plan's acceptance criterion and plan-56-C's hardware gate.

## 3. Design Overview

Two pieces, the second of which deletes code:

1. **Thread the flavor** (§4.1) — `app_mode_imports` takes a
   `LinuxFlavor`; the two Linux backends pass `self.flavor`. The names come from
   a small helper mirroring the console `Platform::libc()`/`libpthread()`.
2. **Delete `lib_for`'s table** (§4.2) — `call_external` resolves a symbol's
   library from the `platform_imports` map it already receives, so the
   relocation label and the import list cannot disagree.

The correctness risk is **not** in the mapping, which is a two-arm match copied
from working code. It is in §4.2's blast radius: `lib_for` is on the codegen path
for *every* app-mode external call, including the existing glibc builds, and this
sub-plan must not move a single byte of the macOS or Linux-glibc output. That is
what the artifact gate is for.

### 3.1 Rejected: flavoring `lib_for`'s table alongside `app_mode_imports`

The obvious minimal change is to give `lib_for` a flavor parameter too and leave
both tables in place. Rejected: it preserves exactly the duplication that makes
this bug class possible, and doubles it (now two tables must agree *per flavor*).
The map lookup is strictly less code and cannot drift.

### 3.2 Rejected: deriving the libc name inside `linux_gtk`

`linux_gtk` could hold its own `fn libc(flavor, arch)`. Rejected: that is a
*third* copy of a mapping the two backend `Platform`s already own, and it would
have to know the arch, which `linux_gtk` otherwise does not. Passing the resolved
names in from the caller keeps one owner per backend.

## 4. Detailed Design

### 4.1 Threading the flavor

`app_mode_imports` gains the two resolved library names rather than the flavor
enum, so `linux_gtk` needs no knowledge of arch or libc naming:

```rust
/// The C-library sonames an app-mode build binds to, resolved by the calling
/// backend's `Platform` (plan-56-A §4.1). `libc` is `libc.so.6` on glibc and
/// `libc.musl-<arch>.so.1` on musl; `libpthread` is `libpthread.so.0` on glibc
/// and the same string as `libc` on musl, where pthread lives in libc.
pub(crate) struct AppLibcNames {
    pub libc: &'static str,
    pub libpthread: &'static str,
}

pub(crate) fn app_mode_imports(libc: AppLibcNames) -> Vec<PlatformImport>
```

Both backends already have the values:

```rust
// linux_x86_64/plan.rs:413, linux_aarch64/plan.rs:404
fn app_mode_imports(&self) -> Vec<PlatformImport> {
    crate::target::linux_gtk::app_mode_imports(AppLibcNames {
        libc: self.libc(),
        libpthread: self.libpthread(),
    })
}
```

The `LIBC`/`LIBPTHREAD` constants are deleted. The GTK/GLib/Cairo constants stay.

### 4.2 Deleting `lib_for`

`call_external` (`linux_gtk/mod.rs:290-305`) currently does:

```rust
let library = match lib_for(symbol) { Ok(l) => l, Err(m) => { …; LIBC } };
```

It becomes a lookup in the map the builder already receives, with the same
bug-176 D behavior on a miss (record the first error, keep going, let `finish`
return it — never `panic!`):

```rust
let library = match self.platform_imports.get(symbol) {
    Some(library) => library.clone(),
    None => {
        if self.err.is_none() {
            self.err = Some(format!(
                "app-mode codegen calls '{symbol}', which is not in the app-mode \
                 import list"
            ));
        }
        // Fall back to the libc name this build actually uses, so the emitted
        // relocation stays self-consistent while `finish` reports the error.
        self.libc.to_string()
    }
};
```

This requires the asm builder to hold the map (today `_platform_imports`) and the
resolved libc name. Both are already passed to the emit entry points; the change
is storing them on the builder struct instead of discarding them.

`lib_for` and its unit test (`mod.rs:993`) are deleted; §5 replaces the test with
one asserting the *invariant it existed to protect* — that every symbol
`call_external` can emit is present in `app_mode_imports`.

### 4.3 What the musl import list looks like

For `linux-x86_64` / `Musl`, every entry currently attributed to `LIBC` or
`LIBPTHREAD` becomes `libc.musl-x86_64.so.1`:

| symbol group | glibc | musl |
| --- | --- | --- |
| `__libc_start_main`, `pipe`, `dup2`, `close`, `setenv`, `write`, `read`, `fcntl`, `malloc`, `free`, `memcpy`, `memset`, `memmove`, `pause`, `_exit`, `isatty`, `tcgetattr`, `tcsetattr` | `libc.so.6` | `libc.musl-x86_64.so.1` |
| `pthread_create`, `pthread_detach` | `libpthread.so.0` | `libc.musl-x86_64.so.1` |
| `gtk_*`, `g_*`, `cairo_*` | unchanged | unchanged |

Note `__libc_start_main` is retained on musl: musl provides it as a compat
symbol (§2.4), and the GTK bootstrap's `_main` trampoline depends on it. Dropping
it is out of scope and would be a codegen change, not an import change.

## Compatibility / Format Impact

**Changes:** none observable. The import *set* and every emitted byte stay
identical for glibc app builds and for every console build, because `Glibc` maps
to exactly the strings the constants held.

**Unchanged:** the `.mfp` format, NIR/nplan/ncode schemas, the manifest, every
macOS artifact, and the `supports_app_mode` gates. `linux-riscv64` still rejects
app mode.

## Phases

### Phase 1 — Thread the flavor into the import list

Artifact-neutral by construction: `Glibc` yields the previous strings.

- [ ] Add `AppLibcNames` and change `app_mode_imports` to take it
      (`src/target/linux_gtk/mod.rs`); delete the `LIBC`/`LIBPTHREAD` constants.
- [ ] Pass `self.libc()` / `self.libpthread()` from
      `src/target/linux_x86_64/plan.rs:413` and
      `src/target/linux_aarch64/plan.rs:404`.
- [ ] Tests: `app_mode_imports(glibc)` is byte-identical to today's list;
      `app_mode_imports(musl)` contains no `libc.so.6` and no
      `libpthread.so.0`, and attributes `pthread_create`/`pthread_detach` to the
      musl libc — one case per arch (`libc.musl-x86_64.so.1`,
      `libc.musl-aarch64.so.1`).

Acceptance: the two unit cases above pass, and `scripts/artifact-gate.sh` is
green — no emitted byte moves.
Commit: b12213d2

### Phase 2 — Collapse `lib_for` into the import map

The blast-radius phase: touches every app-mode external call's relocation label.

- [ ] Store `platform_imports` + the resolved libc name on the asm builder
      (`src/target/linux_gtk/mod.rs`), replacing the `_platform_imports`
      placeholders at `:405,437`.
- [ ] Rewrite `call_external` (`:290-305`) to look the library up in that map,
      preserving bug-176 D's first-error-wins, never-panic behavior.
- [ ] Delete `lib_for` and its `lib_for("close")` / `lib_for("getenv")` test.
- [ ] Tests: replace the deleted test with one asserting every symbol reachable
      from the app-mode emitters appears in `app_mode_imports` (the invariant
      `lib_for`'s doc comment asserted informally); plus a case proving an
      unmapped symbol records an error rather than panicking.

Acceptance: `scripts/artifact-gate.sh` green (relocation labels are cosmetic, so
this must move **zero** bytes), `cargo test` green, and a `linux-x86_64` `--app`
`-ncode` dump shows every external call's `library` matching its import.
Commit: 2c8a803f

⚠️ **Implementation note — the binding must run at ONE choke point, not per
entry point.** Writing it per entry point (as §4.2 originally described) left
five relocations (`malloc`, `memcpy`, `g_idle_add`, `write` ×2) with *no*
library, because the app-mode io/term helpers come from half a dozen separate
`CodegenPlatform` hooks the entry points never see. It now runs in
`shared::code::bind_deferred_relocation_libraries`, over every assembled
function, so a hook added later cannot reintroduce the gap.

⚠️ **The artifact gate did not catch that.** There are no linux-app goldens
(plan-51-D), so the empty labels were invisible to all 1157 goldens; it was
caught by dumping `--app --ncode` and inspecting relocations directly. Do not
treat a green gate as proof for Linux app-mode changes.

## Validation Plan

- **Tests:** unit, in `src/target/linux_gtk/mod.rs` per the house convention —
  the glibc list is unchanged, the musl list is glibc-name-free per arch, pthread
  attribution follows the flavor, and an unmapped symbol errors rather than
  panics. Negative cases are first-class here because the positive case is
  invisible (§2.4).
- **Runtime proof:** none in this sub-plan — it is artifact-neutral, so there is
  nothing new to run. plan-56-C owns the hardware gate, which is where a musl
  binary's `DT_NEEDED` is actually inspected on box 2227.
- **Doc sync:** `src/docs/spec/app/02_linux-runtime.md` states the app-mode
  import surface; note that the C-library names are flavor-derived. The
  "app mode is glibc-only" claims stay accurate until plan-56-B and must **not**
  be edited here.
- **Acceptance:** `scripts/artifact-gate.sh` (the primary gate — this is a
  codegen-adjacent change that must move no bytes), `scripts/test-accept.sh`,
  `cargo test`, `cargo fmt` with the second pass in `repository/`.

## Open Decisions

- **Pass `AppLibcNames` vs. the `LinuxFlavor` enum** — recommend the resolved
  names. `linux_gtk` is arch-agnostic today and would otherwise need the arch to
  build `libc.musl-<arch>.so.1`, duplicating a mapping each backend already
  owns. (§4.1)
- **Keep `__libc_start_main` on musl** — recommend yes. musl provides it as a
  compat symbol and the GTK `_main` trampoline calls it; replacing that path is a
  codegen change belonging to neither this sub-plan nor plan-56. (§4.3)

## Summary

The engineering risk is not the mapping — that is a two-arm match copied from
`Platform::libc()`, which has shipped for both console flavors for a long time.
It is that **this sub-plan's correctness is invisible to every runtime test**:
musl absorbs `libc.so.6` and `libpthread.so.0` into its own loader, so both the
before and after binaries run identically on stock Alpine (§2.4, verified with
gcompat removed). Only `readelf -d` can tell them apart. Everything here is
therefore gated on unit assertions over the import list plus the artifact gate
proving zero byte movement, and the real-hardware `DT_NEEDED` check is deferred
to plan-56-C where a musl AppImage actually exists to inspect.

Left untouched: every emitted byte, the macOS backend, riscv64's rejection, and
the glibc-only gating itself — which plan-56-B removes.
