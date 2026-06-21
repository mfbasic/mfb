use crate::crypto;
use crate::local::{self, LocalPaths};
use crate::server::{
    ChallengeRequest, ChallengeResponse, ErrorResponse, LoginRequest, LoginResponse,
    PackageArtifactRequest, PublishPackageResponse, RegisterRequest, RegisterResponse,
    SigningInfoRequest, SigningInfoResponse, ValidatePackageResponse,
};
use crate::validation::validate_owner_name;
use crate::DEFAULT_REPO_URL;
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;

pub fn repo_url_from_env() -> String {
    std::env::var("MFB_REPO_URL").unwrap_or_else(|_| DEFAULT_REPO_URL.to_string())
}

pub fn register(repo_url: &str, paths: &LocalPaths, owner: &str) -> Result<RegisterResponse, String> {
    validate_owner_name(owner)?;
    let (public, private) = crypto::generate_keypair();
    let message = crypto::registration_message(owner, &public);
    let proof = crypto::sign(&private, &message)?;
    let request = RegisterRequest {
        owner: owner.to_string(),
        auth_key: crypto::encode_bytes(&public),
        proof: crypto::encode_bytes(&proof),
    };
    local::write_keypair(paths, owner, &public, &private)?;
    let response = post_json::<RegisterResponse>(repo_url, "/accounts/register", &request);
    match response {
        Ok(response) => Ok(response),
        Err(err) => {
            local::remove_keypair(paths, owner);
            Err(err)
        }
    }
}

pub fn auth(repo_url: &str, paths: &LocalPaths, owner: &str) -> Result<LoginResponse, String> {
    validate_owner_name(owner)?;
    let private = match local::read_private_key(paths, owner) {
        Ok(private) => private,
        Err(local_err) => {
            let probe = post_json::<ChallengeResponse>(
                repo_url,
                "/auth/challenge",
                &ChallengeRequest {
                    owner: owner.to_string(),
                    auth_fingerprint: String::new(),
                },
            );
            if let Err(remote_err) = probe {
                if remote_err.contains("unknown owner") {
                    return Err(remote_err);
                }
            }
            return Err(local_err);
        }
    };
    let public = crypto::public_from_private(&private)?;
    if let Ok(stored_public) = local::read_public_key(paths, owner) {
        if stored_public != public {
            return Err("mismatched local key fingerprint".to_string());
        }
    }
    let fingerprint = crypto::fingerprint(&public);
    let challenge = post_json::<ChallengeResponse>(
        repo_url,
        "/auth/challenge",
        &ChallengeRequest {
            owner: owner.to_string(),
            auth_fingerprint: fingerprint,
        },
    )?;
    let nonce = crypto::decode_bytes(&challenge.nonce, "nonce")?;
    let message = crypto::challenge_message(&challenge.challenge_id, &nonce);
    let signature = crypto::sign(&private, &message)?;
    let login = post_json::<LoginResponse>(
        repo_url,
        "/auth/login",
        &LoginRequest {
            challenge_id: challenge.challenge_id,
            signature: crypto::encode_bytes(&signature),
        },
    )?;
    local::write_session(paths, owner, &login.session_token)?;
    Ok(login)
}

pub fn signing_info(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<SigningInfoResponse, String> {
    validate_owner_name(owner)?;
    let session_token = local::read_session(paths, owner)?;
    post_json::<SigningInfoResponse>(
        repo_url,
        "/keys/signing",
        &SigningInfoRequest {
            owner: owner.to_string(),
            session_token,
        },
    )
}

pub struct PackageArtifact<'a> {
    pub ident: &'a str,
    pub version: &'a str,
    pub artifact: &'a [u8],
    pub content_hash: &'a str,
    pub ident_fingerprint: &'a str,
    pub signing_fingerprint: &'a str,
}

pub fn validate_package(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    package: &PackageArtifact<'_>,
) -> Result<ValidatePackageResponse, String> {
    validate_owner_name(owner)?;
    let session_token = local::read_session(paths, owner)?;
    post_json::<ValidatePackageResponse>(
        repo_url,
        "/validate",
        &package_request(package, session_token),
    )
}

pub fn publish_package(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    package: &PackageArtifact<'_>,
) -> Result<PublishPackageResponse, String> {
    validate_owner_name(owner)?;
    let session_token = local::read_session(paths, owner)?;
    post_json::<PublishPackageResponse>(
        repo_url,
        "/publish",
        &package_request(package, session_token),
    )
}

fn package_request(package: &PackageArtifact<'_>, session_token: String) -> PackageArtifactRequest {
    PackageArtifactRequest {
        ident: package.ident.to_string(),
        version: package.version.to_string(),
        artifact: crypto::encode_bytes(package.artifact),
        content_hash: package.content_hash.to_string(),
        ident_fingerprint: package.ident_fingerprint.to_string(),
        signing_fingerprint: package.signing_fingerprint.to_string(),
        session_token,
    }
}

fn post_json<T: DeserializeOwned>(
    repo_url: &str,
    path: &str,
    body: &impl serde::Serialize,
) -> Result<T, String> {
    let url = format!("{}{}", repo_url.trim_end_matches('/'), path);
    let response = Client::new()
        .post(&url)
        .json(body)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    let status = response.status();
    if status.is_success() {
        return response
            .json::<T>()
            .map_err(|err| format!("invalid repository response: {err}"));
    }
    let text = response
        .text()
        .unwrap_or_else(|_| "repository request failed".to_string());
    if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
        return Err(error.error);
    }
    Err(format!("repository request failed with status {status}: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_duplicate_failure_leaves_no_local_keys() {
        let temp_repo = tempfile::tempdir().unwrap();
        let opened = crate::store::Store::open_repository(temp_repo.path()).unwrap();
        let store = opened.store;
        let temp_home = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp_home.path().join(".mfb"));

        let (public, private) = crypto::generate_keypair();
        let message = crypto::registration_message("alice", &public);
        let proof = crypto::sign(&private, &message).unwrap();
        store.register_owner("alice", &public, &proof).unwrap();

        local::remove_keypair(&paths, "alice");
        assert!(!paths.private_key_path("alice").exists());
    }
}
