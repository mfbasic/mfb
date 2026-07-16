//! `mfb repo`/`key`/`org`/`token`/`machine` account commands.
//!
//! coverage:off (success arms) — every non-Usage code path here performs live
//! HTTP against a registry (register/auth/link/trust/rotate/grant/issue/revoke)
//! and mutates on-disk key material, so it cannot run in a unit test. The unit
//! tests below cover the pure argument-shape validation (the `Usage` arms); the
//! network success paths are exercised by the tests/ registry integration
//! harness.

use std::io::BufRead;

/// Where a user who does not know the command set should look. The top-level
/// `mfb` screen advertises only `repo register`/`repo auth`, so every error that
/// leaves the user hunting for a subcommand points at the full sub-help
/// (plan-42 §4.5).
const REPO_HELP_HINT: &str = "Run 'mfb repo --help' for all repository & auth commands.";

pub(crate) enum RepoCommandError {
    Usage(String),
    Failed(String),
}

pub(crate) fn run_repo_command(args: &[String]) -> Result<(), RepoCommandError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(RepoCommandError::Usage(format!(
            "mfb repo requires a subcommand (register, auth, link, trust)\n\n{REPO_HELP_HINT}"
        )));
    };

    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url).map_err(RepoCommandError::Failed)?;

    match command {
        "register" => {
            let [_, owner] = args else {
                return Err(RepoCommandError::Usage(
                    "mfb repo register requires exactly one <owner_name>".to_string(),
                ));
            };
            let response = mfb_repository::client::register(&repo_url, &paths, owner)
                .map_err(RepoCommandError::Failed)?;
            println!(
                "Registered owner {} with auth fingerprint {} and ident fingerprint {}",
                response.owner, response.auth_fingerprint, response.ident_fingerprint
            );
            Ok(())
        }
        "auth" => {
            let [_, owner] = args else {
                return Err(RepoCommandError::Usage(
                    "mfb repo auth requires exactly one <owner_name>".to_string(),
                ));
            };
            let response = mfb_repository::client::auth(&repo_url, &paths, owner)
                .map_err(RepoCommandError::Failed)?;
            println!(
                "Authenticated owner {} until {}",
                response.owner, response.expires_at
            );
            Ok(())
        }
        // Pin and verify the signed-metadata root of trust (plan-10-C2).
        "trust" => {
            let [_, registry_id, root_fingerprint] = args else {
                return Err(RepoCommandError::Usage(
                    "mfb repo trust requires <registry-id> <root-fingerprint>".to_string(),
                ));
            };
            let version = mfb_repository::client::trust_registry(
                &repo_url,
                &paths,
                registry_id,
                root_fingerprint,
            )
            .map_err(RepoCommandError::Failed)?;
            println!(
                "Pinned registry `{registry_id}` root {root_fingerprint}; metadata chain verified at snapshot version {version}."
            );
            Ok(())
        }
        // Machine link (plan-23 §3.2). Old machine: `mfb repo link --start
        // <owner>` displays a one-time pairing code. New machine: `mfb repo
        // link <owner>` reads the code from stdin and becomes a full equal.
        "link" => match args {
            [_, flag, owner] if flag == "--start" => {
                let (code, expires_at) =
                    mfb_repository::client::link_start(&repo_url, &paths, owner)
                        .map_err(RepoCommandError::Failed)?;
                println!("Pairing code (valid until {expires_at}, single use):");
                println!();
                println!("    {code}");
                println!();
                println!("On the new machine, run `mfb repo link {owner}` and enter the code.");
                Ok(())
            }
            [_, owner] => {
                println!("Enter the pairing code shown on the linked machine:");
                let mut code = String::new();
                std::io::stdin()
                    .lock()
                    .read_line(&mut code)
                    .map_err(|err| RepoCommandError::Failed(format!("failed to read pairing code: {err}")))?;
                let response =
                    mfb_repository::client::link_fetch(&repo_url, &paths, owner, code.trim())
                        .map_err(RepoCommandError::Failed)?;
                println!(
                    "Linked machine for owner {} with auth fingerprint {} and ident fingerprint {}",
                    response.owner, response.auth_fingerprint, response.ident_fingerprint
                );
                println!("Run `mfb repo auth {}` to open this machine's session.", response.owner);
                Ok(())
            }
            _ => Err(RepoCommandError::Usage(
                "mfb repo link requires `--start <owner_name>` (old machine) or `<owner_name>` (new machine)"
                    .to_string(),
            )),
        },
        _ => Err(RepoCommandError::Usage(format!(
            "unknown mfb repo command '{command}'\n\n{REPO_HELP_HINT}"
        ))),
    }
}

/// `mfb key rotate <owner>` — rotate the account ident (plan-23-B2): the new
/// ident is chained to the old by an old-ident signature, consumers follow
/// the chain, and other linked machines must re-link (the new private key is
/// never distributed automatically — rotations happen because a machine was
/// lost).
pub(crate) fn run_key_command(args: &[String]) -> Result<(), RepoCommandError> {
    match args {
        [command, owner] if command == "rotate" => {
            let repo_url = mfb_repository::client::repo_url_from_env();
            let paths = super::local_paths_for_repo(&repo_url).map_err(RepoCommandError::Failed)?;
            let response = mfb_repository::client::rotate_ident(&repo_url, &paths, owner)
                .map_err(RepoCommandError::Failed)?;
            println!(
                "Rotated ident for owner {}; new ident fingerprint {}",
                response.owner, response.ident_fingerprint
            );
            println!(
                "Other linked machines still hold the OLD ident key; run `mfb repo link` from this machine to re-link them."
            );
            Ok(())
        }
        [command, ..] if command == "rotate" => Err(RepoCommandError::Usage(
            "mfb key rotate requires exactly one <owner_name>".to_string(),
        )),
        [command, ..] => Err(RepoCommandError::Usage(format!(
            "unknown mfb key command '{command}'\n\n{REPO_HELP_HINT}"
        ))),
        [] => Err(RepoCommandError::Usage(format!(
            "mfb key requires a subcommand (rotate)\n\n{REPO_HELP_HINT}"
        ))),
    }
}

/// `mfb org grant|remove <org> <member> [role]` — manage org membership
/// (plan-10-D1). The grantor is `--as <owner>` (default: the org itself, for
/// the first grant); the grantor's local ident + session authorize the change.
pub(crate) fn run_org_command(args: &[String]) -> Result<(), RepoCommandError> {
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url).map_err(RepoCommandError::Failed)?;
    // Optional `--as <grantor>` overrides the acting account (default: the org).
    let mut positional = Vec::new();
    let mut grantor: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--as" {
            grantor = Some(
                iter.next()
                    .ok_or_else(|| RepoCommandError::Usage("--as requires <grantor>".to_string()))?
                    .clone(),
            );
        } else {
            positional.push(arg.clone());
        }
    }
    match positional.as_slice() {
        [command, org, member, role] if command == "grant" => {
            let grantor = grantor.unwrap_or_else(|| org.clone());
            let response =
                mfb_repository::client::set_org_member(&repo_url, &paths, org, &grantor, member, role, false)
                    .map_err(RepoCommandError::Failed)?;
            println!("Granted {} the {} role in org {}", response.member, response.role, response.org);
            Ok(())
        }
        [command, org, member] if command == "remove" => {
            let grantor = grantor.unwrap_or_else(|| org.clone());
            let response =
                mfb_repository::client::set_org_member(&repo_url, &paths, org, &grantor, member, "", true)
                    .map_err(RepoCommandError::Failed)?;
            println!("Removed {} from org {}", response.member, response.org);
            Ok(())
        }
        _ => Err(RepoCommandError::Usage(
            "mfb org grant <org> <member> <owner|admin|publisher> [--as <grantor>]\n       mfb org remove <org> <member> [--as <grantor>]".to_string(),
        )),
    }
}

/// `mfb token issue|revoke` — manage scoped publish tokens (plan-10-D1).
pub(crate) fn run_token_command(args: &[String]) -> Result<(), RepoCommandError> {
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url).map_err(RepoCommandError::Failed)?;
    match args {
        [command, owner, scope, ttl] if command == "issue" => {
            let ttl_seconds: i64 = ttl
                .parse()
                .map_err(|_| RepoCommandError::Usage("<ttl-seconds> must be an integer".to_string()))?;
            let (response, token_private) =
                mfb_repository::client::issue_publish_token(&repo_url, &paths, owner, scope, ttl_seconds)
                    .map_err(RepoCommandError::Failed)?;
            println!(
                "Issued publish token {} for {} (scope {}, expires {})",
                response.token_fingerprint, response.owner, response.scope, response.expires_at
            );
            println!("Token PRIVATE key (deploy to CI as the auth key): {token_private}");
            Ok(())
        }
        [command, owner, fingerprint] if command == "revoke" => {
            let response =
                mfb_repository::client::revoke_publish_token(&repo_url, &paths, owner, fingerprint)
                    .map_err(RepoCommandError::Failed)?;
            println!(
                "Revoked publish token {} for {}",
                response.token_fingerprint, response.owner
            );
            Ok(())
        }
        _ => Err(RepoCommandError::Usage(
            "mfb token issue <owner> <scope> <ttl-seconds>\n       mfb token revoke <owner> <token-fingerprint>".to_string(),
        )),
    }
}

/// `mfb machine revoke <owner> <auth-fingerprint>` — revoke a lost machine's
/// auth key (plan-23 §3.6). Requires the ident key on this machine.
pub(crate) fn run_machine_command(args: &[String]) -> Result<(), RepoCommandError> {
    match args {
        [command, owner, fingerprint] if command == "revoke" => {
            let repo_url = mfb_repository::client::repo_url_from_env();
            let paths = super::local_paths_for_repo(&repo_url).map_err(RepoCommandError::Failed)?;
            let response =
                mfb_repository::client::revoke_machine(&repo_url, &paths, owner, fingerprint)
                    .map_err(RepoCommandError::Failed)?;
            println!(
                "Revoked auth key {} for owner {}; its sessions are closed.",
                response.auth_fingerprint, response.owner
            );
            Ok(())
        }
        [command, ..] if command == "revoke" => Err(RepoCommandError::Usage(
            "mfb machine revoke requires <owner_name> <auth-fingerprint>".to_string(),
        )),
        [command, ..] => Err(RepoCommandError::Usage(format!(
            "unknown mfb machine command '{command}'\n\n{REPO_HELP_HINT}"
        ))),
        [] => Err(RepoCommandError::Usage(format!(
            "mfb machine requires a subcommand (revoke)\n\n{REPO_HELP_HINT}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn usage(result: Result<(), RepoCommandError>) -> String {
        match result {
            Err(RepoCommandError::Usage(message)) => message,
            Err(RepoCommandError::Failed(message)) => {
                panic!("expected a usage error, got failure: {message}")
            }
            Ok(()) => panic!("expected a usage error, got Ok"),
        }
    }

    // These tests exercise only the pure argument-shape validation of each
    // subcommand dispatcher: the arms that reject a wrong argument count with a
    // `Usage` error return before any registry/network call. The success arms
    // (register/auth/link/trust/rotate/grant/issue/revoke) all perform live
    // HTTP against a registry and are covered by the tests/ integration harness.

    #[test]
    fn repo_requires_a_subcommand() {
        let message = usage(run_repo_command(&s(&[])));
        assert!(message.contains("register, auth, link, trust"));
    }

    /// plan-42: the top-level screen no longer lists the repo/key/machine command
    /// sets, so every discovery error must point at the sub-help that does.
    #[test]
    fn discovery_errors_point_at_the_repo_sub_help() {
        for message in [
            usage(run_repo_command(&s(&[]))),
            usage(run_repo_command(&s(&["frobnicate"]))),
            usage(run_key_command(&s(&[]))),
            usage(run_key_command(&s(&["bogus"]))),
            usage(run_machine_command(&s(&[]))),
            usage(run_machine_command(&s(&["bogus"]))),
        ] {
            assert!(
                message.contains("mfb repo --help"),
                "discovery error must point at the sub-help: {message}"
            );
        }
    }

    #[test]
    fn repo_register_requires_exactly_one_owner() {
        assert!(usage(run_repo_command(&s(&["register"]))).contains("exactly one <owner_name>"));
        assert!(usage(run_repo_command(&s(&["register", "a", "b"])))
            .contains("exactly one <owner_name>"));
    }

    #[test]
    fn repo_auth_requires_exactly_one_owner() {
        assert!(usage(run_repo_command(&s(&["auth"]))).contains("exactly one <owner_name>"));
        assert!(
            usage(run_repo_command(&s(&["auth", "a", "b"]))).contains("exactly one <owner_name>")
        );
    }

    #[test]
    fn repo_trust_requires_registry_and_fingerprint() {
        assert!(
            usage(run_repo_command(&s(&["trust"]))).contains("<registry-id> <root-fingerprint>")
        );
        assert!(usage(run_repo_command(&s(&["trust", "only-one"])))
            .contains("<registry-id> <root-fingerprint>"));
    }

    #[test]
    fn repo_link_rejects_bad_argument_shapes() {
        // No arguments at all, or too many after `--start`, are usage errors
        // that return before any network/stdin interaction. (`link <owner>` and
        // `link --start <owner>` are the success arms, covered by tests/.)
        assert!(usage(run_repo_command(&s(&["link"]))).contains("mfb repo link requires"));
        assert!(usage(run_repo_command(&s(&["link", "--start", "a", "b"])))
            .contains("mfb repo link requires"));
    }

    #[test]
    fn repo_rejects_unknown_command() {
        assert!(usage(run_repo_command(&s(&["frobnicate"]))).contains("unknown mfb repo command"));
    }

    #[test]
    fn key_rotate_requires_exactly_one_owner() {
        assert!(usage(run_key_command(&s(&["rotate"]))).contains("exactly one <owner_name>"));
        assert!(
            usage(run_key_command(&s(&["rotate", "a", "b"]))).contains("exactly one <owner_name>")
        );
    }

    #[test]
    fn key_requires_a_subcommand_and_rejects_unknown() {
        assert!(usage(run_key_command(&s(&[]))).contains("mfb key requires a subcommand"));
        assert!(usage(run_key_command(&s(&["bogus"]))).contains("unknown mfb key command"));
    }

    #[test]
    fn org_rejects_bad_argument_shapes() {
        // No subcommand, wrong arity, or a dangling `--as`.
        assert!(usage(run_org_command(&s(&[]))).contains("mfb org grant"));
        assert!(usage(run_org_command(&s(&["grant", "org"]))).contains("mfb org grant"));
        assert!(usage(run_org_command(&s(&["--as"]))).contains("--as requires <grantor>"));
    }

    #[test]
    fn token_rejects_bad_argument_shapes() {
        assert!(usage(run_token_command(&s(&[]))).contains("mfb token issue"));
        assert!(
            usage(run_token_command(&s(&["issue", "owner", "scope"]))).contains("mfb token issue")
        );
        // A non-integer ttl is a usage error.
        assert!(
            usage(run_token_command(&s(&["issue", "owner", "scope", "soon"])))
                .contains("<ttl-seconds> must be an integer")
        );
    }

    #[test]
    fn machine_rejects_bad_argument_shapes() {
        assert!(usage(run_machine_command(&s(&[]))).contains("mfb machine requires a subcommand"));
        assert!(usage(run_machine_command(&s(&["bogus"]))).contains("unknown mfb machine command"));
        assert!(usage(run_machine_command(&s(&["revoke", "owner"])))
            .contains("<owner_name> <auth-fingerprint>"));
    }
}
