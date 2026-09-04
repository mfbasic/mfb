use mfb_repository::crypto;
use mfb_repository::store::{PublishMetadata, Store};
use std::path::{Path, PathBuf};

const URL: &str = "http://127.0.0.1:8791";

fn reg(store: &Store, name: &str) -> (Vec<u8>, Vec<u8>, i64, String) {
    let (auth_pub, auth_priv) = crypto::generate_keypair();
    let (id_pub, id_priv) = crypto::generate_keypair();
    let ap = crypto::sign(
        &auth_priv,
        &crypto::registration_message(crypto::ROLE_AUTH, name, &auth_pub),
    )
    .unwrap();
    let ip = crypto::sign(
        &id_priv,
        &crypto::registration_message(crypto::ROLE_IDENT, name, &id_pub),
    )
    .unwrap();
    let (owner, authk, _identk) = store
        .register_owner(name, &auth_pub, &ap, &id_pub, &ip)
        .unwrap();
    (auth_priv, id_priv, owner.id, authk.fingerprint)
}

fn login(client: &reqwest::blocking::Client, owner: &str, fp: &str, auth_priv: &[u8]) -> String {
    let ch: serde_json::Value = client
        .post(format!("{URL}/auth/challenge"))
        .json(&serde_json::json!({"owner": owner, "authFingerprint": fp}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let id = ch["challengeId"].as_str().expect("challengeId");
    let nonce = crypto::decode_bytes(ch["nonce"].as_str().unwrap(), "nonce").unwrap();
    let sig = crypto::sign(auth_priv, &crypto::challenge_message(id, &nonce)).unwrap();
    let lg: serde_json::Value = client
        .post(format!("{URL}/auth/login"))
        .json(&serde_json::json!({"challengeId": id, "signature": crypto::encode_bytes(&sig)}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    lg["sessionToken"].as_str().expect("sessionToken").to_string()
}

fn main() {
    let db = PathBuf::from("/tmp/repo-audit3/meta.db");
    let opened = Store::open_repository(&db, Path::new("/tmp/repo-audit3/blobs")).unwrap();
    let store = opened.store;
    let client = reqwest::blocking::Client::new();

    let suffix = std::env::args().nth(1).unwrap_or_else(|| "1".to_string());
    let alice = format!("alice{suffix}");
    let bob = format!("bob{suffix}");

    let (a_auth, a_ident, a_id, a_fp) = reg(&store, &alice);
    let (_b_auth, _b_ident, _b_id, _b_fp) = reg(&store, &bob);

    let ident = format!("{alice}#widget");
    store
        .publish_package_version(
            a_id,
            &ident,
            "1.0.0",
            &"aa".repeat(32),
            "blobs/aa.mfp",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    // Transfer alice#widget to bob, entirely through the store's own
    // (correctly-guarded) transfer path.
    store.create_transfer_offer(&ident, &alice, &bob).unwrap();
    store.accept_transfer(&ident, &bob).unwrap();
    println!(
        "package owner after transfer = {:?}",
        store.package_owner(&ident).unwrap().map(|o| o.owner_display)
    );

    // Alice no longer owns the package. Log in as alice over HTTP and try to
    // yank it via POST /release-state.
    let session = login(&client, &alice, &a_fp, &a_auth);
    let sig = crypto::sign(
        &a_ident,
        &crypto::release_state_message(&ident, "1.0.0", "yanked"),
    )
    .unwrap();
    let resp = client
        .post(format!("{URL}/release-state"))
        .json(&serde_json::json!({
            "owner": alice,
            "ident": ident,
            "version": "1.0.0",
            "state": "yanked",
            "sessionToken": session,
            "identSignature": crypto::encode_bytes(&sig),
        }))
        .send()
        .unwrap();
    println!("former-owner yank status = {}", resp.status());
    println!("former-owner yank body   = {}", resp.text().unwrap());

    // Former owner asks the registry to attest a NEW version of the package
    // she no longer owns.
    let sresp = client
        .post(format!("{URL}/signing"))
        .json(&serde_json::json!({
            "owner": alice,
            "sessionToken": session,
            "ident": ident,
            "version": "9.9.9",
            "signingFingerprint": "ab".repeat(32),
        }))
        .send()
        .unwrap();
    println!("former-owner /signing status = {}", sresp.status());
    println!("former-owner /signing body   = {}", sresp.text().unwrap());

    // And the real owner (bob) tries the same thing.
    let b_session = login(&client, &bob, &_b_fp, &_b_auth);
    let bsig = crypto::sign(
        &_b_ident,
        &crypto::release_state_message(&ident, "1.0.0", "available"),
    )
    .unwrap();
    let bresp = client
        .post(format!("{URL}/release-state"))
        .json(&serde_json::json!({
            "owner": bob,
            "ident": ident,
            "version": "1.0.0",
            "state": "available",
            "sessionToken": b_session,
            "identSignature": crypto::encode_bytes(&bsig),
        }))
        .send()
        .unwrap();
    println!("current-owner unyank status = {}", bresp.status());
    println!("current-owner unyank body   = {}", bresp.text().unwrap());
}
