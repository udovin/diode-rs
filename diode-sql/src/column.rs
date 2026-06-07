use std::collections::HashMap;

use crate::{Error, IntoValue, ParseValue, Value, Values};

/// Maps column names to their position in a [`Values`] row.
///
/// `Columns` is the schema half of a row: it is built once per layout (for
/// example per [`Object`](crate::Object) type) and used to read and write the
/// positional [`Values`] by name.
#[derive(Debug, Clone, Default)]
pub struct Columns {
    names: Vec<String>,
    index: HashMap<String, usize>,
}

impl Columns {
    /// Builds an index from column names, in order.
    ///
    /// # Panics
    ///
    /// Panics if a column name appears more than once (for example after
    /// flattening two groups that share a column).
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let names: Vec<String> = names.into_iter().map(Into::into).collect();
        let mut index = HashMap::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            if index.insert(name.clone(), i).is_some() {
                panic!("duplicate column `{name}`");
            }
        }
        Self { names, index }
    }

    /// Returns the position of column `name`, if present.
    pub fn get_index(&self, name: impl AsRef<str>) -> Option<usize> {
        self.index.get(name.as_ref()).copied()
    }

    /// Number of columns.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Returns whether there are no columns.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The column names, in order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Returns the value stored for `name` in `values`, if the column exists.
    pub fn get_value<'a>(&self, values: &'a Values, name: impl AsRef<str>) -> Option<&'a Value> {
        self.get_index(name).and_then(|index| values.get(index))
    }

    /// Reads and parses the value of `name` into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownColumn`] if the column is absent, or
    /// [`Error::Invalid`] (attributed to the column) if the value cannot be
    /// parsed into `T`.
    pub fn parse_value<T: ParseValue>(
        &self,
        values: &Values,
        name: impl AsRef<str>,
    ) -> Result<T, Error> {
        let name = name.as_ref();
        let value = self
            .get_value(values, name)
            .ok_or_else(|| Error::unknown_column(name))?;
        T::parse_value(value).map_err(|err| err.at_column(name))
    }

    /// Creates an empty [`Values`] row sized to these columns.
    pub fn new_values(&self) -> Values {
        Values::with_len(self.len())
    }

    /// Writes `value` into the slot for `name`. Returns whether the column
    /// exists (a value for an unknown column is dropped).
    pub fn set_value(
        &self,
        values: &mut Values,
        name: impl AsRef<str>,
        value: impl IntoValue,
    ) -> bool {
        match self.get_index(name) {
            Some(index) => values.set(index, value.into_value()),
            None => false,
        }
    }
}
