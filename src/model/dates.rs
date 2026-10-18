use chrono::{Datelike, Days, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

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

/// A due date with an optional time of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
}

/// Parses a due date with an optional time: anything [`parse_due`] accepts,
/// optionally followed by a time (`friday 15:00`, `tomorrow at 3pm`), an
/// ISO date and time (`2026-10-23T15:00`), or only a time, meaning today.
pub fn parse_due_time(input: &str) -> Result<Due, String> {
    let error = || {
        format!(
            "expected a date (YYYY-MM-DD, \"today\", \"tomorrow\" or a weekday), \
             optionally followed by a time (15:00, 3pm), got \"{input}\""
        )
    };
    let text = input.trim().to_lowercase().replace(" am", "am").replace(" pm", "pm");
    if let Ok(date) = parse_due(&text) {
        return Ok(Due { date, time: None });
    }
    if let Some(time) = parse_time(&text) {
        return Ok(Due { date: today(), time: Some(time) });
    }
    for format in ["%Y-%m-%dt%H:%M", "%Y-%m-%dt%H:%M:%S"] {
        if let Ok(at) = NaiveDateTime::parse_from_str(&text, format) {
            return Ok(Due { date: at.date(), time: Some(at.time()) });
        }
    }
    let (date, time) = text.rsplit_once(' ').ok_or_else(error)?;
    let date = date.trim_end().strip_suffix(" at").unwrap_or(date);
    Ok(Due {
        date: parse_due(date).map_err(|_| error())?,
        time: Some(parse_time(time).ok_or_else(error)?),
    })
}

/// "15:00", "9:30", "3pm", "3:30pm" (lowercase, no spaces).
fn parse_time(text: &str) -> Option<NaiveTime> {
    let (clock, afternoon) = match (text.strip_suffix("am"), text.strip_suffix("pm")) {
        (Some(clock), _) => (clock, Some(false)),
        (_, Some(clock)) => (clock, Some(true)),
        _ => (text, None),
    };
    let (hour, minute) = clock.split_once(':').unwrap_or((clock, "00"));
    let all_digits = |s: &str| (1..=2).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(hour) || minute.len() != 2 || !all_digits(minute) {
        return None;
    }
    let (mut hour, minute): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    match afternoon {
        // "3pm" is 15:00, "12pm" noon and "12am" midnight.
        Some(pm) if (1..=12).contains(&hour) => hour = hour % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        // A bare number without ":" or am/pm is not a time.
        None if !clock.contains(':') => return None,
        None => {}
    }
    NaiveTime::from_hms_opt(hour, minute, 0)
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

    fn at(date: NaiveDate, time: &str) -> Due {
        Due { date, time: Some(time.parse().unwrap()) }
    }

    #[test]
    fn parse_due_time_accepts_dates_with_and_without_times() {
        let day = date("2026-10-23");
        assert_eq!(parse_due_time("2026-10-23"), Ok(Due { date: day, time: None }));
        assert_eq!(parse_due_time("2026-10-23T15:00"), Ok(at(day, "15:00:00")));
        assert_eq!(parse_due_time("2026-10-23 9:30"), Ok(at(day, "09:30:00")));
        assert_eq!(parse_due_time("2026-10-23 at 3pm"), Ok(at(day, "15:00:00")));
        assert_eq!(parse_due_time("2026-10-23 3:30 PM"), Ok(at(day, "15:30:00")));
        assert_eq!(parse_due_time("tomorrow 12am"), Ok(at(today() + Days::new(1), "00:00:00")));
        assert_eq!(parse_due_time("12pm"), Ok(at(today(), "12:00:00")));
        assert_eq!(parse_due_time("today 23:59"), Ok(at(today(), "23:59:00")));
        let friday = parse_due_time("Friday 15:00").unwrap();
        assert_eq!((friday.date.weekday(), friday.time), (Weekday::Fri, at(day, "15:00:00").time));
    }

    #[test]
    fn parse_due_time_rejects_bad_times() {
        for input in ["friday 25:00", "friday 15", "friday 13pm", "friday 0am", "15:0", "someday 15:00", "", "friday at"] {
            assert!(parse_due_time(input).is_err(), "should reject {input:?}");
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
