use super::*;

use crate::arch::aarch64::abi;

// `crypto::randomBytes(count)` — the CSPRNG entry point. `count` arrives in the
// standard return/argument register; the helper allocates and fills a
// `List OF Byte` from OS entropy (`getentropy`, plan-04-crypto.md §A.6). Every
// other `crypto` primitive is a portable software core (see the package source);
// the NIST-EC public-key operations are the only other native helpers.
const CRYPTO_RANDOM_BYTES_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "count",
    type_: "Integer",
    location: abi::RETURN_REGISTER,
}];

pub(crate) const CRYPTO_RANDOM_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Crypto,
    call: "crypto.randomBytes",
    symbol: "_mfb_rt_crypto_crypto_randomBytes",
    abi: RuntimeHelperAbi {
        params: CRYPTO_RANDOM_BYTES_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// NIST-EC public-key helpers (plan-04-crypto.md Part C). Bound to the platform
// key API — `SecKey` on macOS, `EVP_PKEY` on Linux — behind a wire-compatible
// encoding (private = 0x04‖X‖Y‖K, public = 0x04‖X‖Y, signature = DER X9.62).
// `generateP*Raw` returns the raw private bytes; the public `crypto::generateP*`
// is source glue that slices the public point out and builds the `KeyPair`.

const CRYPTO_EC_SIGN_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "privateKey",
        type_: "List OF Byte",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "message",
        type_: "List OF Byte",
        location: "x1",
    },
];

const CRYPTO_EC_VERIFY_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "publicKey",
        type_: "List OF Byte",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "message",
        type_: "List OF Byte",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "signature",
        type_: "List OF Byte",
        location: "x2",
    },
];

macro_rules! crypto_ec_generate_spec {
    ($ident:ident, $call:literal, $symbol:literal) => {
        pub(crate) const $ident: RuntimeHelperSpec = RuntimeHelperSpec {
            helper: RuntimeHelper::Crypto,
            call: $call,
            symbol: $symbol,
            abi: RuntimeHelperAbi {
                params: &[],
                returns: "List OF Byte",
                clobbers: abi::IO_PRINT_CLOBBERS,
            },
        };
    };
}

macro_rules! crypto_ec_sign_spec {
    ($ident:ident, $call:literal, $symbol:literal) => {
        pub(crate) const $ident: RuntimeHelperSpec = RuntimeHelperSpec {
            helper: RuntimeHelper::Crypto,
            call: $call,
            symbol: $symbol,
            abi: RuntimeHelperAbi {
                params: CRYPTO_EC_SIGN_PARAMS,
                returns: "List OF Byte",
                clobbers: abi::IO_PRINT_CLOBBERS,
            },
        };
    };
}

macro_rules! crypto_ec_verify_spec {
    ($ident:ident, $call:literal, $symbol:literal) => {
        pub(crate) const $ident: RuntimeHelperSpec = RuntimeHelperSpec {
            helper: RuntimeHelper::Crypto,
            call: $call,
            symbol: $symbol,
            abi: RuntimeHelperAbi {
                params: CRYPTO_EC_VERIFY_PARAMS,
                returns: "Boolean",
                clobbers: abi::IO_PRINT_CLOBBERS,
            },
        };
    };
}

crypto_ec_generate_spec!(CRYPTO_GENERATE_P256_RAW_SPEC, "crypto.generateP256Raw", "_mfb_rt_crypto_crypto_generateP256Raw");
crypto_ec_generate_spec!(CRYPTO_GENERATE_P384_RAW_SPEC, "crypto.generateP384Raw", "_mfb_rt_crypto_crypto_generateP384Raw");
crypto_ec_generate_spec!(CRYPTO_GENERATE_P521_RAW_SPEC, "crypto.generateP521Raw", "_mfb_rt_crypto_crypto_generateP521Raw");
crypto_ec_sign_spec!(CRYPTO_P256_SIGN_SPEC, "crypto.p256Sign", "_mfb_rt_crypto_crypto_p256Sign");
crypto_ec_sign_spec!(CRYPTO_P384_SIGN_SPEC, "crypto.p384Sign", "_mfb_rt_crypto_crypto_p384Sign");
crypto_ec_sign_spec!(CRYPTO_P521_SIGN_SPEC, "crypto.p521Sign", "_mfb_rt_crypto_crypto_p521Sign");
crypto_ec_verify_spec!(CRYPTO_P256_VERIFY_SPEC, "crypto.p256Verify", "_mfb_rt_crypto_crypto_p256Verify");
crypto_ec_verify_spec!(CRYPTO_P384_VERIFY_SPEC, "crypto.p384Verify", "_mfb_rt_crypto_crypto_p384Verify");
crypto_ec_verify_spec!(CRYPTO_P521_VERIFY_SPEC, "crypto.p521Verify", "_mfb_rt_crypto_crypto_p521Verify");
