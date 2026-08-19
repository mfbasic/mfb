//! `crypto::randomBytes` — descriptor entry + the clean-room `AbiFunction` CSPRNG.
//!
//! A NATIVE member and the one remaining OS-seam helper in `crypto`: every other
//! primitive is either a clean-room `AbiFunction` over a platform key API
//! (`generate`/`sign`/`verify`) or a portable MFB software core, but a secure RNG
//! must read the OS entropy source, so this member binds it directly. Its
//! [`Body::abi_function`] body ([`lower_random_bytes`]) fills a fresh `List OF Byte`
//! from `getentropy` (macOS/Linux) or `BCryptGenRandom` (Windows), emitted once as a
//! shared `_mfb_rt_*` helper and `bl`'d from every call site — exactly like the other
//! crypto `AbiFunction`s. The `getentropy` / `BCryptGenRandom` import is declared by
//! `Platform::runtime_imports` keyed on the `crypto.randomBytes` call name, and the
//! runtime spec is DERIVED by `registry::runtime_specs` (routed through the shared
//! `RuntimeHelper::Abi` family).

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::memory::marshal::emit_build_byte_list;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// `getentropy(buf, len)` accepts at most 256 bytes per call, so the fill runs in
/// <=256-byte chunks.
const GETENTROPY_MAX: usize = 256;

/// Upper bound on `crypto::randomBytes(count)`. Far above any real key-material
/// request (16 MiB), it caps the `count * ENTRY + HEADER + count` collection-size
/// arithmetic well below a u64 overflow and rejects an absurd allocation before it
/// is attempted (bug-177 D). A larger ask is reported as an invalid argument.
const RANDOM_BYTES_MAX_COUNT: usize = 16 * 1024 * 1024;

/// The `AbiFunction` body for `crypto::randomBytes`. `args[0]` is the requested
/// `count` (an `Integer`) in its argument register. It validates the count, fills a
/// scratch buffer from OS entropy in <=256-byte chunks (`getentropy` on macOS/Linux,
/// `BCryptGenRandom` on Windows), builds the `List OF Byte`, wipes the scratch, and
/// self-manages the fallible result — returning the `void` sentinel (the wrapper adds
/// no epilogue).
pub(crate) fn lower_random_bytes(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let count_op = args[0].location.clone();

    // Frame slots (sp-relative locals; the wrapper reserves them via
    // `allocate_stack_object`, base 0, so these are absolute sp offsets).
    const COUNT_OFFSET: usize = 0; // requested byte count
    const BUF_OFFSET: usize = 8; // scratch entropy buffer pointer
    const OFF_OFFSET: usize = 16; // fill cursor
    const COLLECTION_OFFSET: usize = 24; // the List OF Byte being built
    const LOCAL_SIZE: usize = 32;
    builder.allocate_stack_object("crypto_random_bytes_scratch", LOCAL_SIZE);

    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let entropy_fail = format!("{symbol}_entropy_fail");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let chunk_ok = format!("{symbol}_chunk_ok");
    let done = format!("{symbol}_done");

    // Minted scratch vregs (%v9..%v13), so the shared allocator colors them per-ISA.
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();

    // Validate 0 <= count <= RANDOM_BYTES_MAX_COUNT and stash it. The upper bound
    // rejects an absurd request before the count*ENTRY + HEADER + count size
    // arithmetic below can overflow (bug-177 D); the cap is materialized into a
    // vreg so the compare is size-safe on every backend.
    builder.instructions.extend([
        abi::compare_immediate(&count_op, "0"),
        abi::branch_lt(&invalid),
        abi::move_immediate(&v9, "Integer", &RANDOM_BYTES_MAX_COUNT.to_string()),
        abi::compare_registers(&count_op, &v9),
        abi::branch_gt(&invalid),
        abi::store_u64(&count_op, abi::stack_pointer(), COUNT_OFFSET),
        // Allocate a scratch buffer of `count` bytes (arena_alloc rounds up, so a
        // zero request still yields a valid pointer we simply never read). The alloc
        // reads its size from return_register() and its alignment from c_arg(1).
        abi::load_u64(abi::return_register(), abi::stack_pointer(), COUNT_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        &alloc_fail,
    );
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUF_OFFSET),
        // Fill the buffer from OS entropy in <=256-byte chunks.
        abi::move_immediate(&v9, "Integer", "0"),
        abi::store_u64(&v9, abi::stack_pointer(), OFF_OFFSET),
        abi::label(&fill_loop),
        abi::load_u64(&v9, abi::stack_pointer(), OFF_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), COUNT_OFFSET),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&fill_done),
        // chunk = min(count - off, 256)
        abi::subtract_registers(&v11, &v10, &v9),
        abi::move_immediate(&v12, "Integer", &GETENTROPY_MAX.to_string()),
        abi::compare_registers(&v11, &v12),
        abi::branch_le(&chunk_ok),
        abi::move_register(&v11, &v12),
        abi::label(&chunk_ok),
        // getentropy(buf + off, chunk)
        abi::load_u64(&v13, abi::stack_pointer(), BUF_OFFSET),
        abi::add_registers(abi::return_register(), &v13, &v9),
        abi::move_register(abi::c_arg(1), &v11),
    ]);
    if ctx.platform.family() == PlatformFamily::Windows {
        // Windows has no getentropy: BCryptGenRandom(NULL, buf+off, chunk,
        // BCRYPT_USE_SYSTEM_PREFERRED_RNG) fills the same buffer and returns 0
        // (STATUS_SUCCESS) on success, matching the getentropy contract the loop
        // and the `!= 0` check below expect.
        builder.instructions.extend([
            abi::move_register(abi::c_arg(2), abi::c_arg(1)), // chunk  -> r8
            abi::move_register(abi::c_arg(1), abi::return_register()), // buf+off -> rdx
            abi::move_immediate(abi::return_register(), "Integer", "0"), // hAlg = NULL
            abi::move_immediate(abi::c_arg(3), "Integer", "2"), // BCRYPT_USE_SYSTEM_PREFERRED_RNG
        ]);
        ctx.platform.emit_external_call(
            "BCryptGenRandom",
            &symbol,
            ctx.platform_imports,
            &mut builder.instructions,
            &mut builder.relocations,
        )?;
        // bug-447: a Win64 external call leaves its NTSTATUS in the C-return
        // register (`rax` = `c_return(0)`), and the Win64 `emit_external_call` does
        // NOT stage it into the aligned MFB result bank. On Win64 `return_register()`
        // (`mfb_return(0)`) is `rcx`, a distinct caller-saved register the call
        // clobbers, so the shared `compare_immediate(return_register(), 0)` check
        // below would read garbage and spuriously fail (ErrUnknown). Sign-extend the
        // 32-bit NTSTATUS from `c_return(0)` into `return_register()`, matching the
        // CNG `bcrypt_call` staging: this both moves the status into the register the
        // check reads and clears any upper-32-bit garbage, so STATUS_SUCCESS (0)
        // tests equal to 0.
        builder.instructions.push(abi::sign_extend_word(
            abi::return_register(),
            abi::c_return(0),
        ));
    } else {
        ctx.platform.emit_external_call(
            "getentropy",
            &symbol,
            ctx.platform_imports,
            &mut builder.instructions,
            &mut builder.relocations,
        )?;
    }
    builder.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&entropy_fail),
        abi::load_u64(&v9, abi::stack_pointer(), OFF_OFFSET),
        // recompute chunk from off/count for the cursor advance
        abi::load_u64(&v10, abi::stack_pointer(), COUNT_OFFSET),
        abi::subtract_registers(&v11, &v10, &v9),
        abi::move_immediate(&v12, "Integer", &GETENTROPY_MAX.to_string()),
        abi::compare_registers(&v11, &v12),
        abi::branch_le(&format!("{symbol}_adv_ok")),
        abi::move_register(&v11, &v12),
        abi::label(&format!("{symbol}_adv_ok")),
        abi::add_registers(&v9, &v9, &v11),
        abi::store_u64(&v9, abi::stack_pointer(), OFF_OFFSET),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
    ]);

    // Build the `List OF Byte` with `count` elements, copying from the entropy buffer.
    emit_build_byte_list(
        &symbol,
        &format!("{symbol}_rand_build_loop"),
        &format!("{symbol}_rand_build_done"),
        BUF_OFFSET,
        COUNT_OFFSET,
        Some(COLLECTION_OFFSET),
        abi::mfb_return(1),
        &alloc_fail,
        &mut builder.instructions,
        &mut builder.relocations,
    );

    // Wipe the entropy scratch buffer now that its bytes have been copied into the
    // returned List OF Byte, so a later same-program arena allocation cannot be handed
    // a block still holding the generated random bytes (bug-177 D). Call-free guarded
    // zero loop. %v9 = cursor, %v10 = count, %v11 = index.
    let zero_skip = format!("{symbol}_zero_skip");
    let zero_loop = format!("{symbol}_zero_loop");
    let zero_end = format!("{symbol}_zero_end");
    builder.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), BUF_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&zero_skip),
        abi::load_u64(&v10, abi::stack_pointer(), COUNT_OFFSET),
        abi::move_immediate(&v11, "Integer", "0"),
        abi::label(&zero_loop),
        abi::compare_registers(&v11, &v10),
        abi::branch_eq(&zero_end),
        abi::store_u8(abi::ZERO, &v9, 0),
        abi::add_immediate(&v9, &v9, 1),
        abi::add_immediate(&v11, &v11, 1),
        abi::branch(&zero_loop),
        abi::label(&zero_end),
        abi::label(&zero_skip),
    ]);

    builder.instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error exits.
    builder.instructions.push(abi::label(&invalid));
    emit_fail(
        &symbol,
        "ErrInvalidArgument",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );
    builder.instructions.push(abi::label(&entropy_fail));
    emit_fail(
        &symbol,
        "ErrUnknown",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );
    builder.instructions.push(abi::label(&alloc_fail));
    emit_fail(
        &symbol,
        "ErrOutOfMemory",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        type_: "List OF Byte".to_string(),
        location: Operand::from("void"),
        text: "crypto.randomBytes".to_string(),
    })
}

const INTRO: &str =
    r#"Return `count` cryptographically secure random bytes drawn from the OS CSPRNG."#;
const DESC: &str = r#"`crypto::randomBytes` returns a fresh `List OF Byte` of length `count`, filled
from the operating system's cryptographically secure pseudo-random number
generator (CSPRNG). The output is unpredictable to an adversary and is the
correct source for keys, nonces, initialization vectors, salts, tokens, and any
other value whose secrecy or unguessability is a security requirement.

**Range and boundaries.** `count` is validated **before** any allocation and
must satisfy `0 <= count <= 16777216` (16 MiB, the `RANDOM_BYTES_MAX_COUNT`
cap). A `count` of `0` returns an empty list; a negative `count` or one above
the 16 MiB cap raises `ErrInvalidArgument` and allocates nothing. The cap also
keeps the internal collection-size arithmetic well below integer overflow.

**Security caveats.** This generator is cryptographically secure and, by design,
**not** seedable — there is no way to fix, seed, or replay its output, and each
call draws fresh entropy, so results are never reproducible across runs. That is
the deliberate contrast with `math::rand`, a fast, seedable PCG64 generator that
is **not** cryptographically secure and must never be used for keys, tokens, or
nonces. After the returned list is built, the internal entropy scratch buffer is
zeroed, so no later allocation in the same program can observe the generated
bytes. When you later compare secret material derived from these bytes (a MAC, a
token, an API key), never use the ordinary `=` operator — it short-circuits and
leaks timing; use `crypto::constantTimeEqual`.

**Implementation.** Unlike the portable software cores in this package (the
hashes, HMAC, HKDF, PBKDF2, and the AEADs), `randomBytes` is the one member here
that is a **native runtime helper reading OS entropy directly**, not MFBASIC
source. On **macOS and Linux** (glibc and musl) it uses `getentropy(2)`, filling
the buffer in chunks of at most 256 bytes (the per-call `getentropy` limit),
transparent to the caller. On **Windows** it uses `BCryptGenRandom` with the
`BCRYPT_USE_SYSTEM_PREFERRED_RNG` flag. Because the bytes come from OS entropy,
the output is inherently non-reproducible and platform-provided rather than
byte-identical across targets."#;
const EX: &str = r#"Generate a 32-byte key and a 12-byte AEAD nonce:

```
IMPORT crypto

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
END SUB
```

A count of zero returns an empty list:

```
IMPORT crypto

SUB main()
  LET none AS List OF Byte = crypto::randomBytes(0)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "randomBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "count",
                desc: "The number of random bytes to return. Must be in `0` to `16777216` \
                       (16 MiB) inclusive; `0` yields an empty list.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: bytes(),
            errors: vec!["ErrInvalidArgument", "ErrUnknown", "ErrOutOfMemory"],
            body: Body::abi_function(lower_random_bytes),
        }],
    });
}
