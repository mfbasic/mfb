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

pub fn registration_message(owner: &str, public_key: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"mfb-repo-register-v1\0");
    message.extend_from_slice(owner.as_bytes());
    message.push(0);
    message.extend_from_slice(public_key);
    message
}

pub fn challenge_message(challenge_id: &str, nonce: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"mfb-repo-auth-v1\0");
    message.extend_from_slice(challenge_id.as_bytes());
    message.push(0);
    message.extend_from_slice(nonce);
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
}
