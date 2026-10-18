//! A thin wrapper around EventKit, Apple's framework behind the Reminders app.
//! All the `unsafe` code for talking to Objective-C lives in this file; the
//! rest of the crate only sees the plain Rust types in `reminders`.

use std::sync::mpsc;

use anyhow::{Context, Result, anyhow};
use block2::RcBlock;
use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_event_kit::{
    EKAlarm, EKCalendar, EKEntityType, EKEventStore, EKRecurrenceFrequency, EKRecurrenceRule,
    EKReminder,
};
use objc2_foundation::{NSArray, NSDate, NSDateComponents, NSError, NSPredicate, NSString};

use super::{Backend, ListSummary, Reminder, ReminderData, summarize};
use crate::error::{Error, ErrorKind};
use crate::model::{Repeat, RepeatUnit};

/// One list in the Reminders app.
pub struct RemindersList {
    store: Retained<EKEventStore>,
    calendar: Retained<EKCalendar>,
}

impl RemindersList {
    /// Opens the list called `name`, creating it if it doesn't exist yet.
    /// The first time, macOS asks the user for permission.
    pub fn open(name: &str) -> Result<RemindersList> {
        let store = unsafe { EKEventStore::new() };
        request_access(&store)?;
        let calendar = match find_calendar(&store, name) {
            Some(calendar) => calendar,
            None => create_calendar(&store, name)?,
        };
        Ok(RemindersList { store, calendar })
    }

    /// Every Reminders list with its number of unfinished reminders, without
    /// opening (and so creating) any particular list.
    pub fn all_lists() -> Result<Vec<ListSummary>> {
        let store = unsafe { EKEventStore::new() };
        request_access(&store)?;
        list_summaries(&store)
    }

    fn find(&self, id: &str) -> Option<Retained<EKReminder>> {
        let item = unsafe { self.store.calendarItemWithIdentifier(&NSString::from_str(id)) }?;
        item.downcast::<EKReminder>().ok()
    }

    fn save(&self, reminder: &EKReminder, data: &ReminderData) -> Result<()> {
        let title = NSString::from_str(&data.title);
        let notes = (!data.notes.is_empty()).then(|| NSString::from_str(&data.notes));
        unsafe {
            reminder.setTitle(Some(&title));
            reminder.setNotes(notes.as_deref());
            reminder.setPriority(data.priority);
            if reminder.isCompleted() != data.done {
                reminder.setCompleted(data.done);
            }
            // Only touch the due date when it changes, so alarms set on the
            // iPhone survive edits made here.
            if due_of(reminder) != (data.due, data.due_time) {
                set_due(reminder, data.due, data.due_time);
            }
            // Same for the repeat rule, so more complex rules made on the
            // iPhone (e.g. "every Monday and Thursday") aren't overwritten.
            if repeat_of(reminder) != data.repeat {
                let rules = data.repeat.map(|r| NSArray::from_retained_slice(&[recurrence_rule(r)]));
                reminder.setRecurrenceRules(rules.as_deref());
            }
            self.store.saveReminder_commit_error(reminder, true).map_err(ns_error)
        }
    }

    /// Deletes the whole list. Only used to clean up after tests.
    #[cfg(test)]
    pub fn delete_list(self) -> Result<()> {
        unsafe { self.store.removeCalendar_commit_error(&self.calendar, true) }.map_err(ns_error)
    }
}

impl Backend for RemindersList {
    fn name(&self) -> String {
        unsafe { self.calendar.title() }.to_string()
    }

    fn all(&self) -> Result<Vec<Reminder>> {
        let calendars = NSArray::from_retained_slice(std::slice::from_ref(&self.calendar));
        let predicate = unsafe { self.store.predicateForRemindersInCalendars(Some(&calendars)) };
        fetch(&self.store, &predicate)
    }

    fn get(&self, id: &str) -> Option<Reminder> {
        self.find(id).map(|r| to_reminder(&r))
    }

    fn create(&self, data: &ReminderData, list: Option<&str>) -> Result<String> {
        let calendar = match list {
            Some(name) if name != self.name() => {
                match find_calendar(&self.store, name) {
                    Some(calendar) => calendar,
                    None => create_calendar(&self.store, name)?,
                }
            }
            _ => self.calendar.clone(),
        };
        let reminder = unsafe { EKReminder::reminderWithEventStore(&self.store) };
        unsafe { reminder.setCalendar(Some(&calendar)) };
        self.save(&reminder, data)?;
        Ok(unsafe { reminder.calendarItemIdentifier() }.to_string())
    }

    fn update(&self, id: &str, data: &ReminderData) -> Result<()> {
        let reminder = self.find(id).context("the reminder no longer exists")?;
        self.save(&reminder, data)
    }

    fn delete(&self, id: &str) -> Result<()> {
        if let Some(reminder) = self.find(id) {
            unsafe { self.store.removeReminder_commit_error(&reminder, true) }
                .map_err(ns_error)?;
        }
        Ok(())
    }

    fn lists(&self) -> Result<Vec<ListSummary>> {
        list_summaries(&self.store)
    }

    fn select_list(&mut self, name: &str) -> Result<()> {
        if name != self.name() {
            self.calendar = match find_calendar(&self.store, name) {
                Some(calendar) => calendar,
                None => create_calendar(&self.store, name)?,
            };
        }
        Ok(())
    }
}

/// Fetches the reminders matching `predicate`. EventKit answers on a
/// background thread by calling `handler`. The reminders are converted to
/// plain Rust data there and sent back through a channel, which `recv` below
/// waits on.
fn fetch(store: &EKEventStore, predicate: &NSPredicate) -> Result<Vec<Reminder>> {
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |array: *mut NSArray<EKReminder>| {
        let reminders = match unsafe { array.as_ref() } {
            Some(array) => array.iter().map(|r| to_reminder(&r)).collect(),
            None => Vec::new(),
        };
        let _ = tx.send(reminders);
    });
    unsafe { store.fetchRemindersMatchingPredicate_completion(predicate, &handler) };
    rx.recv().context("Reminders did not respond")
}

fn list_summaries(store: &EKEventStore) -> Result<Vec<ListSummary>> {
    let calendars = unsafe { store.calendarsForEntityType(EKEntityType::Reminder) };
    let names = calendars.iter().map(|calendar| unsafe { calendar.title() }.to_string());
    let predicate = unsafe {
        store.predicateForIncompleteRemindersWithDueDateStarting_ending_calendars(None, None, None)
    };
    Ok(summarize(names, &fetch(store, &predicate)?))
}

fn request_access(store: &EKEventStore) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        let _ = tx.send(granted.as_bool());
    });
    unsafe { store.requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&handler)) };
    if !rx.recv().unwrap_or(false) {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "todo has no access to Reminders. Allow your terminal app under \
             System Settings → Privacy & Security → Reminders, then try again.",
        )
        .into());
    }
    Ok(())
}

fn find_calendar(store: &EKEventStore, name: &str) -> Option<Retained<EKCalendar>> {
    let calendars = unsafe { store.calendarsForEntityType(EKEntityType::Reminder) };
    calendars
        .iter()
        .find(|calendar| unsafe { calendar.title() }.to_string() == name)
}

/// Creates a new list in the same account (e.g. iCloud) as the default list.
fn create_calendar(store: &EKEventStore, name: &str) -> Result<Retained<EKCalendar>> {
    let source = unsafe { store.defaultCalendarForNewReminders() }
        .and_then(|calendar| unsafe { calendar.source() })
        .context("found no Reminders account to create the list in")?;
    let calendar =
        unsafe { EKCalendar::calendarForEntityType_eventStore(EKEntityType::Reminder, store) };
    unsafe {
        calendar.setTitle(&NSString::from_str(name));
        calendar.setSource(Some(&source));
        store.saveCalendar_commit_error(&calendar, true).map_err(ns_error)?;
    }
    Ok(calendar)
}

fn to_reminder(reminder: &EKReminder) -> Reminder {
    unsafe {
        Reminder {
            id: reminder.calendarItemIdentifier().to_string(),
            created: reminder.creationDate().map_or(0.0, |d| d.timeIntervalSince1970()),
            list: reminder.calendar().map(|c| c.title().to_string()).unwrap_or_default(),
            data: ReminderData {
                title: reminder.title().to_string(),
                notes: reminder.notes().map(|n| n.to_string()).unwrap_or_default(),
                done: reminder.isCompleted(),
                due: due_of(reminder).0,
                due_time: due_of(reminder).1,
                priority: reminder.priority(),
                repeat: repeat_of(reminder),
            },
        }
    }
}

/// The reminder's repeat rule. Only the first rule is used; Reminders itself
/// only ever creates one.
fn repeat_of(reminder: &EKReminder) -> Option<Repeat> {
    let rules = unsafe { reminder.recurrenceRules() }?;
    let rule = rules.iter().next()?;
    let unit = match unsafe { rule.frequency() } {
        EKRecurrenceFrequency::Daily => RepeatUnit::Day,
        EKRecurrenceFrequency::Weekly => RepeatUnit::Week,
        EKRecurrenceFrequency::Monthly => RepeatUnit::Month,
        EKRecurrenceFrequency::Yearly => RepeatUnit::Year,
        _ => return None,
    };
    let interval = u32::try_from(unsafe { rule.interval() }).unwrap_or(1).max(1);
    Some(Repeat { unit, interval })
}

fn recurrence_rule(repeat: Repeat) -> Retained<EKRecurrenceRule> {
    let frequency = match repeat.unit {
        RepeatUnit::Day => EKRecurrenceFrequency::Daily,
        RepeatUnit::Week => EKRecurrenceFrequency::Weekly,
        RepeatUnit::Month => EKRecurrenceFrequency::Monthly,
        RepeatUnit::Year => EKRecurrenceFrequency::Yearly,
    };
    unsafe {
        EKRecurrenceRule::initRecurrenceWithFrequency_interval_end(
            EKRecurrenceRule::alloc(),
            frequency,
            repeat.interval as isize,
            None,
        )
    }
}

/// The due date and, unless it's an all-day reminder, the time of day.
/// Unset components hold a huge placeholder value, which fails the
/// conversions and gives `None`.
fn due_of(reminder: &EKReminder) -> (Option<NaiveDate>, Option<NaiveTime>) {
    let Some(c) = (unsafe { reminder.dueDateComponents() }) else {
        return (None, None);
    };
    let date = (|| {
        NaiveDate::from_ymd_opt(c.year().try_into().ok()?, c.month().try_into().ok()?, c.day().try_into().ok()?)
    })();
    let time = (|| NaiveTime::from_hms_opt(c.hour().try_into().ok()?, c.minute().try_into().ok()?, 0))();
    (date, date.and(time))
}

/// A date, with a time of day or without one (an all-day due date).
fn date_components(date: NaiveDate, time: Option<NaiveTime>) -> Retained<NSDateComponents> {
    let components = NSDateComponents::new();
    components.setYear(date.year() as isize);
    components.setMonth(date.month() as isize);
    components.setDay(date.day() as isize);
    if let Some(time) = time {
        components.setHour(time.hour() as isize);
        components.setMinute(time.minute() as isize);
    }
    components
}

/// Sets the due date and time, and an alarm at that time so Reminders
/// notifies you. Alarms at another fixed time (such as one for the old due
/// time) are removed; other alarms, like location-based ones, are kept.
fn set_due(reminder: &EKReminder, date: Option<NaiveDate>, time: Option<NaiveTime>) {
    let components = date.map(|date| date_components(date, time));
    let alert_at = date.zip(time).and_then(|(date, time)| {
        let local = Local.from_local_datetime(&date.and_time(time)).earliest()?;
        Some(NSDate::dateWithTimeIntervalSince1970(local.timestamp() as f64))
    });
    unsafe {
        reminder.setDueDateComponents(components.as_deref());
        let mut alarms: Vec<Retained<EKAlarm>> = reminder
            .alarms()
            .map(|alarms| alarms.iter().filter(|alarm| alarm.absoluteDate().is_none()).collect())
            .unwrap_or_default();
        if let Some(at) = alert_at {
            alarms.push(EKAlarm::alarmWithAbsoluteDate(&at));
        }
        reminder.setAlarms(Some(&NSArray::from_retained_slice(&alarms)));
    }
}

fn ns_error(error: Retained<NSError>) -> anyhow::Error {
    anyhow!("Reminders: {}", error.localizedDescription())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn data(title: &str) -> ReminderData {
        ReminderData {
            title: title.to_string(),
            notes: String::new(),
            done: false,
            due: None,
            due_time: None,
            priority: 0,
            repeat: None,
        }
    }

    /// An unsaved reminder. Building one needs no permission, so these tests
    /// run without touching the Reminders app.
    fn unsaved_reminder() -> Retained<EKReminder> {
        let store = unsafe { EKEventStore::new() };
        unsafe { EKReminder::reminderWithEventStore(&store) }
    }

    fn time(text: &str) -> NaiveTime {
        text.parse().unwrap()
    }

    fn absolute_alarms(reminder: &EKReminder) -> Vec<f64> {
        let alarms = unsafe { reminder.alarms() }.map(|a| a.to_vec()).unwrap_or_default();
        alarms.iter().filter_map(|a| unsafe { a.absoluteDate() }).map(|d| d.timeIntervalSince1970()).collect()
    }

    #[test]
    fn due_date_round_trips_through_date_components() {
        let reminder = unsaved_reminder();
        assert_eq!(due_of(&reminder), (None, None));
        for text in ["2026-10-20", "2027-02-28", "2028-02-29"] {
            unsafe { reminder.setDueDateComponents(Some(&date_components(date(text), None))) };
            assert_eq!(due_of(&reminder), (Some(date(text)), None));
        }
    }

    #[test]
    fn due_time_round_trips_through_date_components() {
        let reminder = unsaved_reminder();
        for text in ["00:00:00", "09:05:00", "15:30:00", "23:59:00"] {
            let components = date_components(date("2026-10-20"), Some(time(text)));
            unsafe { reminder.setDueDateComponents(Some(&components)) };
            assert_eq!(due_of(&reminder), (Some(date("2026-10-20")), Some(time(text))));
        }
    }

    #[test]
    fn set_due_adds_and_replaces_the_alarm() {
        let reminder = unsaved_reminder();
        let location_alarm = unsafe { EKAlarm::alarmWithRelativeOffset(-600.0) };
        unsafe { reminder.addAlarm(&location_alarm) };

        set_due(&reminder, Some(date("2026-10-20")), Some(time("15:00:00")));
        let expected = Local.with_ymd_and_hms(2026, 10, 20, 15, 0, 0).unwrap().timestamp() as f64;
        assert_eq!(absolute_alarms(&reminder), [expected]);

        set_due(&reminder, Some(date("2026-10-21")), None);
        assert_eq!(due_of(&reminder), (Some(date("2026-10-21")), None));
        assert!(absolute_alarms(&reminder).is_empty(), "the old alarm is gone");
        assert_eq!(unsafe { reminder.alarms() }.unwrap().len(), 1, "other alarms are kept");

        set_due(&reminder, None, None);
        assert_eq!(due_of(&reminder), (None, None));
    }

    #[test]
    fn repeat_round_trips_through_a_recurrence_rule() {
        let reminder = unsaved_reminder();
        assert_eq!(repeat_of(&reminder), None);
        for unit in [RepeatUnit::Day, RepeatUnit::Week, RepeatUnit::Month, RepeatUnit::Year] {
            for interval in [1, 3] {
                let repeat = Repeat { unit, interval };
                let rules = NSArray::from_retained_slice(&[recurrence_rule(repeat)]);
                unsafe { reminder.setRecurrenceRules(Some(&rules)) };
                assert_eq!(repeat_of(&reminder), Some(repeat));
            }
        }
    }

    #[test]
    fn to_reminder_reads_every_field() {
        let reminder = unsaved_reminder();
        unsafe {
            reminder.setTitle(Some(&NSString::from_str("Dentist")));
            reminder.setNotes(Some(&NSString::from_str("#health")));
            reminder.setPriority(5);
            reminder.setDueDateComponents(Some(&date_components(date("2026-10-20"), Some(time("08:15:00")))));
            reminder.setCompleted(true);
        }
        let read = to_reminder(&reminder).data;
        assert_eq!(
            read,
            ReminderData {
                title: "Dentist".into(),
                notes: "#health".into(),
                done: true,
                due: Some(date("2026-10-20")),
                due_time: Some(time("08:15:00")),
                priority: 5,
                repeat: None,
            }
        );
    }

    #[test]
    fn ns_error_mentions_reminders() {
        let error = unsafe {
            NSError::errorWithDomain_code_userInfo(&NSString::from_str("TestDomain"), 7, None)
        };
        let message = ns_error(error).to_string();
        assert!(message.starts_with("Reminders: ") && message.contains("TestDomain"), "{message}");
    }

    // --- Against the real Reminders app ---------------------------------------
    //
    // These create a temporary list (it syncs to iCloud briefly) and delete it
    // again. They need Reminders access, so they only run when asked for:
    //     cargo test -- --ignored

    /// Opens a fresh temporary list, runs `test` on it, and always deletes it.
    /// Tests run in parallel, so each one passes its own `label`.
    fn with_temporary_list(label: &str, test: impl FnOnce(&RemindersList)) {
        let name = format!("todo-cli-test-{label}-{}", std::process::id());
        let list = RemindersList::open(&name).expect("Reminders access is needed");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(&list)));
        list.delete_list().unwrap();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }

    #[test]
    #[ignore = "uses the real Reminders app; run with `cargo test -- --ignored`"]
    fn real_reminders_create_read_update_delete() {
        with_temporary_list("crud", |list| {
            assert!(list.name().starts_with("todo-cli-test-crud-"));
            assert!(list.all().unwrap().is_empty());

            let mut written = ReminderData {
                notes: "Bring report\n#work".into(),
                due: Some(date("2030-01-15")),
                due_time: Some(time("15:00:00")),
                priority: 1,
                repeat: Some(Repeat { unit: RepeatUnit::Week, interval: 2 }),
                ..data("Integration test")
            };
            let id = list.create(&written, None).unwrap();
            let read = list.get(&id).unwrap();
            assert_eq!((read.list.as_str(), &read.data), (list.name().as_str(), &written));
            assert_eq!(list.all().unwrap().len(), 1);

            written.title = "Renamed".into();
            written.due = None;
            written.due_time = None;
            written.repeat = None;
            list.update(&id, &written).unwrap();
            assert_eq!(list.get(&id).unwrap().data, written);

            list.delete(&id).unwrap();
            assert!(list.get(&id).is_none());
            list.delete(&id).unwrap(); // deleting twice is fine
            assert!(list.update(&id, &written).is_err());
        });
    }

    #[test]
    #[ignore = "uses the real Reminders app; run with `cargo test -- --ignored`"]
    fn real_reminders_completing_a_repeating_reminder_moves_it_forward() {
        with_temporary_list("repeat", |list| {
            let mut written = ReminderData {
                due: Some(date("2030-01-15")),
                repeat: Some(Repeat { unit: RepeatUnit::Week, interval: 1 }),
                ..data("Water plants")
            };
            let id = list.create(&written, None).unwrap();
            written.done = true;
            list.update(&id, &written).unwrap();

            let next = list.get(&id).unwrap().data;
            assert!(!next.done);
            assert_eq!(next.due, Some(date("2030-01-22")));
            let all = list.all().unwrap();
            assert_eq!(all.iter().filter(|r| r.data.done).count(), 1, "a completed copy is kept");
        });
    }
}
