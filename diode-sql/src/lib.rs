//! # diode-sql
//!
//! Backend-agnostic SQL abstractions for the diode framework: a [`Value`] type,
//! row mapping ([`Fields`], [`Object`], [`Keyed`], [`Values`], [`Columns`])
//! and the [`IntoValue`] / [`ParseValue`] conversions. No database driver is
//! involved here - a backend plugs in later by mapping these types to its own.
//!
//! The core is dependency-free. Logical [`Value`] types are feature-gated and
//! carry their standard crate's type: `chrono` (`DateTime<Utc>`), `uuid`
//! (`uuid::Uuid`), `decimal` (`rust_decimal::Decimal`), `json`
//! (`serde_json::Value`). Custom Rust types extend the model by implementing
//! [`IntoValue`] / [`ParseValue`] onto an existing variant.
//!
//! Define a table row with `#[derive(Object)]`:
//!
//! ```rust
//! use std::num::NonZeroU64;
//! use diode_sql::Object;
//!
//! #[derive(Object)]
//! #[object(table = "users")]
//! struct User {
//!     #[column(primary_key)]
//!     id: Option<NonZeroU64>,
//!     name: String,
//!     #[column(name = "email_address")]
//!     email: String,
//! }
//! ```

mod column;
mod error;
mod object;
mod value;

pub use column::*;
pub use error::*;
pub use object::*;
pub use value::*;

#[cfg(feature = "macros")]
pub use diode_sql_macros::*;
