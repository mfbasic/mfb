// GENERATED — minimax coefficients for the NEON f64 math kernels.
// Source: tools/math-kernels/gen_coeffs.py  (Remez exchange via mpmath)
// Regenerate: python3 tools/math-kernels/gen_coeffs.py gen --out <this file>
// Each block lists the function approximated, the reduction it assumes,
// the fit interval, and the achieved minimax relative error. Coefficients
// are ordered by ascending power of the reduced variable (c[0] = constant).
//
// These approximate the *reduced* function; the kernel reconstructs the
// full transcendental as noted in each block (plan-01-simd §4.6). They are
// validated <=1 ULP against the committed macOS-libm reference vectors by
// `gen_coeffs.py verify`.

/// exp: minimax of `exp(x) = 2**n * P(r)`
/// reduction: x = n*ln2 + r,  n = round(x/ln2),  r in [-ln2/2, ln2/2]
/// fit var `r` on [-0.34657359027997264, 0.34657359027997264], degree 11 (relative error)
/// achieved minimax relative error: 3.055e-18 (~0.0138 ULP of the reduced value)
pub const EXP_COEFFS: [f64; 12] = [
    1.0,                    // r^0
    1.0,                    // r^1
    0.5000000000000018,     // r^2
    0.1666666666666617,     // r^3
    0.04166666666649277,    // r^4
    0.008333333333559272,   // r^5
    0.0013888888951224037,  // r^6
    0.0001984126943267626,  // r^7
    2.4801486521375963e-05, // r^8
    2.755762253355922e-06,  // r^9
    2.763229329749704e-07,  // r^10
    2.4994304016107913e-08, // r^11
];

/// log: minimax of `log(x) = k*ln2 + s*P(s**2)    [log10(x) = log(x) * log10(e)]`
/// reduction: x = 2**k * m,  m in [1/sqrt2, sqrt2],  s = (m-1)/(m+1)
/// fit var `s2` on [0, 0.029437251522859413], degree 7 (relative error)
/// achieved minimax relative error: 1.135e-18 (~0.00511 ULP of the reduced value)
pub const LOG_COEFFS: [f64; 8] = [
    2.0,                 // s2^0
    0.6666666666666765,  // s2^1
    0.3999999999929888,  // s2^2
    0.2857142876134677,  // s2^3
    0.22222196988240323, // s2^4
    0.1818363509450266,  // s2^5
    0.1531244375301122,  // s2^6
    0.1481052984310695,  // s2^7
];

/// sin: minimax of `sin(r) = r * P(r**2)   (cos branch / quadrant select per §4.6)`
/// reduction: reduce x to r in [-pi/4, pi/4] (Cody-Waite), quadrant q
/// fit var `x2` on [0, 0.61685027506808487], degree 6 (relative error)
/// achieved minimax relative error: 3.312e-18 (~0.0149 ULP of the reduced value)
pub const SIN_COEFFS: [f64; 7] = [
    1.0,                    // x2^0
    -0.16666666666666616,   // x2^1
    0.008333333333320002,   // x2^2
    -0.0001984126982840213, // x2^3
    2.755731329901509e-06,  // x2^4
    -2.505070584638448e-08, // x2^5
    1.589413637225924e-10,  // x2^6
];

/// cos: minimax of `cos(r) = P(r**2)   (tan = sin/cos)`
/// reduction: reduce x to r in [-pi/4, pi/4] (Cody-Waite), quadrant q
/// fit var `x2` on [0, 0.61685027506808487], degree 7 (relative error)
/// achieved minimax relative error: 3.584e-20 (~0.000161 ULP of the reduced value)
pub const COS_COEFFS: [f64; 8] = [
    1.0,                     // x2^0
    -0.5,                    // x2^1
    0.04166666666666643,     // x2^2
    -0.0013888888888858961,  // x2^3
    2.4801587282899464e-05,  // x2^4
    -2.755731286569637e-07,  // x2^5
    2.0875555145712737e-09,  // x2^6
    -1.1352123207581011e-11, // x2^7
];

/// atan: minimax of `atan(x) = x * P(x**2)   (asin/acos/atan2 via identities, §4.6)`
///
/// No consumer: `atan` is computed from the fdlibm 4-segment `ATAN_AT` table,
/// not from this minimax fit, and no SIMD `atan` kernel exists. Kept anyway
/// because `tools/math-kernels/gen_coeffs.py` emits `atan` as one of its five
/// primitive reduced approximations — deleting the block here would be undone
/// by the next regeneration and would leave the tool and this file disagreeing
/// (bug-326-A6; the item was filed as a deletion, which is wrong for a
/// generated file).
/// reduction: |x|>1 -> pi/2 - atan(1/x); fit on |x| in [0,1]
/// fit var `x2` on [0, 1], degree 18 (relative error)
/// achieved minimax relative error: 1.658e-16 (~0.747 ULP of the reduced value)
#[allow(dead_code)]
pub const ATAN_COEFFS: [f64; 19] = [
    0.9999999999999999,     // x2^0
    -0.33333333333321036,   // x2^1
    0.19999999998481552,    // x2^2
    -0.14285714211125564,   // x2^3
    0.11111109168705821,    // x2^4
    -0.09090878097177522,   // x2^5
    0.0769197729267317,     // x2^6
    -0.06664176113186528,   // x2^7
    0.05868541717145544,    // x2^8
    -0.052051650177506965,  // x2^9
    0.04573429813687501,    // x2^10
    -0.03865240057695672,   // x2^11
    0.030108754291382843,   // x2^12
    -0.020534303615291766,  // x2^13
    0.011595982260264572,   // x2^14
    -0.00509755464867919,   // x2^15
    0.0016125682813088573,  // x2^16
    -0.0003235227297010902, // x2^17
    3.072795379872296e-05,  // x2^18
];
