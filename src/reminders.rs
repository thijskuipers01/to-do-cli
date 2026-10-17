//! A thin wrapper around EventKit, Apple's framework behind the Reminders app.
//! All the `unsafe` code for talking to Objective-C lives in this file; the
//! rest of the program only sees the plain Rust types below.

use std::sync::mpsc;

use anyhow::{Context, Result, anyhow, bail};
use block2::RcBlock;
use chrono::{Datelike, NaiveDate};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_event_kit::{
    EKCalendar, EKEntityType, EKEventStore, EKRecurrenceFrequency, EKRecurrenceRule, EKReminder,
};
use objc2_foundation::{NSArray, NSDateComponents, NSError, NSString};

use crate::task::{Repeat, RepeatUnit};

/// The fields of a reminder that this program reads and writes.
pub struct ReminderData {
    pub title: String,
    pub notes: String,
    pub done: bool,
    pub due: Option<NaiveDate>,
    /// EventKit's scale: 0 = none, 1–4 = high, 5 = medium, 6–9 = low.
    pub priority: usize,
    pub repeat: Option<Repeat>,
}

pub struct Reminder {
    /// EventKit's identifier, e.g. "3A365278-64BD-44F4-8DF6-F1C075B03AB1".
    pub id: String,
    /// Creation time (seconds since 1970), used to number reminders in a stable order.
    pub created: f64,
    /// Name of the list the reminder is in.
    pub list: String,
    pub data: ReminderData,
}

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

    pub fn name(&self) -> String {
        unsafe { self.calendar.title() }.to_string()
    }

    /// All reminders in the list, completed ones included.
    pub fn all(&self) -> Result<Vec<Reminder>> {
        // EventKit answers on a background thread by calling `handler`. The
        // reminders are converted to plain Rust data there and sent back
        // through a channel, which `recv` below waits on.
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |array: *mut NSArray<EKReminder>| {
            let reminders = match unsafe { array.as_ref() } {
                Some(array) => array.iter().map(|r| to_reminder(&r)).collect(),
                None => Vec::new(),
            };
            let _ = tx.send(reminders);
        });
        let calendars = NSArray::from_retained_slice(std::slice::from_ref(&self.calendar));
        unsafe {
            let predicate = self.store.predicateForRemindersInCalendars(Some(&calendars));
            self.store.fetchRemindersMatchingPredicate_completion(&predicate, &handler);
        }
        rx.recv().context("Reminders did not respond")
    }

    pub fn get(&self, id: &str) -> Option<Reminder> {
        self.find(id).map(|r| to_reminder(&r))
    }

    /// Creates a reminder and returns its EventKit identifier. It goes in the
    /// list called `list`, or in this list when `list` is `None`.
    pub fn create(&self, data: &ReminderData, list: Option<&str>) -> Result<String> {
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

    pub fn update(&self, id: &str, data: &ReminderData) -> Result<()> {
        let reminder = self.find(id).context("the reminder no longer exists")?;
        self.save(&reminder, data)
    }

    /// Deletes a reminder. Does nothing if it's already gone.
    pub fn delete(&self, id: &str) -> Result<()> {
        if let Some(reminder) = self.find(id) {
            unsafe { self.store.removeReminder_commit_error(&reminder, true) }
                .map_err(ns_error)?;
        }
        Ok(())
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
            // Only touch the due date when it changes, so a time of day set on
            // the iPhone survives edits made here.
            if due_date(reminder) != data.due {
                let components = data.due.map(date_components);
                reminder.setDueDateComponents(components.as_deref());
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
}

fn request_access(store: &EKEventStore) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        let _ = tx.send(granted.as_bool());
    });
    unsafe { store.requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&handler)) };
    if !rx.recv().unwrap_or(false) {
        bail!(
            "todo has no access to Reminders. Allow your terminal app under \
             System Settings → Privacy & Security → Reminders, then try again."
        );
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
                due: due_date(reminder),
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

/// The due date without its time of day. Unset components hold a huge
/// placeholder value, which fails the conversions and gives `None`.
fn due_date(reminder: &EKReminder) -> Option<NaiveDate> {
    let c = unsafe { reminder.dueDateComponents() }?;
    let (year, month, day) = (c.year(), c.month(), c.day());
    NaiveDate::from_ymd_opt(
        year.try_into().ok()?,
        month.try_into().ok()?,
        day.try_into().ok()?,
    )
}

/// A date with no time of day, which Reminders shows as an all-day due date.
fn date_components(date: NaiveDate) -> Retained<NSDateComponents> {
    let components = NSDateComponents::new();
    components.setYear(date.year() as isize);
    components.setMonth(date.month() as isize);
    components.setDay(date.day() as isize);
    components
}

fn ns_error(error: Retained<NSError>) -> anyhow::Error {
    anyhow!("Reminders: {}", error.localizedDescription())
}
