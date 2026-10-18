use std::cmp::Reverse;

use chrono::NaiveDate;

use crate::model::{Priority, Task};

/// The order of a task list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum SortKey {
    #[default]
    Id,
    Due,
    Priority,
}

impl SortKey {
    /// Sorts `tasks` in place. Ties are broken so the order is always the same.
    pub fn sort(self, tasks: &mut [Task]) {
        // `(due.is_none(), due)` puts tasks without a due date last. On the same
    // day, all-day tasks (no time) come before timed ones.
        match self {
            SortKey::Id => tasks.sort_by_key(|t| t.id),
            SortKey::Due => {
                tasks.sort_by_key(|t| (t.due.is_none(), t.due, t.due_time, Reverse(t.priority), t.id))
            }
            SortKey::Priority => {
                tasks.sort_by_key(|t| (Reverse(t.priority), t.due.is_none(), t.due, t.due_time, t.id))
            }
        }
    }

    /// The next sort key, for cycling through them.
    pub fn next(self) -> SortKey {
        match self {
            SortKey::Id => SortKey::Due,
            SortKey::Due => SortKey::Priority,
            SortKey::Priority => SortKey::Id,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Id => "id",
            SortKey::Due => "due date",
            SortKey::Priority => "priority",
        }
    }
}

/// Which tasks `Store::list` returns, and in what order. The default
/// matches every task, sorted by ID.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub tag: Option<String>,
    pub priority: Option<Priority>,
    pub pending_only: bool,
    /// Text that must appear in the title or notes (upper/lower case ignored).
    pub search: Option<String>,
    /// Only tasks due on or before this date.
    pub due_by: Option<NaiveDate>,
    /// Only tasks due on or after this date.
    pub due_after: Option<NaiveDate>,
    /// Only tasks without a due date.
    pub no_due: bool,
    pub sort: SortKey,
}

impl Filter {
    /// Whether `task` passes every condition of the filter.
    pub fn matches(&self, task: &Task) -> bool {
        self.tag.as_ref().is_none_or(|tag| task.tags.contains(tag))
            && self.priority.is_none_or(|priority| task.priority == priority)
            && !(self.pending_only && task.done)
            && self.search.as_ref().is_none_or(|text| {
                let text = text.to_lowercase();
                task.title.to_lowercase().contains(&text)
                    || task.notes.to_lowercase().contains(&text)
            })
            && self.due_by.is_none_or(|by| task.due.is_some_and(|due| due <= by))
            && self.due_after.is_none_or(|after| task.due.is_some_and(|due| due >= after))
            && !(self.no_due && task.due.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|t| t.to_string()).collect()
    }

    fn all() -> Filter {
        Filter::default()
    }

    fn sample_tasks() -> Vec<Task> {
        vec![
            Task { id: 1, due: Some(date("2026-10-20")), priority: Priority::Low, ..Task::new("Buy milk") },
            Task { id: 2, priority: Priority::High, tags: tags(&["work"]), notes: "about the PLANTS".into(), ..Task::new("Email boss") },
            Task { id: 3, due: Some(date("2026-10-18")), priority: Priority::Low, done: true, ..Task::new("Old thing") },
            Task { id: 4, due: Some(date("2026-10-18")), priority: Priority::High, tags: tags(&["work", "home"]), ..Task::new("Water plants") },
        ]
    }

    fn matching(filter: &Filter) -> Vec<i64> {
        sample_tasks().iter().filter(|t| filter.matches(t)).map(|t| t.id).collect()
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert_eq!(matching(&all()), [1, 2, 3, 4]);
    }

    #[test]
    fn filter_by_tag_priority_and_pending() {
        assert_eq!(matching(&Filter { tag: Some("work".into()), ..all() }), [2, 4]);
        assert_eq!(matching(&Filter { priority: Some(Priority::Low), ..all() }), [1, 3]);
        assert_eq!(matching(&Filter { pending_only: true, ..all() }), [1, 2, 4]);
    }

    #[test]
    fn search_looks_in_title_and_notes_ignoring_case() {
        assert_eq!(matching(&Filter { search: Some("plant".into()), ..all() }), [2, 4]);
        assert_eq!(matching(&Filter { search: Some("MILK".into()), ..all() }), [1]);
        assert_eq!(matching(&Filter { search: Some("nothing".into()), ..all() }), Vec::<i64>::new());
    }

    #[test]
    fn due_by_needs_a_due_date_on_or_before_the_day() {
        assert_eq!(matching(&Filter { due_by: Some(date("2026-10-18")), ..all() }), [3, 4]);
        assert_eq!(matching(&Filter { due_by: Some(date("2026-10-17")), ..all() }), Vec::<i64>::new());
    }

    #[test]
    fn due_after_needs_a_due_date_on_or_after_the_day() {
        assert_eq!(matching(&Filter { due_after: Some(date("2026-10-18")), ..all() }), [1, 3, 4]);
        assert_eq!(matching(&Filter { due_after: Some(date("2026-10-19")), ..all() }), [1]);
    }

    #[test]
    fn date_range_is_inclusive_on_both_ends() {
        let range = Filter { due_after: Some(date("2026-10-18")), due_by: Some(date("2026-10-18")), ..all() };
        assert_eq!(matching(&range), [3, 4]);
        let pending_range = Filter { pending_only: true, tag: Some("work".into()), ..range };
        assert_eq!(matching(&pending_range), [4]);
    }

    #[test]
    fn no_due_keeps_only_undated_tasks() {
        assert_eq!(matching(&Filter { no_due: true, ..all() }), [2]);
        assert_eq!(matching(&Filter { no_due: true, priority: Some(Priority::Low), ..all() }), Vec::<i64>::new());
    }

    #[test]
    fn filters_combine() {
        let filter = Filter { tag: Some("work".into()), search: Some("water".into()), ..all() };
        assert_eq!(matching(&filter), [4]);
    }

    fn sorted(key: SortKey) -> Vec<i64> {
        let mut tasks = sample_tasks();
        tasks.reverse();
        key.sort(&mut tasks);
        tasks.iter().map(|t| t.id).collect()
    }

    #[test]
    fn sort_by_id() {
        assert_eq!(sorted(SortKey::Id), [1, 2, 3, 4]);
    }

    #[test]
    fn sort_by_due_puts_undated_last_and_breaks_ties_by_priority() {
        assert_eq!(sorted(SortKey::Due), [4, 3, 1, 2]);
    }

    #[test]
    fn sort_by_priority_then_due() {
        assert_eq!(sorted(SortKey::Priority), [4, 2, 3, 1]);
    }

    #[test]
    fn sort_keys_cycle_and_have_labels() {
        assert_eq!(SortKey::Id.next(), SortKey::Due);
        assert_eq!(SortKey::Due.next(), SortKey::Priority);
        assert_eq!(SortKey::Priority.next(), SortKey::Id);
        assert_eq!(SortKey::Id.label(), "id");
        assert_eq!(SortKey::Due.label(), "due date");
        assert_eq!(SortKey::Priority.label(), "priority");
    }
}
