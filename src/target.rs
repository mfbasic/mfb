use std::env;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildTarget {
    pub os: String,
    pub arch: String,
}

impl BuildTarget {
    pub fn host() -> Self {
        Self {
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
        }
    }
}
