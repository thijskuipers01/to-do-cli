use std::cell::{Cell, RefCell};

use anyhow::{Context, Result};

use super::{Backend, Reminder, ReminderData};

/// A reminders list kept entirely in memory: useful for tests, or for trying
/// the library without touching the Reminders app. Reminders in other lists
/// are kept too, so moving tasks between lists works like the real thing.
pub struct MemoryList {
    name: RefCell<String>,
    reminders: RefCell<Vec<Reminder>>,
    next_id: Cell<u32>,
}

impl MemoryList {
    /// An empty list called `name`.
    pub fn new(name: &str) -> MemoryList {
        MemoryList {
            name: RefCell::new(name.to_string()),
            reminders: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
        }
    }

    /// Makes another list the current one, like running `todo --list <name>`.
    pub fn switch_to(&self, name: &str) {
        *self.name.borrow_mut() = name.to_string();
    }

    /// Adds a reminder "from the outside", like one created on an iPhone.
    /// Returns its identifier.
    pub fn push(&self, data: ReminderData, list: &str) -> String {
        self.create(&data, Some(list)).expect("creating in memory can't fail")
    }

    /// The data of a reminder, to check what was written.
    pub fn data(&self, id: &str) -> Option<ReminderData> {
        self.get(id).map(|r| r.data)
    }

    /// Identifiers of all reminders, in every list.
    pub fn ids(&self) -> Vec<String> {
        self.reminders.borrow().iter().map(|r| r.id.clone()).collect()
    }
}

impl Backend for MemoryList {
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
        let id = format!("memory-{number}");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_get_update_delete() {
        let list = MemoryList::new("Todo");
        let id = list.create(&ReminderData::new("a"), None).unwrap();
        assert_eq!(list.get(&id).unwrap().list, "Todo");

        let changed = ReminderData { done: true, ..ReminderData::new("b") };
        list.update(&id, &changed).unwrap();
        assert_eq!(list.data(&id), Some(changed));

        list.delete(&id).unwrap();
        assert!(list.get(&id).is_none());
        assert!(list.update(&id, &ReminderData::new("c")).is_err());
        list.delete(&id).unwrap(); // deleting twice is fine
    }

    #[test]
    fn all_only_returns_the_current_list() {
        let list = MemoryList::new("Todo");
        let todo = list.push(ReminderData::new("a"), "Todo");
        let shopping = list.push(ReminderData::new("b"), "Shopping");
        assert_eq!(list.all().unwrap().iter().map(|r| &r.id).collect::<Vec<_>>(), [&todo]);

        list.switch_to("Shopping");
        assert_eq!(list.name(), "Shopping");
        assert_eq!(list.all().unwrap().iter().map(|r| &r.id).collect::<Vec<_>>(), [&shopping]);
        assert_eq!(list.ids(), [todo, shopping]);
    }

    #[test]
    fn identifiers_and_creation_order_increase() {
        let list = MemoryList::new("Todo");
        let first = list.create(&ReminderData::new("a"), None).unwrap();
        let second = list.create(&ReminderData::new("b"), None).unwrap();
        assert_eq!((first.as_str(), second.as_str()), ("memory-1", "memory-2"));
        assert!(list.get(&first).unwrap().created < list.get(&second).unwrap().created);
    }
}
