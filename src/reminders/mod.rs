//! Where tasks are stored: the [`Backend`] trait and its two implementations,
//! [`RemindersList`] (the real Reminders app) and [`MemoryList`] (in memory).

mod eventkit;
mod memory;

use anyhow::Result;
use chrono::NaiveDate;

use crate::model::Repeat;
pub use eventkit::RemindersList;
pub use memory::MemoryList;

/// The fields of a reminder that this crate reads and writes.
#[derive(Debug, Clone, PartialEq)]
pub struct ReminderData {
    pub title: String,
    pub notes: String,
    pub done: bool,
    pub due: Option<NaiveDate>,
    /// EventKit's scale: 0 = none, 1–4 = high, 5 = medium, 6–9 = low.
    pub priority: usize,
    pub repeat: Option<Repeat>,
}

impl ReminderData {
    /// An open reminder with only a title.
    pub fn new(title: &str) -> ReminderData {
        ReminderData {
            title: title.to_string(),
            notes: String::new(),
            done: false,
            due: None,
            priority: 0,
            repeat: None,
        }
    }
}

/// A reminder as stored in a list.
#[derive(Debug, Clone)]
pub struct Reminder {
    /// The backend's identifier, e.g. "3A365278-64BD-44F4-8DF6-F1C075B03AB1".
    pub id: String,
    /// Creation time (seconds since 1970), used to number reminders in a stable order.
    pub created: f64,
    /// Name of the list the reminder is in.
    pub list: String,
    pub data: ReminderData,
}

/// What a [`Store`](crate::Store) needs from a reminders list.
pub trait Backend {
    /// Name of the list.
    fn name(&self) -> String;
    /// All reminders in the list, completed ones included.
    fn all(&self) -> Result<Vec<Reminder>>;
    fn get(&self, id: &str) -> Option<Reminder>;
    /// Creates a reminder and returns its identifier. It goes in the list
    /// called `list`, or in this list when `list` is `None`.
    fn create(&self, data: &ReminderData, list: Option<&str>) -> Result<String>;
    fn update(&self, id: &str, data: &ReminderData) -> Result<()>;
    /// Deletes a reminder. Does nothing if it's already gone.
    fn delete(&self, id: &str) -> Result<()>;
}
