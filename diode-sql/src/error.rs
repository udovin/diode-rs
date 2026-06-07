/// Errors produced while converting between [`Value`](crate::Value)s and Rust
/// types or while reading and writing rows.
#[derive(Debug)]
pub enum Error {
    /// The requested column is not part of the [`Columns`](crate::Columns).
    UnknownColumn(String),
    /// A [`Value`](crate::Value) could not be converted into the requested Rust
    /// type. `column` is filled in once the failure is attributed to a column.
    Invalid {
        column: Option<String>,
        message: String,
    },
}

impl Error {
    /// Creates an [`Error::UnknownColumn`].
    pub fn unknown_column(name: impl Into<String>) -> Self {
        Self::UnknownColumn(name.into())
    }

    /// Creates an [`Error::Invalid`] not yet attributed to a column.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            column: None,
            message: message.into(),
        }
    }

    /// Attributes an [`Error::Invalid`] to `column` if it does not name one yet.
    pub(crate) fn at_column(mut self, name: &str) -> Self {
        if let Self::Invalid { column, .. } = &mut self
            && column.is_none()
        {
            *column = Some(name.to_string());
        }
        self
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnknownColumn(name) => write!(f, "unknown column `{name}`"),
            Error::Invalid {
                column: Some(column),
                message,
            } => write!(f, "invalid value for column `{column}`: {message}"),
            Error::Invalid {
                column: None,
                message,
            } => write!(f, "invalid value: {message}"),
        }
    }
}

impl std::error::Error for Error {}
