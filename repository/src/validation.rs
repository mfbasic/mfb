pub const OWNER_LIMIT: usize = 255;

pub fn fold_owner(owner: &str) -> String {
    owner.to_ascii_lowercase()
}

pub fn validate_owner_name(owner: &str) -> Result<(), String> {
    if owner.is_empty() {
        return Err("missing owner name".to_string());
    }
    if owner.len() > OWNER_LIMIT {
        return Err("invalid owner name: owner name is too long".to_string());
    }
    if !owner.is_ascii() {
        return Err("invalid owner name: owner name must be ASCII".to_string());
    }
    if owner.eq_ignore_ascii_case("std") {
        return Err("reserved owner name: std".to_string());
    }

    let mut chars = owner.chars();
    let Some(first) = chars.next() else {
        return Err("missing owner name".to_string());
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err("invalid owner name: must start with a letter or underscore".to_string());
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err(
            "invalid owner name: only ASCII letters, digits, and underscores are allowed"
                .to_string(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_validation_accepts_valid_names() {
        for owner in ["alice", "Alice", "_owner", "owner_1", "A123"] {
            validate_owner_name(owner).expect(owner);
        }
    }

    #[test]
    fn owner_validation_rejects_invalid_names() {
        for owner in [
            "",
            "std",
            "STD",
            "1alice",
            "alice-bob",
            "alice/bob",
            "alice.bob",
            "éclair",
        ] {
            assert!(validate_owner_name(owner).is_err(), "{owner}");
        }
        assert!(validate_owner_name(&"a".repeat(OWNER_LIMIT + 1)).is_err());
    }
}
