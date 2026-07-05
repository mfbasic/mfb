//! RFC 6962 Merkle tree math for the transparency log (plan-23-B3).
//!
//! Leaf hash:  SHA-256(0x00 || leaf bytes)
//! Node hash:  SHA-256(0x01 || left || right)
//! MTH, audit (inclusion) paths, and consistency proofs follow RFC 6962 §2.1.
//! The tree is computed from the ordered leaf hashes; the append-only store
//! keeps one leaf hash per log entry.

use crate::crypto;

pub const HASH_LEN: usize = 32;

pub fn leaf_hash(leaf: &[u8]) -> [u8; HASH_LEN] {
    let mut input = Vec::with_capacity(leaf.len() + 1);
    input.push(0x00);
    input.extend_from_slice(leaf);
    crypto::sha256(&input)
}

fn node_hash(left: &[u8; HASH_LEN], right: &[u8; HASH_LEN]) -> [u8; HASH_LEN] {
    let mut input = Vec::with_capacity(1 + HASH_LEN * 2);
    input.push(0x01);
    input.extend_from_slice(left);
    input.extend_from_slice(right);
    crypto::sha256(&input)
}

/// The largest power of two strictly less than `n` (n >= 2).
fn split_point(n: usize) -> usize {
    let mut k = 1usize;
    while k * 2 < n {
        k *= 2;
    }
    k
}

/// Merkle tree head over the ordered leaf hashes (RFC 6962 §2.1 MTH).
/// The empty tree hashes to SHA-256 of the empty string.
pub fn root(leaves: &[[u8; HASH_LEN]]) -> [u8; HASH_LEN] {
    match leaves.len() {
        0 => crypto::sha256(b""),
        1 => leaves[0],
        n => {
            let k = split_point(n);
            node_hash(&root(&leaves[..k]), &root(&leaves[k..]))
        }
    }
}

/// Audit path for leaf `index` in a tree of `leaves` (RFC 6962 §2.1.1 PATH).
pub fn inclusion_path(index: usize, leaves: &[[u8; HASH_LEN]]) -> Vec<[u8; HASH_LEN]> {
    let n = leaves.len();
    if n <= 1 {
        return Vec::new();
    }
    let k = split_point(n);
    if index < k {
        let mut path = inclusion_path(index, &leaves[..k]);
        path.push(root(&leaves[k..]));
        path
    } else {
        let mut path = inclusion_path(index - k, &leaves[k..]);
        path.push(root(&leaves[..k]));
        path
    }
}

/// Verify an audit path: recompute the root from the leaf hash and compare.
pub fn verify_inclusion(
    index: usize,
    tree_size: usize,
    leaf: &[u8; HASH_LEN],
    path: &[[u8; HASH_LEN]],
    expected_root: &[u8; HASH_LEN],
) -> Result<(), String> {
    if index >= tree_size {
        return Err("inclusion proof index is outside the tree".to_string());
    }
    // RFC 6962 §2.1.3 verification walk.
    let mut fn_index = index;
    let mut sn = tree_size - 1;
    let mut hash = *leaf;
    for sibling in path {
        if sn == 0 {
            return Err("inclusion proof is too long".to_string());
        }
        if fn_index % 2 == 1 || fn_index == sn {
            hash = node_hash(sibling, &hash);
            if fn_index % 2 == 0 {
                while fn_index % 2 == 0 && fn_index != 0 {
                    fn_index /= 2;
                    sn /= 2;
                }
            }
        } else {
            hash = node_hash(&hash, sibling);
        }
        fn_index /= 2;
        sn /= 2;
    }
    if sn != 0 {
        return Err("inclusion proof is too short".to_string());
    }
    if &hash != expected_root {
        return Err("inclusion proof does not match the checkpoint root".to_string());
    }
    Ok(())
}

/// Consistency proof between tree sizes `m` and `n` over `leaves[..n]`
/// (RFC 6962 §2.1.2 PROOF).
pub fn consistency_path(m: usize, leaves: &[[u8; HASH_LEN]]) -> Vec<[u8; HASH_LEN]> {
    let n = leaves.len();
    if m == 0 || m > n {
        return Vec::new();
    }
    if m == n {
        return Vec::new();
    }
    subproof(m, leaves, true)
}

fn subproof(m: usize, leaves: &[[u8; HASH_LEN]], complete: bool) -> Vec<[u8; HASH_LEN]> {
    let n = leaves.len();
    if m == n {
        if complete {
            return Vec::new();
        }
        return vec![root(leaves)];
    }
    let k = split_point(n);
    if m <= k {
        let mut path = subproof(m, &leaves[..k], complete);
        path.push(root(&leaves[k..]));
        path
    } else {
        let mut path = subproof(m - k, &leaves[k..], false);
        path.push(root(&leaves[..k]));
        path
    }
}

/// Verify a consistency proof between an old head `(m, old_root)` and a new
/// head `(n, new_root)` (RFC 6962 §2.1.4).
pub fn verify_consistency(
    m: usize,
    n: usize,
    old_root: &[u8; HASH_LEN],
    new_root: &[u8; HASH_LEN],
    path: &[[u8; HASH_LEN]],
) -> Result<(), String> {
    if m == n {
        if old_root == new_root && path.is_empty() {
            return Ok(());
        }
        return Err("inconsistent same-size roots".to_string());
    }
    if m == 0 {
        // Any tree is consistent with the empty tree.
        if path.is_empty() {
            return Ok(());
        }
        return Err("consistency proof for an empty tree must be empty".to_string());
    }
    if m > n {
        return Err("consistency proof sizes are reversed (rollback)".to_string());
    }
    let mut path = path.iter();
    // If m is a power of two, the old root is an internal node of the new
    // tree; seed with it. Otherwise the first path element seeds the walk.
    let mut fn_index = m - 1;
    let mut sn = n - 1;
    while fn_index % 2 == 1 {
        fn_index /= 2;
        sn /= 2;
    }
    let (mut fr, mut sr) = if fn_index == 0 {
        (*old_root, *old_root)
    } else {
        let Some(first) = path.next() else {
            return Err("consistency proof is too short".to_string());
        };
        (*first, *first)
    };
    let mut fn_index = m - 1;
    let mut sn = n - 1;
    while fn_index % 2 == 1 {
        fn_index /= 2;
        sn /= 2;
    }
    for sibling in path {
        if sn == 0 {
            return Err("consistency proof is too long".to_string());
        }
        if fn_index % 2 == 1 || fn_index == sn {
            fr = node_hash(sibling, &fr);
            sr = node_hash(sibling, &sr);
            while fn_index % 2 == 0 && fn_index != 0 {
                fn_index /= 2;
                sn /= 2;
            }
        } else {
            sr = node_hash(&sr, sibling);
        }
        fn_index /= 2;
        sn /= 2;
    }
    if sn != 0 {
        return Err("consistency proof is too short".to_string());
    }
    if &fr != old_root {
        return Err("consistency proof does not reproduce the old root".to_string());
    }
    if &sr != new_root {
        return Err("consistency proof does not reproduce the new root".to_string());
    }
    Ok(())
}

/// Signing input for the server-signed tree head (checkpoint).
pub fn checkpoint_signing_input(size: u64, root: &[u8; HASH_LEN]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"mfb-log-checkpoint-v1\0");
    message.extend_from_slice(&size.to_le_bytes());
    message.extend_from_slice(root);
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6962 §2.1.1 example inputs: the seven-leaf tree from the RFC's
    // diagrams, using distinct one-byte leaves.
    fn leaves(n: usize) -> Vec<[u8; HASH_LEN]> {
        (0..n).map(|index| leaf_hash(&[index as u8])).collect()
    }

    #[test]
    fn empty_tree_root_is_sha256_of_empty_string() {
        assert_eq!(
            hex::encode(root(&[])),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn inclusion_proofs_verify_for_every_leaf_and_size() {
        for n in 1..=16usize {
            let leaves = leaves(n);
            let tree_root = root(&leaves);
            for index in 0..n {
                let path = inclusion_path(index, &leaves);
                verify_inclusion(index, n, &leaves[index], &path, &tree_root)
                    .unwrap_or_else(|err| panic!("n={n} index={index}: {err}"));
                // A different leaf must not verify with the same path.
                let wrong = leaf_hash(b"wrong");
                assert!(
                    verify_inclusion(index, n, &wrong, &path, &tree_root).is_err(),
                    "n={n} index={index}: tampered leaf accepted"
                );
            }
        }
    }

    #[test]
    fn consistency_proofs_verify_across_all_growth_steps() {
        for n in 1..=16usize {
            let all = leaves(n);
            let new_root = root(&all);
            for m in 1..=n {
                let old_root = root(&all[..m]);
                let path = consistency_path(m, &all);
                verify_consistency(m, n, &old_root, &new_root, &path)
                    .unwrap_or_else(|err| panic!("m={m} n={n}: {err}"));
                // A rollback (sizes reversed) is always rejected.
                if m < n {
                    assert!(verify_consistency(n, m, &new_root, &old_root, &path).is_err());
                }
            }
        }
    }

    #[test]
    fn tampering_any_leaf_changes_the_root() {
        let mut all = leaves(8);
        let before = root(&all);
        all[3] = leaf_hash(b"tampered");
        assert_ne!(before, root(&all));
    }
}
