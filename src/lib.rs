//! A to-do list stored in Apple Reminders.
//!
//! The crate has three layers:
//!
//! - [`model`]: the data types ([`Task`], [`Priority`], [`Repeat`]) and the
//!   parsing of dates and repeat intervals.
//! - [`reminders`]: where tasks are kept. [`RemindersList`] talks to the
//!   Reminders app through Apple's EventKit; [`MemoryList`] keeps everything in
//!   memory. Both implement [`Backend`].
//! - [`store`]: the [`Store`], which adds short task numbers, filtering,
//!   sorting and an undo history on top of a backend.
//!
//! [`ErrorKind::of`] tells apart errors such as a missing task or no access
//! to Reminders.
//!
//! The `todo` command-line program in `src/bin/todo` is built on this
//! library. Its extra dependencies sit behind the `cli` feature (on by
//! default); use `default-features = false` to get only the library.
//!
//! # Example
//!
//! ```
//! use todo::{Filter, MemoryList, Priority, SortKey, Store, Task};
//!
//! // `Store::open(path, "Todo")` would use the real Reminders app instead.
//! let mut store = Store::in_memory(MemoryList::new("Todo"))?;
//! store.add(&Task::new("Buy milk"))?;
//! let report = store.add(&Task { priority: Priority::High, ..Task::new("Write report") })?;
//! store.set_done(report.id, true)?;
//!
//! let open = store.list(&Filter { pending_only: true, sort: SortKey::Priority, ..Filter::default() })?;
//! assert_eq!(open.len(), 1);
//! assert_eq!(open[0].title, "Buy milk");
//!
//! store.undo()?; // reopens "Write report"
//! assert!(!store.get(report.id)?.unwrap().done);
//! # Ok::<(), anyhow::Error>(())
//! ```

pub mod error;
pub mod json;
pub mod model;
pub mod reminders;
pub mod store;

pub use error::{Error, ErrorKind};
pub use model::{DueStatus, Priority, Repeat, RepeatUnit, Task};
pub use reminders::{Backend, ListSummary, MemoryList, Reminder, ReminderData, RemindersList};
pub use store::{Change, Filter, SortKey, Store};
