/*
 * capture_ref.c — macOS libm reference oracle for the NEON math kernels.
 *
 * This is the accuracy reference of record for plan-01-simd Phase 5: each NEON
 * f64 polynomial kernel must land within <=1 ULP of the value macOS's system
 * libm produces for the same input (see planning/plan-01-simd.md §4.6, Open
 * Decision #7). This program is the *only* thing that defines "the macOS-libm
 * value": it links the system math library and calls the libm symbol directly
 * with zero reimplementation, so its output is, by construction, macOS libm.
 *
 * It is a pure filter: read input bit-patterns from stdin, apply the named libm
 * function, write input+output bit-patterns to stdout. Input selection lives in
 * gen_inputs.py so the oracle stays trivially auditable — all this file does is
 * `result = <libm fn>(x)`.
 *
 * MUST be built and run on the reference macOS (this project's Darwin/aarch64).
 * The committed `(input, expected_bits)` vectors it emits are then read by the
 * Rust kernel tests on every target (macOS + both Linux flavors), so CI/Linux
 * validate against the macOS-libm oracle without needing a Mac. Re-capture only
 * when intentionally re-pinning the reference (record the OS/libm version in the
 * file header — capture.sh does this).
 *
 * Line format (lowercase hex of the IEEE-754 bits, matching "%016llx"):
 *   unary  functions:  "<x_bits> <result_bits>\n"
 *   binary functions:  "<x_bits> <y_bits> <result_bits>\n"
 * Lines beginning with '#', and blank lines, are passed through unchanged so a
 * provenance header can be carried in the same stream.
 *
 * Build:  cc -O0 -std=c11 -Wall -Wextra -o capture_ref capture_ref.c -lm
 * Usage:  ./capture_ref <fn>   (fn = exp|log|log10|sin|cos|tan|
 *                                    asin|acos|atan|atan2|pow|fmod)
 *         gen_inputs.py emits the stdin stream; capture.sh wires them together.
 */

#include <errno.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

/* Reinterpret between f64 and its 64-bit pattern without aliasing UB. */
static double bits_to_f64(uint64_t b) {
    double d;
    memcpy(&d, &b, sizeof d);
    return d;
}
static uint64_t f64_to_bits(double d) {
    uint64_t b;
    memcpy(&b, &d, sizeof b);
    return b;
}

typedef double (*unary_fn)(double);
typedef double (*binary_fn)(double, double);

struct unary_entry {
    const char *name;
    unary_fn fn;
};
struct binary_entry {
    const char *name;
    binary_fn fn;
};

/* The 9 unary transcendentals and the binary ones from §4.6, plus fmod (the
 * Float MOD operator; plan-01-libm-kernels §4.1). These resolve to the system
 * libm symbols at link time — the call below IS the macOS-libm value. fmod is
 * exact, so its reference locks the kernel to a bit-identical (0 ULP) result. */
static const struct unary_entry UNARY[] = {
    {"exp", exp},   {"log", log},   {"log10", log10},
    {"sin", sin},   {"cos", cos},   {"tan", tan},
    {"asin", asin}, {"acos", acos}, {"atan", atan},
};
static const struct binary_entry BINARY[] = {
    {"atan2", atan2}, {"pow", pow}, {"fmod", fmod},
};

static int parse_bits(const char *tok, uint64_t *out) {
    char *end = NULL;
    errno = 0;
    unsigned long long v = strtoull(tok, &end, 16);
    if (end == tok || errno != 0)
        return 0;
    *out = (uint64_t)v;
    return 1;
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s <fn>\n", argv[0]);
        return 2;
    }
    const char *fn = argv[1];

    unary_fn ufn = NULL;
    binary_fn bfn = NULL;
    for (size_t i = 0; i < sizeof UNARY / sizeof UNARY[0]; i++)
        if (strcmp(fn, UNARY[i].name) == 0)
            ufn = UNARY[i].fn;
    for (size_t i = 0; i < sizeof BINARY / sizeof BINARY[0]; i++)
        if (strcmp(fn, BINARY[i].name) == 0)
            bfn = BINARY[i].fn;
    if (!ufn && !bfn) {
        fprintf(stderr, "%s: unknown function '%s'\n", argv[0], fn);
        return 2;
    }

    char line[256];
    long lineno = 0;
    while (fgets(line, sizeof line, stdin)) {
        lineno++;
        /* Pass through comments / blank lines untouched (provenance header). */
        const char *p = line;
        while (*p == ' ' || *p == '\t')
            p++;
        if (*p == '#' || *p == '\n' || *p == '\0') {
            fputs(line, stdout);
            continue;
        }

        char buf[256];
        strncpy(buf, line, sizeof buf - 1);
        buf[sizeof buf - 1] = '\0';

        char *tok1 = strtok(buf, " \t\r\n");
        char *tok2 = strtok(NULL, " \t\r\n");

        if (ufn) {
            uint64_t xb;
            if (!tok1 || !parse_bits(tok1, &xb)) {
                fprintf(stderr, "%s: bad unary input on line %ld\n", argv[0], lineno);
                return 1;
            }
            double r = ufn(bits_to_f64(xb));
            printf("%016llx %016llx\n",
                   (unsigned long long)xb, (unsigned long long)f64_to_bits(r));
        } else {
            uint64_t xb, yb;
            if (!tok1 || !tok2 || !parse_bits(tok1, &xb) || !parse_bits(tok2, &yb)) {
                fprintf(stderr, "%s: bad binary input on line %ld\n", argv[0], lineno);
                return 1;
            }
            double r = bfn(bits_to_f64(xb), bits_to_f64(yb));
            printf("%016llx %016llx %016llx\n",
                   (unsigned long long)xb, (unsigned long long)yb,
                   (unsigned long long)f64_to_bits(r));
        }
    }
    return 0;
}
