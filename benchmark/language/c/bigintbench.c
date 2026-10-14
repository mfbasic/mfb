/* GROUP: bigint — the C column for bigint.mfb (the big:: arbitrary-precision
 * integer package). libc has no bignum and the suite vendors none (a GMP-class
 * dependency would compare a hand-tuned assembly library, not C), so every
 * row is mfb+python only and records a `--` placeholder here to keep the
 * tables row-aligned — the same convention as `crypto ed25519`. */
#include <stddef.h>

#include "bench.h"
#include "bigintbench.h"

void run_bigint_group(void) {
  record("bigint", "pidigits", NULL, 0);
  record("bigint", "factorial", NULL, 0);
  record("bigint", "modpow", NULL, 0);
  record("bigint", "gcd", NULL, 0);
}
