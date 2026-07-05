use std::io::BufRead;

pub(crate) enum RepoCommandError {
    Usage(String),
    Failed(String),
}

pub(crate) fn run_repo_command(args: &[String]) -> Result<(), RepoCommandError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(RepoCommandError::Usage(
            "mfb repo requires register, auth, or link".to_string(),
        ));
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
            "unknown mfb repo command '{command}'"
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
            let paths =
                super::local_paths_for_repo(&repo_url).map_err(RepoCommandError::Failed)?;
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
            "unknown mfb key command '{command}'"
        ))),
        [] => Err(RepoCommandError::Usage(
            "mfb key requires a subcommand (rotate)".to_string(),
        )),
    }
}

/// `mfb machine revoke <owner> <auth-fingerprint>` — revoke a lost machine's
/// auth key (plan-23 §3.6). Requires the ident key on this machine.
pub(crate) fn run_machine_command(args: &[String]) -> Result<(), RepoCommandError> {
    match args {
        [command, owner, fingerprint] if command == "revoke" => {
            let repo_url = mfb_repository::client::repo_url_from_env();
            let paths =
                super::local_paths_for_repo(&repo_url).map_err(RepoCommandError::Failed)?;
            let response = mfb_repository::client::revoke_machine(
                &repo_url,
                &paths,
                owner,
                fingerprint,
            )
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
            "unknown mfb machine command '{command}'"
        ))),
        [] => Err(RepoCommandError::Usage(
            "mfb machine requires a subcommand (revoke)".to_string(),
        )),
    }
}
