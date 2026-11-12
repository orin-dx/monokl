//! File I/O failures that keep the path they were operating on.

use std::io;

use camino::Utf8PathBuf;

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

    /// Returns the kind of the underlying I/O error.
    #[must_use]
    pub fn io_kind(&self) -> io::ErrorKind {
        match self {
            Self::Read { source, .. } => source.kind(),
        }
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
        assert_eq!(err.source().map(ToString::to_string).as_deref(), Some("gone"));
    }

    #[test]
    fn the_kind_is_the_wrapped_io_errors() {
        let missing = FileIoError::read("/a", not_found());
        assert_eq!(missing.io_kind(), io::ErrorKind::NotFound);

        let denied = FileIoError::read("/a", io::Error::new(io::ErrorKind::PermissionDenied, "no"));
        assert_eq!(denied.io_kind(), io::ErrorKind::PermissionDenied);
    }
}
