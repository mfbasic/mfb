use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use sha2::{Digest, Sha256};

pub const PUBLIC_KEY_LEN: usize = 32;
pub const PRIVATE_KEY_LEN: usize = 32;
pub const SIGNATURE_LEN: usize = 64;

pub fn generate_keypair() -> (Vec<u8>, Vec<u8>) {
    let signing = SigningKey::generate(&mut OsRng);
    (
        signing.verifying_key().to_bytes().to_vec(),
        signing.to_bytes().to_vec(),
    )
}

pub fn public_from_private(private_key: &[u8]) -> Result<Vec<u8>, String> {
    let bytes: [u8; PRIVATE_KEY_LEN] = private_key
        .try_into()
        .map_err(|_| "malformed local private key".to_string())?;
    let signing = SigningKey::from_bytes(&bytes);
    Ok(signing.verifying_key().to_bytes().to_vec())
}

pub fn sign(private_key: &[u8], message: &[u8]) -> Result<Vec<u8>, String> {
    let bytes: [u8; PRIVATE_KEY_LEN] = private_key
        .try_into()
        .map_err(|_| "malformed local private key".to_string())?;
    let signing = SigningKey::from_bytes(&bytes);
    Ok(signing.sign(message).to_bytes().to_vec())
}

pub fn verify(public_key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), String> {
    let public_bytes: [u8; PUBLIC_KEY_LEN] = public_key
        .try_into()
        .map_err(|_| "malformed public key".to_string())?;
    let signature_bytes: [u8; SIGNATURE_LEN] = signature
        .try_into()
        .map_err(|_| "invalid signature".to_string())?;
    let verifying =
        VerifyingKey::from_bytes(&public_bytes).map_err(|_| "malformed public key".to_string())?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying
        .verify(message, &signature)
        .map_err(|_| "invalid signature".to_string())
}

pub fn fingerprint(public_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public_key);
    hex::encode(hasher.finalize())
}

pub fn encode_bytes(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn decode_bytes(value: &str, field: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| format!("malformed {field}"))
}

/// Key roles carried by the registration proof-of-possession message. The
/// role is baked into the signed bytes so a proof made for one role can never
/// be replayed as a proof for the other (plan-23 Phase A1).
pub const ROLE_AUTH: &str = "auth";
pub const ROLE_IDENT: &str = "ident";

pub fn registration_message(role: &str, owner: &str, public_key: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"mfb-repo-register-v1\0");
    message.extend_from_slice(role.as_bytes());
    message.push(0);
    message.extend_from_slice(owner.as_bytes());
    message.push(0);
    message.extend_from_slice(public_key);
    message
}

/// Domain-tagged signing input for the ident-signed build proof (plan-23 §5).
pub fn proof_signing_input(proof_json: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"MFP-PROOF-v1\0");
    message.extend_from_slice(proof_json);
    message
}

/// Domain-tagged signing input for the server-signed attestation (plan-23 §5).
pub fn attestation_signing_input(attestation_json: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"MFP-ATTEST-v1\0");
    message.extend_from_slice(attestation_json);
    message
}

/// Domain-tagged signing input for the container v1.0 package signature:
/// `"MFP-PACKAGE-v2\0" || SHA-256(header bytes [0 .. offset of signature))`
/// (plan-23 §4). The caller passes the raw signed prefix; the hash is taken
/// here so every signer/verifier agrees on the construction.
pub fn package_signing_input(signed_prefix: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"MFP-PACKAGE-v2\0");
    message.extend_from_slice(&sha256(signed_prefix));
    message
}

/// One-time pairing code for a machine link (plan-23 §3.2): 25 random
/// base32 characters in five groups (~125 bits), displayed on the old
/// machine and typed on the new. High-entropy so the code-derived key
/// cannot be brute-forced, even by the relaying server.
pub fn generate_pairing_code() -> String {
    use rand::RngCore;
    const ALPHABET: &[u8; 32] = b"abcdefghjkmnpqrstuvwxyz23456789A";
    let mut raw = [0u8; 25];
    rand::thread_rng().fill_bytes(&mut raw);
    let mut code = String::with_capacity(29);
    for (index, byte) in raw.iter().enumerate() {
        if index > 0 && index % 5 == 0 {
            code.push('-');
        }
        code.push(ALPHABET[(*byte & 31) as usize] as char);
    }
    code
}

/// The server-visible lookup key for a pairing code. One-way, so the server
/// (which relays the blob) never learns the code the blob key derives from.
pub fn pairing_lookup(code: &str) -> String {
    let mut message = Vec::new();
    message.extend_from_slice(b"mfb-pairing-lookup-v1\0");
    message.extend_from_slice(code.as_bytes());
    hex::encode(sha256(&message))
}

fn pairing_key(code: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    use argon2::Argon2;
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(code.as_bytes(), salt, &mut key)
        .map_err(|err| format!("failed to derive pairing key: {err}"))?;
    Ok(key)
}

/// Encrypt the ident keypair under the pairing code (argon2id +
/// ChaCha20-Poly1305). Returns `(blob, salt)`; the blob is
/// `nonce(12) || ciphertext+tag`.
pub fn seal_pairing_blob(code: &str, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng as AeadOsRng};
    use chacha20poly1305::ChaCha20Poly1305;
    use rand::RngCore;
    let mut salt = vec![0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = pairing_key(code, &salt)?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    let nonce = ChaCha20Poly1305::generate_nonce(&mut AeadOsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| "failed to encrypt pairing blob".to_string())?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ciphertext);
    Ok((blob, salt))
}

/// Decrypt a pairing blob with the typed code. A wrong code fails the AEAD
/// tag — the blob is unreadable without the code.
pub fn open_pairing_blob(code: &str, blob: &[u8], salt: &[u8]) -> Result<Vec<u8>, String> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::ChaCha20Poly1305;
    if blob.len() < 13 {
        return Err("malformed pairing blob".to_string());
    }
    let key = pairing_key(code, salt)?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    let (nonce, ciphertext) = blob.split_at(12);
    cipher
        .decrypt(nonce.into(), ciphertext)
        .map_err(|_| "pairing code does not decrypt this blob".to_string())
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hash = [0; 32];
    hash.copy_from_slice(&digest);
    hash
}

pub fn challenge_message(challenge_id: &str, nonce: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"mfb-repo-auth-v1\0");
    message.extend_from_slice(challenge_id.as_bytes());
    message.push(0);
    message.extend_from_slice(nonce);
    message
}

/// Ident-signed auth-key revocation (plan-23-B1): binds the server challenge
/// AND the fingerprint being revoked, so a signature can neither be replayed
/// nor redirected at a different machine's key.
pub fn revocation_message(challenge_id: &str, nonce: &[u8], fingerprint: &str) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"mfb-repo-revoke-v1\0");
    message.extend_from_slice(challenge_id.as_bytes());
    message.push(0);
    message.extend_from_slice(nonce);
    message.push(0);
    message.extend_from_slice(fingerprint.as_bytes());
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_and_verifies_messages() {
        let (public, private) = generate_keypair();
        let message = b"hello";
        let signature = sign(&private, message).unwrap();
        verify(&public, message, &signature).unwrap();
        assert!(verify(&public, b"other", &signature).is_err());
    }

    #[test]
    fn public_key_round_trips_from_private_key() {
        let (public, private) = generate_keypair();
        assert_eq!(public_from_private(&private).unwrap(), public);
    }

    #[test]
    fn registration_message_separates_roles() {
        let (public, _private) = generate_keypair();
        assert_ne!(
            registration_message(ROLE_AUTH, "alice", &public),
            registration_message(ROLE_IDENT, "alice", &public)
        );
    }

    #[test]
    fn pairing_blob_round_trips_and_rejects_wrong_code() {
        let code = generate_pairing_code();
        assert_eq!(code.len(), 29);
        let plaintext = b"ident-keypair-goes-here".to_vec();
        let (blob, salt) = seal_pairing_blob(&code, &plaintext).unwrap();
        assert_ne!(blob, plaintext);
        assert_eq!(open_pairing_blob(&code, &blob, &salt).unwrap(), plaintext);
        // The blob is unreadable without the exact code.
        let wrong = generate_pairing_code();
        assert!(open_pairing_blob(&wrong, &blob, &salt).is_err());
        // The lookup is deterministic per code and never equals the code.
        assert_eq!(pairing_lookup(&code), pairing_lookup(&code));
        assert_ne!(pairing_lookup(&code), pairing_lookup(&wrong));
        assert_eq!(pairing_lookup(&code).len(), 64);
    }

    #[test]
    fn signing_inputs_are_domain_separated() {
        let payload = b"{\"owner\":\"alice\"}";
        let proof = proof_signing_input(payload);
        let attestation = attestation_signing_input(payload);
        assert_ne!(proof, attestation);
        assert!(proof.starts_with(b"MFP-PROOF-v1\0"));
        assert!(attestation.starts_with(b"MFP-ATTEST-v1\0"));
        let package = package_signing_input(payload);
        assert!(package.starts_with(b"MFP-PACKAGE-v2\0"));
        assert_eq!(package.len(), b"MFP-PACKAGE-v2\0".len() + 32);
    }
}
