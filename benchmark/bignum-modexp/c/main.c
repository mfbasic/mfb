/* Bignum modexp over the P-256 prime, base-2^28 limbs — the identical
 * algorithm to the MFBASIC version (schoolbook multiply + bit-serial binary
 * long-division reduction) on fixed-size native arrays, as the "how fast can
 * this exact algorithm go" oracle. Prints checksum 1627198717. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define MASK 268435455ULL /* 2^28 - 1 */
#define CAP 24            /* product of two 10-limb values is <= 20 limbs */

typedef struct {
    uint64_t v[CAP];
    int n;
} bn;

static void bn_norm(bn *a) {
    while (a->n > 1 && a->v[a->n - 1] == 0)
        a->n--;
}

static int bn_cmp(const bn *a, const bn *b) {
    int n = a->n > b->n ? a->n : b->n;
    for (int i = n - 1; i >= 0; i--) {
        uint64_t ai = i < a->n ? a->v[i] : 0;
        uint64_t bi = i < b->n ? b->v[i] : 0;
        if (ai < bi)
            return -1;
        if (ai > bi)
            return 1;
    }
    return 0;
}

static void bn_add(bn *r, const bn *a, const bn *b) {
    int n = a->n > b->n ? a->n : b->n;
    uint64_t c = 0;
    for (int i = 0; i < n; i++) {
        uint64_t ai = i < a->n ? a->v[i] : 0;
        uint64_t bi = i < b->n ? b->v[i] : 0;
        uint64_t s = ai + bi + c;
        r->v[i] = s & MASK;
        c = s >> 28;
    }
    r->n = n;
    if (c)
        r->v[r->n++] = c;
}

/* a - b, requires a >= b. */
static void bn_sub(bn *r, const bn *a, const bn *b) {
    int64_t brw = 0;
    for (int i = 0; i < a->n; i++) {
        int64_t bi = i < b->n ? (int64_t)b->v[i] : 0;
        int64_t s = (int64_t)a->v[i] - bi - brw;
        if (s < 0) {
            s += 268435456;
            brw = 1;
        } else {
            brw = 0;
        }
        r->v[i] = (uint64_t)s;
    }
    r->n = a->n;
    bn_norm(r);
}

static void bn_mul(bn *r, const bn *a, const bn *b) {
    memset(r->v, 0, sizeof(r->v));
    r->n = a->n + b->n;
    for (int i = 0; i < a->n; i++) {
        uint64_t c = 0;
        uint64_t ai = a->v[i];
        for (int j = 0; j < b->n; j++) {
            uint64_t t = r->v[i + j] + ai * b->v[j] + c;
            r->v[i + j] = t & MASK;
            c = t >> 28;
        }
        r->v[i + b->n] += c;
    }
    bn_norm(r);
}

static void bn_shl1(bn *a) {
    uint64_t c = 0;
    for (int i = 0; i < a->n; i++) {
        uint64_t t = (a->v[i] << 1) | c;
        a->v[i] = t & MASK;
        c = t >> 28;
    }
    if (c)
        a->v[a->n++] = c;
}

/* x mod m by bit-serial binary long division — the hot path. */
static void bn_mod(bn *x, const bn *m) {
    if (bn_cmp(x, m) < 0)
        return;
    int nbits = x->n * 28;
    bn r, one, t;
    r.v[0] = 0;
    r.n = 1;
    one.v[0] = 1;
    one.n = 1;
    for (int i = nbits - 1; i >= 0; i--) {
        uint64_t bit = (x->v[i / 28] >> (i % 28)) & 1;
        bn_shl1(&r);
        if (bit) {
            bn_add(&t, &r, &one);
            r = t;
        }
        if (bn_cmp(&r, m) >= 0) {
            bn_sub(&t, &r, m);
            r = t;
        }
    }
    *x = r;
}

static void bn_modmul(bn *r, const bn *a, const bn *b, const bn *m) {
    bn_mul(r, a, b);
    bn_mod(r, m);
}

int main(void) {
    /* p256 = 2^256 - 2^224 + 2^192 + 2^96 - 1, base-2^28 little-endian limbs. */
    bn p = {{268435455, 268435455, 268435455, 4095, 0, 0, 16777216, 0, 268435455, 15}, 10};
    /* g = bytes 01..20 (big-endian) as a field element. */
    bn g = {{220077856, 27374017, 102176793, 20005201, 252711186, 12636384, 134810123, 5267568, 16909060}, 9};
    uint64_t e = 6822318947648322238ULL;

    bn r = {{1}, 1};
    bn b = g;
    bn t;
    for (int i = 0; i < 63; i++) {
        if ((e >> i) & 1) {
            bn_modmul(&t, &r, &b, &p);
            r = t;
        }
        bn_modmul(&t, &b, &b, &p);
        b = t;
    }

    uint64_t acc = 0;
    for (int j = 0; j < r.n; j++)
        acc += r.v[j];
    printf("checksum: %llu\n", (unsigned long long)acc);
    return 0;
}
