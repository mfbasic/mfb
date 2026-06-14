#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxFlavor {
    Glibc,
    Musl,
}

impl LinuxFlavor {
    pub(crate) const ALL: [Self; 2] = [Self::Glibc, Self::Musl];

    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Self::Glibc => "glibc",
            Self::Musl => "musl",
        }
    }
}
