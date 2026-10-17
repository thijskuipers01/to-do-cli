use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// How urgent a task is. The derived `Ord` follows declaration order,
/// so `None < Low < Medium < High`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Priority {
    None,
    Low,
    Medium,
    High,
}

impl Priority {
    /// Converts from EventKit's scale: 0 = none, 1–4 = high, 5 = medium, 6–9 = low.
    pub fn from_eventkit(value: usize) -> Priority {
        match value {
            0 => Priority::None,
            1..=4 => Priority::High,
            5 => Priority::Medium,
            _ => Priority::Low,
        }
    }

    /// The values the Reminders app itself uses for each level.
    pub fn to_eventkit(self) -> usize {
        match self {
            Priority::None => 0,
            Priority::High => 1,
            Priority::Medium => 5,
            Priority::Low => 9,
        }
    }

    /// Text shown in tables; empty when there is no priority.
    pub fn label(self) -> &'static str {
        match self {
            Priority::None => "",
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
        }
    }
}

/// Parses "none", "low", "medium" or "high", ignoring upper/lower case.
impl FromStr for Priority {
    type Err = String;

    fn from_str(input: &str) -> Result<Priority, String> {
        match input.trim().to_lowercase().as_str() {
            "none" => Ok(Priority::None),
            "low" => Ok(Priority::Low),
            "medium" => Ok(Priority::Medium),
            "high" => Ok(Priority::High),
            _ => Err(format!("expected none, low, medium or high, got \"{input}\"")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Priority; 4] = [Priority::None, Priority::Low, Priority::Medium, Priority::High];

    #[test]
    fn from_eventkit_covers_the_whole_scale() {
        assert_eq!(Priority::from_eventkit(0), Priority::None);
        for value in 1..=4 {
            assert_eq!(Priority::from_eventkit(value), Priority::High);
        }
        assert_eq!(Priority::from_eventkit(5), Priority::Medium);
        for value in 6..=9 {
            assert_eq!(Priority::from_eventkit(value), Priority::Low);
        }
    }

    #[test]
    fn survives_a_round_trip_through_eventkit() {
        for priority in ALL {
            assert_eq!(Priority::from_eventkit(priority.to_eventkit()), priority);
        }
    }

    #[test]
    fn labels_and_order() {
        assert_eq!(Priority::None.label(), "");
        assert_eq!(Priority::Low.label(), "low");
        assert_eq!(Priority::Medium.label(), "medium");
        assert_eq!(Priority::High.label(), "high");
        assert!(Priority::None < Priority::Low);
        assert!(Priority::Low < Priority::Medium);
        assert!(Priority::Medium < Priority::High);
    }

    #[test]
    fn parses_names_ignoring_case() {
        assert_eq!("none".parse(), Ok(Priority::None));
        assert_eq!(" Low ".parse(), Ok(Priority::Low));
        assert_eq!("MEDIUM".parse(), Ok(Priority::Medium));
        assert_eq!("high".parse(), Ok(Priority::High));
        assert!("urgent".parse::<Priority>().is_err());
        assert!("".parse::<Priority>().is_err());
    }
}
