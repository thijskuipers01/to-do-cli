use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatUnit {
    Day,
    Week,
    Month,
    Year,
}

/// How often a task comes back, e.g. every 2 weeks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repeat {
    pub unit: RepeatUnit,
    pub interval: u32,
}

impl Repeat {
    /// "every week", "every 2 weeks", ...
    pub fn label(self) -> String {
        let unit = match self.unit {
            RepeatUnit::Day => "day",
            RepeatUnit::Week => "week",
            RepeatUnit::Month => "month",
            RepeatUnit::Year => "year",
        };
        match self.interval {
            1 => format!("every {unit}"),
            n => format!("every {n} {unit}s"),
        }
    }
}

/// Parses a repeat interval: "week", "every week", "2 weeks", "weekly", ...
pub fn parse_repeat(input: &str) -> Result<Repeat, String> {
    let error = || {
        format!("expected something like \"week\", \"2 weeks\" or \"monthly\", got \"{input}\"")
    };
    let lower = input.trim().to_lowercase();
    let mut words: Vec<&str> = lower.split_whitespace().collect();
    if words.first() == Some(&"every") {
        words.remove(0);
    }
    let (interval, unit) = match words.as_slice() {
        [unit] => (1, *unit),
        [count, unit] => (count.parse().map_err(|_| error())?, *unit),
        _ => return Err(error()),
    };
    let unit = match unit.trim_end_matches('s') {
        "day" | "daily" => RepeatUnit::Day,
        "week" | "weekly" => RepeatUnit::Week,
        "month" | "monthly" => RepeatUnit::Month,
        "year" | "yearly" => RepeatUnit::Year,
        _ => return Err(error()),
    };
    if interval == 0 {
        return Err(error());
    }
    Ok(Repeat { unit, interval })
}

/// Same as [`parse_repeat`], so `"2 weeks".parse::<Repeat>()` works too.
impl FromStr for Repeat {
    type Err = String;

    fn from_str(input: &str) -> Result<Repeat, String> {
        parse_repeat(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repeat(unit: RepeatUnit, interval: u32) -> Repeat {
        Repeat { unit, interval }
    }

    #[test]
    fn labels() {
        assert_eq!(repeat(RepeatUnit::Day, 1).label(), "every day");
        assert_eq!(repeat(RepeatUnit::Week, 2).label(), "every 2 weeks");
        assert_eq!(repeat(RepeatUnit::Month, 3).label(), "every 3 months");
        assert_eq!(repeat(RepeatUnit::Year, 1).label(), "every year");
    }

    #[test]
    fn parse_accepts_common_forms() {
        let cases = [
            ("week", repeat(RepeatUnit::Week, 1)),
            ("every week", repeat(RepeatUnit::Week, 1)),
            ("weekly", repeat(RepeatUnit::Week, 1)),
            ("2 weeks", repeat(RepeatUnit::Week, 2)),
            ("every 3 days", repeat(RepeatUnit::Day, 3)),
            ("daily", repeat(RepeatUnit::Day, 1)),
            ("Monthly", repeat(RepeatUnit::Month, 1)),
            ("  every 1 year ", repeat(RepeatUnit::Year, 1)),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_repeat(input), Ok(expected), "input {input:?}");
        }
    }

    #[test]
    fn parse_rejects_nonsense() {
        for input in ["", "every", "fortnight", "0 weeks", "two weeks", "2 weeks please", "-1 day"] {
            assert!(parse_repeat(input).is_err(), "should reject {input:?}");
        }
    }

    #[test]
    fn labels_parse_back_to_the_same_repeat() {
        for r in [repeat(RepeatUnit::Day, 1), repeat(RepeatUnit::Week, 2), repeat(RepeatUnit::Year, 5)] {
            assert_eq!(r.label().parse(), Ok(r));
        }
    }
}
