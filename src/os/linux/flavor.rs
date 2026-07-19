#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxFlavor {
    Glibc,
    Musl,
}

impl LinuxFlavor {
    pub(crate) const ALL: [Self; 2] = [Self::Glibc, Self::Musl];

    /// The locator-axis `Libc` this flavor selects (plan-56-B §4.3), so vendor
    /// resolution and artifact emission agree on which blob belongs in which
    /// AppImage.
    pub(crate) fn libc(self) -> crate::manifest::libraries::Libc {
        match self {
            Self::Glibc => crate::manifest::libraries::Libc::Glibc,
            Self::Musl => crate::manifest::libraries::Libc::Musl,
        }
    }

    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Self::Glibc => "glibc",
            Self::Musl => "musl",
        }
    }
}
