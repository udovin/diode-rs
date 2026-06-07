use std::num::{NonZeroU32, NonZeroU64};

use crate::Error;

/// A backend-agnostic SQL value.
///
/// The always-present variants are physical primitives. Logical types are
/// feature-gated and carry their standard crate's type directly: `Timestamp`
/// (`chrono`, as `DateTime<Utc>`), `Uuid` (`uuid`), `Decimal` (`rust_decimal`),
/// `Json` (`serde_json`). A Rust type without a dedicated variant extends the
/// model by implementing [`IntoValue`] / [`ParseValue`] onto an existing one.
///
/// Integers are stored as [`Value::I64`]; unsigned 64-bit types round-trip
/// losslessly through a bit-reinterpreting cast (a real backend renders them per
/// its own rules).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    /// An instant in UTC. Requires the `chrono` feature.
    #[cfg(feature = "chrono")]
    Timestamp(chrono::DateTime<chrono::Utc>),
    /// A UUID. Requires the `uuid` feature.
    #[cfg(feature = "uuid")]
    Uuid(uuid::Uuid),
    /// An arbitrary-precision decimal. Requires the `decimal` feature.
    #[cfg(feature = "decimal")]
    Decimal(rust_decimal::Decimal),
    /// A JSON document. Requires the `json` feature.
    #[cfg(feature = "json")]
    Json(serde_json::Value),
}

impl Value {
    /// A short name for this value's kind, used in error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::I64(_) => "i64",
            Value::F64(_) => "f64",
            Value::Text(_) => "text",
            Value::Bytes(_) => "bytes",
            #[cfg(feature = "chrono")]
            Value::Timestamp(_) => "timestamp",
            #[cfg(feature = "uuid")]
            Value::Uuid(_) => "uuid",
            #[cfg(feature = "decimal")]
            Value::Decimal(_) => "decimal",
            #[cfg(feature = "json")]
            Value::Json(_) => "json",
        }
    }

    /// Returns whether this value is [`Value::Null`].
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// A positional row of [`Value`]s, addressed by name through a
/// [`Columns`](crate::Columns).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Values(Vec<Value>);

impl Values {
    /// Creates a row of `len` [`Value::Null`]s.
    pub fn with_len(len: usize) -> Self {
        Self(vec![Value::Null; len])
    }

    /// Number of values in the row.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the row has no values.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the value at `index`, if any.
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.0.get(index)
    }

    /// Appends a value to the end of the row.
    pub fn push(&mut self, value: Value) {
        self.0.push(value);
    }

    /// Sets the value at `index`. Returns whether the index was in bounds.
    pub fn set(&mut self, index: usize, value: Value) -> bool {
        match self.0.get_mut(index) {
            Some(slot) => {
                *slot = value;
                true
            }
            None => false,
        }
    }

    /// The values as a slice.
    pub fn as_slice(&self) -> &[Value] {
        &self.0
    }
}

impl From<Vec<Value>> for Values {
    fn from(values: Vec<Value>) -> Self {
        Self(values)
    }
}

/// Converts a Rust value into a [`Value`].
pub trait IntoValue {
    fn into_value(self) -> Value;
}

/// Parses a Rust value out of a [`Value`].
pub trait ParseValue: Sized {
    fn parse_value(value: &Value) -> Result<Self, Error>;
}

impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}

impl ParseValue for Value {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        Ok(value.clone())
    }
}

impl IntoValue for bool {
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
}

impl ParseValue for bool {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Bool(v) => Ok(*v),
            other => Err(Error::invalid(format!("expected bool, found {}", other.kind()))),
        }
    }
}

/// Integer types stored as [`Value::I64`] with a checked range on the way back.
macro_rules! int_value {
    ($($t:ty),* $(,)?) => {$(
        impl IntoValue for $t {
            fn into_value(self) -> Value {
                Value::I64(self as i64)
            }
        }

        impl ParseValue for $t {
            fn parse_value(value: &Value) -> Result<Self, Error> {
                match value {
                    Value::I64(v) => <$t>::try_from(*v).map_err(|_| {
                        Error::invalid(format!(
                            "value {v} out of range for {}",
                            stringify!($t)
                        ))
                    }),
                    other => Err(Error::invalid(format!(
                        "expected integer, found {}",
                        other.kind()
                    ))),
                }
            }
        }
    )*};
}

int_value!(i8, i16, i32, i64, u8, u16, u32);

impl IntoValue for u64 {
    fn into_value(self) -> Value {
        Value::I64(self as i64)
    }
}

impl ParseValue for u64 {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::I64(v) => Ok(*v as u64),
            other => Err(Error::invalid(format!("expected integer, found {}", other.kind()))),
        }
    }
}

impl IntoValue for NonZeroU64 {
    fn into_value(self) -> Value {
        Value::I64(self.get() as i64)
    }
}

impl ParseValue for NonZeroU64 {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::I64(v) => {
                NonZeroU64::new(*v as u64).ok_or_else(|| Error::invalid("expected non-zero integer"))
            }
            other => Err(Error::invalid(format!("expected integer, found {}", other.kind()))),
        }
    }
}

impl IntoValue for NonZeroU32 {
    fn into_value(self) -> Value {
        Value::I64(self.get() as i64)
    }
}

impl ParseValue for NonZeroU32 {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        let v = u32::parse_value(value)?;
        NonZeroU32::new(v).ok_or_else(|| Error::invalid("expected non-zero integer"))
    }
}

impl IntoValue for f64 {
    fn into_value(self) -> Value {
        Value::F64(self)
    }
}

impl ParseValue for f64 {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::F64(v) => Ok(*v),
            other => Err(Error::invalid(format!("expected float, found {}", other.kind()))),
        }
    }
}

impl IntoValue for f32 {
    fn into_value(self) -> Value {
        Value::F64(self as f64)
    }
}

impl ParseValue for f32 {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        Ok(f64::parse_value(value)? as f32)
    }
}

impl IntoValue for String {
    fn into_value(self) -> Value {
        Value::Text(self)
    }
}

impl IntoValue for &str {
    fn into_value(self) -> Value {
        Value::Text(self.to_string())
    }
}

impl ParseValue for String {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Text(v) => Ok(v.clone()),
            other => Err(Error::invalid(format!("expected text, found {}", other.kind()))),
        }
    }
}

impl IntoValue for Vec<u8> {
    fn into_value(self) -> Value {
        Value::Bytes(self)
    }
}

impl ParseValue for Vec<u8> {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Bytes(v) => Ok(v.clone()),
            other => Err(Error::invalid(format!("expected bytes, found {}", other.kind()))),
        }
    }
}

impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Value {
        match self {
            Some(value) => value.into_value(),
            None => Value::Null,
        }
    }
}

impl<T: ParseValue> ParseValue for Option<T> {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Null => Ok(None),
            other => T::parse_value(other).map(Some),
        }
    }
}

// Feature-gated impls mapping common third-party types onto the logical
// [`Value`] variants. These keep the core dependency-free: enabling a feature
// turns its crate into an optional dependency and adds the conversions.

#[cfg(feature = "uuid")]
impl IntoValue for uuid::Uuid {
    fn into_value(self) -> Value {
        Value::Uuid(self)
    }
}

#[cfg(feature = "uuid")]
impl ParseValue for uuid::Uuid {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Uuid(v) => Ok(*v),
            other => Err(Error::invalid(format!("expected uuid, found {}", other.kind()))),
        }
    }
}

#[cfg(feature = "chrono")]
impl IntoValue for chrono::DateTime<chrono::Utc> {
    fn into_value(self) -> Value {
        Value::Timestamp(self)
    }
}

#[cfg(feature = "chrono")]
impl ParseValue for chrono::DateTime<chrono::Utc> {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Timestamp(v) => Ok(*v),
            other => Err(Error::invalid(format!("expected timestamp, found {}", other.kind()))),
        }
    }
}

#[cfg(feature = "decimal")]
impl IntoValue for rust_decimal::Decimal {
    fn into_value(self) -> Value {
        Value::Decimal(self)
    }
}

#[cfg(feature = "decimal")]
impl ParseValue for rust_decimal::Decimal {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Decimal(v) => Ok(*v),
            other => Err(Error::invalid(format!("expected decimal, found {}", other.kind()))),
        }
    }
}

#[cfg(feature = "json")]
impl IntoValue for serde_json::Value {
    fn into_value(self) -> Value {
        Value::Json(self)
    }
}

#[cfg(feature = "json")]
impl ParseValue for serde_json::Value {
    fn parse_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Json(v) => Ok(v.clone()),
            other => Err(Error::invalid(format!("expected json, found {}", other.kind()))),
        }
    }
}
