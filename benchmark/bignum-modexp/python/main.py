"""Bignum modexp over the P-256 prime, base-2^28 limb lists — the identical
algorithm to the MFBASIC version (schoolbook multiply + bit-serial binary
long-division reduction), deliberately NOT Python's native pow(), so all three
implementations do the same list work. Prints checksum 1181356819."""

MASK = 268435455  # 2^28 - 1


def bn_norm(a):
    n = len(a)
    while n > 1 and a[n - 1] == 0:
        n -= 1
    return a[:n]


def bn_cmp(a, b):
    la, lb = len(a), len(b)
    for i in range(max(la, lb) - 1, -1, -1):
        ai = a[i] if i < la else 0
        bi = b[i] if i < lb else 0
        if ai < bi:
            return -1
        if ai > bi:
            return 1
    return 0


def bn_add(a, b):
    la, lb = len(a), len(b)
    r = []
    c = 0
    for i in range(max(la, lb)):
        ai = a[i] if i < la else 0
        bi = b[i] if i < lb else 0
        s = ai + bi + c
        r.append(s & MASK)
        c = s >> 28
    if c:
        r.append(c)
    return r


def bn_sub(a, b):
    """a - b, requires a >= b."""
    lb = len(b)
    r = []
    brw = 0
    for i in range(len(a)):
        bi = b[i] if i < lb else 0
        s = a[i] - bi - brw
        if s < 0:
            s += 268435456
            brw = 1
        else:
            brw = 0
        r.append(s)
    return bn_norm(r)


def bn_mul(a, b):
    la, lb = len(a), len(b)
    r = [0] * (la + lb)
    for i in range(la):
        c = 0
        ai = a[i]
        for j in range(lb):
            t = r[i + j] + ai * b[j] + c
            r[i + j] = t & MASK
            c = t >> 28
        r[i + lb] += c
    return bn_norm(r)


def bn_shl1(a):
    r = []
    c = 0
    for i in range(len(a)):
        t = (a[i] << 1) | c
        r.append(t & MASK)
        c = t >> 28
    if c:
        r.append(c)
    return r


def bn_mod(x, m):
    """x mod m by bit-serial binary long division — the hot path."""
    if bn_cmp(x, m) < 0:
        return x
    nbits = len(x) * 28
    r = [0]
    for i in range(nbits - 1, -1, -1):
        limb, off = divmod(i, 28)
        bit = (x[limb] >> off) & 1
        r = bn_shl1(r)
        if bit:
            r = bn_add(r, [1])
        if bn_cmp(r, m) >= 0:
            r = bn_sub(r, m)
    return r


def bn_modmul(a, b, m):
    return bn_mod(bn_mul(a, b), m)


def main():
    # p256 = 2^256 - 2^224 + 2^192 + 2^96 - 1, base-2^28 little-endian limbs.
    p = [268435455, 268435455, 268435455, 4095, 0, 0, 16777216, 0, 268435455, 15]
    # g = bytes 01..20 (big-endian) as a field element.
    g = [220077856, 27374017, 102176793, 20005201, 252711186, 12636384, 134810123, 5267568, 16909060]
    e = 45

    r = [1]
    b = g
    for i in range(6):
        if (e >> i) & 1:
            r = bn_modmul(r, b, p)
        b = bn_modmul(b, b, p)

    print("checksum: " + str(sum(r)))
    return 0


if __name__ == "__main__":
    main()
