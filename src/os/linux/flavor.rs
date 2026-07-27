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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::libraries::Libc;

    #[test]
    fn libc_maps_each_flavor_to_its_axis() {
        assert_eq!(LinuxFlavor::Glibc.libc(), Libc::Glibc);
        assert_eq!(LinuxFlavor::Musl.libc(), Libc::Musl);
    }

    #[test]
    fn suffix_names_each_flavor() {
        assert_eq!(LinuxFlavor::Glibc.suffix(), "glibc");
        assert_eq!(LinuxFlavor::Musl.suffix(), "musl");
    }

    #[test]
    fn all_lists_both_flavors_in_order() {
        assert_eq!(LinuxFlavor::ALL, [LinuxFlavor::Glibc, LinuxFlavor::Musl]);
    }
}
