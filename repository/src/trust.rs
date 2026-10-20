//! Client-side trust in a package's owners across signed transfers (plan-168-C).
//!
//! `/index` names the ident-prefix owner under a server name binding and, since
//! plan-168-A, serves the package's accepted transfers as signed links. This
//! module decides, without trusting the registry's word about who owns the
//! package, whether a release's header `identKey` is trusted for the release's
//! publisher. It is pure: the caller injects the server key, the verified
//! index, and a source of owners' rotation chains, so every rule here is
//! exercised by unit tests with no network.
//!
//! The walk:
//!
//! * **Owners.** `O_0` is the index's (prefix) owner; link `k` moves the package
//!   from `O_k` to `O_{k+1}`.
//! * **Link checks**, link by link: the position is `k`; a legacy link (any of
//!   its keys, digest or signatures null) ENDS the walk before anything about it
//!   is read; the from-owner is `O_k`; the to-owner's server binding verifies;
//!   the accept key reaches that bound key along the to-owner's rotation chain;
//!   the offer verifies under the from key and the accept under the to key, over
//!   the v2 terms; and the served versions of epoch `<= k` hash to the link's
//!   release digest (so a release cannot be withheld, added or re-dated).
//! * **Tenures.** Owner `O_i`'s keys while holding the package run from the key
//!   it accepted with (`A_{i-1}`; no lower bound for `i = 0`) to the key it
//!   offered with (`F_i`), or to its server-bound current key for the last
//!   trusted owner. A tenure must be non-empty: the accept key reaches the offer
//!   key.
//! * **Anchor.** `Pinned(P)`: `P` must lie in some tenure; the first such tenure
//!   `j` (a deterministic choice when an owner appears twice) trusts the keys
//!   forward from `P`, every later tenure trusts its own keys, and releases from
//!   before `j` are trusted through the release digest the link into `j` signs.
//!   `FirstUse`: the origin owner's tenure is trusted when its offer key is an
//!   ancestor of its server-bound key (the trust-on-first-use `pkg add` has
//!   always granted), and every later tenure through the signed links.
//! * **Releases.** The publisher is the header author and the owner of the
//!   release's epoch; its header key must be trusted for that owner, or covered
//!   by the digest. Past a legacy link only the origin owner can be verified,
//!   under the anchor alone (the prefix-owner trust every client applied before
//!   transfers were signed).

use std::collections::HashMap;

use crate::client::{chain_path, verify_owner_binding};
use crate::crypto;
use crate::package::decode_metadata_key;
use crate::server::{IdentChainLink, IndexResponse, IndexVersion, TransferLink};
use crate::validation::fold_owner;

/// Where a consumer's trust in the package starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// The identKey a project already pins for the package.
    Pinned(Vec<u8>),
    /// No pin yet: the registry's name binding, trusted on first use.
    FirstUse,
}

/// One release to verify: its `/index` entry and what its downloaded header
/// says.
pub struct Release<'a> {
    pub version: &'a IndexVersion,
    /// The content hash of the bytes actually fetched or installed.
    pub content_hash: &'a str,
    /// The header's `identKey`, decoded.
    pub header_ident_key: &'a [u8],
    /// The header's `author`.
    pub header_author: &'a str,
}

/// A release whose header key is trusted for its publisher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trusted {
    /// The header ident key: the new pin.
    pub key: Vec<u8>,
    /// The owner who published the release.
    pub publisher: String,
    /// Whether the publisher is not the owner the anchor belongs to: the pin is
    /// moving across an ownership transfer.
    pub crossed_transfer: bool,
}

/// Why a release is not trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustError {
    /// The pinned key is on no owner's chain to the package's current
    /// identities: the owner's ident changed with no signed link from it.
    Reanchored(String),
    /// Anything else the chain or the release fails.
    Untrusted(String),
}

impl TrustError {
    /// The diagnostic rule code a consumer reports this under.
    pub fn rule(&self) -> &'static str {
        match self {
            TrustError::Reanchored(_) => "PACKAGE_IDENT_REANCHORED",
            TrustError::Untrusted(_) => "PACKAGE_IDENT_KEY_UNTRUSTED",
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            TrustError::Reanchored(detail) | TrustError::Untrusted(detail) => detail,
        }
    }
}

impl std::fmt::Display for TrustError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.detail())
    }
}

fn untrusted(detail: String) -> TrustError {
    TrustError::Untrusted(detail)
}

/// One owner position in the verified chain.
struct OwnerSlot {
    /// The display name as served.
    name: String,
    /// The owner's current ident key, under a verified server binding.
    current: Vec<u8>,
    /// The key the owner accepted the package with (`A_{i-1}`).
    lower: Option<Vec<u8>>,
    /// The key the owner offered the package away with (`F_i`), or the bound
    /// current key for the last trusted owner.
    upper: Vec<u8>,
    chain: Vec<IdentChainLink>,
}

impl OwnerSlot {
    /// Every key `from` reaches on this owner's rotation chain, `from` first.
    fn forward(&self, from: &[u8]) -> Result<Vec<Vec<u8>>, TrustError> {
        chain_path(&self.name, from, &self.chain).map_err(untrusted)
    }

    fn reaches(&self, from: &[u8], to: &[u8]) -> Result<bool, TrustError> {
        Ok(self.forward(from)?.iter().any(|key| key == to))
    }

    /// Whether `key` lies in this owner's tenure.
    fn in_tenure(&self, key: &[u8]) -> Result<bool, TrustError> {
        if !self.reaches(key, &self.upper)? {
            return Ok(false);
        }
        match &self.lower {
            Some(lower) => self.reaches(lower, key),
            None => Ok(true),
        }
    }

    /// Every key of this owner's tenure.
    fn tenure_keys(&self) -> Result<Vec<Vec<u8>>, TrustError> {
        let mut candidates: Vec<Vec<u8>> = vec![self.upper.clone(), self.current.clone()];
        candidates.extend(self.lower.iter().cloned());
        for link in &self.chain {
            for key in [&link.old_key, &link.new_key] {
                if let Ok(key) = crypto::decode_bytes(key, "chain key") {
                    candidates.push(key);
                }
            }
        }
        let mut keys = Vec::new();
        for key in candidates {
            if !keys.contains(&key) && self.in_tenure(&key)? {
                keys.push(key);
            }
        }
        Ok(keys)
    }
}

/// A transfer chain that passed every link check, with the anchor applied.
pub struct VerifiedChain {
    owners: Vec<OwnerSlot>,
    /// The number of links the index serves.
    served_links: usize,
    /// The number of signed links before the first legacy link (or all of
    /// them): owners `0..=trusted_links` are in the verified chain.
    trusted_links: usize,
    /// The anchor's tenure (`j`); `0` for a first use.
    anchor_position: usize,
    /// The keys trusted per owner position. Empty for a position before the
    /// anchor's, whose releases are trusted through the release digest.
    trusted_keys: Vec<Vec<Vec<u8>>>,
    /// The anchor as given, for the origin-owner fallback past a legacy link.
    anchor: Anchor,
}

/// Run every link check over `index`'s transfers and apply `anchor`.
///
/// `chain_of(owner)` returns an owner's rotation links, oldest first (the
/// `GET /idents/<owner>` chain); it is asked once per distinct owner. Each link
/// is self-authenticating (signed by the key it retires), so the chain needs no
/// server signature: the server-signed bindings name the endpoints.
pub fn verify_chain(
    server_key: &[u8],
    index: &IndexResponse,
    chain_of: &dyn Fn(&str) -> Result<Vec<IdentChainLink>, String>,
    anchor: Anchor,
) -> Result<VerifiedChain, TrustError> {
    let ident = index.ident.as_str();
    let mut chains: HashMap<String, Vec<IdentChainLink>> = HashMap::new();
    let mut chain_for = |owner: &str| -> Result<Vec<IdentChainLink>, TrustError> {
        let folded = fold_owner(owner);
        if let Some(chain) = chains.get(&folded) {
            return Ok(chain.clone());
        }
        let chain = chain_of(owner).map_err(|err| {
            untrusted(format!(
                "the ident chain of `{owner}` could not be read: {err}"
            ))
        })?;
        chains.insert(folded, chain.clone());
        Ok(chain)
    };

    // The origin owner, under the index's own binding.
    let origin_binding = crate::server::OwnerBinding {
        owner: index.owner.clone(),
        ident_key: index.ident_key.clone(),
        ident_fingerprint: index.ident_fingerprint.clone(),
        name_binding_signature: index.name_binding_signature.clone(),
    };
    let origin_current = verify_owner_binding(server_key, &origin_binding).map_err(untrusted)?;
    let mut owners = vec![OwnerSlot {
        name: index.owner.clone(),
        upper: origin_current.clone(),
        current: origin_current,
        lower: None,
        chain: chain_for(&index.owner)?,
    }];

    let mut trusted_links = 0;
    for (position, link) in index.transfers.iter().enumerate() {
        if link.sequence != position as i64 {
            return Err(untrusted(format!(
                "transfer link {position} of {ident} is out of sequence (it says {})",
                link.sequence
            )));
        }
        let Some(signed) = SignedLink::of(link)? else {
            // A legacy link: nothing about it can be authenticated, so it ends
            // the walk before its owners are compared with anything.
            break;
        };
        let from = &owners[position];
        if fold_owner(&link.from_owner) != fold_owner(&from.name) {
            return Err(untrusted(format!(
                "transfer link {position} of {ident} is from `{}`, but `{}` held the package",
                link.from_owner, from.name
            )));
        }
        let bound = verify_owner_binding(server_key, &link.to_binding).map_err(untrusted)?;
        if fold_owner(&link.to_binding.owner) != fold_owner(&link.to_owner) {
            return Err(untrusted(format!(
                "transfer link {position} of {ident} binds `{}` but moves the package to `{}`",
                link.to_binding.owner, link.to_owner
            )));
        }
        let digest = crypto::release_digest(
            index
                .versions
                .iter()
                .filter(|version| (version.epoch as usize) <= position)
                .map(|version| (version.version.as_str(), version.hash.as_str())),
        );
        if digest != signed.release_digest {
            return Err(untrusted(format!(
                "the releases served for {ident} do not match the release digest transfer link \
                 {position} signed: a release was withheld, added or re-dated"
            )));
        }
        let from_fingerprint = crypto::fingerprint(&signed.from_key);
        let terms = crypto::TransferTermsRef {
            ident,
            sequence: position as i64,
            from_owner: &link.from_owner,
            from_ident_fingerprint: &from_fingerprint,
            to_owner: &link.to_owner,
            release_digest: &signed.release_digest,
        };
        crypto::verify(
            &signed.from_key,
            &crypto::transfer_offer_message_v2(terms),
            &signed.offer_signature,
        )
        .map_err(|_| {
            untrusted(format!(
                "transfer link {position} of {ident} carries an offer signature that does not \
                 verify under its from key"
            ))
        })?;
        crypto::verify(
            &signed.to_key,
            &crypto::transfer_accept_message_v2(terms),
            &signed.accept_signature,
        )
        .map_err(|_| {
            untrusted(format!(
                "transfer link {position} of {ident} carries an accept signature that does not \
                 verify under its to key"
            ))
        })?;
        let recipient = OwnerSlot {
            name: link.to_owner.clone(),
            upper: bound.clone(),
            current: bound,
            lower: Some(signed.to_key.clone()),
            chain: chain_for(&link.to_owner)?,
        };
        if !recipient.reaches(&signed.to_key, &recipient.current)? {
            return Err(untrusted(format!(
                "transfer link {position} of {ident} was accepted with a key that is not on \
                 `{}`'s ident chain",
                link.to_owner
            )));
        }
        owners[position].upper = signed.from_key;
        owners.push(recipient);
        trusted_links = position + 1;
    }

    // Every tenure is non-empty: the key an owner accepted with reaches the key
    // it offered with (a linear rotation chain allows nothing else).
    for (position, owner) in owners.iter().enumerate().skip(1) {
        if !owner.in_tenure(&owner.upper)? {
            return Err(untrusted(format!(
                "`{}` offered {ident} away (transfer link {position}) with a key that is not on \
                 their ident chain after the key they accepted it with",
                owner.name
            )));
        }
    }

    let (anchor_position, mut trusted_keys) = match &anchor {
        Anchor::FirstUse => {
            let origin = &owners[0];
            if !origin.reaches(&origin.upper, &origin.current)? {
                return Err(untrusted(format!(
                    "`{}` offered {ident} away with a key that is not on their registry-bound \
                     ident chain",
                    origin.name
                )));
            }
            (0, Vec::new())
        }
        Anchor::Pinned(pin) => {
            let mut found = None;
            for (position, owner) in owners.iter().enumerate() {
                if owner.in_tenure(pin)? {
                    found = Some(position);
                    break;
                }
            }
            let Some(position) = found else {
                // A pin that still chains to some owner's current key belongs
                // to that owner; the transfer record just does not place it in
                // their ownership. Only a pin off every chain is a re-anchor.
                for owner in &owners {
                    if owner.reaches(pin, &owner.current)? {
                        return Err(untrusted(format!(
                            "the pinned identKey (fingerprint {}) belongs to `{}`, but no signed \
                             transfer of {ident} places it in their ownership",
                            crypto::fingerprint(pin),
                            owner.name
                        )));
                    }
                }
                return Err(TrustError::Reanchored(format!(
                    "the pinned identKey (fingerprint {}) is on no owner's ident chain for {ident}: \
                     the ident changed with NO chain link from it",
                    crypto::fingerprint(pin)
                )));
            };
            (position, Vec::new())
        }
    };
    for (position, owner) in owners.iter().enumerate() {
        let keys = if position < anchor_position {
            Vec::new()
        } else if position == anchor_position {
            match &anchor {
                Anchor::Pinned(pin) => {
                    let tenure = owner.tenure_keys()?;
                    owner
                        .forward(pin)?
                        .into_iter()
                        .filter(|key| tenure.contains(key))
                        .collect()
                }
                Anchor::FirstUse => owner.tenure_keys()?,
            }
        } else {
            owner.tenure_keys()?
        };
        trusted_keys.push(keys);
    }

    Ok(VerifiedChain {
        owners,
        served_links: index.transfers.len(),
        trusted_links,
        anchor_position,
        trusted_keys,
        anchor,
    })
}

/// [`verify_chain`] then [`VerifiedChain::verify_release`].
pub fn verify_release(
    server_key: &[u8],
    index: &IndexResponse,
    chain_of: &dyn Fn(&str) -> Result<Vec<IdentChainLink>, String>,
    anchor: Anchor,
    release: &Release<'_>,
) -> Result<Trusted, TrustError> {
    verify_chain(server_key, index, chain_of, anchor)?.verify_release(release)
}

impl VerifiedChain {
    /// Decide one release of the verified index.
    pub fn verify_release(&self, release: &Release<'_>) -> Result<Trusted, TrustError> {
        let version = release.version;
        let label = &version.version;
        if release.content_hash != version.hash {
            return Err(untrusted(format!(
                "release {label} does not match the registry's content hash for it"
            )));
        }
        let epoch = version.epoch as usize;
        if epoch > self.served_links {
            return Err(untrusted(format!(
                "release {label} names ownership epoch {epoch}, past the {} transfer(s) served",
                self.served_links
            )));
        }
        let publisher = match &version.publisher {
            Some(publisher) => publisher.owner.clone(),
            // An older registry records no publisher: every release it holds
            // was published by the prefix owner (plan-168-A).
            None => self.owners[0].name.clone(),
        };
        if fold_owner(&publisher) != fold_owner(release.header_author) {
            return Err(untrusted(format!(
                "release {label} is published by `{publisher}` but its header names author `{}`",
                release.header_author
            )));
        }
        let anchor_owner = &self.owners[self.anchor_position].name;
        let trusted = |publisher: String| Trusted {
            key: release.header_ident_key.to_vec(),
            crossed_transfer: fold_owner(&publisher) != fold_owner(anchor_owner),
            publisher,
        };

        if epoch <= self.trusted_links {
            let owner = &self.owners[epoch];
            if fold_owner(&publisher) != fold_owner(&owner.name) {
                return Err(untrusted(format!(
                    "release {label} is published by `{publisher}`, but `{}` owned the package \
                     in its ownership epoch",
                    owner.name
                )));
            }
            if epoch < self.anchor_position {
                // Covered by the release digest the link into the anchor's
                // tenure signed (checked against every served release of an
                // epoch before it), and the content hash pins the bytes.
                return Ok(trusted(publisher));
            }
            if self.trusted_keys[epoch]
                .iter()
                .any(|key| key.as_slice() == release.header_ident_key)
            {
                return Ok(trusted(publisher));
            }
            return Err(untrusted(format!(
                "release {label} is signed by an ident key (fingerprint {}) that is not trusted \
                 for `{}`",
                crypto::fingerprint(release.header_ident_key),
                owner.name
            )));
        }

        // Past a legacy link: the origin-owner fallback, under the anchor alone.
        let origin = &self.owners[0];
        if fold_owner(&publisher) != fold_owner(&origin.name) {
            return Err(untrusted(format!(
                "release {label} was published by `{publisher}` after a transfer the registry \
                 recorded before transfers were signed; only the origin owner `{}` can be \
                 verified there",
                origin.name
            )));
        }
        let key_trusted = match &self.anchor {
            Anchor::Pinned(pin) => {
                origin.reaches(pin, &origin.current)?
                    && origin
                        .forward(pin)?
                        .iter()
                        .any(|key| key.as_slice() == release.header_ident_key)
            }
            Anchor::FirstUse => origin.reaches(release.header_ident_key, &origin.current)?,
        };
        if !key_trusted {
            return Err(untrusted(format!(
                "release {label} is signed by an ident key (fingerprint {}) that is not trusted \
                 for `{}`",
                crypto::fingerprint(release.header_ident_key),
                origin.name
            )));
        }
        Ok(Trusted {
            key: release.header_ident_key.to_vec(),
            crossed_transfer: fold_owner(&origin.name) != fold_owner(anchor_owner),
            publisher,
        })
    }

    /// Whether `key` is an ident key the verified chain trusts for `owner` in
    /// some ownership epoch, and if so that owner's current bound key — the
    /// test `mfb info` applies to a signed executable, which is no registry
    /// release (plan-168-C). Positions before the anchor are not consulted.
    pub fn owner_key(&self, owner: &str, key: &[u8]) -> Option<Vec<u8>> {
        self.owners
            .iter()
            .zip(&self.trusted_keys)
            .take(self.trusted_links + 1)
            .find(|(slot, keys)| {
                fold_owner(&slot.name) == fold_owner(owner)
                    && keys.iter().any(|trusted| trusted.as_slice() == key)
            })
            .map(|(slot, _)| slot.current.clone())
    }
}

/// The signed parts of a non-legacy link, decoded.
struct SignedLink {
    from_key: Vec<u8>,
    to_key: Vec<u8>,
    release_digest: String,
    offer_signature: Vec<u8>,
    accept_signature: Vec<u8>,
}

impl SignedLink {
    /// `None` for a legacy link: any of the five signed fields is null.
    fn of(link: &TransferLink) -> Result<Option<SignedLink>, TrustError> {
        let (Some(from), Some(to), Some(digest), Some(offer), Some(accept)) = (
            &link.from_ident_key,
            &link.to_ident_key,
            &link.release_digest,
            &link.offer_signature,
            &link.accept_signature,
        ) else {
            return Ok(None);
        };
        let decode = |value: &str, field: &str| {
            crypto::decode_bytes(value, field).map_err(|err| {
                untrusted(format!(
                    "transfer link {} is malformed: {err}",
                    link.sequence
                ))
            })
        };
        Ok(Some(SignedLink {
            from_key: decode_metadata_key(from, "fromIdentKey").map_err(untrusted)?,
            to_key: decode_metadata_key(to, "toIdentKey").map_err(untrusted)?,
            release_digest: digest.clone(),
            offer_signature: decode(offer, "offerSignature")?,
            accept_signature: decode(accept, "acceptSignature")?,
        }))
    }
}

#[cfg(test)]
mod tests;
