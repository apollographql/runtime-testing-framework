//! Generic helpers for working with operations that can return multiple errors, such as templating
//! and validation.
use std::{fmt, slice, vec};

/// One or more [Error]s.
///
/// If you know that you only have a single error to report then [Errors::new] can be used to
/// quickly construct an new [Errors]. Otherwise, you should programmatically build up the set of
/// errors using an [ErrorBuilder]. See the documentation there for usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Errors<K>
where
    K: fmt::Debug + fmt::Display + Copy,
{
    inner: Vec<Error<K>>,
}

impl<K> Errors<K>
where
    K: fmt::Debug + fmt::Display + Copy,
{
    /// Construct a new [Errors] containing a single [Error].
    ///
    /// Prefer using [ErrorBuilder] when there is the possibility of multiple errors arising from a
    /// single operation.
    pub fn new(kind: K, message: impl Into<String>) -> Self {
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
    pub fn unwrap_single(mut self) -> Error<K> {
        if self.inner.len() > 1 {
            panic!(
                "expected a single error but there were {}",
                self.inner.len()
            )
        }

        self.inner.remove(0)
    }

    /// Extract the underlying [Vec] of [Error]s.
    pub fn into_vec(self) -> Vec<Error<K>> {
        self.inner
    }

    /// Iterate over the underlying errors in the order they were encountered.
    pub fn iter(&self) -> slice::Iter<'_, Error<K>> {
        self.inner.iter()
    }
}

impl<K> IntoIterator for Errors<K>
where
    K: fmt::Debug + fmt::Display + Copy,
{
    type Item = Error<K>;
    type IntoIter = vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<K> fmt::Display for Errors<K>
where
    K: fmt::Debug + fmt::Display + Copy,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msgs: Vec<String> = self.inner.iter().map(|e| e.to_string()).collect();

        write!(f, "{}", msgs.join("\n"))
    }
}

impl<K> std::error::Error for Errors<K> where K: fmt::Debug + fmt::Display + Copy {}

/// Programmatically build up an ordered list of errors encountered by a single operation.
#[derive(Default, Debug)]
pub struct ErrorBuilder<K>
where
    K: fmt::Debug + fmt::Display + Copy,
{
    inner: Vec<Error<K>>,
}

impl<K> ErrorBuilder<K>
where
    K: fmt::Debug + fmt::Display + Copy,
{
    /// Construct a new empty [ErrorBuilder].
    pub fn new() -> Self {
        Self { inner: Vec::new() }
    }

    /// Append a new [Error] to the builder.
    pub fn push(&mut self, kind: K, message: impl Into<String>) {
        self.inner.push(Error {
            kind,
            message: message.into(),
        });
    }

    /// Add all errors from `other` to the end of this builder.
    ///
    /// Typically used to combine errors from nested sources when working with a larger structure
    /// such as a config file.
    pub fn extend(&mut self, other: Errors<K>) {
        self.inner.extend(other.inner);
    }

    /// Add all errors from `other` to the end of this builder with an additional prefix added to
    /// each of their messages.
    ///
    /// Typically used to combine errors from nested sources when working with a larger structure
    /// such as a config file.
    pub fn extend_with_prefix(&mut self, other: Errors<K>, prefix: impl Into<String>) {
        let prefix = prefix.into();
        self.inner.extend(other.inner.into_iter().map(|mut e| {
            e.message = format!("{prefix} {}", e.message);
            e
        }));
    }

    /// Construct a result from this builder, returning `Ok(t)` if the builder is empty or
    /// `Err(Errors)` if at least one [Error] is present.
    ///
    /// Other than calling [Errors::new] for a single error, this is the only way to construct a
    /// new [Errors] instance.
    pub fn into_result<T>(self, t: T) -> Result<T, Errors<K>> {
        if self.inner.is_empty() {
            Ok(t)
        } else {
            Err(Errors { inner: self.inner })
        }
    }
}

/// An individual error pairing a given error kind with a string message.
///
/// See [Errors::new] and [ErrorBuilder::push] for details on how to construct a new error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error<K>
where
    K: fmt::Debug + fmt::Display + Copy,
{
    kind: K,
    message: String,
}

impl<K> Error<K>
where
    K: fmt::Debug + fmt::Display + Copy,
{
    /// The kind associated with this error.
    pub fn kind(&self) -> K {
        self.kind
    }

    /// The meta-data message attached to this error.
    ///
    /// This will be included in the user facing error message is presented to the user.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl<K> fmt::Display for Error<K>
where
    K: fmt::Debug + fmt::Display + Copy,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.kind, self.message)
    }
}

impl<K> std::error::Error for Error<K> where K: fmt::Debug + fmt::Display + Copy {}
