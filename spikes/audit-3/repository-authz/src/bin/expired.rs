// Does an EXPIRED publish token still authenticate?
use mfb_repository::crypto;
use mfb_repository::store::Store;
use std::path::{Path, PathBuf};

const URL: &str = "http://127.0.0.1:8791";

fn main() {
    let db = PathBuf::from("/tmp/repo-audit3/meta.db");
    let opened = Store::open_repository(&db, Path::new("/tmp/repo-audit3/blobs")).unwrap();
    let store = opened.store;
    let client = reqwest::blocking::Client::new();
    let owner = std::env::args().nth(1).unwrap_or_else(|| "expA".to_string());

    let (a_pub, a_priv) = crypto::generate_keypair();
    let (i_pub, i_priv) = crypto::generate_keypair();
    let ap = crypto::sign(
        &a_priv,
        &crypto::registration_message(crypto::ROLE_AUTH, &owner, &a_pub),
    )
    .unwrap();
    let ip = crypto::sign(
        &i_priv,
        &crypto::registration_message(crypto::ROLE_IDENT, &owner, &i_pub),
    )
    .unwrap();
    store
        .register_owner(&owner, &a_pub, &ap, &i_pub, &ip)
        .unwrap();

    let (t_pub, t_priv) = crypto::generate_keypair();
    let tp = crypto::sign(
        &t_priv,
        &crypto::registration_message(crypto::ROLE_AUTH, &owner, &t_pub),
    )
    .unwrap();
    let (_o, tkey, exp) = store
        .issue_publish_token(&owner, &t_pub, &tp, &format!("{owner}#x"), 1)
        .unwrap();
    println!("token expires_at = {exp}");
    std::thread::sleep(std::time::Duration::from_secs(3));
    println!("now              = {}", mfb_repository::store::now_unix());

    let ch: serde_json::Value = client
        .post(format!("{URL}/auth/challenge"))
        .json(&serde_json::json!({"owner": owner, "authFingerprint": tkey.fingerprint}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    println!("challenge for EXPIRED token -> {ch}");
    let Some(id) = ch["challengeId"].as_str() else {
        return;
    };
    let nonce = crypto::decode_bytes(ch["nonce"].as_str().unwrap(), "nonce").unwrap();
    let sig = crypto::sign(&t_priv, &crypto::challenge_message(id, &nonce)).unwrap();
    let lg: serde_json::Value = client
        .post(format!("{URL}/auth/login"))
        .json(&serde_json::json!({"challengeId": id, "signature": crypto::encode_bytes(&sig)}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let Some(session) = lg["sessionToken"].as_str() else {
        println!("login refused: {lg}");
        return;
    };
    println!("EXPIRED token obtained a session (len {})", session.len());

    // PUT /blob with the expired token's session.
    let body = b"expired-token-can-still-upload".to_vec();
    let hash = hex_of(&crypto::sha256(&body));
    let r = client
        .put(format!("{URL}/blob/{hash}"))
        .header("Authorization", format!("Bearer {session}"))
        .body(body)
        .send()
        .unwrap();
    println!("PUT /blob with expired token -> {}", r.status());

    let r = client
        .post(format!("{URL}/signing"))
        .json(&serde_json::json!({
            "owner": owner, "sessionToken": session, "ident": format!("{owner}#x"),
            "version": "1.0.0", "signingFingerprint": "12".repeat(32)}))
        .send()
        .unwrap();
    println!("/signing with expired token -> {} {}", r.status(), r.text().unwrap());
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
