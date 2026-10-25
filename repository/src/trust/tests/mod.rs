use super::*;
use crate::server::{OwnerBinding, Publisher};
use std::cell::RefCell;

mod refusals;

const IDENT: &str = "alice#toolbox";

/// A registry and its owners, built in memory: every owner's rotation chain,
/// the release list, and the signed transfer links.
struct World {
    server_public: Vec<u8>,
    server_private: Vec<u8>,
    chains: RefCell<HashMap<String, Vec<IdentChainLink>>>,
    versions: Vec<IndexVersion>,
    links: Vec<PendingLink>,
}

/// A link as recorded at accept time; its `toBinding` is minted when the
/// index is served, against the recipient's then-current key.
struct PendingLink {
    link: TransferLink,
}

#[derive(Clone)]
struct Owner {
    name: String,
    public: Vec<u8>,
    private: Vec<u8>,
}

fn key(public: &[u8]) -> String {
    format!("ed25519:{}", crypto::encode_bytes(public))
}

impl World {
    fn new() -> World {
        let (server_public, server_private) = crypto::generate_keypair();
        World {
            server_public,
            server_private,
            chains: RefCell::new(HashMap::new()),
            versions: Vec::new(),
            links: Vec::new(),
        }
    }

    fn owner(&self, name: &str) -> Owner {
        let (public, private) = crypto::generate_keypair();
        Owner {
            name: name.to_string(),
            public,
            private,
        }
    }

    /// Rotate `owner` to a fresh key through a link signed by the old one.
    fn rotate(&self, owner: &mut Owner) {
        let (public, private) = crypto::generate_keypair();
        let signature = crypto::sign(
            &owner.private,
            &crypto::ident_rotation_message(
                &owner.name,
                &crypto::fingerprint(&owner.public),
                &public,
            ),
        )
        .unwrap();
        self.chains
            .borrow_mut()
            .entry(owner.name.clone())
            .or_default()
            .push(IdentChainLink {
                old_key: crypto::encode_bytes(&owner.public),
                new_key: crypto::encode_bytes(&public),
                signature: crypto::encode_bytes(&signature),
                issued: 0,
            });
        owner.public = public;
        owner.private = private;
    }

    /// Publish `version` as `publisher` in the current epoch.
    fn publish(&mut self, version: &str, publisher: &Owner) {
        self.versions.push(IndexVersion {
            version: version.to_string(),
            hash: format!("hash-{version}"),
            published_at: 0,
            state: "available".to_string(),
            abi_index: serde_json::json!({}),
            log_entry: None,
            publisher: Some(Publisher {
                owner: publisher.name.clone(),
                ident_fingerprint: Some(crypto::fingerprint(&publisher.public)),
            }),
            epoch: self.links.len() as u32,
        });
    }

    fn digest_now(&self) -> String {
        crypto::release_digest(
            self.versions
                .iter()
                .map(|v| (v.version.as_str(), v.hash.as_str())),
        )
    }

    /// A signed transfer, `from` offering with its current key and `to`
    /// accepting with its current key.
    fn transfer(&mut self, from: &Owner, to: &Owner) {
        let sequence = self.links.len() as i64;
        let digest = self.digest_now();
        let from_fp = crypto::fingerprint(&from.public);
        let terms = crypto::TransferTermsRef {
            ident: IDENT,
            sequence,
            from_owner: &from.name,
            from_ident_fingerprint: &from_fp,
            to_owner: &to.name,
            release_digest: &digest,
        };
        let offer = crypto::sign(&from.private, &crypto::transfer_offer_message_v2(terms)).unwrap();
        let accept = crypto::sign(&to.private, &crypto::transfer_accept_message_v2(terms)).unwrap();
        self.links.push(PendingLink {
            link: TransferLink {
                sequence,
                from_owner: from.name.clone(),
                from_ident_key: Some(key(&from.public)),
                to_owner: to.name.clone(),
                to_ident_key: Some(key(&to.public)),
                release_digest: Some(digest),
                offer_signature: Some(crypto::encode_bytes(&offer)),
                accept_signature: Some(crypto::encode_bytes(&accept)),
                offered_at: 0,
                accepted_at: 0,
                to_binding: placeholder_binding(),
            },
        });
    }

    /// A legacy (pre-signed-transfers) link: owners and position only.
    fn legacy_transfer(&mut self, from: &str, to: &str) {
        self.links.push(PendingLink {
            link: TransferLink {
                sequence: self.links.len() as i64,
                from_owner: from.to_string(),
                from_ident_key: None,
                to_owner: to.to_string(),
                to_ident_key: None,
                release_digest: None,
                offer_signature: None,
                accept_signature: None,
                offered_at: 0,
                accepted_at: 0,
                to_binding: placeholder_binding(),
            },
        });
    }

    fn binding(&self, owner: &Owner) -> OwnerBinding {
        let fingerprint = crypto::fingerprint(&owner.public);
        let signature = crypto::sign(
            &self.server_private,
            &crypto::name_binding_message(&owner.name, &fingerprint),
        )
        .unwrap();
        OwnerBinding {
            owner: owner.name.clone(),
            ident_key: key(&owner.public),
            ident_fingerprint: fingerprint,
            name_binding_signature: crypto::encode_bytes(&signature),
        }
    }

    /// Serve the index: `origin` is the prefix owner as it stands now, and
    /// `current` the current key of every recipient, by name.
    fn index(&self, origin: &Owner, current: &[&Owner]) -> IndexResponse {
        let origin_binding = self.binding(origin);
        IndexResponse {
            ident: IDENT.to_string(),
            owner: origin.name.clone(),
            ident_key: origin_binding.ident_key,
            ident_fingerprint: origin_binding.ident_fingerprint,
            name_binding_signature: origin_binding.name_binding_signature,
            server_fingerprint: crypto::fingerprint(&self.server_public),
            versions: self.versions.clone(),
            transfers: self
                .links
                .iter()
                .map(|pending| {
                    let mut link = pending.link.clone();
                    let recipient = current
                        .iter()
                        .find(|owner| owner.name == link.to_owner)
                        .expect("a current key for every recipient");
                    link.to_binding = self.binding(recipient);
                    link
                })
                .collect(),
        }
    }

    fn chain_of(&self) -> impl Fn(&str) -> Result<Vec<IdentChainLink>, String> + '_ {
        move |owner: &str| Ok(self.chains.borrow().get(owner).cloned().unwrap_or_default())
    }

    fn check(
        &self,
        index: &IndexResponse,
        anchor: Anchor,
        version: &str,
        signer: &[u8],
    ) -> Result<Trusted, TrustError> {
        let entry = index
            .versions
            .iter()
            .find(|v| v.version == version)
            .expect("served version");
        let author = entry
            .publisher
            .as_ref()
            .map(|p| p.owner.clone())
            .unwrap_or_else(|| index.owner.clone());
        verify_release(
            &self.server_public,
            index,
            &self.chain_of(),
            anchor,
            &Release {
                version: entry,
                content_hash: &entry.hash,
                header_ident_key: signer,
                header_author: &author,
            },
        )
    }
}

fn placeholder_binding() -> OwnerBinding {
    OwnerBinding {
        owner: String::new(),
        ident_key: String::new(),
        ident_fingerprint: String::new(),
        name_binding_signature: String::new(),
    }
}

fn pinned(owner: &Owner) -> Anchor {
    Anchor::Pinned(owner.public.clone())
}

fn untrusted_detail(result: Result<Trusted, TrustError>) -> String {
    match result {
        Err(TrustError::Untrusted(detail)) => detail,
        other => panic!("expected an untrusted release, got {other:?}"),
    }
}

/// alice publishes 1.0.0, then hands the package to bob.
fn alice_to_bob() -> (World, Owner, Owner) {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    world.publish("1.0.0", &alice);
    world.transfer(&alice, &bob);
    (world, alice, bob)
}

#[test]
fn pre_transfer_release_still_verifies() {
    let (world, alice, bob) = alice_to_bob();
    let index = world.index(&alice, &[&bob]);
    let trusted = world
        .check(&index, pinned(&alice), "1.0.0", &alice.public)
        .expect("alice's release verifies under her pin");
    assert_eq!(trusted.publisher, "alice");
    assert!(!trusted.crossed_transfer);
    assert!(world
        .check(&index, Anchor::FirstUse, "1.0.0", &alice.public)
        .is_ok());
}

#[test]
fn recipient_release_verifies_through_the_chain() {
    let (mut world, alice, bob) = alice_to_bob();
    world.publish("2.0.0", &bob);
    let index = world.index(&alice, &[&bob]);
    let trusted = world
        .check(&index, pinned(&alice), "2.0.0", &bob.public)
        .expect("bob's release verifies through the signed link");
    assert_eq!(trusted.publisher, "bob");
    assert_eq!(trusted.key, bob.public);
    assert!(trusted.crossed_transfer);
    // And under first use, with no pin at all.
    assert!(world
        .check(&index, Anchor::FirstUse, "2.0.0", &bob.public)
        .is_ok());
    // bob's release under alice's key is not bob's release.
    untrusted_detail(world.check(&index, pinned(&alice), "2.0.0", &alice.public));
}

#[test]
fn chain_of_two_transfers_verifies() {
    let (mut world, alice, bob) = alice_to_bob();
    world.publish("2.0.0", &bob);
    let carol = world.owner("carol");
    world.transfer(&bob, &carol);
    world.publish("3.0.0", &carol);
    let index = world.index(&alice, &[&bob, &carol]);
    for (version, signer) in [("1.0.0", &alice), ("2.0.0", &bob), ("3.0.0", &carol)] {
        world
            .check(&index, pinned(&alice), version, &signer.public)
            .unwrap_or_else(|err| panic!("{version}: {err}"));
    }
}

#[test]
fn forged_offer_is_rejected() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    world.publish("1.0.0", &alice);
    // The registry presents a transfer "from alice" signed by a key that is
    // not on alice's chain.
    let impostor = Owner {
        name: "alice".to_string(),
        ..world.owner("alice")
    };
    world.transfer(&impostor, &bob);
    world.publish("2.0.0", &bob);
    let index = world.index(&alice, &[&bob]);
    untrusted_detail(world.check(&index, pinned(&alice), "2.0.0", &bob.public));
    let detail = untrusted_detail(world.check(&index, Anchor::FirstUse, "2.0.0", &bob.public));
    assert!(
        detail.contains("not on their registry-bound ident chain"),
        "{detail}"
    );
}

#[test]
fn forged_accept_is_rejected() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    world.publish("1.0.0", &alice);
    // Accepted "by bob" under a key bob never held.
    let impostor = Owner {
        name: "bob".to_string(),
        ..world.owner("bob")
    };
    world.transfer(&alice, &impostor);
    world.publish("2.0.0", &impostor);
    let index = world.index(&alice, &[&bob]);
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "2.0.0", &impostor.public));
    assert!(detail.contains("not on `bob`'s ident chain"), "{detail}");
}

#[test]
fn replayed_link_at_wrong_sequence_is_rejected() {
    let (mut world, alice, bob) = alice_to_bob();
    world.transfer(&bob, &alice);
    // The recorded alice -> bob link replayed as a third transfer.
    let replay = world.links[0].link.clone();
    world.links.push(PendingLink { link: replay });
    let index = world.index(&alice, &[&bob, &alice]);
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "1.0.0", &alice.public));
    assert!(detail.contains("out of sequence"), "{detail}");

    // Renumbering it does not help: its signatures name position 0.
    let mut index = index;
    index.transfers[2].sequence = 2;
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "1.0.0", &alice.public));
    assert!(detail.contains("offer signature"), "{detail}");
}

#[test]
fn withheld_release_breaks_the_digest() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    world.publish("1.0.0", &alice);
    world.publish("1.1.0", &alice);
    world.transfer(&alice, &bob);
    let mut index = world.index(&alice, &[&bob]);
    index.versions.retain(|v| v.version != "1.1.0");
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "1.0.0", &alice.public));
    assert!(detail.contains("release digest"), "{detail}");

    // Re-dating a release into the next epoch breaks it the same way.
    let mut index = world.index(&alice, &[&bob]);
    index.versions[1].epoch = 1;
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "1.0.0", &alice.public));
    assert!(detail.contains("release digest"), "{detail}");
}

#[test]
fn rotation_before_and_after_transfer_verifies() {
    let mut world = World::new();
    let mut alice = world.owner("alice");
    let mut bob = world.owner("bob");
    let alice_0 = alice.clone();
    world.publish("1.0.0", &alice);
    world.rotate(&mut alice);
    let alice_1 = alice.clone();
    world.publish("1.1.0", &alice);
    world.transfer(&alice, &bob);
    let bob_0 = bob.clone();
    world.publish("2.0.0", &bob);
    world.rotate(&mut bob);
    world.publish("2.1.0", &bob);
    let index = world.index(&alice, &[&bob]);
    for (version, signer) in [
        ("1.0.0", &alice_0),
        ("1.1.0", &alice_1),
        ("2.0.0", &bob_0),
        ("2.1.0", &bob),
    ] {
        world
            .check(&index, pinned(&alice_0), version, &signer.public)
            .unwrap_or_else(|err| panic!("{version}: {err}"));
    }
    // A pin is forward-only: pinned at alice's rotated key, her
    // pre-rotation key is no longer trusted.
    untrusted_detail(world.check(&index, pinned(&alice_1), "1.0.0", &alice_0.public));
}

#[test]
fn legacy_link_terminates_the_chain() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    world.publish("1.0.0", &alice);
    world.legacy_transfer("alice", "bob");
    world.publish("2.0.0", &bob);
    let index = world.index(&alice, &[&bob]);
    assert!(world
        .check(&index, pinned(&alice), "1.0.0", &alice.public)
        .is_ok());
    let detail = untrusted_detail(world.check(&index, pinned(&alice), "2.0.0", &bob.public));
    assert!(detail.contains("only the origin owner"), "{detail}");
    untrusted_detail(world.check(&index, Anchor::FirstUse, "2.0.0", &bob.public));
}

#[test]
fn a_legacy_link_with_a_foreign_from_owner_does_not_fail_pre_transfer_releases() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    world.publish("1.0.0", &alice);
    world.legacy_transfer("mallory", "bob");
    let index = world.index(&alice, &[&bob]);
    assert!(world
        .check(&index, pinned(&alice), "1.0.0", &alice.public)
        .is_ok());
    assert!(world
        .check(&index, Anchor::FirstUse, "1.0.0", &alice.public)
        .is_ok());
}

#[test]
fn legacy_round_trip_origin_owner_release_verifies() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let bob = world.owner("bob");
    world.publish("1.0.0", &alice);
    world.legacy_transfer("alice", "bob");
    world.legacy_transfer("bob", "alice");
    world.publish("2.0.0", &alice);
    let index = world.index(&alice, &[&bob, &alice]);
    assert_eq!(index.versions[1].epoch, 2);
    let trusted = world
        .check(&index, pinned(&alice), "2.0.0", &alice.public)
        .expect("the origin owner verifies past the legacy links");
    assert!(!trusted.crossed_transfer);
    assert!(world
        .check(&index, Anchor::FirstUse, "2.0.0", &alice.public)
        .is_ok());
}

#[test]
fn round_trip_owner_pin_selects_its_first_tenure() {
    let (mut world, alice, bob) = alice_to_bob();
    world.publish("2.0.0", &bob);
    world.transfer(&bob, &alice);
    world.publish("3.0.0", &alice);
    let index = world.index(&alice, &[&bob, &alice]);
    let chain = verify_chain(
        &world.server_public,
        &index,
        &world.chain_of(),
        pinned(&alice),
    )
    .unwrap();
    // alice never rotated, so her pin lies in tenures 0 and 2; the first
    // is chosen.
    assert_eq!(chain.anchor_position, 0);
    for (version, signer) in [("1.0.0", &alice), ("2.0.0", &bob), ("3.0.0", &alice)] {
        world
            .check(&index, pinned(&alice), version, &signer.public)
            .unwrap_or_else(|err| panic!("{version}: {err}"));
    }
}

#[test]
fn a_pin_on_bob_verifies_alice_release_by_digest() {
    let (mut world, alice, bob) = alice_to_bob();
    world.publish("2.0.0", &bob);
    let index = world.index(&alice, &[&bob]);
    let trusted = world
        .check(&index, pinned(&bob), "1.0.0", &alice.public)
        .expect("alice's release is covered by the digest bob accepted");
    assert_eq!(trusted.publisher, "alice");
    assert!(trusted.crossed_transfer);
    assert!(world
        .check(&index, pinned(&bob), "2.0.0", &bob.public)
        .is_ok());
}

#[test]
fn first_use_trusts_bound_key_ancestors_only_for_the_origin() {
    let mut world = World::new();
    let mut alice = world.owner("alice");
    let mut bob = world.owner("bob");
    let alice_0 = alice.clone();
    world.publish("1.0.0", &alice);
    world.rotate(&mut alice);
    // bob held an older key before the one he accepted with.
    let bob_old = bob.clone();
    world.rotate(&mut bob);
    world.transfer(&alice, &bob);
    // A release "by bob" under the key he had BEFORE accepting.
    world.publish("2.0.0", &bob_old);
    let index = world.index(&alice, &[&bob]);
    // The origin's pre-rotation key is an ancestor of her bound key: trusted.
    assert!(world
        .check(&index, Anchor::FirstUse, "1.0.0", &alice_0.public)
        .is_ok());
    // bob's ancestor key predates his tenure: not trusted.
    untrusted_detail(world.check(&index, Anchor::FirstUse, "2.0.0", &bob_old.public));
}

#[test]
fn index_from_an_older_server_is_a_single_owner_chain() {
    let mut world = World::new();
    let alice = world.owner("alice");
    world.publish("1.0.0", &alice);
    let mut index = world.index(&alice, &[]);
    // What an older registry serves: no publisher, epoch 0, no transfers.
    let body = serde_json::to_value(&index).unwrap();
    let mut body = body.as_object().unwrap().clone();
    body.remove("transfers");
    let versions: Vec<_> = body["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let mut v = v.as_object().unwrap().clone();
            v.remove("publisher");
            v.remove("epoch");
            serde_json::Value::Object(v)
        })
        .collect();
    body.insert("versions".to_string(), serde_json::Value::Array(versions));
    index = serde_json::from_value(serde_json::Value::Object(body)).unwrap();
    assert!(index.transfers.is_empty() && index.versions[0].publisher.is_none());
    for anchor in [pinned(&alice), Anchor::FirstUse] {
        let trusted = world
            .check(&index, anchor, "1.0.0", &alice.public)
            .expect("an older index verifies as a single-owner chain");
        assert_eq!(trusted.publisher, "alice");
    }
}

#[test]
fn a_pin_off_every_chain_is_a_reanchor() {
    let mut world = World::new();
    let alice = world.owner("alice");
    let stranger = world.owner("alice");
    world.publish("1.0.0", &alice);
    let index = world.index(&alice, &[]);
    let error = world
        .check(&index, pinned(&stranger), "1.0.0", &alice.public)
        .unwrap_err();
    assert_eq!(error.rule(), "PACKAGE_IDENT_REANCHORED");
    assert!(error.detail().contains("NO chain link"), "{error}");
}
