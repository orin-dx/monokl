//! File I/O failures that keep the path they were operating on.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};

/// A filesystem operation that failed, with the path it was operating on.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "miette", derive(miette::Diagnostic))]
#[non_exhaustive]
pub enum FileIoError {
    #[error("failed to read {path}")]
    Read {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write {path}")]
    Write {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to create directory {path}")]
    CreateDir {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to remove {path}")]
    Remove {
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

    /// Creates a [`FileIoError::Write`].
    #[must_use]
    pub fn write(path: impl Into<Utf8PathBuf>, source: io::Error) -> Self {
        Self::Write { path: path.into(), source }
    }

    /// Creates a [`FileIoError::CreateDir`].
    #[must_use]
    pub fn create_dir(path: impl Into<Utf8PathBuf>, source: io::Error) -> Self {
        Self::CreateDir { path: path.into(), source }
    }

    /// Creates a [`FileIoError::Remove`].
    #[must_use]
    pub fn remove(path: impl Into<Utf8PathBuf>, source: io::Error) -> Self {
        Self::Remove { path: path.into(), source }
    }

    /// Returns the path the failed operation was acting on.
    #[must_use]
    pub fn path(&self) -> &Utf8Path {
        match self {
            Self::Read { path, .. }
            | Self::Write { path, .. }
            | Self::CreateDir { path, .. }
            | Self::Remove { path, .. } => path.as_path(),
        }
    }

    /// Returns the underlying I/O error.
    #[must_use]
    pub fn io_source(&self) -> &io::Error {
        match self {
            Self::Read { source, .. }
            | Self::Write { source, .. }
            | Self::CreateDir { source, .. }
            | Self::Remove { source, .. } => source,
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

    use rstest::rstest;

    use super::*;

    fn not_found() -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, "gone")
    }

    #[rstest]
    #[case(FileIoError::read("/a", not_found()), "failed to read /a")]
    #[case(FileIoError::write("/a", not_found()), "failed to write /a")]
    #[case(FileIoError::create_dir("/a", not_found()), "failed to create directory /a")]
    #[case(FileIoError::remove("/a", not_found()), "failed to remove /a")]
    fn display_names_the_operation_and_path(#[case] err: FileIoError, #[case] expected: &str) {
        assert_eq!(err.to_string(), expected);
        assert_eq!(err.path(), Utf8Path::new("/a"));
        assert_eq!(err.source().map(ToString::to_string).as_deref(), Some("gone"));
    }

    #[test]
    fn kind_accessors_read_the_wrapped_io_error() {
        let missing = FileIoError::read("/a", not_found());
        assert_eq!(missing.io_kind(), io::ErrorKind::NotFound);
        assert!(missing.is_not_found());

        let denied = FileIoError::write("/a", io::Error::new(io::ErrorKind::PermissionDenied, "no"));
        assert_eq!(denied.io_kind(), io::ErrorKind::PermissionDenied);
        assert!(!denied.is_not_found());
    }
}
