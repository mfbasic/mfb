pub(crate) enum RepoCommandError {
    Usage(String),
    Failed(String),
}

pub(crate) fn run_repo_command(args: &[String]) -> Result<(), RepoCommandError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(RepoCommandError::Usage(
            "mfb repo requires register or auth".to_string(),
        ));
    };
    if args.len() != 2 {
        return Err(RepoCommandError::Usage(format!(
            "mfb repo {command} requires exactly one <owner_name>"
        )));
    }

    let owner = &args[1];
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url).map_err(RepoCommandError::Failed)?;

    match command {
        "register" => {
            let response = mfb_repository::client::register(&repo_url, &paths, owner)
                .map_err(RepoCommandError::Failed)?;
            println!(
                "Registered owner {} with auth fingerprint {} and ident fingerprint {}",
                response.owner, response.auth_fingerprint, response.ident_fingerprint
            );
            Ok(())
        }
        "auth" => {
            let response = mfb_repository::client::auth(&repo_url, &paths, owner)
                .map_err(RepoCommandError::Failed)?;
            println!(
                "Authenticated owner {} until {}",
                response.owner, response.expires_at
            );
            Ok(())
        }
        _ => Err(RepoCommandError::Usage(format!(
            "unknown mfb repo command '{command}'"
        ))),
    }
}
