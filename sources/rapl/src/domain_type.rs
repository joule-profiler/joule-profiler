use crate::error::RaplError;
use std::fmt::Display;

/// Unique identifier for a domain and socket.
pub type RaplDomainIndex = (RaplDomainType, u32);

/// Types of RAPL (Running Average Power Limit) energy/power measurement domains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RaplDomainType {
    /// Entire processor socket.
    ///
    /// Includes core and uncore components.
    /// Available on all Intel processor since Sandy Bridge generation.
    /// Also known as PKG.
    Package,

    /// CPU cores only.
    ///
    /// Also known as PP0.
    Core,

    /// Integrated graphics device if available.
    ///
    /// Also known as PP1.
    Uncore,

    /// Random access memory attached to the CPU memory controller.
    Dram,

    /// Platform-level power (System on Chip).
    ///
    /// Measures total platform power including Package and additional `SoC` components.
    Psys,
}

impl RaplDomainType {
    /// Get the domain name, with its type and socket number.
    pub fn to_string_socket(self, socket: u32) -> String {
        match self {
            RaplDomainType::Psys => RaplDomainType::Psys.to_string(),
            domain_type => format!("{domain_type}-{socket}"),
        }
    }

    /// Converts event to str.
    pub fn to_perf_event(self) -> &'static str {
        match self {
            RaplDomainType::Package => "energy-pkg",
            RaplDomainType::Core => "energy-cores",
            RaplDomainType::Uncore => "energy-gpu",
            RaplDomainType::Dram => "energy-ram",
            RaplDomainType::Psys => "energy-psys",
        }
    }
}

impl Display for RaplDomainType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let domain_type = match self {
            RaplDomainType::Package => "PACKAGE",
            RaplDomainType::Core => "CORE",
            RaplDomainType::Uncore => "UNCORE",
            RaplDomainType::Dram => "DRAM",
            RaplDomainType::Psys => "PSYS",
        };
        f.write_str(domain_type)
    }
}

impl TryFrom<&str> for RaplDomainType {
    type Error = RaplError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let eq = |s| value.eq_ignore_ascii_case(s);

        let domain_type = if eq("core") || eq("pp0") || eq("energy-cores") {
            RaplDomainType::Core
        } else if eq("uncore") || eq("pp1") || eq("energy-uncore") || eq("energy-gpu") {
            RaplDomainType::Uncore
        } else if eq("dram") || eq("ram") || eq("energy-ram") {
            RaplDomainType::Dram
        } else if (value.len() >= 7 && value[..7].eq_ignore_ascii_case("package"))
            || eq("energy-pkg")
        {
            RaplDomainType::Package
        } else if eq("psys") || eq("platform") || eq("energy-psys") {
            RaplDomainType::Psys
        } else {
            return Err(RaplError::UnknownDomain(value.to_string()));
        };

        Ok(domain_type)
    }
}

#[cfg(test)]
mod test {
    use crate::{RaplError, domain_type::RaplDomainType};

    fn str_to_domain(domain: &str) -> RaplDomainType {
        RaplDomainType::try_from(domain).unwrap()
    }

    fn str_to_domain_err(domain: &str) -> RaplError {
        let result: Result<RaplDomainType, RaplError> = RaplDomainType::try_from(domain);
        result.unwrap_err()
    }

    #[test]
    fn string_to_rapl_domain_type_conversion_existing_domain() {
        assert_eq!(RaplDomainType::Core, str_to_domain("core"));
        assert_eq!(RaplDomainType::Core, str_to_domain("pp0"));
        assert_eq!(RaplDomainType::Core, str_to_domain("energy-cores"));

        assert_eq!(RaplDomainType::Uncore, str_to_domain("uncore"));
        assert_eq!(RaplDomainType::Uncore, str_to_domain("pp1"));
        assert_eq!(RaplDomainType::Uncore, str_to_domain("energy-uncore"));
        assert_eq!(RaplDomainType::Uncore, str_to_domain("energy-gpu"));

        assert_eq!(RaplDomainType::Package, str_to_domain("package"));
        assert_eq!(RaplDomainType::Package, str_to_domain("energy-pkg"));

        assert_eq!(RaplDomainType::Dram, str_to_domain("dram"));
        assert_eq!(RaplDomainType::Dram, str_to_domain("ram"));
        assert_eq!(RaplDomainType::Dram, str_to_domain("energy-ram"));

        assert_eq!(RaplDomainType::Psys, str_to_domain("psys"));
        assert_eq!(RaplDomainType::Psys, str_to_domain("platform"));
        assert_eq!(RaplDomainType::Psys, str_to_domain("energy-psys"));
    }

    #[test]
    fn string_to_rapl_domain_type_is_case_insensitive() {
        assert_eq!(RaplDomainType::Core, str_to_domain("CORE"));
        assert_eq!(RaplDomainType::Core, str_to_domain("Core"));
        assert_eq!(RaplDomainType::Dram, str_to_domain("DRAM"));
        assert_eq!(RaplDomainType::Package, str_to_domain("PACKAGE"));
        assert_eq!(RaplDomainType::Psys, str_to_domain("PSYS"));
    }

    #[test]
    fn string_to_rapl_domain_type_unknown_returns_error() {
        assert!(matches!(
            str_to_domain_err("unknown"),
            RaplError::UnknownDomain(s) if s == "unknown"
        ));
    }

    #[test]
    fn string_to_rapl_domain_type_empty_string_returns_error() {
        assert!(matches!(
            str_to_domain_err(""),
            RaplError::UnknownDomain(s) if s.is_empty()
        ));
    }

    #[test]
    fn string_to_rapl_domain_type_similar_but_invalid_names_return_error() {
        for invalid in &[
            "cores",
            "pp2",
            "energy-core",
            "energy-package",
            "energy-dram",
            "sys",
        ] {
            assert!(
                matches!(str_to_domain_err(invalid), RaplError::UnknownDomain(_)),
                "Expected UnknownDomain error for input: {invalid}"
            );
        }
    }

    #[test]
    fn perf_rapl_domain_to_perf_event_conversion() {
        assert_eq!("energy-cores", RaplDomainType::Core.to_perf_event());
        assert_eq!("energy-gpu", RaplDomainType::Uncore.to_perf_event());
        assert_eq!("energy-pkg", RaplDomainType::Package.to_perf_event());
        assert_eq!("energy-ram", RaplDomainType::Dram.to_perf_event());
        assert_eq!("energy-psys", RaplDomainType::Psys.to_perf_event());
    }

    #[test]
    fn to_string_socket_formats_correctly() {
        assert_eq!("CORE-0", RaplDomainType::Core.to_string_socket(0));
        assert_eq!("PACKAGE-1", RaplDomainType::Package.to_string_socket(1));
        assert_eq!("DRAM-2", RaplDomainType::Dram.to_string_socket(2));

        // Psys is socket with, thus socket number should be ignored
        assert_eq!("PSYS", RaplDomainType::Psys.to_string_socket(0));
    }
}
