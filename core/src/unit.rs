use std::fmt::{self, Display};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid metric unit `{0}`")]
pub struct UnitParseError(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitPrefix {
    Nano,
    Micro,
    Milli,
    None,
    Kilo,
    Mega,
    Giga,
}

impl UnitPrefix {
    const PREFIXED: [Self; 6] = [
        Self::Nano,
        Self::Micro,
        Self::Milli,
        Self::Kilo,
        Self::Mega,
        Self::Giga,
    ];

    pub const fn exponent(self) -> i8 {
        match self {
            Self::Nano => -9,
            Self::Micro => -6,
            Self::Milli => -3,
            Self::None => 0,
            Self::Kilo => 3,
            Self::Mega => 6,
            Self::Giga => 9,
        }
    }

    const fn symbol(self) -> &'static str {
        match self {
            Self::Nano => "n",
            Self::Micro => "µ",
            Self::Milli => "m",
            Self::None => "",
            Self::Kilo => "k",
            Self::Mega => "M",
            Self::Giga => "G",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    Joule,
    Watt,
    Second,
    Byte,
    Count,
    Percent,
}

impl Unit {
    const ALL: [Self; 6] = [
        Self::Joule,
        Self::Watt,
        Self::Second,
        Self::Byte,
        Self::Count,
        Self::Percent,
    ];

    const fn symbol(self) -> &'static str {
        match self {
            Self::Joule => "J",
            Self::Watt => "W",
            Self::Second => "s",
            Self::Byte => "B",
            Self::Count => "count",
            Self::Percent => "%",
        }
    }

    /// Counts and percentages are never scaled by a prefix.
    const fn takes_prefix(self) -> bool {
        !matches!(self, Self::Count | Self::Percent)
    }
}

/// A unit and its SI prefix, written as usual (`mJ`, `µs`, `kB`, `count`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MetricUnit {
    pub prefix: UnitPrefix,
    pub unit: Unit,
}

impl MetricUnit {
    pub const JOULE: Self = Self::new(UnitPrefix::None, Unit::Joule);
    pub const MILLIJOULE: Self = Self::new(UnitPrefix::Milli, Unit::Joule);
    pub const MICROJOULE: Self = Self::new(UnitPrefix::Micro, Unit::Joule);
    pub const MILLISECOND: Self = Self::new(UnitPrefix::Milli, Unit::Second);
    pub const MICROSECOND: Self = Self::new(UnitPrefix::Micro, Unit::Second);
    pub const BYTE: Self = Self::new(UnitPrefix::None, Unit::Byte);
    pub const COUNT: Self = Self::new(UnitPrefix::None, Unit::Count);
    pub const PERCENT: Self = Self::new(UnitPrefix::None, Unit::Percent);

    pub const fn new(prefix: UnitPrefix, unit: Unit) -> Self {
        Self { prefix, unit }
    }

    pub fn precision(self) -> usize {
        let exponent = i32::from(self.prefix.exponent());
        let places = match self.unit {
            Unit::Joule | Unit::Watt | Unit::Second => 6 + exponent,
            Unit::Byte | Unit::Count => exponent,
            Unit::Percent => 2,
        };

        usize::try_from(places.clamp(0, 9)).unwrap_or_default()
    }
}

impl Display for MetricUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.prefix.symbol())?;
        f.write_str(self.unit.symbol())
    }
}

impl FromStr for MetricUnit {
    type Err = UnitParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let unit = |symbol: &str| Unit::ALL.into_iter().find(|unit| unit.symbol() == symbol);

        if let Some(unit) = unit(s) {
            return Ok(Self::new(UnitPrefix::None, unit));
        }

        s.strip_prefix('u')
            .map(|rest| (UnitPrefix::Micro, rest))
            .into_iter()
            .chain(
                UnitPrefix::PREFIXED.into_iter().filter_map(|prefix| {
                    s.strip_prefix(prefix.symbol()).map(|rest| (prefix, rest))
                }),
            )
            .find_map(|(prefix, rest)| {
                unit(rest)
                    .filter(|unit| unit.takes_prefix())
                    .map(|unit| Self::new(prefix, unit))
            })
            .ok_or_else(|| UnitParseError(s.to_owned()))
    }
}

impl Serialize for MetricUnit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for MetricUnit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(s: &str) -> MetricUnit {
        s.parse().expect("a valid unit")
    }

    #[test]
    fn units_are_read_with_their_prefix() {
        assert_eq!(parsed("J"), MetricUnit::JOULE);
        assert_eq!(parsed("mJ"), MetricUnit::MILLIJOULE);
        assert_eq!(parsed("µJ"), MetricUnit::MICROJOULE);
        assert_eq!(parsed("uJ"), MetricUnit::MICROJOULE);
        assert_eq!(parsed("ms"), MetricUnit::MILLISECOND);
        assert_eq!(parsed("GW"), MetricUnit::new(UnitPrefix::Giga, Unit::Watt));
        assert_eq!(parsed("kB"), MetricUnit::new(UnitPrefix::Kilo, Unit::Byte));
        assert_eq!(parsed("count"), MetricUnit::COUNT);
        assert_eq!(parsed("%"), MetricUnit::PERCENT);
    }

    #[test]
    fn what_is_not_a_unit_is_refused() {
        for invalid in ["", "k", "Hz", "kcount", "m%", "JJ", "mmJ"] {
            assert!(invalid.parse::<MetricUnit>().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn a_unit_reads_back_as_it_is_written() {
        for unit in ["J", "mW", "ns", "kB", "GJ", "µs", "count", "%"] {
            assert_eq!(parsed(unit).to_string(), unit);
        }
    }

    #[test]
    fn precision_goes_down_to_the_micro_base_unit_for_energy() {
        assert_eq!(MetricUnit::JOULE.precision(), 6);
        assert_eq!(MetricUnit::MILLIJOULE.precision(), 3);
        assert_eq!(MetricUnit::MICROJOULE.precision(), 0);
        assert_eq!(MetricUnit::BYTE.precision(), 0);
        assert_eq!(MetricUnit::new(UnitPrefix::Kilo, Unit::Byte).precision(), 3);
        assert_eq!(MetricUnit::PERCENT.precision(), 2);
    }
}
