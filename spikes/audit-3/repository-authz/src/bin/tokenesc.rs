// Can a SCOPED, short-TTL publish token upgrade itself into a permanent,
// UNSCOPED auth key via the machine-pairing relay?
use mfb_repository::crypto;
use mfb_repository::store::Store;
use std::path::{Path, PathBuf};

const URL: &str = "http://127.0.0.1:8791";

fn login(client: &reqwest::blocking::Client, owner: &str, fp: &str, priv_key: &[u8]) -> String {
    let ch: serde_json::Value = client
        .post(format!("{URL}/auth/challenge"))
        .json(&serde_json::json!({"owner": owner, "authFingerprint": fp}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let id = ch["challengeId"].as_str().unwrap_or_else(|| panic!("{ch}"));
    let nonce = crypto::decode_bytes(ch["nonce"].as_str().unwrap(), "nonce").unwrap();
    let sig = crypto::sign(priv_key, &crypto::challenge_message(id, &nonce)).unwrap();
    let lg: serde_json::Value = client
        .post(format!("{URL}/auth/login"))
        .json(&serde_json::json!({"challengeId": id, "signature": crypto::encode_bytes(&sig)}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    lg["sessionToken"]
        .as_str()
        .unwrap_or_else(|| panic!("{lg}"))
        .to_string()
}

fn main() {
    let db = PathBuf::from("/tmp/repo-audit3/meta.db");
    let opened = Store::open_repository(&db, Path::new("/tmp/repo-audit3/blobs")).unwrap();
    let store = opened.store;
    let client = reqwest::blocking::Client::new();
    let owner = std::env::args().nth(1).unwrap_or_else(|| "orgA".to_string());

    // Account setup (server-side equivalent of `mfb repo register`).
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

    // The owner issues a CI token narrowly scoped to one package, 1 hour TTL.
    let (t_pub, t_priv) = crypto::generate_keypair();
    let tp = crypto::sign(
        &t_priv,
        &crypto::registration_message(crypto::ROLE_AUTH, &owner, &t_pub),
    )
    .unwrap();
    let scope = format!("{owner}#ci-only");
    let (_o, tkey, _exp) = store
        .issue_publish_token(&owner, &t_pub, &tp, &scope, 3600)
        .unwrap();
    println!("token fp = {}  scope = {scope}", tkey.fingerprint);

    // 1. The token opens a session (all it is supposed to be able to do).
    let session = login(&client, &owner, &tkey.fingerprint, &t_priv);

    // 2. In-scope /signing works, out-of-scope /signing is correctly refused.
    for ident in [format!("{owner}#ci-only"), format!("{owner}#flagship")] {
        let r = client
            .post(format!("{URL}/signing"))
            .json(&serde_json::json!({
                "owner": owner, "sessionToken": session, "ident": ident,
                "version": "1.0.0", "signingFingerprint": "cd".repeat(32)}))
            .send()
            .unwrap();
        println!("token /signing {ident} -> {}", r.status());
    }

    // 3. ESCALATION: the token session parks its own pairing blob ...
    let code = crypto::generate_pairing_code();
    let lookup = crypto::pairing_lookup(&code);
    let (blob, salt) = crypto::seal_pairing_blob(&code, b"anything").unwrap();
    let r = client
        .post(format!("{URL}/machines/link"))
        .json(&serde_json::json!({
            "owner": owner, "sessionToken": session, "lookup": lookup,
            "blob": crypto::encode_bytes(&blob), "salt": crypto::encode_bytes(&salt)}))
        .send()
        .unwrap();
    println!("/machines/link -> {} {}", r.status(), r.text().unwrap());

    // ... and then fetches it with a BRAND NEW keypair of its own choosing.
    let (n_pub, n_priv) = crypto::generate_keypair();
    let np = crypto::sign(
        &n_priv,
        &crypto::registration_message(crypto::ROLE_AUTH, &owner, &n_pub),
    )
    .unwrap();
    let r: serde_json::Value = client
        .post(format!("{URL}/machines/link/fetch"))
        .json(&serde_json::json!({
            "owner": owner, "lookup": lookup,
            "authKey": crypto::encode_bytes(&n_pub),
            "proof": crypto::encode_bytes(&np)}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let new_fp = r["authFingerprint"].as_str().unwrap_or_else(|| panic!("{r}"));
    println!("new UNSCOPED auth key fp = {new_fp}");

    // 4. The owner notices and revokes the CI token.
    let revoked = store.revoke_publish_token(&owner, &tkey.fingerprint).unwrap();
    println!("CI token revoked = {revoked}");

    // 5. The escalated key still logs in and attests the FLAGSHIP package.
    let session2 = login(&client, &owner, new_fp, &n_priv);
    let r = client
        .post(format!("{URL}/signing"))
        .json(&serde_json::json!({
            "owner": owner, "sessionToken": session2,
            "ident": format!("{owner}#flagship"),
            "version": "6.6.6", "signingFingerprint": "ef".repeat(32)}))
        .send()
        .unwrap();
    println!("escalated key /signing {owner}#flagship -> {}", r.status());
    println!("body: {}", r.text().unwrap());
}
