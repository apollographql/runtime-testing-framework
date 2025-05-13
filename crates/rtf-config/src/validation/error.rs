//! Error handling and reporting for problems encountered while validating config files.
use std::{fmt, slice, vec};

/// User facing descriptions of the reason that validation failed.
///
/// Paired with an additional message to form an [Error].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum ErrorKind {
    #[strum(to_string = "non-unique value names found")]
    DuplicateValueNames,

    #[strum(to_string = "the requested file did not exist")]
    FileNotFound,

    #[strum(to_string = "the given relative path was not a valid path")]
    InvalidRelativePath,

    #[strum(to_string = "a directory was provided when a file was expected")]
    IsADirectory,
}

/// Validation logic should always return this result type where the error variant is [Errors] as
/// we always want to report all known validation errors, not just the first one.
pub type Result<T> = std::result::Result<T, Errors>;

/// One or more validaton errors.
///
/// If you know that you only have a single error to report then [Errors::new] can be used to
/// quickly construct an new [Errors]. Otherwise, you should programmatically build up the set of
/// validation errors using an [ErrorBuilder]. See the documentation there for usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Errors {
    inner: Vec<Error>,
}

impl Errors {
    /// Construct a new [Errors] containing a single [Error].
    ///
    /// Prefer using [ErrorBuilder] when there is the possibility of multiple validation errors
    /// arising from a single operation.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            inner: vec![Error {
                kind,
                message: message.into(),
            }],
        }
    }

    /// Assert that this [Errors] only contains a single [Error] and extract it.
    ///
    /// # Panics
    /// This method will panic if there is more than one error.
    pub fn unwrap_single(mut self) -> Error {
        if self.inner.len() > 1 {
            panic!(
                "expected a single error but there were: {}",
                self.inner.len()
            )
        }

        self.inner.remove(0)
    }

    /// Extract the underlying [Vec] of errors.
    pub fn into_vec(self) -> Vec<Error> {
        self.inner
    }

    /// Iterate over the underlying errors in the order they were encountered.
    pub fn iter(&self) -> slice::Iter<'_, Error> {
        self.inner.iter()
    }
}

impl IntoIterator for Errors {
    type Item = Error;
    type IntoIter = vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl fmt::Display for Errors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msgs: Vec<String> = self.inner.iter().map(|e| e.to_string()).collect();

        write!(f, "{}", msgs.join("\n"))
    }
}

impl std::error::Error for Errors {}

/// Programmatically build up an ordered list of validation errors encountered by a single
/// operation.
#[derive(Default, Debug)]
pub struct ErrorBuilder {
    inner: Vec<Error>,
}

impl ErrorBuilder {
    /// Construct a new empty [ErrorBuilder].
    pub fn new() -> Self {
        Self { inner: Vec::new() }
    }

    /// Append a new [Error] to the builder.
    pub fn push(&mut self, kind: ErrorKind, message: impl Into<String>) {
        self.inner.push(Error {
            kind,
            message: message.into(),
        })
    }

    /// Add all errors from `other` to the end of this builder.
    ///
    /// Typically used to combine validation errors from child sources when validating a larger
    /// structure such as a config file.
    pub fn extend(&mut self, other: Errors) {
        self.inner.extend(other.inner);
    }

    /// Add all errors from `other` to the end of this builder with an additional prefix added to
    /// each of their messages.
    ///
    /// Typically used to combine validation errors from child sources when validating a larger
    /// structure such as a config file.
    pub fn extend_with_prefix(&mut self, other: Errors, prefix: impl Into<String>) {
        let prefix = prefix.into();
        self.inner.extend(other.inner.into_iter().map(|mut e| {
            e.message = format!("{prefix} {}", e.message);
            e
        }));
    }

    /// Construct a result from this builder, returning `Ok(t)` if the builder is empty or
    /// `Err(Errors)` at least one [Error] is present.
    ///
    /// Other than calling [Errors::new] for a single error, this is the only way to construct a
    /// new [Errors] instance.
    pub fn into_result<T>(self, t: T) -> Result<T> {
        if self.inner.is_empty() {
            Ok(t)
        } else {
            Err(Errors { inner: self.inner })
        }
    }
}

/// An individual validation error with a given [ErrorKind] and string message.
///
/// See [Errors::new] and [ErrorBuilder::push] for details on how to construct a new error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    /// The [ErrorKind] associated with this error.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The meta-data message attached to this error.
    ///
    /// This will be included in the user facing error message that results from failed validation.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.kind, self.message)
    }
}

impl std::error::Error for Error {}
