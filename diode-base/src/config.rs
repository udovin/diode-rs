use std::collections::BTreeMap;
use std::path::Path;

use diode::{Extract, StdError};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(flatten)]
    pub(crate) configs: BTreeMap<String, serde_json::Value>,
}

pub trait ConfigSection: DeserializeOwned {
    fn key() -> &'static str;
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get<T>(&self, name: impl AsRef<str>) -> Result<T, StdError>
    where
        T: DeserializeOwned,
    {
        Ok(serde_json::from_value(
            self.configs
                .get(name.as_ref())
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )?)
    }

    pub fn set<T>(&mut self, name: impl Into<String>, value: T) -> Result<(), StdError>
    where
        T: Serialize,
    {
        self.configs
            .insert(name.into(), serde_json::to_value(value)?);
        Ok(())
    }

    pub fn with<T>(mut self, name: impl Into<String>, value: T) -> Self
    where
        T: Serialize,
    {
        self.configs
            .insert(name.into(), serde_json::to_value(value).unwrap());
        self
    }

    pub fn merge_from(&mut self, other: Self) -> Result<(), StdError> {
        for (key, value) in other.configs {
            let entry = self.configs.entry(key);
            merge_json_from(entry.or_insert(serde_json::Value::Null), value)?;
        }
        Ok(())
    }

    pub fn parse<T>(text: T) -> Result<Self, StdError>
    where
        T: AsRef<str>,
    {
        Ok(serde_json::from_str(text.as_ref())?)
    }

    pub async fn parse_file(path: impl AsRef<Path>) -> Result<Self, StdError> {
        let text = tokio::fs::read_to_string(path).await?;
        Self::parse(text)
    }

    /// Check if the config is empty
    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    /// Get the number of config entries
    pub fn len(&self) -> usize {
        self.configs.len()
    }
}

impl<T> Extract<T> for Config
where
    T: ConfigSection,
{
    fn extract(app: &diode::AppBuilder) -> Result<T, diode::AppError> {
        Ok(app
            .get_component_ref::<Config>()
            .unwrap()
            .get::<T>(T::key())
            .unwrap())
    }
}

fn merge_json_from(lhs: &mut serde_json::Value, rhs: serde_json::Value) -> Result<(), StdError> {
    match lhs {
        serde_json::Value::Object(l) => match rhs {
            serde_json::Value::Object(r) => {
                for (key, value) in r {
                    let entry = l.entry(key);
                    merge_json_from(entry.or_insert(serde_json::Value::Null), value)?;
                }
            }
            _ => *lhs = rhs,
        },
        serde_json::Value::Array(l) => match rhs {
            serde_json::Value::Array(r) => {
                l.extend(r);
            }
            _ => *lhs = rhs,
        },
        _ => *lhs = rhs,
    }
    Ok(())
}
