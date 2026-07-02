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
