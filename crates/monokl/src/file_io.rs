//! File I/O failures that keep the path they were operating on.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};

/// A filesystem operation that failed, with the path it was operating on.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FileIoError {
    #[error("failed to read {path}")]
    Read {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
}

impl FileIoError {
    /// Creates a [`FileIoError::Read`].
    #[must_use]
    pub fn read(path: impl Into<Utf8PathBuf>, source: io::Error) -> Self {
        Self::Read { path: path.into(), source }
    }

    /// Returns the path the failed operation was acting on.
    #[must_use]
    pub fn path(&self) -> &Utf8Path {
        match self {
            Self::Read { path, .. } => path.as_path(),
        }
    }

    /// Returns the underlying I/O error.
    #[must_use]
    pub fn io_source(&self) -> &io::Error {
        match self {
            Self::Read { source, .. } => source,
        }
    }

    /// Returns the kind of the underlying I/O error.
    #[must_use]
    pub fn io_kind(&self) -> io::ErrorKind {
        self.io_source().kind()
    }

    /// Returns whether the operation failed because the path does not exist.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        self.io_kind() == io::ErrorKind::NotFound
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    fn not_found() -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, "gone")
    }

    #[test]
    fn display_names_the_operation_and_path() {
        let err = FileIoError::read("/a", not_found());
        assert_eq!(err.to_string(), "failed to read /a");
        assert_eq!(err.path(), Utf8Path::new("/a"));
        assert_eq!(err.source().map(ToString::to_string).as_deref(), Some("gone"));
    }

    #[test]
    fn kind_accessors_read_the_wrapped_io_error() {
        let missing = FileIoError::read("/a", not_found());
        assert_eq!(missing.io_kind(), io::ErrorKind::NotFound);
        assert!(missing.is_not_found());

        let denied = FileIoError::read("/a", io::Error::new(io::ErrorKind::PermissionDenied, "no"));
        assert_eq!(denied.io_kind(), io::ErrorKind::PermissionDenied);
        assert!(!denied.is_not_found());
    }
}
