use crate::crypto;
use crate::local::{self, LocalPaths};
use crate::server::{
    ChallengeRequest, ChallengeResponse, CheckpointResponse, ConsistencyProofResponse,
    ErrorResponse, IdentChainResponse, InclusionProofResponse, IndexResponse, LinkFetchRequest,
    LinkFetchResponse, LinkStartRequest, LinkStartResponse, LogEntry, LoginRequest, LoginResponse,
    OrgMemberRequest, OrgMemberResponse, OwnerBinding, PackageArtifactRequest,
    PublishPackageResponse, RegisterProofs, RegisterRequest, RegisterResponse, ReleaseStateRequest,
    ReleaseStateResponse, RevokeChallengeRequest, RevokeRequest, RevokeResponse, RootResponse,
    RotateRequest, RotateResponse, ServerIdentResponse, SignedMetadataResponse, SigningRequest,
    SigningResponse, TokenIssueRequest, TokenIssueResponse, TokenRevokeRequest,
    TokenRevokeResponse, TransferAcceptRequest, TransferOfferRequest, TransferResponse,
    ValidatePackageResponse,
};
use crate::validation::{fold_owner, validate_owner_name};
use crate::DEFAULT_REPO_URL;
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use std::sync::OnceLock;
use std::time::Duration;

mod governance;
mod identity;
mod packages;
mod registry_trust;
mod transport;
pub use governance::*;
pub use identity::*;
pub use packages::*;
pub use registry_trust::*;
pub use transport::*;

#[cfg(test)]
mod tests;
