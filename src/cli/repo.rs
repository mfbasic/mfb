//! `mfb repo`/`key`/`org`/`token`/`machine` account commands, plus the dispatch
//! for the publisher-side package commands (plan-60-A).
//!
//! `publish`/`check-abi`/`release-state`/`transfer`/`transfer-accept` are
//! dispatched here but *implemented* in `super::pkg`, next to the private
//! helpers they use. Only the command surface moved.
//!
//! coverage:off (success arms) — every non-Usage code path here performs live
//! HTTP against a registry (register/auth/link/trust/rotate/grant/issue/revoke,
//! and the five publisher commands) and mutates on-disk key material, so it
//! cannot run in a unit test. The unit tests below cover the pure argument-shape
//! validation (the `Usage` arms); the network success paths are exercised by the
//! tests/ registry integration harness.
//!
//! Note the publisher arms are *not* wholly coverage:off: their argument
//! destructuring is unit-tested on both sides of every arity boundary, and the
//! `reaches_dispatch` probes execute each arm body all the way to its
//! `super::pkg::…` call. Only the network-bound implementations are excluded.

use std::io::BufRead;
use std::path::Path;

/// Where a user who does not know the command set should look. The top-level
/// `mfb` screen advertises only `repo register`/`repo auth`, so every error that
/// leaves the user hunting for a subcommand points at the full sub-help
/// (plan-42 §4.5).
pub(crate) const REPO_HELP_HINT: &str =
    "Run 'mfb repo --help' for all repository, auth & publishing commands.";

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
    // Resolved lazily, inside the arms that need it (plan-60-A §4.3). Resolving
    // eagerly here would make the publisher commands' *arity* errors depend on
    // key-store state: `mfb repo check-abi a b c` must report an argument error
    // without ever touching the key store, exactly as `mfb pkg check-abi a b c`
    // did. The five publisher arms never call this — each of their
    // implementations in `super::pkg` resolves its own paths.
    let paths = || super::local_paths_for_repo(&repo_url).map_err(RepoCommandError::Failed);

    match command {
        "register" => {
            let [_, owner] = args else {
                return Err(RepoCommandError::Usage(
                    "mfb repo register requires exactly one <owner_name>".to_string(),
                ));
            };
            let response = mfb_repository::client::register(&repo_url, &paths()?, owner)
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
            let response = mfb_repository::client::auth(&repo_url, &paths()?, owner)
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
                &paths()?,
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
                    mfb_repository::client::link_start(&repo_url, &paths()?, owner)
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
                    mfb_repository::client::link_fetch(&repo_url, &paths()?, owner, code.trim())
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
        // Publisher-side commands (plan-60-A). Dispatch lives here; the
        // implementations stay in `super::pkg` alongside the private helpers
        // they use. Each implementation resolves its own repo paths, so these
        // arms never touch `paths()` — an arity error here never depends on
        // key-store state.
        "publish" => {
            // `[path]` is optional and defaults to the current directory. Note
            // the second positional is a project *directory*, not a package
            // name — `publish_package_project` takes it as `project_dir`.
            let (owner, project_dir) = match args {
                [_, owner] => (owner, Path::new(".")),
                [_, owner, path] => (owner, Path::new(path.as_str())),
                _ => {
                    return Err(RepoCommandError::Usage(
                        "mfb repo publish requires <owner_name> [path]".to_string(),
                    ))
                }
            };
            super::pkg::publish_package_project(owner, project_dir).map_err(RepoCommandError::Failed)
        }
        "check-abi" => {
            let project_dir = match args {
                [_] => Path::new("."),
                [_, location] => Path::new(location.as_str()),
                _ => {
                    return Err(RepoCommandError::Usage(
                        "mfb repo check-abi accepts at most one [location]".to_string(),
                    ))
                }
            };
            super::pkg::check_abi(project_dir).map_err(RepoCommandError::Failed)
        }
        "release-state" => {
            let (state, version) = match args {
                [_, state] => (state, None),
                [_, state, version] => (state, Some(version.as_str())),
                _ => {
                    return Err(RepoCommandError::Usage(
                        "mfb repo release-state requires <available|deprecated|yanked> [version]"
                            .to_string(),
                    ))
                }
            };
            super::pkg::set_release_state(Path::new("."), state, version)
                .map_err(RepoCommandError::Failed)
        }
        "transfer" => {
            let [_, ident, to_owner] = args else {
                return Err(RepoCommandError::Usage(
                    "mfb repo transfer requires <owner>#<package> <to-owner>".to_string(),
                ));
            };
            super::pkg::transfer_offer(ident, to_owner).map_err(RepoCommandError::Failed)
        }
        "transfer-accept" => {
            let [_, ident] = args else {
                return Err(RepoCommandError::Usage(
                    "mfb repo transfer-accept requires <owner>#<package>@<to-owner>".to_string(),
                ));
            };
            super::pkg::transfer_accept(ident).map_err(RepoCommandError::Failed)
        }
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

    /// Assert the command *reached its implementation* rather than being
    /// rejected on arity. Every one of these argument shapes is chosen to fail
    /// early inside the implementation on a pure, offline check — a malformed
    /// ident, an invalid state name, or a directory with no `project.json` — so
    /// nothing here touches the network or the key store.
    fn reaches_dispatch(result: Result<(), RepoCommandError>) -> String {
        match result {
            Err(RepoCommandError::Failed(message)) => message,
            Err(RepoCommandError::Usage(message)) => {
                panic!(
                    "expected the arity to be accepted and dispatch reached, got usage: {message}"
                )
            }
            Ok(()) => panic!("expected a failure from the implementation, got Ok"),
        }
    }

    // A directory that cannot contain a project.json, so `publish`/`check-abi`
    // fail on manifest validation immediately after dispatch.
    const NO_PROJECT: &str = "/nonexistent-plan60-no-project-here";

    /// plan-60-A §4.2: the five publisher-side commands dispatch from `repo`,
    /// with the arity table this pins. The risk this covers is the translation
    /// from `pkg`'s slice matching to `repo`'s `match` + destructure style,
    /// where an arity check can silently loosen without anything noticing.
    #[test]
    fn repo_publisher_commands_pin_their_arity() {
        // publish <owner> [path] — 1 or 2.
        assert!(usage(run_repo_command(&s(&["publish"]))).contains("requires <owner_name> [path]"));
        assert!(usage(run_repo_command(&s(&["publish", "a", "b", "c"])))
            .contains("requires <owner_name> [path]"));
        reaches_dispatch(run_repo_command(&s(&["publish", "alice", NO_PROJECT])));

        // check-abi [path] — 0 or 1.
        assert!(usage(run_repo_command(&s(&["check-abi", "a", "b"])))
            .contains("accepts at most one [location]"));
        reaches_dispatch(run_repo_command(&s(&["check-abi", NO_PROJECT])));

        // release-state <state> [version] — 1 or 2.
        assert!(usage(run_repo_command(&s(&["release-state"]))).contains("requires <available"));
        assert!(
            usage(run_repo_command(&s(&["release-state", "a", "b", "c"])))
                .contains("requires <available")
        );
        // A bogus state is rejected by the implementation, not by dispatch —
        // which is exactly what proves both arities got through.
        assert!(
            reaches_dispatch(run_repo_command(&s(&["release-state", "bogus"])))
                .contains("state must be one of")
        );
        assert!(
            reaches_dispatch(run_repo_command(&s(&["release-state", "bogus", "1.0.0"])))
                .contains("state must be one of")
        );

        // transfer <ident> <to-owner> — exactly 2.
        assert!(usage(run_repo_command(&s(&["transfer", "a"])))
            .contains("requires <owner>#<package> <to-owner>"));
        assert!(usage(run_repo_command(&s(&["transfer", "a", "b", "c"])))
            .contains("requires <owner>#<package> <to-owner>"));
        assert!(
            reaches_dispatch(run_repo_command(&s(&["transfer", "no-hash", "bob"])))
                .contains("ident must use")
        );

        // transfer-accept <ident>@<to-owner> — exactly 1.
        assert!(usage(run_repo_command(&s(&["transfer-accept"]))).contains("requires <owner>"));
        assert!(usage(run_repo_command(&s(&["transfer-accept", "a", "b"])))
            .contains("requires <owner>"));
        assert!(
            reaches_dispatch(run_repo_command(&s(&["transfer-accept", "ada#shape"])))
                .contains("<owner>#<package>@<to-owner>")
        );
    }

    /// plan-60-A §4.2: `publish` gains an optional path defaulting to `.`, so
    /// `mfb repo publish alice` must behave exactly as `mfb repo publish alice
    /// .` does. Both forms must reach dispatch (not a `Usage` error) and produce
    /// the identical result.
    ///
    /// Deliberately does NOT `chdir`: the working directory is process-global,
    /// and mutating it races every other test in this binary that resolves a
    /// relative path — `ENV_LOCK` would not protect them, since they do not take
    /// it. The real current-directory semantics are proven instead in a
    /// subprocess, where they are safe to exercise, by
    /// `tests/repo_acceptance.rs::repo_publish_without_a_path_publishes_the_current_directory`.
    /// What this test pins is the dispatch-level claim: the one-argument form is
    /// accepted and forwards exactly `Path::new(".")`.
    #[test]
    fn repo_publish_defaults_its_path_to_the_current_directory() {
        let implicit = reaches_dispatch(run_repo_command(&s(&["publish", "alice"])));
        let explicit = reaches_dispatch(run_repo_command(&s(&["publish", "alice", "."])));
        assert_eq!(implicit, explicit);

        // ...and `.` is genuinely what got forwarded: calling the implementation
        // directly with `Path::new(".")` yields the same failure.
        let direct = super::super::pkg::publish_package_project("alice", Path::new("."))
            .expect_err("the crate root is not a package project");
        assert_eq!(implicit, direct);
        assert!(
            implicit.contains("package project validation failed"),
            "{implicit}"
        );
    }

    /// plan-60-A §4.3: an argument-shape error must never depend on key-store
    /// state. `local_paths_for_repo` is the only thing in `run_repo_command`
    /// that can fail before dispatch, and it fails when neither `MFB_HOME` nor
    /// `HOME` is set — so with both unset, a wrong-arity publisher command must
    /// still report its arity error rather than "HOME is not set".
    ///
    /// This is the regression guard for the eager-resolution difference between
    /// the old `pkg` dispatch and the new `repo` one.
    #[test]
    fn arity_errors_do_not_depend_on_the_key_store() {
        let _lock = super::super::tests::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _mfb = super::super::tests::EnvVarGuard::unset("MFB_HOME");
        let _home = super::super::tests::EnvVarGuard::unset("HOME");

        // Sanity: the key store really is unresolvable in this state, so the
        // assertions below are not vacuous.
        assert!(super::super::local_paths_for_repo("repo")
            .unwrap_err()
            .contains("HOME is not set"));

        for args in [
            vec!["check-abi", "a", "b"],
            vec!["publish", "a", "b", "c"],
            vec!["release-state", "a", "b", "c"],
            vec!["transfer", "a"],
            vec!["transfer-accept", "a", "b"],
        ] {
            let message = usage(run_repo_command(&s(&args)));
            assert!(
                !message.contains("HOME is not set"),
                "`repo {args:?}` must report its arity, not the key store: {message}"
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
