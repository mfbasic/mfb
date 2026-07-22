use super::*;

// `crypto::randomBytes(count)` — the CSPRNG entry point. `count` arrives in the
// standard return/argument register; the helper allocates and fills a
// `List OF Byte` from OS entropy (`getentropy`, plan-04-crypto.md §A.6). Every
// other `crypto` primitive is a portable software core (see the package source);
// the NIST-EC public-key operations are the only other native helpers.
pub(crate) const CRYPTO_RANDOM_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Crypto,
    call: "crypto.randomBytes",
    abi: RuntimeHelperAbi {
        returns: "List OF Byte",
    },
};

// NIST-EC public-key helpers (plan-04-crypto.md Part C). Bound to the platform
// key API — `SecKey` on macOS, `EVP_PKEY` on Linux — behind a wire-compatible
// encoding (private = 0x04‖X‖Y‖K, public = 0x04‖X‖Y, signature = DER X9.62).
// `generateP*Raw` returns the raw private bytes; the public `crypto::generateP*`
// is source glue that slices the public point out and builds the `KeyPair`.

macro_rules! crypto_ec_generate_spec {
    ($ident:ident, $call:literal) => {
        pub(crate) const $ident: RuntimeHelperSpec = RuntimeHelperSpec {
            helper: RuntimeHelper::Crypto,
            call: $call,
            abi: RuntimeHelperAbi {
                returns: "List OF Byte",
            },
        };
    };
}

macro_rules! crypto_ec_sign_spec {
    ($ident:ident, $call:literal) => {
        pub(crate) const $ident: RuntimeHelperSpec = RuntimeHelperSpec {
            helper: RuntimeHelper::Crypto,
            call: $call,
            abi: RuntimeHelperAbi {
                returns: "List OF Byte",
            },
        };
    };
}

macro_rules! crypto_ec_verify_spec {
    ($ident:ident, $call:literal) => {
        pub(crate) const $ident: RuntimeHelperSpec = RuntimeHelperSpec {
            helper: RuntimeHelper::Crypto,
            call: $call,
            abi: RuntimeHelperAbi { returns: "Boolean" },
        };
    };
}

crypto_ec_generate_spec!(CRYPTO_GENERATE_P256_RAW_SPEC, "crypto.generateP256Raw");
crypto_ec_generate_spec!(CRYPTO_GENERATE_P384_RAW_SPEC, "crypto.generateP384Raw");
crypto_ec_generate_spec!(CRYPTO_GENERATE_P521_RAW_SPEC, "crypto.generateP521Raw");
crypto_ec_sign_spec!(CRYPTO_P256_SIGN_SPEC, "crypto.p256Sign");
crypto_ec_sign_spec!(CRYPTO_P384_SIGN_SPEC, "crypto.p384Sign");
crypto_ec_sign_spec!(CRYPTO_P521_SIGN_SPEC, "crypto.p521Sign");
crypto_ec_verify_spec!(CRYPTO_P256_VERIFY_SPEC, "crypto.p256Verify");
crypto_ec_verify_spec!(CRYPTO_P384_VERIFY_SPEC, "crypto.p384Verify");
crypto_ec_verify_spec!(CRYPTO_P521_VERIFY_SPEC, "crypto.p521Verify");
