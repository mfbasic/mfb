# Design Work-Pad — `MEMORY` / direct-memory block

> **Status:** scratch. This is a design work-pad, *not* a plan. It captures the
> current shape of the idea, the decisions we've settled, and the open questions.
> Nothing here is committed to implement yet.

## Goal

Give MFBASIC source direct hardware access — memory-mapped registers, framebuffers,
and (eventually) device transports for USB/serial — **without** dropping to a C lib
linked through `LINK`. Gated behind a single coarse `--mem` build switch that
propagates through the package graph: importing a package that uses the feature
requires `--mem` too, or the build fails.

## The core analogy (load-bearing)

**`MEMORY` block : field-access / raw memory  ::  `LINK` block : `CPtr`.**

The raw capability (dereferencing an overlay's fields, indexing a window, pointer
math) is legal **only inside the declaring block**. Outside, the overlay type is an
**opaque resource** — you can only pass it through the block's own `FUNC`s. This
reuses the §15 resource model's opacity, borrow-only, no-copy, no-store-in-collection,
and thread rules *for free*, and makes the block the single audited trust boundary,
exactly like `LINK`.

```basic
' Compile error: UartRegs is opaque outside its MEMORY block (NATIVE_CPTR_ESCAPE-style)
FUNC getUartStatusTest(RES r AS UartRegs) AS Integer
    RETURN bits::band(r.status, 1) = 1
END FUNC
```

## Settled decisions

1. **Producer FUNCs return a `RES`** (like `LINK` wrappers). Resources come from a
   call, are scope-bound, and drop through a real close. This is §15-consistent and
   removes the earlier "static producerless global" oddity. The overlay *descriptor*
   (the small struct of pointers/offsets) is allocated normally; only the *target
   memory* is raw / not heap-backed.

2. **`CLOSE BY` can do real work on drop** — e.g. quiesce the device by writing a
   control register. Spelled `CLOSE BY <fn>` to align with the existing `RESOURCE
   Name CLOSE BY fn` form; `CLOSE BY Nothing` for windows that need no teardown.

3. **Two distinct tools inside the same block** — do NOT unify them through `CPtr`:

   - **(a) Register overlays** — fixed register maps (a UART). Width-typed fields,
     and **field access *is* the volatile register I/O**: `r.status` is one volatile
     read of the field's width; `r.control = 0` is one volatile write. The struct's
     field offsets *are* the register map. No `CPtr`, no `PEEK`/`POKE`.
   - **(b) Dynamic windows** — runtime-sized byte regions (a framebuffer). A
     fat-pointer **view** (`MEM` / `MemView` = pointer **and** length) with indexed,
     bounds-checked access: `PEEK r.mem AT off FOR n` / `POKE r.mem AT off FOR n = val`
     (`FOR n` mandatory on both — #15).

   Rationale: forcing both through `CPtr` fields makes the window case work but makes
   the register case ambiguous — `r.control = 0` then nulls a pointer instead of
   writing the register. See "Why not unify" below.

4. **Views carry their length** (`MEM`/`MemView` fat pointer), **not bare `CPtr`.**
   A bare `CPtr` is a naked address with no length, so `AT`/`FOR` would have nothing
   to bounds-check — that throws away the one safety invariant the feature keeps
   (`index ≥ len → ErrBounds`). Reserve `CPtr` for the genuinely address-only case
   and treat it as explicitly unchecked.

5. **Field assignment is a MEMORY-block-local exception.** The language has no
   field assignment generally; inside the block, `r.control = 0` (register write) and
   overlay construction are permitted and lower to volatile stores. Quarantined to
   the trust boundary, like `CPtr` is quarantined to `LINK` ABI slots.

6. **C-width types in source positions are OK *because* they're quarantined.**
   `CUInt16`/`CUInt32` as overlay field types never escape (the type is opaque
   outside), and a field read marshals `CUInt32 → Integer` (zero-extended), exactly
   like a `LINK` return. Same boundary, same marshaling rule.

7. **Block keyword is `MEMORY … END MEMORY`.** Chosen over reusing `RESOURCE` (which
   would collide with the existing `RESOURCE Name CLOSE BY fn` declaration form and
   the inner `RES … CLOSE BY`). The block names the feature directly.

8. **`LAYOUT` overlay type: transparent inside, opaque outside.** A `LAYOUT … END
   LAYOUT` declared in a `MEMORY` block is an overlay/resource type whose fields are
   **accessible inside the block** (field read/write, indexing) and **opaque outside**
   (pass-only through the block's `FUNC`s, exactly like `UartRegs` in the compile-error
   example).

9. **Low-`n`-bytes placement — one marshaling rule, `n = 8` is the degenerate case.**
   A field/`PEEK` of `n` bytes places those `n` source bytes into the low `n` bytes of the
   destination `Integer`; the high `8 − n` bytes are zero. For `n < 8` (`CUInt8/16/32`) the
   result is therefore always non-negative. For **`n = 8`** (`CUInt64` / `PEEK … FOR 8`)
   there are no high bytes left to zero — the read is an **exact 64-bit bit-copy**, so a
   value with bit 63 set reads back as a *negative* `Integer`. The bit **pattern** is
   preserved, not the unsigned magnitude: `Integer` is MFB's only 64-bit carrier and it is
   signed, so full-width reads are bit patterns, and any code needing unsigned semantics on
   them uses `bits::` (which operates on the raw pattern) rather than a signed compare. This
   also settles the `PEEK`-section open question: the bound is **`≤`** — `FOR 8` is allowed,
   with the sign caveat above.

9b. **Access width equals field width — no splitting.** A `CUInt64` field is *one* 64-bit
   bus access. A device that permits only 32-bit MMIO must be modeled with two `CUInt32`
   fields, not one `CUInt64` — the block never splits a wide field into narrower cycles.

10. **`MEM` is a simple primitive-ish byte-window type — `PEEK`/`POKE` only.** Not a
    parameterized `MemView OF T`, not a List: just a raw bounded byte window (ptr+len)
    with no `length`/index/iterate/slice surface. All reads/writes go through
    `PEEK`/`POKE … AT off FOR n`; to materialize a List you `PEEK` each element and
    `append()`. **`MEM` is valid *only* inside a `MEMORY` block** — it cannot be named,
    returned, or stored in ordinary code (like `CPtr` relative to `LINK`).

11. **Overlay layout is defined + auto-padded.** Fields lay out at natural alignment,
    no implicit reordering. The base (`OVERLAY … AT base`) must be naturally aligned
    for the widest field (checked at compile time for a literal base). Trailing padding
    to the struct's alignment is **inserted automatically** at the end (so explicit
    `_pad` tail fields aren't required for alignment).

12. **Scope is MMIO only.** `MEMORY` is for memory-mapped I/O. For USB/serial and
    other host devices, open the device node with `fs::open` or bind a library through
    `LINK` — those are *not* userspace MMIO and are out of scope for this block.

13. **Multi-byte access is defined little-endian.** A `PEEK`/`POKE … FOR n` (and any
    multi-byte `LAYOUT` field) interprets the bytes as **little-endian**, regardless of
    host. The memory-access layer is responsible for the translation:
    - **Free on every current target.** aarch64 (macOS/Linux), x86-64, and rv64 are all
      little-endian, so the LE definition compiles to the *plain* native `LDR`/`STR` —
      no byteswap, no overhead, identical codegen to "machine order." A swap (`REV*`)
      would only be emitted on a big-endian host, which the toolchain doesn't target;
      even then it's one ALU op, **not** an extra bus access (volatile "one access"
      guarantee preserved).
    - **Source is portable.** Because the semantics are fixed LE rather than "whatever
      the host is," the same `MEMORY` source behaves identically across targets — no
      `app::endianness` branching needed on the common path.
    - **Big-endian *wire* data still needs an explicit swap.** Network protocols / some
      device formats that are big-endian on the wire are byteswapped by the developer
      via `bits::` — the same under any model, and now the *only* place a swap appears.
    - **Build the translation seam *now*, even with no BE target.** The LE guarantee is
      a contract, not "we happen to be LE." Every `PEEK`/`POKE`/`LAYOUT`-field access must
      route through a single host-order → little-endian translation point (in-place /
      a per-access hook), which is the identity no-op on today's LE targets. Pre-planning
      this means adding a big-endian backend later is a localized change (flip the hook
      to emit `REV*`), not a tree-wide retrofit of every access site. Do **not** hardcode
      the assumption that host order == LE at each call site.

14. **Grammar is settled; `PEEK` is an expression, `POKE` is a statement.** The `OVERLAY`
    / `VIEW` / `PEEK` / `POKE` forms as written in the sketches are the intended grammar
    (incl. `POKE … AT off FOR n = value`). **Locked asymmetry:** `PEEK … AT off FOR n` is
    expression-like (it yields a primitive, usable inline as an arg/operand); `POKE … AT
    off FOR n = value` is statement-only (it never yields a value). This asymmetry is not
    an invention — it is exactly what `PEEK()`/`POKE` have meant since classic BASIC, which
    is part of why those names were chosen (#26).

15. **`FOR n` is required on both `PEEK` and `POKE`.** No width inference. MFB literals
    are initially untyped, so a width-less `POKE r.mem AT off = 0` would be ambiguously
    1/2/4/8 bytes — intolerable in a low-level block. Mandatory `FOR n` forces the
    developer to state the hardware register width on every read and write; omitting it
    is a compile error.

16. **Keep the `MEMORY` domain small.** A `MEMORY` block contains only the overlay-type
    declarations and the `FUNC`/`SUB` wrappers — no constants, type aliases, or other
    top-level forms inside it. It *may reference* symbols defined outside the block
    (e.g. an outside `CONST`), but the block's own surface stays minimal. The goal is a
    tight, auditable trust boundary, not a second module system.

17. **Overlay declaration keyword is `LAYOUT … END LAYOUT`** (not `TYPE`). Reusing
    `TYPE` was confusing because field access is overloaded by scope — ordinary record
    access vs volatile register I/O. `LAYOUT` reads as "a memory layout, not a value
    type" and makes the overload explicit at the declaration site.

18. **`MEMORY`-only keywords — and this is load-bearing, not tidiness.** `LAYOUT`, `MEM`,
    `PEEK`, `POKE`, `OVERLAY`, `VIEW`, and `FENCE` are valid **only inside a `MEMORY`
    block**. Outside one they are not in scope (a plain identifier / parse error), the same
    way `MEM` cannot be named in ordinary code. The block is the sole place this vocabulary
    exists.

    **Why it's load-bearing.** `keyword()` (`src/lexer.rs`) matches
    `eq_ignore_ascii_case`, and `::` lexes as a separate `DoubleColon` token, so the
    identifier *after* `::` goes through the same keyword lookup — a global keyword would
    capture `collections::mem`, `foo::layout`, and friends. Measured across the tree's
    27,369 `.mfb` files: `mem` 75 word-matches, `layout` 52, `memory` 233 (`view`, `fence`,
    and `overlay` are clean at 0). Scoping the vocabulary to the block is therefore what
    keeps these spellings usable at all.

    **Implementation requirement.** The lexer must recognize this vocabulary
    *contextually* — tracking `MEMORY … END MEMORY` nesting — not via the global keyword
    table. Precedent exists: the lexer already does context-sensitive work for `DOC`
    raw-capture and for `REM` at statement start (`is_statement_start`). Do not add these
    to `keyword()` unconditionally.

    **Residual cost, accepted.** Inside a `MEMORY` block these spellings are unavailable as
    identifiers, so a block cannot call e.g. a `mem::` package or name a local `layout`.
    Given #16 (keep the block's surface minimal), that's tolerable — but it is a real
    constraint on block-local code, not a theoretical one.

19. **Volatile by definition — no annotation.** Everything in a `MEMORY` block is
    volatile because it's in the block; there is no `VOLATILE` marker and no need to tag
    individual `FUNC`/`SUB`s. Every `LAYOUT` field / `PEEK` / `POKE` access is one
    in-order bus access, exempt from fold/CSE/reorder/cache. (See the volatile-crux
    section.)

20. **Non-cacheable is the OS's job, not our guarantee.** Volatile (one in-order
    instruction per access) is the compiler's contract. *Non-cacheable* (that the access
    actually reaches the device, bypassing the data cache / write buffer) is a **mapping
    property set by the OS / page tables** — MFB cannot enforce it. We document the
    requirement; the caller/OS must map the region uncached. Only if MFB does the mapping
    itself (region-relative path) can it *request* a device/non-cacheable type at `mmap`
    time — ask the OS, not enforce.

21. **An unwired `MEM` field is the null view `{ptr: 0, len: 0}`.** `OVERLAY` without a
    `WITH` for a `MEM` field leaves it null. Because every access is bounds-checked against
    `len`, any `PEEK`/`POKE` on an unwired field fails the check (`off + n > 0` for any
    `n ≥ 1`) and traps `ErrBounds` — it **never dereferences null**. So the dynamic form
    (registers live, `pixels` not yet wired in `attachDisplay`) is safe by construction:
    touching `pixels` before `r.pixels = VIEW …` traps cleanly.

22. **`VIEW … FOR size` validates `size ≥ 0`; bounds are overflow-free.** A negative size
    is an error (compile-time when constant, otherwise `size < 0 → ErrValue` at
    construction), never a giant window; `len` is stored as a 64-bit count. An access is
    in-bounds iff `0 ≤ off` **and** `1 ≤ n` **and** `n ≤ len` **and** `off ≤ len − n` — the
    subtraction is guarded by `n ≤ len`, so it can't underflow. This replaces the naive
    `off + n > len` (which can wrap). `off` is a signed `Integer` expression, so the
    `off ≥ 0` guard is mandatory.

23. **Alignment: register fields aligned by construction; `MEM` windows are the
    programmer's responsibility.** Register fields lay out at natural alignment and the base
    is checked aligned to the widest field (#11), so **every register-field access is
    naturally aligned** — the one-bus-access guarantee holds unconditionally, no per-access
    check. `PEEK`/`POKE … AT off` on a `MEM` window is byte-addressed; `off` need not be
    aligned, but the single-access / atomicity guarantee applies **only to naturally-aligned
    accesses** (`off % n = 0`). An unaligned `FOR n>1` may lower to multiple accesses and
    **will fault on strict-alignment device mappings** — consistent with `MEM` being the
    looser, `--mem`-gated tool while the register overlay stays the strict path.

24. **`FENCE` — a `MEMORY`-block-only barrier statement.** Emits a full data memory barrier
    ordering **all prior** `MEMORY`-block accesses (register *and* `MEM`) before **any
    subsequent** one — the tool for ordering across the MMIO↔normal-memory boundary and
    across separately-mapped regions, which per-access volatility does *not* provide. Lowers
    to the target's full data barrier (`DMB SY` aarch64, `mfence` x86-64, `fence rw,rw`
    rv64), no-op cost only where the ISA needs none. MFB never auto-inserts barriers; the
    developer places `FENCE` where cross-region ordering matters (the fill-buffer-then-poke-
    "go" DMA pattern). Acquire/release granularity is a future refinement — one full `FENCE`
    covers v1.

25. **`OVERLAY … [WITH …]` is an expression, not a statement.** It yields the overlay
    resource and appears either as a bare trailing expression (implicit return, `openUart`)
    or as the RHS of `RES r AS T = …` (`attachDisplay`). There is no separate statement
    form — both usages are the one expression.

26. **The window accessors are `PEEK` / `POKE`, not `GET` / `SET`.** Two independent
    reasons, either of which would be sufficient:

    - **`GET`/`SET` are unusable as keywords here.** They collide head-on with the
      `collections::` API. Measured across the tree's 27,369 `.mfb` files: **`get` 14,238
      word-matches, `set` 3,135** — sampling confirms these are overwhelmingly real call
      sites (`IF collections::get`, `r = collections::set`), not comments. Because the
      lexer is case-insensitive and `::` is its own token, `GET`/`SET` keywords would
      capture the identifier in every one of them. Decision #18's contextual lexing would
      technically contain the damage to `MEMORY` blocks — but the residual is exactly
      backwards for this feature: #10 says you materialize a `List` by `PEEK`-ing each
      element and `append()`-ing it, which is precisely the block-local code most likely to
      want `collections::get`. `PEEK`/`POKE` measure **0 / 0** — no collision anywhere, and
      no entry in `keyword()`.
    - **They're the right names on their own merits.** `PEEK`/`POKE` are the BASIC
      lineage's own vocabulary for exactly this operation — raw, address-indexed,
      width-explicit memory access — so the feature reads as native to the language rather
      than grafted on. They also carry the correct connotation: `collections::get` is a
      safe, checked container read, and reusing that verb for an unchecked-address device
      access would have understated the danger at precisely the wrong site.

    **The lineage also reinforces #14's locked asymmetry for free.** In classic BASIC
    `PEEK(addr)` is a *function* (it yields a value) and `POKE addr, val` is a *statement*
    (it does not) — the same expression/statement split #14 arrives at independently. The
    asymmetry now needs no defending; it is what the names have always meant.

## Why not unify register-maps and windows through `CPtr`

`=` cannot mean both "set the pointer" and "write through the pointer":

```basic
uart.data = dataView    ' set-the-pointer  (RHS is a view)
r.control = 0           ' write-through-the-pointer (RHS is a scalar) — intended
```

Overloading `=` by RHS type is ambiguous and fragile. Splitting into (a) field-access
overlays and (b) indexed `MEM` views removes the ambiguity: in (a) the field *is* the
register so `=` is always a register write; in (b) all access is explicit `PEEK`/`POKE`.

## Sketch (current synthesized shape)

### (a) Register overlay — fields are registers; access = volatile I/O

A fixed register map. Width-typed fields; **field access *is* the volatile register
I/O** — `r.status` is one volatile read of the field width, `r.control = 0` one
volatile write. The struct's field offsets *are* the register map. No `MEM`, no
`PEEK`/`POKE`.

```basic
MEMORY
    LAYOUT UartRegs
        data    AS CUInt32
        status  AS CUInt32
        control AS CUInt16          ' trailing pad to 4-byte struct alignment is
    END LAYOUT                       ' inserted automatically (#11) — no explicit _pad

    FUNC openUart(base AS Integer) AS UartRegs CLOSE BY closeUart
        OVERLAY UartRegs AT base    ' lay the struct over the address
    END FUNC

    FUNC getUartStatus(RES r AS UartRegs) AS Boolean
        RETURN bits::band(r.status, 1) = 1   ' r.status = one volatile 32-bit read
    END FUNC

    FUNC closeUart(RES r AS UartRegs) AS Nothing
        r.control = 0                        ' one volatile 16-bit write
    END FUNC
END MEMORY
```

### (b) Dynamic window — indexed bounded I/O

A runtime-sized byte region. A fat-pointer **view** (`MEM` = pointer **and** length)
with indexed, bounds-checked access via `PEEK`/`POKE … AT off FOR n` (`FOR n` is
mandatory, never inferred — #15).

```basic
MEMORY
    LAYOUT Framebuffer
        mem AS MEM                  ' a view (ptr + len), NOT a bare CPtr
    END LAYOUT

    FUNC openFb(addr AS Integer, size AS Integer) AS Framebuffer CLOSE BY Nothing
        OVERLAY Framebuffer WITH mem = VIEW AT addr FOR size
    END FUNC

    SUB setPixel(RES r AS Framebuffer, x AS Integer, y AS Integer, val AS Byte)
        POKE r.mem AT y * width + x FOR 1 = val   ' FOR required; 1-byte write, bounds-checked
    END SUB
END MEMORY
```

### (c) Mixed register + buffer in ONE overlay

The split in decision #3 is about *access modes*, not about *resources*. A single
opaque resource can hold **both**: width-typed **register fields** (field access =
volatile I/O) *and* one or more **`MEM` buffer fields** (indexed bounded `PEEK`/`POKE`).
Field type decides the mode — a fixed-width C int is a register; a `MEM` is a buffer
view. This is the common real shape: a device with a control/status register block
*plus* a data buffer (framebuffer, DMA ring, FIFO payload).

```basic
MEMORY
    ' (c) mixed overlay — registers AND an embedded data buffer, one resource
    LAYOUT Display
        ' --- registers: field access = volatile I/O ---
        width   AS CUInt32          ' @ 0x00
        height  AS CUInt32          ' @ 0x04
        control AS CUInt32          ' @ 0x08
        bufPhys AS CUInt64          ' @ 0x0C  device's DMA base for the pixel buffer
        ' --- buffer: indexed bounded PEEK/POKE ---
        pixels  AS MEM              ' a view (ptr+len); registers are auto-laid-out,
                                    ' but a MEM field is wired explicitly (see below)
    END LAYOUT
```

Two ways the buffer gets wired — both keep registers and buffer in the same resource:

**Static / contiguous** — the buffer sits at a fixed offset in the *same* mapping as
the registers:

```basic
    FUNC openDisplay(base AS Integer) AS Display CLOSE BY blankDisplay
        OVERLAY Display AT base
            WITH pixels = VIEW AT base + 0x1000 FOR 1920 * 1080 * 4
    END FUNC
```

**Dynamic / register-described** — the registers *tell you* where and how big the
buffer is. Bind the overlay (registers live), read them, then form the view. This is
the compelling "mix": the device self-describes its DMA region.

```basic
    FUNC attachDisplay(base AS Integer) AS Display CLOSE BY blankDisplay
        RES r AS Display = OVERLAY Display AT base    ' registers usable now; pixels unset
        ' Read registers DIRECTLY in the VIEW expression — do not bind them into
        ' LET/MUT first: a LET/MUT materializes an owned heap value, which moves the
        ' read off the device. Field access stays an in-place volatile register read.
        r.pixels = VIEW AT r.bufPhys FOR r.width * r.height * 4
        RETURN r
    END FUNC
```

Both access modes then coexist in the wrappers — buffer writes *and* register writes
through the one resource:

```basic
    SUB setPixel(RES r AS Display, x AS Integer, y AS Integer, argb AS Integer)
        POKE r.pixels AT (y * r.width + x) * 4 FOR 4 = argb   ' bounds-checked buffer write
    END SUB

    FUNC getPixel(RES r AS Display, x AS Integer, y AS Integer) AS Integer
      RETURN PEEK r.pixels AT (y * r.width + x) * 4 FOR 4   ' bounds-checked ARGB read
    END FUNC

    SUB present(RES r AS Display)
        FENCE                                                ' order pixel writes first (#24)
        r.control = bits::bor(r.control, 1)                  ' now the poke sees a full buffer
    END SUB

    FUNC blankDisplay(RES r AS Display) AS Nothing
        r.control = 0                                        ' register write on drop
    END FUNC
END MEMORY
```

Notes specific to the mix:

- **Auto-layout vs explicit wiring.** Register fields are laid out at fixed offsets
  from the overlay base (defined layout, little-endian, explicit padding). A `MEM`
  field is *not* auto-placed — its base/len may be static-contiguous, register-derived,
  or a separately-mapped region, so the producer wires it (`WITH …` or field
  assignment). Possible sugar: allow `pixels AS MEM AT 0x1000 FOR <const>` inline for
  the pure-contiguous case, desugaring to the `WITH` form.
- **The view is a snapshot of base+len at construction.** In the dynamic form the
  length came from `width`/`height` reads at attach time; if the device later changes
  geometry, the cached view length is stale. Re-attach if geometry changes — the
  resource does not track the registers live.
- **`bufPhys` is a register holding an address.** Reading it yields an `Integer`
  (a `CUInt64` full-width bit-copy per #9 — treated as a raw address pattern, so
  signedness is irrelevant), which is fed to `VIEW`; only the `FOR` length is
  bounds-enforced, the address itself is the programmer's responsibility (same trust
  posture as the rest of the `--mem`-gated block).
- **Bounds + volatile both still hold:** `MEM` field access is bounds-checked against
  the view length; register field access is one volatile bus access of the field width.
- **Binding a register read into `LET`/`MUT` copies it to the heap — a snapshot, not
  the live register.** `LET current = r.control` performs one volatile read and copies
  the value into an owned MFB heap value; from then on `current` is decoupled from the
  device and never re-reads. That's exactly right for read-modify-write (you *want* a
  stable snapshot). It's wrong when you need the *live* value later — there, access the
  field **directly** at the point of use (`r.width`, `r.bufPhys`, … inside the `VIEW`
  expression, a comparison, an arg) so each use is a fresh volatile read. Not a
  prohibition — just know which one you're getting.

This means decision #3 stays — they're still two *access modes* — but they are not two
*resources*; a single overlay can carry both. (a) and (b) are just the degenerate cases
where a `Display`-like type has only registers, or only a `MEM` field.

## `PEEK` / `POKE` semantics

`PEEK <memField> AT <off> FOR <n>` reads `n` raw bytes from the view at `off` and
produces an MFBASIC **primitive**.

- **Width check + low-`n`-bytes placement.** `n` must fit the destination primitive:
  `n ≤` the primitive's byte size (e.g. `FOR 4` into an `Integer`/8 bytes ✓, `FOR 8` ✓).
  The `n` source bytes land in the low `n` bytes; for `n < 8` the high `8 − n` bytes are
  zero (non-negative result), and `n = 8` is an exact bit-copy that can read negative —
  see #9. Reading more bytes than the destination holds is a compile error (static when
  `FOR` is constant).
- **Primitives only — `PEEK` cannot build a Collection.** No `List`/`Map`/record/bulk
  form. To fill a `List OF Byte` / `List OF Integer` you `PEEK` each element and
  `append()` it into the list. There is no "read N bytes straight into a List."
- **Bounds-checked** against the view length, overflow-free (#22): in-bounds iff
  `0 ≤ off` and `1 ≤ n` and `n ≤ len` and `off ≤ len − n`; otherwise `ErrBounds`. (Not
  `off + n > len`, which can wrap.)

`POKE <memField> AT <off> FOR <n> = <value>` writes the low `n` bytes of `value` to the
view at `off` (same width + bounds rules in reverse).

- **`FOR n` is REQUIRED on `POKE`** — mirroring `PEEK`. Do **not** infer width from the
  value's type. MFB literals are initially untyped, so `POKE r.mem AT off = 0` would be
  ambiguously 1/2/4/8 bytes depending on context — exactly the ambiguity a low-level
  block can't tolerate. Forcing `POKE r.mem AT off FOR 2 = 0` makes the developer state
  the hardware's register width explicitly. Omitting `FOR` is a compile error.

Example — read a row into a `List`, element by element (because `PEEK` can't build the
list itself):

```basic
FUNC readRow(RES r AS Display, y AS Integer) AS List OF Integer
    MUT row AS List OF Integer = []
    MUT x AS Integer = 0
    WHILE x < r.width                         ' r.width re-read each iter (live register)
        ' PEEK makes a primitive; append it — PEEK cannot make the List
        collections::append(row, PEEK r.pixels AT (y * r.width + x) * 4 FOR 4)
        x = x + 1
    END WHILE
    RETURN row
END FUNC
```

## Gating — a coarse `--mem` switch

No fine-grained capability system. Touching raw memory is a single, blunt
build-time switch:

- A source file that contains a `MEMORY` block (direct-memory) only compiles under
  `--mem`. Without it, the build **fails at compile time** with a diagnostic at the
  block, telling the user to pass `--mem`.
- It propagates transitively: importing any package that uses the feature requires
  `--mem` on the build too. A `.mfp` carries a one-bit "uses raw memory" flag; `mfb
  build` fails if any imported package has it set and `--mem` was not passed.
- That's the whole model — on or off. No per-capability grants, no manifest cap set,
  no `mem`-vs-`io` split. (If a separate device-transport/`io` plane ever lands, it
  can get its own coarse switch then.)

## Volatile semantics — the implementation crux

**Everything in a `MEMORY` block is volatile by definition — no annotation.** Volatility
is a property of the block (every `LAYOUT` field, `PEEK`, and `POKE` access), not an opt-in
on individual `FUNC`/`SUB`s. There is no `VOLATILE` marker to write; being inside a
`MEMORY` block *is* the marker. So every `r.status` / `PEEK` / `POKE` is **exactly one bus
access of the declared width, in program order**, and the compiler must exempt these
access nodes from:

- store-to-load forwarding (plan-01 float),
- CSE / common-subexpression folding (two `r.status` reads must not collapse),
- dead-store elimination,
- reordering / coalescing,
- regalloc value caching.

Read-only does **not** make a view a constant — a status register read-to-clears or
changes between reads. This is more work than the syntax and is the real risk.

**Volatile (ours) vs non-cacheable (the OS's) — both needed, only one is our guarantee.**
Volatile is the *compiler* contract above: one instruction per access, in order, never
optimized away. **Non-cacheable** is a *mapping* property — that the load/store actually
reaches the device instead of hitting the data cache or sitting in a write buffer — and
that is set by the **OS / page tables**, not by MFB. We can mention it, but we **cannot
guarantee** it from the compiler. The one exception: if/when MFB performs the mapping
itself (the region-relative addressing path), it could *request* a non-cacheable / device
memory type from the OS at `mmap` time (e.g. the right `mmap`/`O_*`/PROT flags or a UIO
attribute) — i.e. MFB can *ask* the OS, not enforce it. Until then, mapping the region
uncached is the caller's/OS's responsibility. (Ordering across the MMIO↔normal-memory
boundary — e.g. fill a DMA buffer, then poke "go" — needs an explicit barrier, spelled
`FENCE` per #24.)

**Why the exemption is cheap.** Because the feature is behind `--mem`, the backend can
apply these "pessimistic" no-fold/no-reorder rules **only to `MEMORY`-block access
nodes** — the rest of the application keeps full store-to-load forwarding, CSE, etc.
The exemption is scoped to exactly the nodes that need it, so it costs nothing
elsewhere.

Demonstration — a read-modify-write bit-bash through the overlay, where every line is a
distinct, un-foldable bus access:

```basic
' A helper FUNC inside the MEMORY block — bit-bash sequence through the overlay
FUNC resetDevice(RES r AS UartRegs) AS Nothing
  LET current = r.control               ' one volatile read of control
  LET next    = bits::bor(current, 128) ' flip the reset bit (bit 7)
  r.control = next                      ' one volatile write
  r.control = current                   ' one volatile write (restore) — NOT dead-store-eliminated
  RETURN NOTHING
END FUNC
```

The two `r.control = …` writes look like a dead store followed by a live one to an
ordinary optimizer; the volatile exemption is what keeps **both** writes on the bus, in
order.

Note `LET current = r.control` here copies the register value out of device memory into
an owned heap value — a **snapshot**. That is correct for read-modify-write: you read
once, compute, and write back, and the saved `current` is precisely the pre-modify value
you want to restore. The volatile guarantee makes that initial `LET` exactly one bus
read; the snapshot is then decoupled from the live register (see the mix-notes rule on
`LET`/`MUT` binding). (`r.control` is `CUInt16`, so the field write is already 2 bytes —
no `toByte` narrowing needed; write `next` directly.)

## How these differ from §15 resources (spell out as a subclass)

The overlay/window types are a resource subclass with these deltas from §15:

1. **Field access / indexing permitted inside the declaring block** (lifts the §15
   "never field-accessed" rule — but only inside, like `CPtr` inside `LINK` ABI).
2. **Backed by raw memory, not an arena allocation** (the descriptor is allocated;
   the target memory is not).
3. Otherwise unchanged: bound with `RES`, borrowed at calls, auto-closed by lexical
   drop, never copied/stored-in-collection, thread-sendable only with
   `THREAD_SENDABLE`.

(Note: producer-FUNC + `CLOSE BY` brings these *back* in line with normal resource
lifetime — the only standing deltas are #1 and #2.)

In the **static bare-metal case** (`AT 0x…` + `CLOSE BY Nothing`) the resource machinery
does no lifetime work — drop is a no-op. There it buys the trust boundary and scope-bound
opacity, **not** RAII. Only the region-relative path (real `mmap`/unmap) exercises `CLOSE
BY` for actual teardown.

## Open questions / TODO

- [ ] **Static-absolute vs region-relative addressing.** Fixed `AT 0x...` with
      `CLOSE BY Nothing` = bare-metal MMIO. Host `mmap`/`/dev/mem`/UIO gives a *runtime*
      base and *must* be unmapped → real producer + non-`Nothing` close owning the
      unmap, reusing the same overlay/view types over a `Region`. Keep the door open
      for both.
- [ ] **Write-only / read-to-clear registers** can't be expressed by field
      read/write (you can always read a field). Accepted limitation — read-to-clear is
      worked through in "Read-to-clear registers — a limit of the model" below; write-only
      still just a noted gap.

## Reference points in the codebase

- `LINK` design & resource model: `src/docs/spec/language/17_native-libraries.md`,
  `src/docs/spec/language/15_resource-management.md`.
- C ABI type names + marshaling: `src/target/shared/code/link_thunk.rs`,
  `is_c_abi_type` allow-list in `src/typecheck.rs`.
- Resource resolution (`CLOSE BY`, opaque types, `NATIVE_CPTR_ESCAPE`):
  `src/resolver/resolution.rs`.
- Volatile exemptions will touch: store-to-load forwarding (plan-01 float), peephole
  (`src/target/shared/code/peephole.rs`), register allocator (plan-03).

## Read-to-clear registers — a limit of the model

Some hardware registers have a **side effect on read**: the act of reading them mutates
device state. The classic case is a status register whose error/event bits *clear when
you read them*. The volatile guarantee gets us close but **cannot model this perfectly**;
it needs a hand-written convention.

**Concrete example — the 16550 UART Line Status Register (LSR).** Reading `lsr` returns
the current line state *and clears* the error/break bits (overrun, parity, framing,
break) as a side effect:

```text
LSR bit 0  Data Ready
    bit 1  Overrun Error    \
    bit 2  Parity Error      |  these clear on read
    bit 3  Framing Error     |
    bit 4  Break Interrupt  /
    bit 5  THR Empty
    bit 6  Transmitter Empty
```

**Why the model can't express it.** Volatile guarantees *one bus read per source-level
read* — but it does **not** guarantee *read-once* or read-linearity. Field-access syntax
makes `r.lsr` look like an ordinary, freely-repeatable read, so nothing stops a caller
from doing:

```basic
IF bits::band(r.lsr, 2) <> 0 THEN ...   ' read #1 — clears the error bits
IF bits::band(r.lsr, 4) <> 0 THEN ...   ' read #2 — Parity bit is already GONE
```

Both reads are individually correct (one bus access each), yet the program is wrong: the
second read can never see a bit the first read cleared. The type system can't say "this
field must be read exactly once and its result fully consumed," so the hazard is real and
silent.

**The mitigation — a read-once snapshot wrapper.** Read the register **exactly once**
into a local (a `LET` snapshot is correct here — it's a copy off the device, decision on
`LET`/`MUT` binding), then decode every bit from the snapshot. Callers consume the
decoded record and never touch the live RC register:

```basic
' Ordinary record, declared OUTSIDE the MEMORY block, referenced inside (decision #16)
TYPE LineStatus
    dataReady AS Boolean
    overrun   AS Boolean
    parity    AS Boolean
    framing   AS Boolean
    txEmpty   AS Boolean
END TYPE

MEMORY
    ' ... LAYOUT UartRegs with `lsr AS CUInt8` ...

    ' The ONLY sanctioned way to read LSR: one bus read, all bits decoded together.
    FUNC readLineStatus(RES r AS UartRegs) AS LineStatus
        LET s = r.lsr                       ' <-- the single read-to-clear access
        RETURN LineStatus[
            bits::band(s, 1)  <> 0,
            bits::band(s, 2)  <> 0,
            bits::band(s, 4)  <> 0,
            bits::band(s, 8)  <> 0,
            bits::band(s, 32) <> 0
        ]
    END FUNC
END MEMORY
```

**Net:** read-to-clear (and the related write-1-to-clear) registers are an **accepted
limitation** — the block can't enforce read-once, only single-access-per-read. The
discipline is a per-register wrapper that snapshots once and exposes the decoded view; the
raw RC field should never be read directly by callers. Worth a documented convention
(naming, "do not read this field directly") rather than a language guarantee. A future
idea, if it ever matters: a field annotation like `READCLEAR` that makes the compiler
reject more than one direct read on a path — but that's linear-typing-flavored and out of
scope here.
