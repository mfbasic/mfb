//! Clean-room Argon2id / BLAKE2b reference, structured exactly like the MFBASIC
//! core it is the oracle for. Pinned against RFC 9106 §5.3 and RFC 7693 App. A.

// ---------------- BLAKE2b ----------------
const IV: [u64; 8] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
    0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
];
const SIGMA: [[usize; 16]; 12] = [
    [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15],
    [14,10,4,8,9,15,13,6,1,12,0,2,11,7,5,3],
    [11,8,12,0,5,2,15,13,10,14,3,6,7,1,9,4],
    [7,9,3,1,13,12,11,14,2,6,5,10,4,0,15,8],
    [9,0,5,7,2,4,10,15,14,1,11,12,6,8,3,13],
    [2,12,6,10,0,11,8,3,4,13,7,5,15,14,1,9],
    [12,5,1,15,14,13,4,10,0,7,6,3,9,2,8,11],
    [13,11,7,14,12,1,3,9,5,0,15,4,8,6,2,10],
    [6,15,14,9,11,3,0,8,12,2,13,7,1,4,10,5],
    [10,2,8,4,7,6,1,5,15,11,9,14,3,12,13,0],
    [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15],
    [14,10,4,8,9,15,13,6,1,12,0,2,11,7,5,3],
];

fn b2b_g(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

fn b2b_compress(h: &mut [u64; 8], block: &[u8], t: u128, last: bool) {
    let mut m = [0u64; 16];
    for i in 0..16 {
        let mut w = 0u64;
        for j in 0..8 { w |= (block[i * 8 + j] as u64) << (8 * j); }
        m[i] = w;
    }
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(h);
    v[8..].copy_from_slice(&IV);
    v[12] ^= (t & 0xFFFF_FFFF_FFFF_FFFF) as u64;
    v[13] ^= (t >> 64) as u64;
    if last { v[14] ^= u64::MAX; }
    for r in 0..12 {
        let s = &SIGMA[r];
        b2b_g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
        b2b_g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
        b2b_g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
        b2b_g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
        b2b_g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
        b2b_g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        b2b_g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
        b2b_g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
    }
    for i in 0..8 { h[i] ^= v[i] ^ v[i + 8]; }
}

/// Unkeyed BLAKE2b with `outlen` (1..=64) output bytes.
pub fn blake2b(data: &[u8], outlen: usize) -> Vec<u8> {
    let mut h = IV;
    h[0] ^= 0x0101_0000 ^ (outlen as u64);
    let mut t: u128 = 0;
    let mut off = 0usize;
    while data.len() - off > 128 {
        t += 128;
        b2b_compress(&mut h, &data[off..off + 128], t, false);
        off += 128;
    }
    let rest = &data[off..];
    let mut last = [0u8; 128];
    last[..rest.len()].copy_from_slice(rest);
    t += rest.len() as u128;
    b2b_compress(&mut h, &last, t, true);
    let mut out = Vec::with_capacity(outlen);
    for i in 0..outlen { out.push(((h[i / 8] >> (8 * (i % 8))) & 0xFF) as u8); }
    out
}

/// Argon2's variable-length hash H'^T.
fn h_prime(a: &[u8], outlen: usize) -> Vec<u8> {
    let mut input = Vec::with_capacity(4 + a.len());
    input.extend_from_slice(&(outlen as u32).to_le_bytes());
    input.extend_from_slice(a);
    if outlen <= 64 { return blake2b(&input, outlen); }
    let r = (outlen + 31) / 32 - 2;
    let mut out = Vec::with_capacity(outlen);
    let mut v = blake2b(&input, 64);
    out.extend_from_slice(&v[..32]);
    for _ in 1..r {
        v = blake2b(&v, 64);
        out.extend_from_slice(&v[..32]);
    }
    let tail = outlen - 32 * r;
    let vlast = blake2b(&v, tail);
    out.extend_from_slice(&vlast);
    out
}

// ---------------- Argon2 ----------------
fn gb(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize) {
    let m = |x: u64, y: u64| -> u64 {
        2u64.wrapping_mul((x & 0xFFFF_FFFF).wrapping_mul(y & 0xFFFF_FFFF))
    };
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(m(v[a], v[b]));
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]).wrapping_add(m(v[c], v[d]));
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(m(v[a], v[b]));
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]).wrapping_add(m(v[c], v[d]));
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

fn permute(v: &mut [u64; 16]) {
    gb(v, 0, 4, 8, 12); gb(v, 1, 5, 9, 13); gb(v, 2, 6, 10, 14); gb(v, 3, 7, 11, 15);
    gb(v, 0, 5, 10, 15); gb(v, 1, 6, 11, 12); gb(v, 2, 7, 8, 13); gb(v, 3, 4, 9, 14);
}

/// R = x^y ; rows then columns ; out = Z ^ R [^ old]
fn fill_block(prev: &[u64], reff: &[u64], next: &mut [u64], with_xor: bool) {
    let mut r = [0u64; 128];
    for i in 0..128 { r[i] = prev[i] ^ reff[i]; }
    let mut tmp = r;
    if with_xor { for i in 0..128 { tmp[i] ^= next[i]; } }
    for i in 0..8 {
        let mut v = [0u64; 16];
        for k in 0..16 { v[k] = r[16 * i + k]; }
        permute(&mut v);
        for k in 0..16 { r[16 * i + k] = v[k]; }
    }
    for i in 0..8 {
        let mut v = [0u64; 16];
        for k in 0..8 {
            v[2 * k] = r[2 * i + 16 * k];
            v[2 * k + 1] = r[2 * i + 16 * k + 1];
        }
        permute(&mut v);
        for k in 0..8 {
            r[2 * i + 16 * k] = v[2 * k];
            r[2 * i + 16 * k + 1] = v[2 * k + 1];
        }
    }
    for i in 0..128 { next[i] = tmp[i] ^ r[i]; }
}

fn index_alpha(pass: u32, lane_len: u32, seg_len: u32, slice: u32, index: u32,
               pseudo_rand: u32, same_lane: bool) -> u32 {
    let ref_area: u32 = if pass == 0 {
        if slice == 0 { index - 1 }
        else if same_lane { slice * seg_len + index - 1 }
        else { slice * seg_len + if index == 0 { u32::MAX } else { 0 } }
    } else if same_lane { lane_len - seg_len + index - 1 }
    else { lane_len - seg_len + if index == 0 { u32::MAX } else { 0 } };
    let mut rel = pseudo_rand as u64;
    rel = (rel * rel) >> 32;
    rel = ref_area as u64 - 1 - ((ref_area as u64 * rel) >> 32);
    let start = if pass != 0 && slice != 3 { (slice + 1) * seg_len } else { 0 };
    (((start as u64 + rel) % lane_len as u64)) as u32
}

pub fn argon2id(pwd: &[u8], salt: &[u8], secret: &[u8], ad: &[u8],
                m_kib: u32, t: u32, p: u32, taglen: u32) -> Vec<u8> {
    // H0
    let mut h0in: Vec<u8> = Vec::new();
    for v in [p, taglen, m_kib, t, 0x13u32, 2u32] { h0in.extend_from_slice(&v.to_le_bytes()); }
    h0in.extend_from_slice(&(pwd.len() as u32).to_le_bytes());  h0in.extend_from_slice(pwd);
    h0in.extend_from_slice(&(salt.len() as u32).to_le_bytes()); h0in.extend_from_slice(salt);
    h0in.extend_from_slice(&(secret.len() as u32).to_le_bytes()); h0in.extend_from_slice(secret);
    h0in.extend_from_slice(&(ad.len() as u32).to_le_bytes());   h0in.extend_from_slice(ad);
    let h0 = blake2b(&h0in, 64);
    if std::env::var("DUMP_H0").is_ok() { println!("H0={}", hex(&h0)); }

    let mblocks: u32 = 4 * p * (m_kib / (4 * p));
    let lane_len = mblocks / p;
    let seg_len = lane_len / 4;
    let mut mem: Vec<u64> = vec![0u64; mblocks as usize * 128];

    for lane in 0..p {
        for j in 0..2u32 {
            let mut inp = h0.clone();
            inp.extend_from_slice(&j.to_le_bytes());
            inp.extend_from_slice(&lane.to_le_bytes());
            let blk = h_prime(&inp, 1024);
            let base = (lane * lane_len + j) as usize * 128;
            for k in 0..128 {
                let mut w = 0u64;
                for b in 0..8 { w |= (blk[k * 8 + b] as u64) << (8 * b); }
                mem[base + k] = w;
            }
        }
    }

    let zero = [0u64; 128];
    for pass in 0..t {
        for slice in 0..4u32 {
            for lane in 0..p {
                let data_independent = slice < 2 && pass == 0; // Argon2id
                let mut input = [0u64; 128];
                let mut addr = [0u64; 128];
                if data_independent {
                    input[0] = pass as u64;
                    input[1] = lane as u64;
                    input[2] = slice as u64;
                    input[3] = mblocks as u64;
                    input[4] = t as u64;
                    input[5] = 2u64; // Argon2id
                }
                let start = if pass == 0 && slice == 0 { 2u32 } else { 0u32 };
                if data_independent && start == 2 {
                    input[6] += 1;
                    let mut tmp = [0u64; 128];
                    fill_block(&zero, &input, &mut tmp, false);
                    fill_block(&zero, &tmp.clone(), &mut addr, false);
                    // addr = G(zero, G(zero, input))
                    let _ = tmp;
                }
                let mut curr = lane * lane_len + slice * seg_len + start;
                let mut prev = if curr % lane_len == 0 { curr + lane_len - 1 } else { curr - 1 };
                for i in start..seg_len {
                    if curr % lane_len == 1 { prev = curr - 1; }
                    let pseudo_rand: u64 = if data_independent {
                        if i % 128 == 0 {
                            input[6] += 1;
                            let mut tmp = [0u64; 128];
                            fill_block(&zero, &input, &mut tmp, false);
                            let mut a2 = [0u64; 128];
                            fill_block(&zero, &tmp, &mut a2, false);
                            addr = a2;
                        }
                        addr[(i % 128) as usize]
                    } else {
                        mem[prev as usize * 128]
                    };
                    let mut ref_lane = ((pseudo_rand >> 32) % p as u64) as u32;
                    if pass == 0 && slice == 0 { ref_lane = lane; }
                    let ref_index = index_alpha(pass, lane_len, seg_len, slice, i,
                                                (pseudo_rand & 0xFFFF_FFFF) as u32,
                                                ref_lane == lane);
                    let refoff = (ref_lane * lane_len + ref_index) as usize * 128;
                    let prevoff = prev as usize * 128;
                    let curroff = curr as usize * 128;
                    let pb: Vec<u64> = mem[prevoff..prevoff + 128].to_vec();
                    let rb: Vec<u64> = mem[refoff..refoff + 128].to_vec();
                    let mut nb: Vec<u64> = mem[curroff..curroff + 128].to_vec();
                    fill_block(&pb, &rb, &mut nb, pass != 0);
                    mem[curroff..curroff + 128].copy_from_slice(&nb);
                    curr += 1;
                    prev += 1;
                }
            }
        }
        if std::env::var("DUMP_PASS").is_ok() {
            let last = (mblocks as usize - 1) * 128;
            println!("pass{} B0[0]={:016x} B{}[127]={:016x}", pass, mem[0], mblocks - 1, mem[last + 127]);
        }
    }

    let mut c = [0u64; 128];
    for lane in 0..p {
        let off = ((lane * lane_len + lane_len - 1) as usize) * 128;
        for k in 0..128 { c[k] ^= mem[off + k]; }
    }
    let mut cb = Vec::with_capacity(1024);
    for k in 0..128 { cb.extend_from_slice(&c[k].to_le_bytes()); }
    h_prime(&cb, taglen as usize)
}

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{:02x}", x)).collect() }

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "run" {
        // run <pwd_hex> <salt_hex> <m> <t> <p> <len>
        let pwd = hexd(&args[2]); let salt = hexd(&args[3]);
        let m: u32 = args[4].parse().unwrap(); let t: u32 = args[5].parse().unwrap();
        let p: u32 = args[6].parse().unwrap(); let l: u32 = args[7].parse().unwrap();
        println!("{}", hex(&argon2id(&pwd, &salt, &[], &[], m, t, p, l)));
        return;
    }
    let mut fail = 0;

    // --- RFC 7693 Appendix A: BLAKE2b-512("abc")
    let want = "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d1\
7d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923";
    let got = hex(&blake2b(b"abc", 64));
    println!("blake2b-512(abc) {} {}", if got == want {"OK"} else {fail+=1;"FAIL"}, got);

    // --- RFC 9106 s5.3 Argon2id
    let pwd = vec![0x01u8; 32];
    let salt = vec![0x02u8; 16];
    let secret = vec![0x03u8; 8];
    let ad = vec![0x04u8; 12];
    let h0want = "288900de487eb42ae500c0007ed9252f1069eadec40d5765b485de6dc2437a67\
b8546a2f0acc1a0882db8fcf74714b472e94df421a5da1112ffa11434370a1e997";
    // (H0 printed below; compare visually against the RFC text.)
    std::env::set_var("DUMP_H0", "1");
    let tag = argon2id(&pwd, &salt, &secret, &ad, 32, 3, 4, 32);
    std::env::remove_var("DUMP_H0");
    let _ = h0want;
    let want2 = "0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659";
    let got2 = hex(&tag);
    println!("argon2id rfc9106 {} {}", if got2 == want2 {"OK"} else {fail+=1;"FAIL"}, got2);

    // --- cross-check vs RustCrypto argon2 0.5.3 (no secret / no AD)
    use argon2::{Argon2, Algorithm, Version, Params};
    let cases: Vec<(&str, &str, u32, u32, u32, u32)> = vec![
        ("password", "somesalt12345678", 8, 1, 1, 32),
        ("password", "somesalt12345678", 16, 2, 1, 32),
        ("password", "somesalt12345678", 32, 3, 4, 32),
        ("password", "somesalt12345678", 64, 4, 2, 64),
        ("", "0123456789abcdef", 32, 1, 1, 4),
        ("a-much-longer-passphrase-with-unicode-\u{00e9}", "abcdefgh", 128, 2, 3, 100),
        ("x", "saltsalt", 4096, 1, 1, 16),
        ("password", "somesalt12345678", 19456, 2, 1, 32),
    ];
    for (pw, sa, m, t, p, l) in cases {
        let mine = hex(&argon2id(pw.as_bytes(), sa.as_bytes(), &[], &[], m, t, p, l));
        let params = Params::new(m, t, p, Some(l as usize)).unwrap();
        let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut out = vec![0u8; l as usize];
        a2.hash_password_into(pw.as_bytes(), sa.as_bytes(), &mut out).unwrap();
        let theirs = hex(&out);
        println!("xcheck m={} t={} p={} l={} {} {}", m, t, p, l,
                 if mine == theirs {"OK"} else {fail+=1;"FAIL"}, mine);
        if mine != theirs { println!("   rustcrypto: {}", theirs); }
    }
    println!("failures={}", fail);
    std::process::exit(if fail == 0 {0} else {1});
}

fn hexd(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i+2], 16).unwrap()).collect()
}
