# MFBASIC Native Linker Generalization Plan

Last updated: 2026-06-19

This document proposes generalizing both native linkers — Mach-O (macOS) and
ELF (Linux) — from their current "tiny fixed import surface" scope to a model
that can link arbitrary numbers of dynamic libraries, imported functions,
imported data globals, and load-time initializers.

It is a prerequisite for the GUI app-mode plans:

- `specifications/plan-macos-app.md` (AppKit / Foundation / libobjc)
- a future `specifications/plan-linux-app.md` (GTK4 / GObject / GLib / GIO)

It complements:

- `specifications/linker.md` — current linker behavior
- `specifications/architecture.md`
- `specifications/package_format.md`

This is a planning document. It describes the current compiler↔linker contract,
the gaps in each backend, the required contract extensions, the per-backend
linker work, a validation strategy, and a recommended implementation sequence.

## 1. Summary

Today both linkers are correct but deliberately scoped to the small import
surface used by console programs:

- libc / libm / libpthread on Linux
- a single hardcoded `libSystem` dylib on macOS

GUI app modes change the import surface by an order of magnitude: dozens of
symbols across four-plus shared libraries, imported *data* globals, and
library/runtime initialization at load time. The linkers must be generalized
before either app mode can link.

The good news is that the compiler↔linker **contract already carries per-symbol
library information end to end**. The work is mostly (1) teaching the macOS
backend to honor multiple libraries instead of hardcoding `libSystem`, (2)
teaching the Linux backend to import data globals and emit symbol versioning,
and (3) adding a load-time initializer concept that neither backend models
today.

## 2. Current Contract

Imports and relocations flow through four representations. Each lowering stage
narrows the previous one.

```text
NativePlan.platform_imports : Vec<PlatformImport>     src/target/shared/plan.rs
    PlatformImport { library, symbol, required_by }
        |  (required_by dropped at code-plan lowering)
        v
NativeCodePlan                                        src/target/shared/code/mod.rs
    CodeImport     { library, symbol }
    CodeRelocation { kind, binding, library: Option<String> }
        v
EncodedImage                                          src/arch/aarch64/encode.rs
    EncodedImport     { library, symbol }
    EncodedRelocation { offset, target, kind, binding, library: Option<String> }
    EncodedSymbol     { name, section: Text | Data, offset }
        v
target linker consumes EncodedImage                   src/os/{linux,macos}/link.rs
```

### 2.1 Import Collection

Imports are gathered in `plan.rs` through the `NativePlanPlatform` trait. Each
method returns `Vec<PlatformImport>`:

```text
entry_imports
entry_error_imports
program_exit_imports
runtime_imports          (per runtime helper)
native_call_imports      (per native built-in call, e.g. math)
```

`PlatformImport` already names the `library` for every symbol, and `plan.rs`
validates that `library`, `symbol`, and `required_by` are all non-empty. The
compiler therefore already tells the linker which library each symbol belongs
to; the linkers simply do not all use that information.

### 2.2 Relocation Binding Vocabulary

The relocation `binding` field is a stringly-typed enum, paired with the
relocation `kind` and an optional `library`:

| binding      | kind                  | meaning                          | emitted at        |
|--------------|-----------------------|----------------------------------|-------------------|
| `"internal"` | `branch26`            | call to an internal symbol       | encode.rs (emit_bl) |
| `"external"` | `branch26`            | imported **function** call (stub)| encode.rs (emit_bl) |
| `"external"` | `page21` / `pageoff12`| imported **data global** (GOT)   | encode.rs (emit_symbol_ref) |
| `"data"`     | `page21` / `pageoff12`| internal data addressing         | encode.rs (emit_symbol_ref) |

`emit_bl` classifies a target as `internal` when a matching `EncodedSymbol`
exists, otherwise `external` when it is a known import, otherwise it is a hard
error. `emit_symbol_ref` classifies a data reference as `external` (imported
global, resolved through the GOT) when the symbol is an import, otherwise `data`
(internal).

This binding/kind/library triple is the seam through which functions, globals,
and per-library selection are already expressed. No new IR concept is required
to *describe* multi-library functions and globals — only the linkers must learn
to materialize them.

## 3. Current Capabilities And Gaps

| Capability                                  | Linux (ELF)                     | macOS (Mach-O)                          |
|---------------------------------------------|---------------------------------|-----------------------------------------|
| Multiple dynamic libraries                  | Yes — one `DT_NEEDED` per lib   | **No — hardcoded to `libSystem`**       |
| Per-symbol library selection                | By name across all `DT_NEEDED`  | **No — bind ordinal pinned to 1**       |
| Imported function calls                     | Yes (`R_AARCH64_JUMP_SLOT`)     | Yes (stub + bind)                       |
| Imported data globals                       | **No (`GLOB_DAT` missing)**     | Yes (GOT + `external` page21/pageoff12) |
| Symbol versioning                           | **No (`verneed`/`versym` missing)** | N/A (two-level namespace by ordinal)|
| Load-time initializers (`init_array`/mod-init) | **No**                       | **No**                                  |
| TLS relocations                             | No                              | No                                      |

Observations:

- The macOS linker is, on the library axis, **more** hardcoded than Linux:
  `imports_libsystem()` errors unless every import's `library == "libSystem"`,
  and the bind opcode stream sets `SET_DYLIB_ORDINAL_IMM(1)` for every symbol.
- The Linux linker is multi-library for functions but only emits
  `R_AARCH64_JUMP_SLOT`, so it cannot yet reference an imported data global.
- Neither backend runs any load-time initializer for the executable itself.
- Symbol versioning is a Linux/glibc-specific concern. Mach-O encodes the same
  intent through the dylib ordinal instead of a GNU symbol version.

### 3.1 Required Built-in Libraries

This plan serves the **built-in** packages only. User `LINK` binding packages
are resolved separately at runtime through `dlopen`/`dlsym` (an emitted pre-main
initializer), so they do not drive the linker; their resolution and the MFB↔C
marshaling thunk are specified in §12. The built-in surface that this linker must
satisfy maps to a small, fixed set of libraries per platform:

| Built-in package | Linux library | macOS library | Wired today |
|------------------|---------------|---------------|-------------|
| `io`, `fs`, `net` (TCP/UDP/DNS) | `libc.so.6` | `libSystem` | `io`/`fs` yes; `net` not yet |
| `thread` | `libpthread.so.0` (folded into `libc` on glibc ≥ 2.34) | `libSystem` | yes |
| `math` | `libm.so.6` | `libSystem` | yes |
| **`tls`** | **`libssl.so.3` + `libcrypto.so.3`** (OpenSSL 3) | **`Network.framework`** (+ `libSystem`/libdispatch) | **no** |
| `strings`, `regex`, `collections`, `json`, `general` | none (emitted code + embedded Unicode tables) | none | n/a |

Key points:

- `net` adds many new symbols but **no new library** — `socket`, `getaddrinfo`,
  `poll`, `recv`, `send`, and friends already live in `libc` / `libSystem`.
- **`tls` is the only current built-in package that forces a genuinely new
  linker-based dependency**, and it alone exercises the bulk of this plan
  independent of any GUI app mode:
  - **Multiple libraries** — OpenSSL is two libs (`libssl` + `libcrypto`); on
    macOS `Network.framework` is the first non-`libSystem` dependency, breaking
    the hardcoded-`libSystem` assumption.
  - **Linux symbol versioning** — OpenSSL 3 exports versioned symbols such as
    `SSL_connect@@OPENSSL_3.0.0`, so unversioned references can mis-bind. This is
    the concrete, non-GUI driver for §6.2.
  - **macOS framework loading** — `Network.framework` forces the multi-dylib +
    per-symbol dylib-ordinal + framework-path work in §7.
- The built-in surface (libc / libm / OpenSSL / Network.framework) is effectively
  **function-only**; even `errno` is reached through `__errno_location()` /
  `__error()`. So imported **data globals** (`GLOB_DAT`, §6.1) and **load-time
  initializers** (§5.3) are **not** required by any current built-in package —
  they are needed by the GUI app runtimes (AppKit/GTK) and can be deferred to an
  app-mode-gated phase (see §10).

TLS backend decisions (chosen):

- **Linux: OpenSSL 3** (`libssl.so.3`, `libcrypto.so.3`). The runtime calls a
  small function subset (`SSL_CTX_new`, `SSL_new`, `SSL_set_fd`, `SSL_connect`,
  `SSL_read`, `SSL_write`, `SSL_shutdown`, `SSL_free`, plus trust/verify setup).
  **OpenSSL 3 only** — no `libssl.so.1.1` fallback.
- **macOS: Network.framework** (`nw_connection_*`, `nw_protocol_*`,
  `tls_options_*`). This is Apple's non-deprecated TLS path, but it is
  **asynchronous and block/dispatch-queue based**. Bridging it to the synchronous
  blocking `TlsSocket` model requires running an `nw_connection` against a
  dispatch queue and waiting on completion from the worker thread, and it pulls
  in **libdispatch and the Blocks runtime ABI** (block literals,
  `_NSConcreteStackBlock`/`_NSConcreteGlobalBlock` from `libSystem`). The linker
  impact is just one more framework + `libSystem` symbols; the async→sync bridge
  and block-literal emission are `tls` *codegen* concerns tracked there, not
  linker concerns. (Secure Transport / `Security.framework` was the simpler
  synchronous alternative but is deprecated; it was not chosen.)

## 4. Goals

Generalize both linkers to support, from the existing contract:

1. **Multiple libraries.** Any number of distinct `library` values in the import
   set, each materialized as a real dynamic dependency.
2. **Imported functions** from any of those libraries.
3. **Imported data globals** from any of those libraries (Linux gap).
4. **Load-time initializers** so emitted runtime support and dependent libraries
   are correctly initialized before the program entry runs.
5. **Symbol versioning on Linux** where glibc requires a specific version.

Non-goals for this plan:

- New language surface or built-ins.
- TLS-imported variables (defer until a concrete need appears; note the gap).
- Static linking of GUI toolkits (GTK/AppKit are dynamic by nature).
- Replacing the hand-rolled writers with a third-party linker. The "compiler
  emits and links everything, no host linker" invariant is preserved.

## 5. Contract Changes

Most of the contract already suffices. Three additions are proposed.

### 5.1 Explicit Import Kind

Today function-vs-data is inferred indirectly from the relocation that
references a symbol (`branch26` ⇒ function, `page21`/`pageoff12` ⇒ data). That
works, but an explicit kind on the import record makes linker layout (stub slot
vs GOT-only data slot) deterministic without scanning relocations, and lets the
plan validate that, e.g., a data global is never branched to.

```rust
enum ImportKind {
    Function,
    Data,
}

pub(crate) struct PlatformImport {
    pub(crate) library: String,
    pub(crate) symbol: String,
    pub(crate) kind: ImportKind,        // new
    pub(crate) version: Option<String>, // new (see 5.2)
    pub(crate) required_by: String,
}
```

Thread `kind` through `CodeImport` and `EncodedImport`. The relocation `binding`
vocabulary is unchanged; `kind` is metadata about the imported symbol, not about
a particular reference to it.

### 5.2 Optional Symbol Version (Linux)

glibc exports versioned symbols. Referencing them unversioned mostly resolves to
the default today, but a large import surface raises the chance of an ambiguous
or rejected binding. Add an optional per-import version string:

```text
PlatformImport.version: Option<String>   e.g. Some("GLIBC_2.17")
```

On macOS this field is ignored (Mach-O selects by dylib ordinal). On Linux it
drives `verneed` / `versym` emission (see 6.2). When `None`, the linker emits an
unversioned reference exactly as today.

### 5.3 Load-Time Initializers

Add an optional list of internal initializer symbols to the code plan and
encoded image:

```rust
pub(crate) struct EncodedImage {
    // ...
    pub(crate) initializers: Vec<String>, // internal text symbols, run before entry
}
```

Each initializer is an internal function symbol that must run, in listed order,
after dynamic relocations are applied and before the program entry symbol is
called. This is how emitted runtime support performs one-time setup (e.g. app
runtime state, GObject type bootstrapping wrappers) without abusing the entry
path. Dependent *library* initializers (GLib/GObject/AppKit constructors) are
run by the dynamic loader when their library loads and are not listed here;
`initializers` is only for symbols defined inside the emitted image.

If a backend cannot yet honor a non-empty `initializers` list, it must error
rather than silently skip — consistent with `linker.md`'s "must not silently
omit a required library" rule.

## 6. Linux ELF Linker Changes

File: `src/os/linux/link.rs`, `src/os/linux/object.rs`.

### 6.1 Imported Data Globals (`GLOB_DAT`)

Add `R_AARCH64_GLOB_DAT` (`1025`) alongside the existing
`R_AARCH64_JUMP_SLOT` (`1026`). For an `external` `page21`/`pageoff12`
relocation:

- allocate a GOT entry for the imported data symbol
- emit a `GLOB_DAT` dynamic relocation against that GOT entry
- patch the referencing `adrp`/`add` (or `adrp`/`ldr`) to address the GOT slot

This matches what the macOS backend already does for imported data and closes
the one function/global asymmetry between the two backends.

### 6.2 Symbol Versioning (`verneed` / `versym`)

When any import carries a `version`, emit:

- `.gnu.version` (`versym`) — one `Elf64_Half` per dynamic symbol
- `.gnu.version_r` (`verneed`) — version-need records grouped by library
- `DT_VERNEED`, `DT_VERNEEDNUM`, `DT_VERSYM` dynamic entries

Unversioned imports keep `versym = 1` (global, unversioned). The concrete driver
is **OpenSSL 3** for `tls`: its exports are versioned (`@@OPENSSL_3.0.0`), so this
must be validated against real `libssl`/`libcrypto` symbol tables (GTK's
GLib/GObject exports are a later app-mode validation target).

### 6.3 Multiple Libraries

Already supported: one `DT_NEEDED` per distinct `library`. Confirm the GOT,
hash, and dynamic-symbol tables scale to the larger symbol counts and that stub
generation remains layout-correct (see the GOT/stub page-divergence hazard in
§8).

### 6.4 Initializers

Emit `DT_INIT_ARRAY` / `DT_INIT_ARRAYSZ` (and a `.init_array` section) listing
the `EncodedImage.initializers` symbols. The dynamic loader runs them after
relocation and before transferring control to the ELF entry point.

## 7. macOS Mach-O Linker Changes

File: `src/os/macos/link.rs`, `src/os/macos/object.rs`.

### 7.1 Generalize Beyond `libSystem`

Replace the `imports_libsystem` boolean gate with a per-library model:

- Collect the distinct `library` values from the import set.
- Map each library name to a dylib path (e.g. `libSystem` →
  `/usr/lib/libSystem.B.dylib`, `AppKit` →
  `/System/Library/Frameworks/AppKit.framework/AppKit`, etc.).
- Emit one `LC_LOAD_DYLIB` per library, assigning each a 1-based **dylib
  ordinal** in load order.
- Update `load_commands_size` / `load_command_count` to account for the variable
  number of dylib commands.

### 7.2 Per-Symbol Dylib Ordinal In Bind Opcodes

`bind_info` currently hardcodes `SET_DYLIB_ORDINAL_IMM(1)` (`0x11`) for every
symbol. Replace with the ordinal of the symbol's actual library:

```text
for each import:
    SET_DYLIB_ORDINAL_IMM(ordinal_of(import.library))
    SET_SYMBOL_TRAILING_FLAGS_IMM(0), symbol_name
    SET_TYPE_IMM(BIND_TYPE_POINTER)
    SET_SEGMENT_AND_OFFSET_ULEB(GOT slot)
    DO_BIND
```

Ordinals above 15 require `SET_DYLIB_ORDINAL_ULEB` instead of the `_IMM` form;
handle both.

### 7.3 Framework Path Mapping

Frameworks are not plain `/usr/lib/*.dylib`. Add a small library-name →
load-path table so the plan can request frameworks by name and the linker
resolves the correct install path. This table is the macOS analog of Linux's
`DT_NEEDED` library names. The concrete near-term entry is **`Network.framework`**
for `tls`:

```text
libSystem  -> /usr/lib/libSystem.B.dylib
Network    -> /System/Library/Frameworks/Network.framework/Network
AppKit     -> /System/Library/Frameworks/AppKit.framework/AppKit       (app mode)
Foundation -> /System/Library/Frameworks/Foundation.framework/Foundation (app mode)
libobjc    -> /usr/lib/libobjc.A.dylib                                  (app mode)
```

`Network.framework` is asynchronous and block/dispatch-queue based. The async→sync
bridge and block-literal emission live in `tls` codegen, not the linker; the
linker only needs the framework `LC_LOAD_DYLIB` plus the libdispatch/Blocks
symbols (`dispatch_*`, `_NSConcreteStackBlock`, `_NSConcreteGlobalBlock`), which
resolve from `libSystem`.

### 7.4 Imported Data Globals

Already supported through the existing `external` data binding and `got_entries`
path. Verify it generalizes once data symbols come from multiple dylibs (the
GOT entry must bind with the correct dylib ordinal, per §7.2).

### 7.5 Initializers

Emit an `S_MOD_INIT_FUNC_POINTERS` section (and the matching `LC_DYLD_INFO`
metadata) listing the `EncodedImage.initializers`. dyld runs these after binding
and before `LC_MAIN`. Framework `+load`/mod-init initializers in AppKit/libobjc
run automatically when those dylibs load.

If app mode binds AppKit purely dynamically via `objc_msgSend`, no Objective-C
class registration section is required for the executable. If emitted code ever
defines ObjC classes, an `__objc_imageinfo` section would also be needed; that
is out of scope here and should be tracked in `plan-macos-app.md`.

## 8. Risks

- **GOT / stub layout divergence.** Both backends compute stub branch targets
  from the final, post-stub code length. Larger import counts push the GOT
  across page boundaries; a pre-stub length miscalculation makes every stub
  branch land a page away, producing layout-sensitive `SIGBUS`. This code path
  is already known-fragile (see the macOS import-stub GOT note) and must be
  re-validated at the new scale.
- **glibc symbol versioning.** Getting `verneed`/`versym` subtly wrong yields
  loader rejections or silent mis-binding. Validate against real OpenSSL 3
  (`libssl`/`libcrypto`) exports first, then GLib/GObject for app mode — not
  synthetic stubs.
- **Per-symbol dylib ordinals.** An off-by-one ordinal binds a symbol to the
  wrong framework — often loads fine, then crashes on first call.
- **Initializer ordering.** Initializers must run after relocation; running an
  initializer that calls an unbound import crashes at startup.

## 9. Validation And Testing

- **Golden artifacts.** Native plan / code plan / encoded image goldens must
  show the new `kind`, `version`, and `initializers` fields and multi-library
  import sets.
- **Multi-library link test.** A minimal program importing functions from two
  distinct libraries on each backend; confirm both resolve and run. The natural
  real-world driver is `tls` (OpenSSL `libssl`+`libcrypto` on Linux,
  `Network.framework`+`libSystem` on macOS).
- **Versioned-symbol test (Linux).** Import a versioned OpenSSL 3 symbol
  (`SSL_connect@@OPENSSL_3.0.0`) and confirm the loader accepts the versioned
  reference.
- **Imported-global test (app-mode-gated).** Reference an imported data global on
  Linux; confirm `GLOB_DAT` resolves correctly (macOS already covered). Not
  exercised by any current built-in package.
- **Initializer test (app-mode-gated).** An initializer that records it ran
  (e.g. sets a global observed by the entry); confirm ordering before entry.
- **Acceptance suite.** Run `scripts/test-accept.sh` and update CLI/native
  artifact goldens.

## 10. Recommended Implementation Sequence

The driving consumer is **`tls`**, not GUI app mode. Phases 1–3 are everything a
built-in package needs; the data-globals and initializer work (Phase 4) is gated
on the GUI app plans and exercised by no current built-in package.

### Phase 1: Contract Extensions
- Add `ImportKind`, `version`, and `initializers` to the plan/code/encoded
  representations and validations.
- Update goldens. No behavior change yet; existing imports get
  `kind = Function`, `version = None`, `initializers = []`.

### Phase 2: macOS Multi-Library (driven by `Network.framework`)
- Generalize past `libSystem`: multiple `LC_LOAD_DYLIB`, library→path table,
  per-symbol dylib ordinals in `bind_info`.
- Deliverable: a macOS executable importing from two dylibs (e.g. `libSystem` +
  `Network.framework`) links and runs.

### Phase 3: Linux Symbol Versioning (driven by OpenSSL 3)
- Add `verneed`/`versym` driven by `version`.
- Deliverable: a Linux executable importing versioned OpenSSL 3 symbols across
  `libssl` + `libcrypto` links and runs.

### Phase 4: App-Mode Enablers (deferred — not needed by built-ins)
- Add `GLOB_DAT` for imported Linux data globals.
- Emit `DT_INIT_ARRAY` (Linux) and `S_MOD_INIT_FUNC_POINTERS` (macOS) for
  `EncodedImage.initializers`.
- Deliverable: imported data globals resolve and listed initializers run, in
  order, before entry on both backends.

### Phase 5: Hardening
- Re-validate GOT/stub layout at GUI-scale import counts.
- Stress tests with dozens of imports across four-plus libraries per backend.

## 11. Open Questions

1. Should `ImportKind` distinguish a future `Tls` variant now, or defer until a
   TLS-imported global is actually needed?
2. Library-name vocabulary: do the plan layers use logical names (`AppKit`,
   `libgtk-4`) resolved to paths in the linker, or fully-qualified paths
   threaded from the plan? Logical names keep the plan portable; resolution
   belongs in the per-OS linker.
3. Should `initializers` carry a priority/order field, or is list order
   sufficient? List order is simpler and matches both `init_array` and
   `mod_init` semantics.
4. Does any near-term feature require imported **TLS** variables, or can that
   relocation class stay unsupported with an explicit error?
TLS backend decisions are **settled**, not open: Linux uses **OpenSSL 3 only**
(`libssl.so.3` / `libcrypto.so.3`; no OpenSSL 1.1 `libssl.so.1.1` fallback —
3-only is simpler and matches current distros) and macOS uses `Network.framework`. The macOS choice deliberately accepts the harder
asynchronous, block/dispatch-based programming model in exchange for **not
building on a deprecated API** — Secure Transport / `Security.framework` is
deprecated, so `Network.framework` is the better long-term foundation even though
the synchronous Secure Transport API would have been easier short-term. The
async→sync bridge cost is a one-time `tls` codegen investment; the linker impact
is limited to one framework plus libdispatch/Blocks symbols (§7.3).

## 12. User `LINK` Bindings: Resolution & MFB↔C Marshaling

`LINK` binding packages (`plan-link-update.md`) do **not** drive the static linker
(§3.1): they contribute no compile-time import records, emit no `DT_NEEDED` /
`LC_LOAD_DYLIB`, and add no relocations against their symbols. They are resolved
**entirely in emitted code** at run time. This section is therefore layered *under*
`plan-link-update.md` (which owns LINK *semantics* — resources, ownership, and the
`ABI` / `CONST` / `SUCCESS_ON` surface) and sits *beside* the rest of this plan,
consuming none of its static-linker machinery. It is the home `plan-link-update.md`
§14 recommends for the native-binding frontend's lowering.

### 12.1 Resolution (pre-main initializer)

For each `LINK "<lib>" AS <binding>` the compiler emits, into the program's
pre-main initializer:

- one `dlopen("<lib>", RTLD_NOW | RTLD_LOCAL)` against the platform library name
  (`"sqlite3"` → `libsqlite3.so.0` / `libsqlite3.dylib` / `sqlite3.dll`; the same
  logical-name → path resolution the §11 library-name question raises for the
  static linker, resolved per-OS);
- one `dlsym(handle, "<SYMBOL>")` per `FUNC`, stored into a private per-binding
  function-pointer table slot.

A failed `dlopen`/`dlsym` is a **startup error** — the binding cannot be honored —
reported and aborting before `main`, never a zero/placeholder pointer (mirrors the
linker's no-silent-placeholder rule, linker.md §8). `dlopen`/`dlsym`/`dlclose`
live in `libc`/`libSystem`, which is always linked.

### 12.2 The marshaling thunk

Because MFBASIC values are not C values, each `LINK` `FUNC` lowers to a generated
**thunk** — `_mfb_linker_<binding>_<fn>` (e.g. `_mfb_linker_sqliteLink_bindText`),
matching the `_mfb_pkg_*` naming scheme (linker.md §7). The thunk is the single
MFB↔C boundary; it:

1. **in-marshals** each wrapper argument into its `ABI` slot per §12.3;
2. **pins** each `CONST` slot to its fixed value (`plan-link-update.md` §5c);
3. **allocates** the storage each `OUT` slot points at;
4. **calls** through the §12.1 pointer using the target C calling convention
   (AAPCS64 / Darwin arm64), placing slots in C argument order;
5. **out-marshals** the native return and any `OUT` slots back into MFBASIC values
   per §12.3, then applies `SUCCESS_ON` (or, where specified, the result mapping)
   to turn the status into a success value or an `Error`.

The thunk owns lifetime correctness at the boundary: input buffers it passes stay
alive for the synchronous call (owned by the caller frame); foreign memory it
receives is copied into owned MFBASIC values before returning (§12.4); a produced
`CPtr` is written only into a LUT resource entry and never escapes as a value
(`plan-link-update.md` §11).

### 12.3 MFB ↔ C type mapping

Each `ABI` slot names a C-ABI type; the thunk maps it to and from the wrapper's
MFBASIC type. `Integer` is 64-bit.

| ABI type | C type | MFBASIC type | In (MFB→C) | Out (C→MFB) |
|----------|--------|--------------|-----------|-------------|
| `CInt32` | `int32_t` | `Integer` | range-check, narrow 64→32 | sign-extend 32→64 |
| `CInt64` | `int64_t` | `Integer` | direct | direct |
| `CDouble` | `double` | `Float` | direct | reject NaN/Inf at the boundary (mfbasic.md §3) |
| `CPtr` | `void*` | *(resource hidden repr only)* | LUT `Pointer` of the borrowed `RES` arg | new LUT entry (producer `OUT` / pointer return) |
| `CString` | `const char*` | `String` | copy into a NUL-terminated UTF-8 buffer valid for the call | **copy the returned `char*` into an owned `String`** (§12.4) |
| `CBool` | `int` / `_Bool` | `Boolean` | `0` / `1` | `!= 0` |
| `CByte` | `uint8_t` | `Byte` | direct | direct |

Rules:

- **`CPtr` is resource-only.** A `CPtr` slot's MFBASIC side is always a declared
  `RESOURCE` (a borrowed `RES` argument in, a produced handle out) or a `CONST`
  pin (§5c). It never maps to an ordinary MFBASIC value — that is the §11
  `CPtr`-escape prohibition, enforced at the one place a pointer crosses.
- **Width and validity are checked, not assumed.** `Integer`→`CInt32`
  range-checks (an out-of-range value fails rather than silently truncating); a
  `CDouble` returning NaN/Inf is rejected at the boundary exactly as imported
  `Float`s are.
- **`CString` input** is copied into a fresh NUL-terminated UTF-8 buffer the thunk
  keeps alive across the call (MFBASIC `String`s are length-counted and not
  guaranteed NUL-terminated). Whether the *callee* may retain that pointer past
  the call is the callee's contract, expressed on the input side by a `CONST`
  sentinel such as SQLite's `SQLITE_TRANSIENT` (`plan-link-update.md` §5c); the
  thunk itself does not keep input buffers alive past return.

### 12.4 C-string return marshaling (resolves `plan-link-update.md` §5b gap)

A text **return** (`sqlite3_column_text`'s `const char*`) is *not* a missing ABI
form — it is this thunk handling the output direction, the mirror of the
already-implemented `String`→`char*` input. The declarative surface
(`ABI (...) AS return CPtr`, wrapper `AS String`) already says enough; the thunk
emits the copy. Three policy points it fixes, with defaults that compile
`sqlite3_column_text` as written:

- **Ownership = copy-and-leave (default).** The returned `char*` is assumed owned
  by the callee and valid only until its next call (SQLite frees it on the next
  `step`/`finalize`), so the thunk **copies the bytes into an owned `String`
  immediately** and never frees the source. A future `FREE_RESULT`-style
  annotation can cover C functions that hand the caller a `malloc`'d string to
  free; until then copy-and-leave is the only discipline, and it covers the SQLite
  surface.
- **NULL return.** A NULL `char*` (e.g. a SQL NULL column) marshals to an `Error`
  or an empty `String` per the wrapper; distinguishing NULL from `""` cleanly is
  the same "see the raw result" need as the still-open multi-valued result-code
  gap (`plan-link-update.md` §5b), so a NULL-returning text column ultimately wants
  the `RESULT` mapping too.
- **Encoding.** Returned bytes are validated as UTF-8 at the boundary (consistent
  with `fs`/`net`); `sqlite3_column_text` is defined UTF-8, so it passes.

This closes the C-string-return item in `plan-link-update.md` §5b: it is
generated-thunk codegen, not new ABI vocabulary. The sole remaining declarative
gap there is the multi-valued result-code (`RESULT`) mapping.
