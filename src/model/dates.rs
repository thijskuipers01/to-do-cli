use chrono::{Datelike, Days, Local, NaiveDate, Weekday};

/// Today's date in the local time zone.
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Parses a due date: `YYYY-MM-DD`, `today`, `tomorrow` or a weekday
/// (`friday`, `fri`), which means the next such day after today.
pub fn parse_due(input: &str) -> Result<NaiveDate, String> {
    let today = today();
    match input.trim().to_lowercase().as_str() {
        "today" => Ok(today),
        "tomorrow" => Ok(today + Days::new(1)),
        other => {
            if let Ok(weekday) = other.parse::<Weekday>() {
                return Ok(next_weekday(today, weekday));
            }
            NaiveDate::parse_from_str(other, "%Y-%m-%d").map_err(|_| {
                format!("expected YYYY-MM-DD, \"today\", \"tomorrow\" or a weekday, got \"{input}\"")
            })
        }
    }
}

/// The first `weekday` strictly after `from` (so "friday" on a Friday is next week).
fn next_weekday(from: NaiveDate, weekday: Weekday) -> NaiveDate {
    let ahead = (7 + weekday.num_days_from_monday() - from.weekday().num_days_from_monday()) % 7;
    let ahead = if ahead == 0 { 7 } else { ahead };
    from + Days::new(ahead.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn parse_due_accepts_dates_and_words() {
        assert_eq!(parse_due("2026-12-31"), Ok(date("2026-12-31")));
        assert_eq!(parse_due("today"), Ok(today()));
        assert_eq!(parse_due(" Tomorrow "), Ok(today() + Days::new(1)));
    }

    #[test]
    fn parse_due_weekdays_are_always_in_the_coming_week() {
        for name in ["monday", "Fri", "sunday", "SAT"] {
            let due = parse_due(name).unwrap();
            let days_ahead = (due - today()).num_days();
            assert!((1..=7).contains(&days_ahead), "{name} gave {due}");
            assert_eq!(due.weekday(), name.parse::<Weekday>().unwrap());
        }
    }

    #[test]
    fn parse_due_rejects_invalid_dates() {
        for input in ["", "2026-13-01", "2026-02-30", "31-12-2026", "someday"] {
            assert!(parse_due(input).is_err(), "should reject {input:?}");
        }
    }

    #[test]
    fn next_weekday_skips_to_next_week_on_the_same_day() {
        let saturday = date("2026-10-17");
        assert_eq!(next_weekday(saturday, Weekday::Sat), date("2026-10-24"));
        assert_eq!(next_weekday(saturday, Weekday::Sun), date("2026-10-18"));
        assert_eq!(next_weekday(saturday, Weekday::Fri), date("2026-10-23"));
        assert_eq!(next_weekday(saturday, Weekday::Mon), date("2026-10-19"));
    }
}
