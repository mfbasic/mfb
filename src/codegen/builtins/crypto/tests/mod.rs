use crate::codegen::registry::{self, registry};
use crate::types::ParameterType;

#[test]
fn crypto_registered_on_the_clean_room_registry() {
    let pkg = registry()
        .resolve_package("crypto")
        .expect("crypto package");
    // 21 members: the unified `generate`/`sign`/`verify`/`hash`, the
    // `SymmetricCipher`-selected `seal`/`open`, the hash-generic
    // `hmac`/`hkdf`/`pbkdf2(Hash, …)`, the memory-hard password KDF
    // `argon2id` (bug-515, both its explicit-cost and `Argon2Profile`
    // overloads are one member), the SHAKE256 XOF `shake256`, `convert`
    // (Ed25519→X25519 / Ed448→X448 key conversion), `exchange` (X25519/X448
    // Diffie-Hellman), the `AsymmetricCipher`-selected `encrypt`/`decrypt`
    // (RFC 9180 HPKE), plus
    // `randomBytes`/`randomInt`/`uuid4`/`uuid7`/`ulid`/`constantTimeEqual`. The
    // per-type generate/sign/verify/sha and per-digest `*Sha*`/per-cipher AEAD
    // members were all retired behind the unified surface.
    assert_eq!(pkg.functions().len(), 21);
}

#[test]
fn membership_via_generic_registry() {
    for n in [
        "crypto.hash",
        "crypto.hmac",
        "crypto.hkdf",
        "crypto.pbkdf2",
        "crypto.argon2id",
        "crypto.shake256",
        "crypto.seal",
        "crypto.open",
        "crypto.randomBytes",
        "crypto.randomInt",
        "crypto.uuid4",
        "crypto.uuid7",
        "crypto.ulid",
        "crypto.generate",
        "crypto.sign",
        "crypto.verify",
        "crypto.convert",
        "crypto.exchange",
        "crypto.encrypt",
        "crypto.decrypt",
        "crypto.constantTimeEqual",
    ] {
        assert_eq!(registry().owning_package(n), Some("crypto"), "{n}");
    }
    // The per-digest/per-curve members were folded into `hash`/`sign`/`verify`;
    // the per-digest KDF/MAC and per-cipher AEAD members were retired behind the
    // `Hash`-generic `hmac`/`hkdf`/`pbkdf2` and `SymmetricCipher`-selected
    // `seal`/`open`.
    for gone in [
        "crypto.sha224",
        "crypto.sha256",
        "crypto.sha384",
        "crypto.sha512",
        "crypto.p256Sign",
        "crypto.p256Verify",
        "crypto.p384Sign",
        "crypto.p384Verify",
        "crypto.p521Sign",
        "crypto.p521Verify",
        "crypto.ed25519Sign",
        "crypto.ed25519Verify",
        "crypto.hmacSha256",
        "crypto.hmacSha512",
        "crypto.hkdfSha256",
        "crypto.hkdfSha512",
        "crypto.pbkdf2Sha256",
        "crypto.pbkdf2Sha512",
        "crypto.aes256GcmSeal",
        "crypto.aes256GcmOpen",
        "crypto.chacha20Poly1305Seal",
        "crypto.chacha20Poly1305Open",
    ] {
        assert!(registry().owning_package(gone).is_none(), "{gone}");
    }
    assert!(registry().owning_package("crypto.nope").is_none());
}

#[test]
fn native_and_internal_flags() {
    use crate::codegen::registry::is_abi_function_call;
    // The OS-seam CSPRNG `randomBytes` is a clean-room `AbiFunction` (like
    // `generate`/`sign`/`verify`/`hash`/`seal`), so every `crypto` member is an
    // `AbiFunction`/`Rewrite` — it routes through the shared `RuntimeHelper::Abi` family.
    assert!(is_abi_function_call("crypto.randomBytes"));
    for f in [
        "crypto.generate",
        "crypto.sign",
        "crypto.verify",
        "crypto.hash",
    ] {
        assert!(is_abi_function_call(f), "{f}");
    }
    // The `Body::Rewrite` source members are NOT runtime/abi calls.
    for f in [
        "crypto.hmac",
        "crypto.convert",
        "crypto.exchange",
        "crypto.constantTimeEqual",
    ] {
        assert!(!is_abi_function_call(f), "{f}");
    }
}

#[test]
fn builtin_types_recognized() {
    for t in ["crypto.Sealed", "crypto.KeyPair"] {
        assert!(registry().is_builtin_type(t), "{t}");
    }
    assert!(!registry().is_builtin_type("Nope"));
    assert_eq!(
        registry().qualified_builtin_type("crypto.Sealed"),
        Some("crypto.Sealed".to_string())
    );
}

#[test]
fn bytes_text_overloads_select_distinct_rewrite_targets() {
    let sel = |call: &str, args: &[&str]| -> Option<&'static str> {
        let types: Vec<crate::types::ParameterType> = args
            .iter()
            .map(|s| crate::types::ParameterType::declared(s))
            .collect();
        registry::rewrite_target(call, &types)
    };
    // The unified `hash(Hash, data)`: the `List OF Byte` overload is a native
    // AbiFunction (no source rewrite), the `String` overload rewrites to the
    // `__crypto_hashText` UTF-8 shim.
    assert_eq!(sel("crypto.hash", &["crypto.Hash", "List OF Byte"]), None);
    assert_eq!(
        sel("crypto.hash", &["crypto.Hash", "String"]),
        Some("__crypto_hashText")
    );
    // The unified `hmac(Hash, key, data)` selects on `data` (arg index 2): the
    // `String` form rewrites to the `__crypto_hmacText` UTF-8 shim, the
    // `List OF Byte` form to the hash-generic `__crypto_hmac` core.
    assert_eq!(
        sel("crypto.hmac", &["crypto.Hash", "List OF Byte", "String"]),
        Some("__crypto_hmacText")
    );
    assert_eq!(
        sel(
            "crypto.hmac",
            &["crypto.Hash", "List OF Byte", "List OF Byte"]
        ),
        Some("__crypto_hmac")
    );
    // The unified `pbkdf2(Hash, password, …)` has a single `List OF Byte`
    // overload rewriting to the hash-generic `__crypto_pbkdf2` core.
    assert_eq!(
        sel(
            "crypto.pbkdf2",
            &[
                "crypto.Hash",
                "List OF Byte",
                "List OF Byte",
                "Integer",
                "Integer"
            ]
        ),
        Some("__crypto_pbkdf2")
    );
    // `shake256(data, length)` selects on `data`: the `String` form rewrites to
    // the `__crypto_shake256Text` UTF-8 shim, the `List OF Byte` form to the
    // XOF core.
    assert_eq!(
        sel("crypto.shake256", &["List OF Byte", "Integer"]),
        Some("__crypto_shake256")
    );
    assert_eq!(
        sel("crypto.shake256", &["String", "Integer"]),
        Some("__crypto_shake256Text")
    );
    // Single-body source member.
    assert_eq!(sel("crypto.uuid4", &[]), Some("__crypto_uuid4"));
    assert_eq!(sel("crypto.uuid7", &[]), Some("__crypto_uuid7"));
    assert_eq!(sel("crypto.ulid", &[]), Some("__crypto_ulid"));
    assert_eq!(
        sel(
            "crypto.constantTimeEqual",
            &["List OF Byte", "List OF Byte"]
        ),
        Some("__crypto_constantTimeEqual")
    );
    // Native / AbiFunction member -> no rewrite target.
    assert_eq!(sel("crypto.randomBytes", &["Integer"]), None);
    assert_eq!(
        sel(
            "crypto.sign",
            &["crypto.Certificate", "List OF Byte", "List OF Byte"]
        ),
        None
    );
}

#[test]
fn argument_typed_return_resolution() {
    let r = |call: &str, args: &[&str]| {
        let types: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        registry::resolve_call(call, &types, false)
    };
    assert_eq!(
        r("crypto.hash", &["crypto.Hash", "List OF Byte"]),
        Some("List OF Byte".into())
    );
    assert_eq!(
        r("crypto.hash", &["crypto.Hash", "String"]),
        Some("List OF Byte".into())
    );
    assert_eq!(r("crypto.hash", &["crypto.Hash", "Integer"]), None);
    assert_eq!(
        r(
            "crypto.seal",
            &[
                "crypto.SymmetricCipher",
                "List OF Byte",
                "List OF Byte",
                "List OF Byte"
            ]
        ),
        Some("crypto.Sealed".into())
    );
    assert_eq!(
        r(
            "crypto.seal",
            &[
                "crypto.SymmetricCipher",
                "List OF Byte",
                "List OF Byte",
                "List OF Byte",
                "List OF Byte"
            ]
        ),
        Some("crypto.Sealed".into())
    );
    assert_eq!(r("crypto.uuid4", &[]), Some("String".into()));
    assert_eq!(r("crypto.uuid7", &[]), Some("String".into()));
    assert_eq!(r("crypto.ulid", &[]), Some("String".into()));
    assert_eq!(
        r("crypto.randomBytes", &["Integer"]),
        Some("List OF Byte".into())
    );
    assert_eq!(
        r("crypto.randomInt", &["Integer", "Integer"]),
        Some("Integer".into())
    );
    // plan-127-D: the `big::Int` overload returns `big.Int`, and adding it did not move
    // the two-`Integer` shape above.
    assert_eq!(
        r("crypto.randomInt", &["big.Int", "big.Int"]),
        Some("big.Int".into())
    );
    assert_eq!(
        r(
            "crypto.verify",
            &[
                "crypto.Certificate",
                "List OF Byte",
                "List OF Byte",
                "List OF Byte"
            ]
        ),
        Some("Boolean".into())
    );
}

/// plan-127-D: `crypto::randomInt` is one member with two implementation rows. The
/// `Integer` row keeps its return type, error set and `__crypto_randomInt` body; the
/// `big::Int` row rewrites to the gated `__crypto_randomIntBig` helper and declares the
/// same errors, so the fallibility verdict is "fallible" for either argument shape.
#[test]
fn random_int_big_overload_registry_facts() {
    let types = |args: &[&str]| -> Vec<crate::types::ParameterType> {
        args.iter()
            .map(|s| crate::types::ParameterType::declared(s))
            .collect()
    };
    let integers = types(&["Integer", "Integer"]);
    let bigs = types(&["big.Int", "big.Int"]);
    assert_eq!(
        registry::rewrite_target("crypto.randomInt", &integers),
        Some("__crypto_randomInt")
    );
    assert_eq!(
        registry::rewrite_target("crypto.randomInt", &bigs),
        Some("__crypto_randomIntBig")
    );
    assert_eq!(
        registry::resolve_call_typed("crypto.randomInt", &integers, true)
            .map(|t| t.name().into_owned()),
        Some("Integer".to_string())
    );
    assert_eq!(
        registry::resolve_call_typed("crypto.randomInt", &bigs, true)
            .map(|t| t.name().into_owned()),
        Some("big.Int".to_string())
    );
    let function = registry()
        .resolve_func("crypto.randomInt")
        .expect("crypto.randomInt is registered")
        .function;
    assert_eq!(function.implementations.len(), 2);
    for implementation in &function.implementations {
        assert_eq!(
            implementation.errors,
            vec!["ErrInvalidArgument", "ErrUnknown", "ErrOutOfMemory"],
            "{:?}",
            implementation.return_type
        );
    }
    assert!(!crate::codegen::builtins::inline_builtin_is_infallible(
        "crypto.randomInt",
        &integers,
        &crate::codegen::builtins::NoTypeKinds
    ));
    assert!(!crate::codegen::builtins::inline_builtin_is_infallible(
        "crypto.randomInt",
        &bigs,
        &crate::codegen::builtins::NoTypeKinds
    ));
}

#[test]
fn clock_derived_identifiers_declare_the_clock_overflow() {
    // bug-640: `uuid7` and `ulid` read `datetime::nowNanos`, which raises
    // `ErrOverflow` for a clock reading outside the `Integer` nanosecond range
    // (`tests/runtime/rt_datetime_clock_overflow.rs` pins the clock and observes
    // both raising 77050010). A member that can raise it must declare it.
    for name in ["crypto.uuid7", "crypto.ulid"] {
        let function = registry()
            .resolve_func(name)
            .unwrap_or_else(|| panic!("{name} is registered"))
            .function;
        for implementation in &function.implementations {
            assert!(
                implementation.errors.contains(&"ErrOverflow"),
                "{name} declares {:?}",
                implementation.errors
            );
        }
    }
}

#[test]
fn aead_aad_default_padding() {
    // AEAD `seal(cipher, key, nonce, data, [aad])` pads one trailing `aad` when
    // omitted (4 provided -> 1), none at 5.
    assert_eq!(
        registry::default_argument_padding(
            "crypto.seal",
            &vec![ParameterType::Unknown; 4],
            &crate::codegen::builtins::NoTypeKinds
        )
        .len(),
        1
    );
    assert_eq!(
        registry::default_argument_padding(
            "crypto.seal",
            &vec![ParameterType::Unknown; 5],
            &crate::codegen::builtins::NoTypeKinds
        )
        .len(),
        0
    );
    // `open(cipher, key, nonce, ciphertext, tag, [aad])` pads one trailing `aad`
    // when omitted (5 provided -> 1), none at 6.
    assert_eq!(
        registry::default_argument_padding(
            "crypto.open",
            &vec![ParameterType::Unknown; 5],
            &crate::codegen::builtins::NoTypeKinds
        )
        .len(),
        1
    );
    assert_eq!(
        registry::default_argument_padding(
            "crypto.open",
            &vec![ParameterType::Unknown; 6],
            &crate::codegen::builtins::NoTypeKinds
        )
        .len(),
        0
    );
    assert_eq!(
        registry::default_argument_padding(
            "crypto.hash",
            &vec![ParameterType::Unknown; 2],
            &crate::codegen::builtins::NoTypeKinds
        )
        .len(),
        0
    );
}

/// bug-863: the padding is the RESOLVED overload's. A `Sealed`-form `open` with
/// its `aad` written is complete (it was padded with the ciphertext/tag form's
/// `aad`, a sixth argument no overload accepts); without it, it gets one `aad`.
#[test]
fn open_pads_the_overload_its_arguments_select() {
    let bytes = ParameterType::list_of(ParameterType::Byte);
    let cipher = ParameterType::named("crypto.SymmetricCipher");
    let sealed = ParameterType::named("crypto.Sealed");
    let pad = |args: &[ParameterType]| {
        registry::default_argument_padding(
            "crypto.open",
            args,
            &crate::codegen::builtins::NoTypeKinds,
        )
    };
    let sealed_aad = [
        cipher.clone(),
        bytes.clone(),
        bytes.clone(),
        sealed.clone(),
        bytes.clone(),
    ];
    assert_eq!(pad(&sealed_aad), Vec::new());
    let sealed_only = [cipher.clone(), bytes.clone(), bytes.clone(), sealed];
    let padded = pad(&sealed_only);
    assert_eq!(padded.len(), 1);
    assert_eq!(padded[0].0, bytes);
    let tag_form = [
        cipher,
        bytes.clone(),
        bytes.clone(),
        bytes.clone(),
        bytes.clone(),
    ];
    assert_eq!(pad(&tag_form).len(), 1);
}

/// Structural constant-time audit of the Keccak core (plan-109-B): the
/// permutation and its round must contain no conditional or early exit at
/// all, and the sponge's only conditional may test the PUBLIC output length.
/// Every `collections::get` index in the round is a loop counter or a public
/// table lookup — this pins that no arm branches on, or indexes by, state or
/// message contents.
#[test]
fn keccak_core_is_branch_free() {
    let source = registry()
        .resolve_package("crypto")
        .expect("crypto")
        .get_mfb();
    let body_of = |name: &str| -> String {
        let start = source
            .find(&format!("FUNC {name}("))
            .unwrap_or_else(|| panic!("{name} not in assembled source"));
        let end = source[start..].find("END FUNC").expect("END FUNC") + start;
        source[start..end].to_string()
    };
    for name in [
        "__crypto_keccakRound",
        "__crypto_keccakF",
        "__crypto_leLane",
        "__crypto_appendLeLane",
    ] {
        let body = body_of(name);
        assert!(!body.contains("IF "), "{name} must be branch-free:\n{body}");
        assert!(
            !body.contains("EXIT ") && !body.contains("TRAP"),
            "{name} must have no early exit:\n{body}"
        );
    }
    let sponge = body_of("__crypto_keccakSponge");
    let conditionals: Vec<&str> = sponge.lines().filter(|l| l.contains("IF ")).collect();
    assert_eq!(conditionals.len(), 1, "{conditionals:?}");
    assert!(
        conditionals[0].contains("len(out) < outLen"),
        "the sponge's only branch is on the public output length: {conditionals:?}"
    );
    // Indices into the state within the round: a nested lookup whose inner
    // subject is the state (`get(a, get(a, …))`) would be a data-dependent
    // address; the only nested lookups are the public RHO/PI tables.
    let round = body_of("__crypto_keccakRound");
    for state in ["a", "b", "t", "c", "out"] {
        for inner in ["a", "b", "t", "c", "out"] {
            assert!(
                !round.contains(&format!(
                    "collections::get({state}, collections::get({inner}"
                )),
                "{state} indexed by {inner} contents"
            );
        }
    }
    assert!(round.contains("collections::get(__CRYPTO_KECCAK_RHO, i)"));
    assert!(round.contains("collections::get(__CRYPTO_KECCAK_PI, i)"));
}

/// Structural constant-time audit of the Curve448 secret paths (plan-109-C/D):
/// the X448 ladder, the Ed448 ladder/swap/addition, and the scalar reduction
/// contain no conditional at all, and the field multiply/carry branch only on
/// loop counters (`IF n …`, `IF j …`, `IF i …` — fold weights and bias limbs).
#[test]
fn curve448_secret_paths_are_branch_free() {
    let source = registry()
        .resolve_package("crypto")
        .expect("crypto")
        .get_mfb();
    let body_of = |name: &str| -> String {
        let start = source
            .find(&format!("FUNC {name}("))
            .unwrap_or_else(|| panic!("{name} not in assembled source"));
        let end = source[start..].find("END FUNC").expect("END FUNC") + start;
        source[start..end].to_string()
    };
    for name in [
        "__crypto_x448",
        "__crypto_gf448Select",
        "__crypto_gf448Add",
        "__crypto_gf448MulSmall",
        "__crypto_gf448Carry",
        "__crypto_ed448Scalarmult",
        "__crypto_ed448Cswap",
        "__crypto_ed448Add",
        "__crypto_ed448ModL",
        "__crypto_ed448Fold",
        "__crypto_bnMul",
        "__crypto_ed448Prune",
        "__crypto_clampScalar448",
    ] {
        let body = body_of(name);
        assert!(!body.contains("IF "), "{name} must be branch-free:\n{body}");
    }
    // The multiply, subtraction bias, and inverse/sqrt ladders branch only on
    // loop counters — every `IF` tests `n`, `j`, or `i`.
    for name in [
        "__crypto_gf448Mul",
        "__crypto_gf448Sub",
        "__crypto_gf448Inv",
        "__crypto_gf448PowP34",
        "__crypto_gf448Pack",
    ] {
        let body = body_of(name);
        for line in body.lines().filter(|l| l.trim_start().starts_with("IF ")) {
            let cond = line.trim_start().trim_start_matches("IF ");
            assert!(
                cond.starts_with("n ") || cond.starts_with("j ") || cond.starts_with("i "),
                "{name} branches on a non-counter: {line}"
            );
        }
    }
}

/// Structural constant-time audit of the Curve25519 secret paths (bug-511) —
/// the twin of `curve448_secret_paths_are_branch_free`, which covered the 448
/// field only. Two things on the X25519 path are secret: the private scalar
/// whose bits drive the ladder, and the shared secret whose canonical
/// representative `__crypto_pack25519` selects. Neither may be branched on.
/// `crypto::exchange`'s page promises this for **both** ladders ("a fixed
/// 255-/448-iteration Montgomery ladder whose conditional swap is branch-free,
/// so … no control flow depends on the private key"), so a branch here is the
/// page being false, not merely a missing hardening.
#[test]
fn curve25519_secret_paths_are_branch_free() {
    let source = registry()
        .resolve_package("crypto")
        .expect("crypto")
        .get_mfb();
    let body_of = |name: &str| -> String {
        let start = source
            .find(&format!("FUNC {name}("))
            .unwrap_or_else(|| panic!("{name} not in assembled source"));
        let end = source[start..].find("END FUNC").expect("END FUNC") + start;
        source[start..end].to_string()
    };
    for name in [
        "__crypto_x25519",
        "__crypto_sel25519",
        "__crypto_pack25519",
        "__crypto_clampScalar",
        "__crypto_unpack25519",
        "__crypto_edA",
        "__crypto_edZ",
        "__crypto_edS",
        "__crypto_gf121665",
    ] {
        let body = body_of(name);
        assert!(!body.contains("IF "), "{name} must be branch-free:\n{body}");
    }
    // The multiply, carry, and inversion ladder branch only on loop counters —
    // every `IF` tests `i`, `j`, `j2` (the `edM` convolution indices), or `a`
    // (the `inv25519` exponent-bit position, a public constant schedule).
    for name in ["__crypto_edM", "__crypto_car25519", "__crypto_inv25519"] {
        let body = body_of(name);
        for line in body.lines().filter(|l| l.trim_start().starts_with("IF ")) {
            let cond = line.trim_start().trim_start_matches("IF ");
            assert!(
                cond.starts_with("i ")
                    || cond.starts_with("j ")
                    || cond.starts_with("j2 ")
                    || cond.starts_with("a "),
                "{name} branches on a non-counter: {line}"
            );
        }
    }
    // The masked select is the only swap primitive the ladder may use: it must
    // XOR under the mask, never compare.
    let sel = body_of("__crypto_sel25519");
    assert!(
        sel.contains("bits::band(mask,"),
        "sel25519 must mask:\n{sel}"
    );
    assert!(
        sel.contains("bits::bxor("),
        "sel25519 must XOR-select:\n{sel}"
    );
}

/// Executable bound proof for the GF(2^448−2^224−1) arithmetic (plan-109-C
/// Phase 1): with every limb at its carried maximum 2^28 (`__crypto_gf448Carry`
/// leaves limbs in `0..=2^28` after two passes), the schoolbook convolution
/// columns and the folded output limbs of `__crypto_gf448Mul` — mirroring the
/// helper's exact fold weights — and the biased sums of `Add`/`Sub`/`MulSmall`
/// must all stay below the trapping `Integer` boundary 2^63.
#[test]
fn gf448_mul_accumulators_fit_i63() {
    let limb: u128 = 1 << 28;
    let product = limb * limb;
    // Convolution column k has min(k,15) - max(0,k-15) + 1 products.
    let column = |k: usize| -> u128 {
        let lo = k.saturating_sub(15);
        let hi = k.min(15);
        (hi - lo + 1) as u128 * product
    };
    let mut worst = 0u128;
    for n in 0..16 {
        let mut v = column(n);
        if n <= 14 {
            v += column(n + 16);
        }
        if n >= 8 {
            v += column(n + 8);
            if n <= 14 {
                v += column(n + 16);
            }
        }
        if n <= 6 {
            v += column(n + 24);
        }
        worst = worst.max(v);
        assert!(v < 1u128 << 63, "output limb {n} accumulator {v} >= 2^63");
    }
    // The worst column (limb 8: 9 + 2·7 + 15 = 38 products) is ~2^61.25.
    assert_eq!(worst, 38 * product);
    assert!(worst < 1u128 << 62);
    // Carry-pass input: a limb plus the previous carry (< 2^35) stays < 2^63.
    assert!(worst + (1u128 << 35) < 1u128 << 63);
    // Add: two carried limbs; Sub: a − b + 2p_i with the biases the helper uses;
    // MulSmall: limb × 39081 (the largest constant, a24 / the Edwards d).
    assert!(2 * limb < 1u128 << 63);
    assert!(limb + 536_870_910 < 1u128 << 63);
    assert_eq!(536_870_910, 2 * ((1u128 << 28) - 1));
    assert_eq!(536_870_908, 2 * ((1u128 << 28) - 2));
    assert!(limb * 39081 < 1u128 << 63);
}

#[test]
fn reassembled_source_parses() {
    let source = registry()
        .resolve_package("crypto")
        .expect("crypto")
        .get_mfb();
    crate::ast::parse_source_internal(
        std::path::Path::new("<builtin-crypto>"),
        "builtins/crypto.mfb",
        &source,
    )
    .expect("reassembled crypto source parses");
}
