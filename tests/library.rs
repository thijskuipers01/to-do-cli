//! Uses the `todo` library only through its public API, the way another
//! crate depending on it would.

use todo::model::{parse_due, parse_repeat, today};
use todo::{
    Backend, DueStatus, Filter, MemoryList, Priority, ReminderData, Repeat, RepeatUnit, SortKey,
    Store, Task,
};

fn store() -> Store<MemoryList> {
    Store::in_memory(MemoryList::new("Todo")).unwrap()
}

#[test]
fn add_list_complete_and_undo() {
    let mut store = store();
    let milk = store.add(&Task { tags: vec!["shop".into()], ..Task::new("Buy milk") }).unwrap();
    let report = store
        .add(&Task { priority: Priority::High, due: Some(today()), ..Task::new("Write report") })
        .unwrap();

    let by_priority = Filter { sort: SortKey::Priority, ..Filter::default() };
    let titles: Vec<String> = store.list(&by_priority).unwrap().into_iter().map(|t| t.title).collect();
    assert_eq!(titles, ["Write report", "Buy milk"]);

    store.set_done(report.id, true).unwrap();
    let open = store.list(&Filter { pending_only: true, ..Filter::default() }).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, milk.id);

    assert_eq!(store.undo().unwrap().as_deref(), Some("done of task 2"));
    assert_eq!(store.get(report.id).unwrap().unwrap().due_status(today()), Some(DueStatus::Today));
}

#[test]
fn tags_are_written_as_hashtags_in_the_notes() {
    let mut store = store();
    store.add(&Task { notes: "Ask for the blue one".into(), tags: vec!["shop".into()], ..Task::new("Paint") }).unwrap();
    let reminder = &store.backend().all().unwrap()[0];
    assert_eq!(reminder.data.notes, "Ask for the blue one\n#shop");
}

#[test]
fn reminders_added_directly_to_the_backend_show_up() {
    let store = store();
    store.backend().push(ReminderData { priority: 1, ..ReminderData::new("From the iPhone") }, "Todo");
    let tasks = store.list(&Filter::default()).unwrap();
    assert_eq!((tasks[0].id, tasks[0].priority), (1, Priority::High));
}

#[test]
fn parsing_helpers_and_from_str() {
    let weekly = Repeat { unit: RepeatUnit::Week, interval: 1 };
    assert_eq!(parse_repeat("weekly"), Ok(weekly));
    assert_eq!("every week".parse::<Repeat>(), Ok(weekly));
    assert_eq!("high".parse::<Priority>(), Ok(Priority::High));
    assert_eq!(parse_due("today"), Ok(today()));
}
