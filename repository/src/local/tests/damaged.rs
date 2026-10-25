//! Local state the client cannot use as-is: a pinned root version that is not
//! text, and a staged ident rotation whose live key paths are occupied by
//! directories. Each must fail with the path it could not read or write, and a
//! failed promotion must leave the staged key in place for a retry (bug-793).

use super::*;

#[test]
fn a_pinned_root_version_that_is_not_text_is_a_read_error() {
    let (_temp, paths) = fixture();
    fs::write(paths.root_version_path(), [0xff, 0xfe, 0x00]).unwrap();
    let err = read_root_version(&paths).unwrap_err();
    assert!(
        err.starts_with(&format!(
            "failed to read root version '{}': ",
            paths.root_version_path().display()
        )),
        "{err}"
    );
}

/// Stage a rotation for alice and return the staged public key.
fn staged(paths: &LocalPaths) -> Vec<u8> {
    let (live_public, live_private) = crypto::generate_keypair();
    write_ident_keypair(paths, "alice", &live_public, &live_private).unwrap();
    let (next_public, next_private) = crypto::generate_keypair();
    write_pending_ident_keypair(paths, "alice", &next_public, &next_private).unwrap();
    next_public
}

#[test]
fn a_promotion_that_cannot_write_the_live_public_key_keeps_the_staged_key() {
    let (_temp, paths) = fixture();
    staged(&paths);
    let public_path = paths.ident_public_key_path("alice");
    fs::remove_file(&public_path).unwrap();
    fs::create_dir(&public_path).unwrap();
    let err = promote_pending_ident_keypair(&paths, "alice").unwrap_err();
    assert!(
        err.starts_with(&format!("failed to write '{}': ", public_path.display())),
        "{err}"
    );
    assert!(read_pending_ident_private_key(&paths, "alice")
        .unwrap()
        .is_some());
}

#[test]
fn a_promotion_that_cannot_move_the_staged_private_key_names_both_paths() {
    let (_temp, paths) = fixture();
    let next_public = staged(&paths);
    let from = paths.ident_pending_private_key_path("alice");
    let to = paths.ident_private_key_path("alice");
    fs::remove_file(&to).unwrap();
    fs::create_dir(&to).unwrap();
    fs::write(to.join("occupied"), b"").unwrap();
    let err = promote_pending_ident_keypair(&paths, "alice").unwrap_err();
    assert!(
        err.starts_with(&format!(
            "failed to promote '{}' to '{}': ",
            from.display(),
            to.display()
        )),
        "{err}"
    );
    // The public half was already rewritten and the staged private key is
    // still there, so re-running the promotion finishes the job.
    assert_eq!(read_ident_public_key(&paths, "alice").unwrap(), next_public);
    assert!(from.exists());
}
