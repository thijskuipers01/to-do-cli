//! Errors that callers may want to tell apart, e.g. to pick an exit code.
//!
//! The library returns [`anyhow::Error`] everywhere. Errors of the kinds
//! below carry an [`Error`] inside, which [`ErrorKind::of`] finds again.

use std::fmt;

/// What sort of problem an error is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Bad input: an unknown date or priority, nothing to change, ...
    InvalidInput,
    /// No task (or list) with the given ID or name.
    NotFound,
    /// No access to Reminders.
    PermissionDenied,
    /// Anything else.
    Other,
}

impl ErrorKind {
    /// The kind of `error`: that of the first [`Error`] in its chain of
    /// causes, or [`ErrorKind::Other`].
    pub fn of(error: &anyhow::Error) -> ErrorKind {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<Error>())
            .map_or(ErrorKind::Other, |e| e.kind)
    }

    /// The name used in JSON output: `"invalid_input"`, `"not_found"`, ...
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::InvalidInput => "invalid_input",
            ErrorKind::NotFound => "not_found",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::Other => "other",
        }
    }
}

/// An error of a known [`ErrorKind`] with a message for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error { kind, message: message.into() }
    }

    pub fn invalid_input(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::InvalidInput, message)
    }

    pub fn not_found(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::NotFound, message)
    }

    /// "no task with id 7"
    pub fn no_task(id: i64) -> Error {
        Error::not_found(format!("no task with id {id}"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::*;

    #[test]
    fn kind_is_found_through_context() {
        let error = Err::<(), _>(Error::no_task(7)).context("while undoing").unwrap_err();
        assert_eq!(ErrorKind::of(&error), ErrorKind::NotFound);
        assert_eq!(format!("{error:#}"), "while undoing: no task with id 7");
    }

    #[test]
    fn other_errors_are_other() {
        assert_eq!(ErrorKind::of(&anyhow::anyhow!("disk full")), ErrorKind::Other);
    }

    #[test]
    fn names() {
        assert_eq!(ErrorKind::InvalidInput.name(), "invalid_input");
        assert_eq!(ErrorKind::NotFound.name(), "not_found");
        assert_eq!(ErrorKind::PermissionDenied.name(), "permission_denied");
        assert_eq!(ErrorKind::Other.name(), "other");
    }
}
