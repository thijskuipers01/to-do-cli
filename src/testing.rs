//! Helpers shared by the tests: an in-memory stand-in for the Reminders app.

use std::cell::{Cell, RefCell};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::reminders::{Backend, Reminder, ReminderData};
use crate::store::Store;

/// A fake reminders list that keeps everything in memory. Reminders in other
/// lists are kept too, so moving tasks between lists can be tested.
pub struct FakeList {
    name: RefCell<String>,
    reminders: RefCell<Vec<Reminder>>,
    next_id: Cell<u32>,
}

impl FakeList {
    pub fn new(name: &str) -> FakeList {
        FakeList {
            name: RefCell::new(name.to_string()),
            reminders: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
        }
    }

    /// Makes another list the current one, like running `todo --list <name>`.
    pub fn switch_to(&self, name: &str) {
        *self.name.borrow_mut() = name.to_string();
    }

    /// Adds a reminder "from the outside", like one created on the iPhone.
    pub fn push(&self, data: ReminderData, list: &str) -> String {
        self.create(&data, Some(list)).unwrap()
    }

    /// The raw data of a reminder, to check what was written.
    pub fn data(&self, id: &str) -> Option<ReminderData> {
        self.get(id).map(|r| r.data)
    }

    /// Identifiers of all reminders in every list.
    pub fn ids(&self) -> Vec<String> {
        self.reminders.borrow().iter().map(|r| r.id.clone()).collect()
    }
}

impl Backend for FakeList {
    fn name(&self) -> String {
        self.name.borrow().clone()
    }

    fn all(&self) -> Result<Vec<Reminder>> {
        let name = self.name();
        let reminders = self.reminders.borrow();
        Ok(reminders.iter().filter(|r| r.list == name).cloned().collect())
    }

    fn get(&self, id: &str) -> Option<Reminder> {
        self.reminders.borrow().iter().find(|r| r.id == id).cloned()
    }

    fn create(&self, data: &ReminderData, list: Option<&str>) -> Result<String> {
        let number = self.next_id.get();
        self.next_id.set(number + 1);
        let id = format!("fake-{number}");
        let list = list.map_or_else(|| self.name(), str::to_string);
        self.reminders.borrow_mut().push(Reminder {
            id: id.clone(),
            created: f64::from(number),
            list,
            data: data.clone(),
        });
        Ok(id)
    }

    fn update(&self, id: &str, data: &ReminderData) -> Result<()> {
        let mut reminders = self.reminders.borrow_mut();
        let reminder = reminders
            .iter_mut()
            .find(|r| r.id == id)
            .context("the reminder no longer exists")?;
        reminder.data = data.clone();
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.reminders.borrow_mut().retain(|r| r.id != id);
        Ok(())
    }
}

/// A store backed by a fresh in-memory database and an empty fake list "Todo".
pub fn store() -> Store<FakeList> {
    let conn = Connection::open_in_memory().unwrap();
    Store::with_backend(conn, FakeList::new("Todo")).unwrap()
}

/// Reminder data with only a title, for tests that create reminders directly.
pub fn data(title: &str) -> ReminderData {
    ReminderData {
        title: title.to_string(),
        notes: String::new(),
        done: false,
        due: None,
        priority: 0,
        repeat: None,
    }
}
