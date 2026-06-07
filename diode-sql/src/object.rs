use crate::{Columns, Error, Values};

/// A group of columns that can be read from and written into a [`Values`] row by
/// name.
///
/// `Fields` is the mapping layer shared by table rows ([`Object`]) and embeddable
/// value groups. A field of a `Fields` type can be flattened into its parent's
/// columns with `#[column(flatten)]`; the parent splices in the nested
/// [`columns`](Fields::columns) and delegates [`write_values`](Fields::write_values)
/// and [`parse`](Fields::parse).
///
/// Derive a standalone (table-less) group with `#[derive(Fields)]`; table rows
/// derive [`Object`], which implies `Fields`.
pub trait Fields: Sized {
    /// The column layout of this group.
    fn columns() -> &'static Columns;

    /// Writes this group's fields into `values` by name, using `columns`.
    ///
    /// `columns` may be a wider index than this group's own (for example a
    /// parent that flattened it), so values are placed by name, not position.
    fn write_values(&self, columns: &Columns, values: &mut Values);

    /// Reconstructs this group from a [`Values`] row described by `columns`.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if a required column is missing or a value cannot be
    /// parsed into the corresponding field type.
    fn parse(values: &Values, columns: &Columns) -> Result<Self, Error>;

    /// Convenience: a fresh [`Values`] row laid out by `columns` with this
    /// group's fields written in (see [`write_values`](Fields::write_values)).
    fn values(&self, columns: &Columns) -> Values {
        let mut values = columns.new_values();
        self.write_values(columns, &mut values);
        values
    }
}

/// A [`Fields`] group that maps to a named database table.
///
/// Derive it with `#[derive(Object)]`. It covers every table including key-less
/// ones (join tables, append-only logs, views); identity is added by [`Keyed`].
pub trait Object: Fields {
    /// The table name.
    const TABLE_NAME: &'static str;
}

/// An [`Object`] addressable by a primary key, single or composite.
///
/// The key is exposed as [`Self::Key`] - a single type for a one-column key, or
/// a tuple for a composite one - and as the [`KEY_COLUMNS`](Keyed::KEY_COLUMNS)
/// it maps to. [`key`](Keyed::key) returns `None` for a row that has not been
/// persisted yet (the idiomatic auto key field type is `Option<NonZeroU64>`).
///
/// The codec between the key and its columns ([`key_values`](Keyed::key_values)
/// and [`parse_key`](Keyed::parse_key)) is generated per type by the derive,
/// component by component, so the key type itself only needs to be [`Clone`].
///
/// Derived by `#[derive(Object)]` when at least one field is marked
/// `#[column(primary_key)]` (mark several for a composite key).
pub trait Keyed: Object {
    /// The primary-key type: one type for a single-column key, a tuple for a
    /// composite one.
    type Key: Clone;

    /// The primary-key columns, in key order.
    const KEY_COLUMNS: &'static [&'static str];

    /// The primary key, or `None` if this row has not been persisted.
    fn key(&self) -> Option<Self::Key>;

    /// Sets the primary key (for example after an insert returns it).
    fn set_key(&mut self, key: Self::Key);

    /// Encodes a key into a [`Values`] row aligned with
    /// [`KEY_COLUMNS`](Keyed::KEY_COLUMNS) (used to build `WHERE` clauses).
    fn key_values(key: &Self::Key) -> Values;

    /// Decodes a key from a [`Values`] row described by `columns`.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if a key column is missing or cannot be parsed.
    fn parse_key(values: &Values, columns: &Columns) -> Result<Self::Key, Error>;
}
