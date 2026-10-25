//! The refusals of the transfer-chain walk and the release check: each test
//! serves one index or release a hostile (or broken) registry could serve and
//! pins the rule that refuses it, by its diagnostic text.

use super::*;

fn chain_detail(result: Result<VerifiedChain, TrustError>) -> String {
    match result {
        Err(TrustError::Untrusted(detail)) => detail,
        Err(other) => panic!("expected an untrusted chain, got {other:?}"),
        Ok(_) => panic!("expected an untrusted chain, got a verified one"),
    }
}

/// A release check with every field chosen by the caller.
fn check_release(
    world: &World,
    index: &IndexResponse,
    anchor: Anchor,
    version: &IndexVersion,
    content_hash: &str,
    signer: &[u8],
    author: &str,
) -> Result<Trusted, TrustError> {
    verify_release(
        &world.server_public,
        index,
        &world.chain_of(),
        anchor,
        &Release {
            version,
            content_hash,
            header_ident_key: signer,
            header_author: author,
        },
    )
}

#[test]
fn an_untrusted_error_reports_its_rule_and_displays_its_detail() {
    let error = TrustError::Untrusted("the detail".to_string());
    assert_eq!(error.rule(), "PACKAGE_IDENT_KEY_UNTRUSTED");
    assert_eq!(error.to_string(), "the detail");
    let reanchored = TrustError::Reanchored("moved".to_string());
    assert_eq!(reanchored.to_string(), "moved");
}

#[test]
fn a_link_from_an_owner_who_did_not_hold_the_package_is_refused() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    let mallory = world.owner("mallory");
    world.publish("1.0.0", &alice);
    // A correctly signed link, but mallory never held alice's package.
    world.transfer(&mallory, &bob);
    let index = world.index(&alice, &[&bob]);
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "1.0.0", &alice.public));
    assert_eq!(
        detail,
        "transfer link 0 of alice#toolbox is from `mallory`, but `alice` held the package"
    );
}

#[test]
fn a_link_whose_binding_names_another_recipient_is_refused() {
    let (world, alice, bob) = alice_to_bob();
    let carol = world.owner("carol");
    let mut index = world.index(&alice, &[&bob]);
    // A valid server binding, but for carol, on a link that moves the package
    // to bob.
    index.transfers[0].to_binding = world.binding(&carol);
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "1.0.0", &alice.public));
    assert_eq!(
        detail,
        "transfer link 0 of alice#toolbox binds `carol` but moves the package to `bob`"
    );
}

#[test]
fn an_unreadable_ident_chain_is_refused_naming_the_owner() {
    let (world, alice, bob) = alice_to_bob();
    let index = world.index(&alice, &[&bob]);
    let chain_of = |owner: &str| -> Result<Vec<IdentChainLink>, String> {
        if owner == "bob" {
            Err("connection reset".to_string())
        } else {
            Ok(Vec::new())
        }
    };
    let detail = chain_detail(verify_chain(
        &world.server_public,
        &index,
        &chain_of,
        pinned(&alice),
    ));
    assert_eq!(
        detail,
        "the ident chain of `bob` could not be read: connection reset"
    );
}

#[test]
fn an_offer_made_with_a_key_from_before_the_accept_is_refused() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let mut bob = world.owner("bob");
    let carol = world.owner("carol");
    world.publish("1.0.0", &alice);
    let bob_old = bob.clone();
    world.rotate(&mut bob);
    // bob accepts with his rotated key, then offers the package on with the
    // key he retired before he ever held it: an empty tenure.
    world.transfer(&alice, &bob);
    world.transfer(&bob_old, &carol);
    let index = world.index(&alice, &[&bob, &carol]);
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "1.0.0", &alice.public));
    assert_eq!(
        detail,
        "`bob` offered alice#toolbox away (transfer link 1) with a key that is not on their \
         ident chain after the key they accepted it with"
    );
}

#[test]
fn a_link_with_an_undecodable_signature_is_refused_as_malformed() {
    let (world, alice, bob) = alice_to_bob();
    let mut index = world.index(&alice, &[&bob]);
    index.transfers[0].accept_signature = Some("not base64!".to_string());
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "1.0.0", &alice.public));
    assert_eq!(
        detail,
        "transfer link 0 is malformed: malformed acceptSignature"
    );
}

#[test]
fn an_undecodable_fork_the_walk_has_already_passed_is_no_tenure_key() {
    let mut world = World::new();
    let mut alice = world.owner("alice");
    let alice_0 = alice.clone();
    world.publish("1.0.0", &alice);
    world.rotate(&mut alice);
    // A second entry out of alice's first key, after the rotation the walk
    // follows: the walk has moved past that key when it meets the entry, so
    // it never decodes it, and its undecodable new key is no tenure key.
    world
        .chains
        .borrow_mut()
        .entry("alice".to_string())
        .or_default()
        .push(IdentChainLink {
            old_key: crypto::encode_bytes(&alice_0.public),
            new_key: "not base64!".to_string(),
            signature: String::new(),
            issued: 0,
        });
    world.publish("1.1.0", &alice);
    let index = world.index(&alice, &[]);
    for (version, signer) in [("1.0.0", &alice_0), ("1.1.0", &alice)] {
        let trusted = world
            .check(&index, pinned(&alice_0), version, &signer.public)
            .unwrap_or_else(|err| panic!("{version}: {err}"));
        assert_eq!(trusted.key, signer.public);
    }
}

#[test]
fn a_release_whose_bytes_do_not_match_the_index_hash_is_refused() {
    let (world, alice, bob) = alice_to_bob();
    let index = world.index(&alice, &[&bob]);
    let version = &index.versions[0];
    let detail = untrusted_detail(check_release(
        &world,
        &index,
        pinned(&alice),
        version,
        "hash-of-other-bytes",
        &alice.public,
        "alice",
    ));
    assert_eq!(
        detail,
        "release 1.0.0 does not match the registry's content hash for it"
    );
}

#[test]
fn a_release_dated_past_the_served_transfers_is_refused() {
    let (world, alice, bob) = alice_to_bob();
    let index = world.index(&alice, &[&bob]);
    let mut version = index.versions[0].clone();
    version.epoch = 2;
    let detail = untrusted_detail(check_release(
        &world,
        &index,
        pinned(&alice),
        &version,
        &version.hash,
        &alice.public,
        "alice",
    ));
    assert_eq!(
        detail,
        "release 1.0.0 names ownership epoch 2, past the 1 transfer(s) served"
    );
}

#[test]
fn a_release_whose_header_author_is_not_its_publisher_is_refused() {
    let (world, alice, bob) = alice_to_bob();
    let index = world.index(&alice, &[&bob]);
    let version = &index.versions[0];
    let detail = untrusted_detail(check_release(
        &world,
        &index,
        pinned(&alice),
        version,
        &version.hash,
        &alice.public,
        "bob",
    ));
    assert_eq!(
        detail,
        "release 1.0.0 is published by `alice` but its header names author `bob`"
    );
}

#[test]
fn a_release_published_by_someone_other_than_its_epochs_owner_is_refused() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    // A release recorded as bob's while alice still owned the package.
    world.publish("1.0.0", &bob);
    let index = world.index(&alice, &[]);
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "1.0.0", &bob.public));
    assert_eq!(
        detail,
        "release 1.0.0 is published by `bob`, but `alice` owned the package in its ownership \
         epoch"
    );
}

#[test]
fn an_origin_release_past_a_legacy_link_under_a_foreign_key_is_refused() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    let stranger = world.owner("alice");
    world.publish("1.0.0", &alice);
    world.legacy_transfer("alice", "bob");
    world.legacy_transfer("bob", "alice");
    world.publish("2.0.0", &alice);
    let index = world.index(&alice, &[&bob, &alice]);
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "2.0.0", &stranger.public));
    assert_eq!(
        detail,
        format!(
            "release 2.0.0 is signed by an ident key (fingerprint {}) that is not trusted for \
             `alice`",
            crypto::fingerprint(&stranger.public)
        )
    );
}

#[test]
fn owner_key_answers_only_for_a_key_trusted_for_that_owner() {
    let mut world = World::new();
    let mut alice = world.owner("alice");
    let bob = world.owner("bob");
    let alice_0 = alice.clone();
    world.publish("1.0.0", &alice);
    world.rotate(&mut alice);
    world.transfer(&alice, &bob);
    let index = world.index(&alice, &[&bob]);
    let chain = verify_chain(
        &world.server_public,
        &index,
        &world.chain_of(),
        pinned(&alice_0),
    )
    .unwrap();
    // alice's retired key is trusted for her and maps to her current key.
    assert_eq!(
        chain.owner_key("ALICE", &alice_0.public),
        Some(alice.public.clone())
    );
    assert_eq!(
        chain.owner_key("bob", &bob.public),
        Some(bob.public.clone())
    );
    // A key is trusted for its owner only.
    assert_eq!(chain.owner_key("bob", &alice_0.public), None);
    assert_eq!(chain.owner_key("carol", &bob.public), None);
}
