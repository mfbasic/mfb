# Musings: `-unsafe` inline ASM

> Not a plan. Not scheduled work. This is a snapshot of a design idea as it
> currently stands, captured so it isn't lost. Nothing here is committed to.

## Verdict (2026-06-29): rejected — no unique value over a stdlib builtin

After working the idea through, the conclusion is **don't build this.** The
reasoning, kept so the question isn't reopened:

There are only two coherent ASM designs, and they buy different things:

- **A generic assembler** (opaque text → an assembler → bytes) buys the real
  thing: *any* instruction, exact bytes, device/sysreg/barrier access. This is
  the version with unique value. mfb has no assembler, and issue 2 rejects this
  route (building per-ISA assemblers is huge and breaks the allocator/inspection
  story).
- **Parse into the modeled IR** (the design this doc describes) is safe,
  inspectable, and allocator-aware — but can only emit instructions the backend
  *already encodes*.

This doc picks the second, and that choice defines its value away:

1. Any modeled op **worth exposing should be a stdlib builtin**, not inline ASM.
   It lowers to the same MIR op but is typed, safe, documented, needs no
   `-unsafe`, and gets ISA portability from the MIR for free. The builtin layer
   (`bits::clz`/`ctz`/`popCount`/`rl64`/`rr64`/`sra`, the `vector::` package, …)
   *is* the portable machine-op surface already, and inline-lowers to single MIR
   ops today. A "MIR-level ASM FUNC" is just a more dangerous spelling of calling
   one of those.
2. The one thing hand-asm uniquely justifies — reaching an instruction the
   backend **doesn't** model (the exotic op, a device register, a barrier) — is
   exactly what "not a generic assembler" forbids by construction.

What's left after those two is only *exact-sequence control over the
already-modeled, already-builtin-able op set*: fragile (the win shifts as the
selector improves — and selector improvements help everyone), tiny audience
(constant-time crypto, a few hot loops), and not even fully delivered (a
virtualized body can still spill; true pinning needs the separate fixed-register
clobber-region escape). That residue does not justify the feature's size — the
parser, effects model (issue 4), flags model (issue 5), unsafe propagation
through package metadata/caching, multi-arch fallback + fallibility semantics
(issue 7) — all of the open issues below.

The kicker is this doc's own resolution: issue 2 says reaching an un-modeled
instruction is solved by *adding the encoder to the backend*, not by smuggling
bytes through ASM. But once an op + encoder exist in the backend, you expose it
as a builtin — usable everywhere, inspectable, portable. Every real need
therefore terminates at **"add a modeled MIR op, surface it as a builtin"**; ASM
is never the last step.

Reopen only if a genuine **generic assembler** ever lands on the roadmap — a
different, far larger feature than anything described below. The material below
is preserved as the record of the design that was explored and set aside.

## The idea

An `-unsafe` compiler flag that allows inline assembly via an `ASM` construct.
Without `-unsafe`, any `ASM` block is a compile error; the flag is the explicit
opt-in to hand-written machine code.

**ASM is contagious at the package boundary.** Any package containing an `ASM`
block is itself flagged `unsafe`, and that flag propagates: a program (or
package) that imports an unsafe package can only be compiled with `-unsafe`.
You cannot unknowingly pull hand-written assembly into a "safe" build through a
dependency — the requirement to pass `-unsafe` surfaces all the way up to
whoever runs the final compile. This makes "does my binary contain inline asm?"
answerable from the build command alone.

(`-unsafe` gates the `ASM` construct. A separate `-baremetal` concern — no
runtime / custom entry — is orthogonal; see issue 9.)

Functions inside an `ASM` block define their own calling convention *as their
signature*: the parameters and return are named virtual registers, typed.

```
ASM
  FUNC add(x1 AS Integer, x2 AS Integer) x0 AS Integer
    ARCH aarch64
        add x0, x1, x2
    END ARCH
    ARCH x86_64
        mov x0, x1
        add x0, x2
    END ARCH
  END FUNC

  FUNC fadd(d1 AS Float, d2 AS Float) d0 AS Float
    ARCH aarch64
        fadd d0, d1, d2
    END ARCH
    ARCH x86_64
        movsd d0, d1
        addsd d0, d2
    END ARCH
  END FUNC

  FUNC recordThing(x1 AS SomeRecord) d0 AS Float
    ' x1 holds the memory pointer to SomeRecord (pointer => GP register).
    ' Load the Float field at offset 8 (offset illustrative -- see below).
    ARCH aarch64
        ldr d0, [x1, #8]
    END ARCH
    ARCH x86_64
        movsd d0, [x1 + 8]
    END ARCH
  END FUNC
END ASM
```

Field offsets like the `8` above are not invented: the byte layout behind every
heap value is a documented contract. An asm author dereferencing a record,
list, map, string, etc. looks up the real offsets in `mfb spec memory` —
`heap-values` for record/string/union object bodies, `collections` for the
uniform List/Map layout, `scalar-storage` for payload sizes. (This is also a
reason the bodies stay `unsafe`: they hard-code a layout the compiler owns, so
they must be revisited if that contract ever changes.)

**Symbolic offsets, where the compiler already knows the answer.** A literal `8`
is honest but fragile: it makes a human hand-sync a magic number with a layout
the compiler owns, and that layout can vary by target, packing, scalar size, or
a future ABI version. For *user* records the compiler knows the field's offset,
so it should expose it symbolically rather than make the author transcribe it:

```
OFFSETOF SomeRecord.value     ' byte offset of a field
SIZEOF   SomeRecord           ' total size
ALIGNOF  SomeRecord           ' alignment
```

so the record example reads:

```
ldr d0, [x1, #OFFSETOF(SomeRecord.value)]    ' exact spelling TBD
```

This stays `unsafe` — the body still depends on the layout's existence and shape
— but the value is filled in by the compiler, not copied by hand. Runtime-owned
layouts (the heap-value/collection internals above) may still need raw
`mfb spec memory` offsets where no source-level symbol exists; symbolic offsets
are the strongly-preferred form wherever a symbol *does* exist.

A package that contains any of the above is flagged `unsafe`, so the flag has to
be present at every build that pulls it in:

```
# 'fastmath' contains an ASM block -> it is itself unsafe
mfb build fastmath                       # error: ASM requires -unsafe
mfb build -unsafe fastmath               # ok

# a program that IMPORTs fastmath inherits the requirement transitively
mfb build app                            # error: depends on unsafe package 'fastmath'; rebuild with -unsafe
mfb build -unsafe app                    # ok
```

**Preserving the "answerable from the build command alone" property** takes a few
implementation commitments, worth calling out so they aren't lost:

- Unsafe-ness is recorded in package metadata / build artifacts, not just
  inferred from source at the top-level build.
- A cached compiled package must not be able to hide that it is unsafe; the bit
  travels with the artifact.
- Linking a prebuilt unsafe dependency into a safe build must *fail*, the same as
  importing unsafe source would.
- An `ARCH MFB` fallback does **not** make the package safe (see issue 7): the
  package still *contains* assembly some target can select.

## Register model

- `x0, x1, x2, ...` — virtual general-purpose (integer/pointer) registers.
- `d0, d1, d2, ...` — virtual 64-bit FP registers.
- **All registers are virtual.** The compiler "figures it out" — it allocates
  real registers and substitutes them in; the names in the body are not
  architectural registers, they are operands.
- **Type determines register class.** Integer / pointer / record / string →
  `x`. Float → `d`. A record is passed by pointer, so it goes in an `x`
  register (e.g. `x1 AS SomeRecord`), never a `d`.

**Naming hazard (unresolved).** On aarch64, `x0`/`d0` are *also* real register
names, so a reader who knows the ISA will keep misreading the virtual `x0` as
physical `x0`. Two ways out:

- A distinct spelling that can never be confused with a real register —
  `vx0`/`vd0`, or `%x0`/`%d0`.
- Keep the bare `x0`/`d0` spelling (it is undeniably clean) and repeat the
  warning near every major example: *in ASM signatures and bodies, `x0` means a
  virtual integer operand, not physical AArch64 `x0`.*

This musing keeps `x0`/`d0` for readability; a real plan should pick one and
commit, because this is a predictable recurring source of user mistakes.

## SCRATCH list

```
SCRATCH x3, x4
```

`SCRATCH` means **"give me scratch virtuals for the body"** — virtuals that are
dead on entry and exit. The allocator hands you whatever is free and reclaims
them after. Because registers are virtual, there is no notion of "save the real
register I destroyed"; the name names the actual contract — N throwaway
virtuals — and nothing more.

```
ASM
  ' (a*b) + (a+b), needing one temporary that doesn't outlive the body
  FUNC blend(x1 AS Integer, x2 AS Integer) x0 AS Integer
    SCRATCH x3
    ARCH aarch64
        mul x3, x1, x2      ' x3 = a*b   (scratch; dead after this body)
        add x0, x1, x2      ' x0 = a+b
        add x0, x0, x3      ' x0 = a*b + a+b
    END ARCH
  END FUNC
END ASM
```

`x3` here is whatever physical register the allocator had free; nothing is
saved or restored on its account.

Two clarifications:

- **`SCRATCH` introduces a local temporary, not a global register number.**
  `SCRATCH x3` does not request "virtual number 3" anywhere; it declares a
  body-local dead temp that happens to be *named* `x3` for use in the body. The
  name is scoped to the function, like a `LET` binding.
- **Scratch is typed by class, exactly like operands.** `SCRATCH x3, x4` asks for
  integer/pointer virtuals; FP scratch is `SCRATCH d3`. If future register
  classes appear (vector, flags, predicate, capability), `SCRATCH` follows the
  same class model — the prefix names the class, the allocator hands you a dead
  virtual of that class.

## Relation to plan-03 (the register allocator) — the substrate now exists

This musing was written as if its whole register model had to be invented. It
mostly doesn't: `plan-03-register-allocator` is actively building exactly this
substrate, and **Stage A has already shipped** (commit 72748022). What that
means for this idea:

- **"All registers are virtual" is no longer hypothetical.** plan-03 added a
  `%vN` virtual-register layer (`src/target/shared/code/regalloc/`), a final
  rewrite pass that substitutes physicals, an `AllocationStrategy` trait, and a
  per-ISA `RegisterModel` (`src/arch/aarch64/regmodel.rs`). The "compiler figures
  it out" sentence in the Register model section describes infrastructure that
  now exists rather than a wish.
- **Two register classes already match.** plan-03's `RegClass ∈ {Int, Fp}` with
  `d_n ⊂ v_n` aliasing is precisely this musing's `x` vs `d` split. "Type
  determines register class" maps straight onto it.
- **The clobber model is real.** plan-03 §4.3 encodes caller-saved/helper
  clobber sets as clobber operands on call instructions. That is the machinery
  `SCRATCH` rides on: allocate fresh vregs that are dead on exit, and let the
  allocator reclaim them.

The load-bearing consequence: the open issues below should be read against a
backend that already has virtuals, an allocator interface, and a per-ISA
register description — not against today's bump-and-reset model the musing
originally assumed.

## Result and error handling: infallible inline fragments

This is the part that makes ASM functions "work like any other function," and it
turns out the language already has the exact category they belong in — but the
naive example above hides a collision that has to be resolved first.

**The conceptual category, stated up front.** Two claims have to be kept
separate or they read as a contradiction:

- **Native `ARCH` bodies are infallible machine fragments.** A spliced sequence
  of encoded instructions has no error envelope and cannot fail in the
  language's sense.
- **The declared `ASM FUNC` is infallible *unless* an `ARCH MFB` fallback makes
  the declaration fallible.** Fallibility is a property of the declared function,
  fixed across all targets (see issue 7) — never of the body the build happens to
  select.

For a first cut the cleanest rule is the stricter one: **`ARCH MFB` fallbacks
must themselves be infallible**, so the declared function is infallible
everywhere and the whole four-register-envelope question never arises. If you
need fallibility, use the safe wrapper pattern below. Fallible `ARCH MFB`
(and the ABI-shaping it forces) is a deferred extension, written up in issue 7
but explicitly out of the v1 scope.

### The collision the `x0 AS Integer` example hides

Every MFBASIC function is implicitly fallible: `FUNC f(...) AS T` desugars to
`FUNC f(...) AS Result OF T`, and at the machine level a **fallible call returns
a four-register tagged outcome**, not a bare value
(`mfb spec memory fallible-call-abi`):

```
x0  tag      0 = success, 1 = error, 2 = program exit
x1  value    success: the result value;  error: the Error code
x2  message  error: pointer to the message string
x3  source   error: pointer to the origin ErrorLoc
```

So in a normal function **`x0` is the tag, and the value lives in `x1`.** The
musing's `FUNC add(...) x0 AS Integer` with `add x0, x1, x2` reads as "result in
x0" — which is only true for an **infallible** callable. Per
`mfb spec memory native-calling-convention`: *"An infallible callable returns
its single value in x0; a fallible callable returns the four-register
{tag,value,message,source} outcome in x0..x3."* The two ABIs are different, and
the asm author must not be writing into a register whose meaning depends on
which ABI the function uses.

### Resolution: an `ASM FUNC` is an infallible *inline fragment*

The clean answer — and the one that makes them first-class rather than special —
is that **an `ASM FUNC` is an inline-only, infallible function-like
declaration**, the same category inline-lowered built-ins already use. The
signature describes *typed operands*, not a runtime ABI; resolution happens by
inlining (issue 8), not by a `bl` to a callable. With that primary model fixed,
the error story falls out:

- **The named result is the *value*, not the tag — and there is no physical
  return register involved.** At the language boundary the declaration has the
  same effect as an infallible callable returning a value. But because the body
  is *inlined*, no physical return register exists: `x0 AS Integer` names a
  *value operand* that is simply unified with the caller's destination virtual,
  and the four-register {tag,value,message,source} envelope is never
  materialized. (Were materialization as a *real* callable ever added — a
  separate future feature, see issue 8 — it would use the existing infallible
  native calling convention, where the value does land in physical `x0`. That is
  the only context in which talking about "physical `x0`" is meaningful, and it
  is out of scope here.)
- **Callers emit no per-call error check.** The check
  (`compare x0, OK_TAG; b.eq ok; <route error>`) is gated on the call being
  fallible — `emit_call` only emits it when `return_type.is_none()`
  (`builder_emit_helpers.rs:147`). An inlined infallible fragment produces its
  value directly with zero error-handling overhead, and auto-unwrap is a no-op.
  It type-checks and composes at every call site exactly like an inline-lowered
  built-in.
- **No `FAIL`, no `TRAP`, and no inline `TRAP` on the call.** A raw asm body has
  no way to call `error(...)` and must not hand-assemble the four-register error
  envelope (doing so would reintroduce exactly the real-register/internal-ABI
  thinking the whole design avoids, and couple the body to an ABI the spec
  explicitly owns and may change). This mirrors **error-model rule 14**:
  inline-lowered built-ins (`strings::find`, `bits::*`, `len`, …) cannot take an
  inline `TRAP` because they are spliced machine code that owns no callable
  error envelope. An `ASM FUNC` is precisely a *user-defined* member of that
  same family — spliced, infallible, envelope-less — so it inherits the same
  rule for the same reason. That is the strongest sign this is the right model:
  it needs no new compiler concept.

### When you genuinely need a fallible primitive: wrap it

The asm body stays infallible; fallibility is added by a thin **normal** `FUNC`
that inspects the returned value and `FAIL`s. All error machinery stays in safe
MFBASIC; the asm body stays pure. This is also how real low-level code is
structured — a raw operation returns a sentinel/`-errno`, and a safe wrapper
turns it into an `Error`:

```
ASM
  FUNC rawDiv(x1 AS Integer, x2 AS Integer) x0 AS Integer
    ' infallible: returns quotient, or a sentinel the wrapper checks
  END FUNC
END ASM

FUNC divide(a AS Integer, b AS Integer) AS Integer
  IF b = 0 THEN FAIL error(ErrInvalidArgument, "divide by zero")
  RETURN rawDiv(a, b)        ' infallible asm primitive; error stays in safe code
END FUNC
```

The wrapper is an ordinary fallible function — four-register ABI, normal
`TRAP`/propagation, normal scope-drop on the error path — and the asm primitive
underneath it is none of those things. The two compose with zero special cases,
which is the precise sense in which ASM functions "work just like any other
function": they *are* just infallible callables, a category that already exists
and already composes with the fallible world through the value a caller
inspects.

## Verifying ASM functions: the existing dump flags

A developer writing inline asm needs to *see* what the compiler did with it,
and the toolchain already has the inspection surface for free. `mfb build`
takes a set of mutually-exclusive output-mode flags
(`src/cli/build.rs`) that each dump one stage of the lowering pipeline instead
of producing a binary:

| Flag | Stage | Use for ASM verification |
|------|-------|--------------------------|
| `-ast` | parsed AST | confirm the `ASM`/`ARCH`/signature parsed as intended |
| `-ir` | mid-level IR | confirm the front end lowered the call/inline correctly |
| `-br` | binary representation | the serialized program form |
| `-nir` | native IR | the per-instruction stream **before** register allocation — see your `x1`/`d0` operands as virtuals |
| `-nplan` | native plan | instruction selection / layout pre-encode |
| `-nobj` | native object plan | relocations, symbols, section layout |
| `-ncode` | native code | the **final encoded instructions** — the single most useful flag: it shows exactly which physical registers the allocator assigned to your ASM body and the bytes that were emitted |

The two that matter most for ASM work are **`-nir`** (your virtual operands
before coloring — did `x1`/`d0` land in the right class, did `SCRATCH` get its
virtuals?) and **`-ncode`** (what they became after allocation — did `mul x2`
encode against `rax:rdx`, did a loop-carried value stay in a callee-saved
register?). Diffing `-nir` against `-ncode` is the natural way to confirm the
allocator did what the asm author expected — and `-regalloc bump` vs the
default gives a second axis for differential debugging (plan-03 keeps `bump` as
a permanent oracle). No new tooling is needed to make `ASM` verifiable; it
inherits this pipeline visibility.

**One diagnostic worth adding for dependency hygiene** (not core, but cheap and
valuable): make the unsafe-propagation failure *show the chain* rather than just
naming the leaf package, and/or offer an explicit report:

```
error: depends on unsafe package 'fastmath'
  app imports imageproc
  imageproc imports fastmath
  fastmath contains ASM FUNC 'mulHigh'
```

```
mfb build -unsafe-report app    # print the unsafe-dependency tree without building
```

The "answerable from the build command alone" property is much more useful when
the build also tells you *why* a package is unsafe and *who* pulled it in.

## Open issues / known tensions

These are the unresolved parts. They are why this stays a musing.

### 1. Fixed-register instructions are an encoder constraint, not a body directive

Some x86 instructions hard-wire a real register: `mul`/`div` (implicit
`rax:rdx`), variable shift count in `cl`, string ops (`rsi`/`rdi`). The question
is *where that knowledge lives*. It must not be something the programmer writes
in the asm body: any body-level "put this virtual in `rax`" directive drags
real-register thinking back into a system designed to avoid it, and reinvents
GCC's constraint letters at the source level.

A fixed-register requirement is a property of the *instruction*, so it lives as
a constraint on the virtual in the IR/encoder — the one place that knowledge
already belongs. The encoder for `mul` already knows it reads/writes `rax:rdx`.
The body just says `mul x2`; the compiler knows the operand and result land in
the virtuals that map to those fixed registers, and the allocator satisfies the
constraint when assigning them. The programmer never names a real register, the
asm body stays clean, and the ugly part stays encapsulated in the encoder where
it's unavoidable anyway.

So this isn't really an open tension: aarch64 is mostly orthogonal and needs
none of this, and the x86 fixed-register cases are finite, each belonging to a
specific encoder.

**plan-03 already commits to exactly this mechanism.** Its §5 (target
portability) lists the same x86 cases — "`idiv` pins `rax`/`rdx`, variable
shifts pin `cl`" — and resolves them as *"pinned-operand + clobber
constraints"* in the abstract instruction view, plus *pre-colored vregs* for
ABI-pinned registers (§4.1). That is the encoder-level constraint this section
argues for, already chosen for the allocator's own needs (calls, scratch). So
inline asm gets the fixed-register story for free: `mul x2` lowers to an
instruction whose operands carry the `rax:rdx` constraints the RegisterModel
already has to express — no body directive, no new concept.

The contrast is visible in a single function — the high half of a 64×64
multiply. x86 hides `rdx:rax` in the encoder; aarch64 has a dedicated op and
needs no pinning at all, so neither body names a real register:

```
ASM
  FUNC mulHigh(x1 AS Integer, x2 AS Integer) x0 AS Integer
    ARCH x86_64
        mul x2          ' encoder knows: implicit rax operand, result in rdx:rax
    END ARCH
    ARCH aarch64
        umulh x0, x1, x2   ' orthogonal: high-multiply straight into the result
    END ARCH
  END FUNC
END ASM
```

**Caveat — implicit results must still be nameable.** When an architectural
instruction's result is *implicit* (x86 `mul` puts the high half in `rdx`, the
low half in `rax`), the IR pattern has to expose that result so the body can bind
it to an operand virtual. A bare `mul x2` can name its single explicit operand,
but "I want the high half as my result `x0`" needs the encoder to surface that
second result. The body must not do this by naming `rdx`; instead the surface may
need an explicit result form or a small pseudo-instruction whose result operand
*is* the high half. (Pushed far enough this shades into portable-intrinsic design
— e.g. an abstract `umulhi x0, x1, x2` that lowers to `mul`/`rdx` on x86 and
`umulh` on aarch64 — which is arguably a *different* feature than raw ASM, so it
is noted here but not adopted.)

### 2. How the ASM body becomes bytes — parse into the instruction IR

**The ASM body is parsed into the existing instruction IR and lowered through
the backend's own encoders** — it is not opaque text handed to an assembler.
(mfb has no assembler in the toolchain, so the opaque-text route would mean
building or embedding one; that path is rejected.)

The consequence is a real bound, stated plainly: `ASM` is not "write any
instruction," it is "write any instruction the backend already knows how to
encode." Adding a new mnemonic to the ASM surface == adding an encoder. That
keeps the allocator in the loop and means no inline instruction can do something
the compiler can't model — but it caps the "drop in arbitrary asm" dream. The
cap is the price, paid knowingly.

Two things make this the strong choice, not merely the only feasible one:

- **plan-03 already built the substrate.** The vreg layer and the rewrite pass
  that substitutes physicals into instruction register fields are *now built*
  (Stage A). An ASM body parsed into that same instruction IR drops straight
  into the allocator with no new plumbing — "virtual regs are first-class"
  stopped being aspirational.
- **The verification story is already built too.** The single most useful tool
  for an asm author is `-ncode` (final encoded instructions, post-allocation;
  see "Verifying ASM functions"). That output *only exists because the body went
  through the IR and the encoders* — it is a dump of exactly this pipeline.
  Parsing into the IR means the whole inspection surface
  (`-nir`/`-nplan`/`-nobj`/`-ncode`, plus the `-regalloc bump` oracle) comes for
  free; an opaque-assembler approach would have to build its own, or offer none.
  This materially lowers the cost of the whole feature.

The encoder-coverage cap then has a precise shape: the ASM vocabulary is exactly
the mnemonics with a `RegisterModel`-aware encoder, and adding a mnemonic (=
adding an encoder) automatically makes it visible to every dump flag above. And
the cap is *soft*: the compiler is MIT/Expat-licensed, so when a body needs a
mnemonic the backend doesn't encode yet, adding the encoder is a small,
unencumbered contribution — not a fork, not a license problem. The vocabulary
grows by ordinary pull request.

**Scope — what this is for, and what it is not.** `ASM` is intended for exactly
two things:

1. **A speed escape hatch** — hand-tuning a hot path the optimizer can't reach.
2. **Direct hardware / memory access** — touching memory and devices at a level
   the safe language deliberately doesn't expose.

It is **not** a mechanism for adding new architectural features or new target
architectures to the compiler. Reaching an instruction the backend can't encode
is resolved by *adding the encoder to the backend* (above) — making it a
first-class, modeled, allocator-aware instruction available to all of codegen —
not by smuggling raw bytes through an `ASM` body to dodge that work. The line:
`ASM` lets you *use* what the backend can model; it is not a back door for
things the backend can't.

### 3. Allocator interaction

Originally: "the allocator has no spilling and freely uses scratch (x9–x15)."
plan-03 is removing that limitation — Stage B adds liveness + linear-scan +
spilling; Stage C the FP class; Stage D loop-carried residency. Inline ASM
virtuals must flow through that same allocator rather than fighting it — which
is the upside of parsing into the IR. Two notes from plan-03 that matter for
inline asm:

- **Don't virtualize hand-tuned sequences.** plan-03 §4.6 deliberately keeps the
  ULP-validated `math::` NEON kernels on *fixed* `v`-registers and models them
  only as a clobber region — it does not rewrite them to virtual registers. The
  same option exists for inline asm: a body that must use exact registers can be
  treated as an opaque clobber region rather than virtualized. So the virtual
  model is the default, not a straitjacket.
- **Spilling has a real cost surface now.** Once inline asm virtuals can spill,
  an ASM body is no longer guaranteed register-resident — fine for most uses,
  but a body written assuming "my operands are in registers" is only true modulo
  the allocator's spill decisions. That is acceptable (it is what every other
  lowering already lives with), worth stating so it isn't a surprise.

### 4. Effects model — the largest missing practical piece

If the ASM body is parsed into instruction IR (issue 2), the compiler knows
register def/use — but for the *second* stated purpose, hardware / memory access,
def/use is not enough. The optimizer and allocator also need to know the body's
**effects**, or an aggressive enough optimizer will reorder, duplicate, or delete
something it must not. The effects that matter:

- reads memory / writes memory
- may trap
- touches volatile / device memory
- clobbers condition flags / depends on condition flags
- acts as an ordering barrier
- must not be deleted even when its result is unused

For an arithmetic-only body, ordinary dataflow suffices and none of this bites.
But a device-register store has no SSA result the optimizer can see, so under DCE
it looks dead; and two stores to the same MMIO address look redundant. Without an
effects contract those are miscompiles waiting for the optimizer to get smarter.

**Conservative first-cut rule (blunt but safe):**

> Every `ASM FUNC` is treated as having opaque memory side effects — reads and
> writes unknown memory, is not deletable, is not reorderable across other
> effecting operations — unless it is declared `PURE`.

`PURE` opts a body back into pure-dataflow treatment (the `add`/`fadd` examples
qualify). Later this can be refined into a small effect vocabulary so the
optimizer gets back the freedom it can safely have:

```
PURE
READS MEMORY
WRITES MEMORY
VOLATILE
FLAGS SCRATCH
```

The important commitment is that the *default is conservative*: a body says
nothing, the compiler assumes the worst, and correctness never depends on the
author having remembered to annotate.

### 5. Flags / status registers must be modeled

x86 `RFLAGS` and aarch64 `NZCV` are real shared state that ordinary instructions
both produce and consume, and an ASM body will use them:

```
cmp x1, x2        ' defines flags
cset x0, eq       ' aarch64: consumes flags
```

```
cmp x1, x2        ' defines flags
sete x0           ' x86: consumes flags
```

This only stays correct if the compiler does not reorder, separate, or schedule
something between the producer and the consumer. Two viable models:

- **Per-instruction flag def/use in the IR.** `cmp` defines a flags value,
  `cset`/`sete` use it; the scheduler/allocator treat flags as a (single,
  non-renameable) resource like any other.
- **Treat the whole `ARCH` body as an ordered, unschedulable mini-region** with
  flags *local to that region* and invisible outside it.

The second is almost certainly easier for inline ASM and composes with the
effects model above. The load-bearing rule either way:

> Flags do not live across the `ASM FUNC` boundary. If an instruction needs
> flags, the instruction that produces them must be inside the same `ARCH` body.

That keeps flags out of the cross-function operand contract entirely — the
signature only ever describes typed value operands, never condition state.

### 6. GC / runtime invariants — one paragraph the doc currently lacks

Even though MFBASIC has no moving GC or complex safepoints today, raw ASM has to
state what it may and may not do, or the invariant is undefined exactly where it
is most dangerous. The questions to answer: can ASM allocate? call runtime
helpers? trigger a safepoint? write pointer fields into heap objects (and if so
are write barriers required)? can scratch hold untracked pointers? Conservative
first-cut rule:

> Native `ARCH` bodies cannot call, allocate, or safepoint. Pointer values may be
> read and written as raw data, but preserving runtime invariants is the
> author's responsibility. The compiler only tracks the typed operands that cross
> the ASM boundary; anything a body does to memory in between is unmodeled and
> unchecked.

If heap pointer stores ever require write barriers in the runtime model, then raw
ASM stores into pointer fields are *especially* dangerous — that is acceptable
under `-unsafe`, but the doc must say it out loud rather than leave it implied.
This rule also explains why "no `FAIL`/`TRAP`/call in a body" (the
infallible-fragment section) is not an arbitrary restriction: a body that cannot
call is a body that cannot allocate or safepoint, which is precisely what keeps
the runtime invariants the compiler isn't tracking from being violated.

### 7. Multi-arch fallback: missing ARCH is a compile error — with an `ARCH MFB` escape

If a `FUNC` provides `ARCH aarch64` but no `ARCH x86_64`, compiling for x86_64
**fails at compile time** — it is not a runtime trap, and there is no implicit
fallback. Rationale: inline asm exists precisely because the programmer is
hand-writing the machine-level behavior; silently substituting nothing (or a
trap) would produce a binary that's missing a function the program depends on,
and pushing the failure to runtime hides a defect the compiler can see
statically. The contract is therefore: **every target you build for must have a
matching `ARCH` body.** A program that wants to be portable supplies every
arch it targets; one that only targets aarch64 simply won't compile elsewhere,
loudly and immediately.

```
ASM
  FUNC clz(x1 AS Integer) x0 AS Integer
    ARCH aarch64
        clz x0, x1
    END ARCH
    ' no ARCH x86_64 body
  END FUNC
END ASM
```

```
mfb build -unsafe -target macos-arm64   app   # ok
mfb build -unsafe -target macos-x86_64  app   # error: ASM FUNC 'clz' has no
                                              #   'ARCH x86_64' body for target macos-x86_64
```

**Committed sugar: `ARCH MFB <funcName>` — a typed catch-all.** Forcing a full
`ARCH x86_64` body that just re-implements the generic algorithm in x86 assembly
is pure busywork, and it is the *common* case: hand-tune the one or two arches
you care about, run a portable implementation everywhere else. So commit to a
fallback now rather than leaving it as someday-sugar. `ARCH MFB <funcName>`
names an ordinary MFBASIC function with a matching signature; on any target with
no matching native `ARCH <isa>` body, the `ASM FUNC` *is* that function.

```
ASM
  FUNC bswap64(x1 AS Integer) x0 AS Integer
    ARCH aarch64
        rev x0, x1               ' one-instruction byte reverse
    END ARCH
    ARCH MFB genericBswap        ' every other target: the portable version
  END FUNC
END ASM

' written once, in plain safe MFBASIC, reused on all non-aarch64 targets
FUNC genericBswap(n AS Integer) AS Integer
  ' ... shift/mask the 8 bytes into reverse order with bits:: ops ...
END FUNC
```

This does **not** weaken the compile-error rule — it satisfies it. The rule
becomes: every target must have *either* a matching native `ARCH <isa>` body
*or* the function must provide an `ARCH MFB` fallback. A function with neither,
for some target you build, is still a loud compile error. `ARCH MFB` is simply
the explicit, type-checked way to say "and generic everywhere else."

Constraints:

- **Signature match.** `genericBswap`'s parameters (by position/type) and return
  type must match the `ASM FUNC`'s, checked at compile time — the same contract
  any two overloads-by-arch must already satisfy.
- **Fallibility must stay target-independent** (the subtle one). A caller's code
  — whether it emits the per-call error check, whether the call presents as
  infallible or as the fallible (four-register) ABI — must not change based on
  which target you happen to build. But a native `ARCH` body is infallible while
  an MFBASIC fallback *can* fail (even integer `+` can raise `ErrOverflow`). The
  governing principle in all cases: fallibility is a property of the **declared
  function, fixed across all targets**, not of the selected body.

  **v1 decision: the `ARCH MFB` fallback must itself be infallible.** That keeps
  the entire feature on the simple side of the line — the declared function is
  infallible everywhere, every target presents the identical infallible boundary,
  and the success-envelope / ABI-shaping machinery below is never needed. If you
  need fallibility, add it in a thin safe wrapper (see
  [infallible inline fragments](#result-and-error-handling-infallible-inline-fragments)),
  not in the fallback. The core appeal of `ASM FUNC` is "inline dataflow
  fragment"; making the fallback fallible would graft ABI-shaping complexity onto
  exactly that feature, so it is deliberately deferred.

  **Deferred extension: fallible `ARCH MFB`.** If a later version wants to allow a
  fallback that can fail, the consistent design is: the whole `ASM FUNC` becomes
  fallible on *every* target, and the compiler wraps the infallible native bodies
  in the success envelope (`tag = OK, value`) so every target still presents one
  uniform ABI decided at the declaration, never by the build target. This is a
  clean extension of the v1 rule, not a contradiction of it — but it is out of
  scope for the first cut.
- **Still unsafe.** A package with an `ASM FUNC` is unsafe even when every real
  build of it takes the `ARCH MFB` path — it still *contains* hand-written
  assembly that some target will select. `-unsafe` is still required.

(The alternative — writing the fallback inline as MFBASIC *inside* an `ARCH MFB`
… `END ARCH` block — was considered and rejected: the body would have to use the
register-style parameter names (`x1`, `x2`) as if they were ordinary variables,
mixing two naming worlds. Naming a separately-declared normal function keeps the
fallback written in ordinary MFBASIC with ordinary parameter names.)

### 8. The boundary calling convention: the signature is a dataflow contract, resolved by inlining

"The signature is the calling convention" is elegant but only holds up if we're
clear about what the parameter names *are*. `x1`, `x2`, `d0` in the signature
are **local operand names, not ABI register assignments.** They name slots in a
dataflow contract — "argument 1 is materialized in this operand, the return is
read from that operand" — not real registers the caller must load.

The natural resolution, consistent with the rest of this design, is that
`ASM FUNC`s are **inlined** rather than called through a runtime ABI. That
makes marshaling a renaming problem the existing allocator already solves:

- **Called from high-level code.** The compiler evaluates each argument
  expression to a value (already its job), then unifies that value's virtual
  with the parameter's named virtual at the inline site — i.e. it substitutes
  the caller's argument virtuals for `x1`/`x2`/… in the body, and the return
  virtual becomes an ordinary virtual in the caller's namespace. No registers
  are loaded by hand; the allocator colors the spliced-in virtuals together
  with the surrounding code. There is no spill, no stack frame, no ABI at the
  seam.
- **Called from another `ASM` block.** Strictly simpler — the caller's values
  are already in virtuals, so unification is a direct virtual-to-virtual
  rename with no value-materialization step. Same mechanism.

```
' high-level call site — looks like any other function call:
LET total = add(price, tax)

' conceptually, after inlining the body of add (no bl, no ABI):
'   %v_price and %v_tax are unified with the body's x1, x2
'   the body's `add x0, x1, x2` writes a fresh virtual %v_total
'   `total` simply binds %v_total — the allocator colors it with the rest
```

Net: there is **no runtime calling convention at the boundary at all.** The
signature describes which value flows into which operand; inlining + the
allocator make that real. This is also why `SCRATCH` and the fixed-register
encoder constraints compose cleanly — everything is in one virtual namespace by
the time the allocator runs.

**There is already a working precedent in the tree.** plan-03 §4.6 describes the
`math::` transcendental kernels as code that *inlines into the calling user
function* and is reconciled with the caller's registers by the allocator (via
its clobber set). That is structurally the same move this section proposes for
`ASM FUNC`s — inline the body, let the allocator unify the namespace — so the
inline model isn't speculative; the backend does it for the kernels today. The
nuance plan-03 adds (previous item): an inlined body can keep fixed registers
and be modeled as a clobber region instead of being virtualized, which is a
second, coarser way to satisfy the same boundary.

Open sub-question (deferred): anything that forces a *real* call rather than an
inline — taking the address of an `ASM FUNC`, recursion, or calling one through
a function value — would require synthesizing an actual ABI and reintroduces
exactly the real-register questions inlining avoids. Simplest first cut:
disallow those (an `ASM FUNC` is inline-only), and revisit if a use case
demands it.

### 9. Axis separation — SETTLED: `-unsafe` gates ASM, `-baremetal` is separate

`-baremetal` and `unsafe inline asm` are orthogonal, so they are two flags.
Inline asm is useful in hosted mode too (intrinsics, fast paths); baremetal is
really about "no runtime / custom entry." Resolution:

- **`-unsafe`** gates the `ASM` construct (and is what the unsafe-package
  propagation above keys off). This is the flag this musing is about.
- **`-baremetal`** (if it ever exists) separately gates the runtime/entry
  story. It would almost always be used *with* `-unsafe` but does not imply it,
  and `-unsafe` does not imply baremetal — most inline asm runs in a normal
  hosted binary.

## Recommended first-cut rules

If this is ever promoted from musing to plan, the tightest version that keeps the
design's elegance while being implementable is roughly:

- `-unsafe` required for any package containing `ASM`.
- Unsafe propagates transitively through imports; the bit is recorded in package
  metadata and survives caching / prebuilt linking (see the unsafe-propagation
  note up top).
- `ASM FUNC` is **inline-only** — no function pointers, recursion, dynamic
  dispatch, or external symbol export. Real-callable materialization is a
  separate future feature.
- Native `ARCH` bodies are infallible machine fragments.
- `ARCH MFB` fallback must be signature-compatible **and infallible in v1**;
  fallible fallback is deferred (issue 7).
- Missing target `ARCH` without an `ARCH MFB` fallback is a compile error
  (issue 7).
- ASM parses into the native instruction IR; only backend-modeled instructions
  are accepted (issue 2).
- All operands are virtual; fixed-register constraints live in the
  encoder / `RegisterModel`, never in the body (issue 1).
- `SCRATCH` introduces local, class-typed dead temporaries.
- Flags are either explicitly modeled or local to an ordered ASM region; they do
  not cross the boundary (issue 5).
- ASM is conservatively treated as having opaque memory side effects unless
  declared `PURE` (issue 4).
- Native ASM bodies cannot call, allocate, or safepoint (issue 6).

That is a tight, safe-enough foundation. Fallible `ARCH MFB`, a finer effect
vocabulary, real-callable materialization, symbolic-offset syntax, and
opaque non-virtual register regions can all come later without reopening the
core abstraction — *inline asm as typed virtual-register IR, not text assembly
and not real-register constraints*.
