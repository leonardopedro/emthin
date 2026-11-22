//! Document grants and verified peer principals (Kyneta K4).
//!
//! Grants live on the share owner. A principal is only trusted when the
//! transport verified it — local Unix peer credentials or an independently
//! authenticated remote identity. A display name published in presence is
//! cosmetic and is never consulted here. Denied and missing share lookups
//! return the same value so a peer cannot probe which shares exist.

use std::collections::BTreeMap;
use std::fmt;

use crate::wire::ShareId;

/// A share permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Grant {
    /// Read authorized content and copy it.
    Read,
    /// Commit and relay edits.
    Edit,
    /// Discover other shares and grant/revoke peers.
    Invite,
}

impl Grant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Invite => "invite",
        }
    }
}

/// A transport-verified identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub id: String,
    /// Set only by the transport's credential check, never by a presence
    /// display name or by a peer-supplied string.
    pub verified: bool,
}

impl Principal {
    pub fn verified(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            verified: true,
        }
    }

    /// An unverified principal (a raw transport id before credential check).
    pub fn unverified(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            verified: false,
        }
    }
}

/// The same outward result is returned for a denied lookup and for a share
/// that does not exist, so a peer cannot enumerate shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareLookup {
    Access {
        share_id: ShareId,
        grants: Vec<Grant>,
    },
    Unavailable,
}

/// Why a grant edit was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantError {
    /// Only the verified owner may grant or revoke.
    NotOwner,
    /// The caller's identity was not verified by the transport.
    NotAuthenticated,
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOwner => f.write_str("only the verified owner may grant or revoke"),
            Self::NotAuthenticated => f.write_str("caller identity was not verified"),
        }
    }
}

impl std::error::Error for GrantError {}

/// The owner's grant table for one document share.
#[derive(Debug, Clone)]
pub struct GrantTable {
    owner: Principal,
    grants: BTreeMap<String, Vec<Grant>>,
    /// The connection epoch at which each peer was revoked. Frames from an
    /// older epoch are pending work and must be invalidated.
    revoked_epochs: BTreeMap<String, u64>,
}

impl GrantTable {
    pub fn new(owner: Principal) -> Self {
        Self {
            owner,
            grants: BTreeMap::new(),
            revoked_epochs: BTreeMap::new(),
        }
    }

    pub fn owner(&self) -> &Principal {
        &self.owner
    }

    /// Whether any peer has an explicit grant. A share with none is a legacy
    /// open share and keeps the unscoped compatibility path; once a grant is
    /// made, every non-owner access is checked.
    pub fn has_explicit_grants(&self) -> bool {
        !self.grants.is_empty()
    }

    fn may_administer(&self, by: &Principal) -> Result<(), GrantError> {
        if !by.verified {
            return Err(GrantError::NotAuthenticated);
        }
        if by.id != self.owner.id {
            return Err(GrantError::NotOwner);
        }
        Ok(())
    }

    /// Grant `grant` to `peer`. Only the verified owner may do this.
    pub fn grant(&mut self, by: &Principal, peer: &str, grant: Grant) -> Result<(), GrantError> {
        self.may_administer(by)?;
        let grants = self.grants.entry(peer.to_string()).or_default();
        if !grants.contains(&grant) {
            grants.push(grant);
            grants.sort();
        }
        // A fresh grant clears a prior revocation marker for this peer.
        self.revoked_epochs.remove(peer);
        Ok(())
    }

    /// Revoke every grant for `peer` and invalidate its pending connection
    /// epoch. Copies the peer already received cannot be recovered here.
    pub fn revoke(
        &mut self,
        by: &Principal,
        peer: &str,
        current_epoch: u64,
    ) -> Result<(), GrantError> {
        self.may_administer(by)?;
        self.grants.remove(peer);
        self.revoked_epochs.insert(peer.to_string(), current_epoch);
        Ok(())
    }

    /// Resolve a share lookup. Both a denied principal and an unknown share
    /// produce `Unavailable`.
    pub fn lookup(&self, share_id: &ShareId, principal: &Principal) -> ShareLookup {
        if !principal.verified {
            return ShareLookup::Unavailable;
        }
        if principal.id == self.owner.id {
            return ShareLookup::Access {
                share_id: *share_id,
                grants: vec![Grant::Read, Grant::Edit, Grant::Invite],
            };
        }
        match self.grants.get(&principal.id) {
            Some(grants) if !grants.is_empty() => ShareLookup::Access {
                share_id: *share_id,
                grants: grants.clone(),
            },
            _ => ShareLookup::Unavailable,
        }
    }

    fn has(&self, principal: &Principal, grant: Grant) -> bool {
        if !principal.verified {
            return false;
        }
        if principal.id == self.owner.id {
            return true;
        }
        self.grants
            .get(&principal.id)
            .is_some_and(|grants| grants.contains(&grant))
    }

    pub fn can_read(&self, principal: &Principal) -> bool {
        self.has(principal, Grant::Read)
    }

    pub fn can_edit(&self, principal: &Principal) -> bool {
        self.has(principal, Grant::Edit)
    }

    pub fn can_invite(&self, principal: &Principal) -> bool {
        self.has(principal, Grant::Invite)
    }

    /// Whether `peer`'s connection epoch was invalidated by a revoke. Frames
    /// from that epoch or older are pending work and must be dropped.
    pub fn is_epoch_revoked(&self, peer: &str, epoch: u64) -> bool {
        self.revoked_epochs
            .get(peer)
            .is_some_and(|revoked| epoch <= *revoked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> GrantTable {
        GrantTable::new(Principal::verified("owner"))
    }

    #[test]
    fn only_the_verified_owner_can_grant_or_revoke() {
        let mut table = table();
        assert_eq!(
            table.grant(&Principal::unverified("owner"), "bob", Grant::Read),
            Err(GrantError::NotAuthenticated)
        );
        assert_eq!(
            table.grant(&Principal::verified("mallory"), "bob", Grant::Read),
            Err(GrantError::NotOwner)
        );
        assert!(table
            .grant(&Principal::verified("owner"), "bob", Grant::Read)
            .is_ok());
        assert_eq!(
            table.revoke(&Principal::verified("bob"), "bob", 1),
            Err(GrantError::NotOwner)
        );
    }

    #[test]
    fn a_read_only_peer_cannot_commit_or_discover() {
        let mut table = table();
        let owner = Principal::verified("owner");
        table.grant(&owner, "bob", Grant::Read).unwrap();
        let bob = Principal::verified("bob");
        assert!(table.can_read(&bob));
        assert!(!table.can_edit(&bob));
        assert!(!table.can_invite(&bob));
    }

    #[test]
    fn a_revoked_peer_cannot_read_relay_or_invite() {
        let mut table = table();
        let owner = Principal::verified("owner");
        table.grant(&owner, "bob", Grant::Read).unwrap();
        table.grant(&owner, "bob", Grant::Edit).unwrap();
        table.revoke(&owner, "bob", 4).unwrap();
        let bob = Principal::verified("bob");
        assert!(!table.can_read(&bob));
        assert!(!table.can_edit(&bob));
        assert!(!table.can_invite(&bob));
        assert!(table.is_epoch_revoked("bob", 4));
        assert!(table.is_epoch_revoked("bob", 3));
        assert!(!table.is_epoch_revoked("bob", 5));
    }

    #[test]
    fn denied_and_missing_shares_have_the_same_outward_result() {
        let table = table();
        let share = [7u8; 16];
        let denied = table.lookup(&share, &Principal::verified("stranger"));
        let missing_identity = table.lookup(&share, &Principal::unverified("owner"));
        assert_eq!(denied, ShareLookup::Unavailable);
        assert_eq!(missing_identity, ShareLookup::Unavailable);
        let owner = table.lookup(&share, &Principal::verified("owner"));
        assert!(matches!(owner, ShareLookup::Access { .. }));
    }

    #[test]
    fn a_display_name_alone_confers_no_permission() {
        let mut table = table();
        // A presence display name called "owner" is not a verified principal.
        let spoof = Principal::unverified("owner");
        assert_eq!(
            table.grant(&spoof, "mallory", Grant::Edit),
            Err(GrantError::NotAuthenticated)
        );
        assert!(!table.can_edit(&spoof));
    }

    #[test]
    fn a_grant_clears_a_prior_revocation_marker() {
        let mut table = table();
        let owner = Principal::verified("owner");
        table.grant(&owner, "bob", Grant::Read).unwrap();
        table.revoke(&owner, "bob", 2).unwrap();
        table.grant(&owner, "bob", Grant::Edit).unwrap();
        assert!(!table.is_epoch_revoked("bob", 2));
        assert!(table.can_edit(&Principal::verified("bob")));
    }
}
