#[cfg(feature = "uuid")]
#[test]
fn uuid_round_trips() {
    use diode_sql::{IntoValue, ParseValue, Value};

    let id = uuid::Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
    let value = id.into_value();
    assert_eq!(value.kind(), "uuid");
    assert!(matches!(value, Value::Uuid(_)));
    assert_eq!(uuid::Uuid::parse_value(&value).unwrap(), id);
}

#[cfg(feature = "chrono")]
#[test]
fn chrono_round_trips() {
    use chrono::{TimeZone, Utc};
    use diode_sql::{IntoValue, ParseValue, Value};

    let ts = Utc.timestamp_micros(1_700_000_000_000_000).unwrap();
    let value = ts.into_value();
    assert_eq!(value.kind(), "timestamp");
    assert!(matches!(value, Value::Timestamp(_)));
    assert_eq!(chrono::DateTime::<Utc>::parse_value(&value).unwrap(), ts);
}

#[cfg(feature = "decimal")]
#[test]
fn decimal_round_trips() {
    use diode_sql::{IntoValue, ParseValue, Value};
    use rust_decimal::Decimal;

    let d = Decimal::new(12345, 2); // 123.45
    let value = d.into_value();
    assert_eq!(value.kind(), "decimal");
    assert!(matches!(value, Value::Decimal(_)));
    assert_eq!(Decimal::parse_value(&value).unwrap(), d);
}

#[cfg(feature = "json")]
#[test]
fn json_round_trips() {
    use diode_sql::{IntoValue, ParseValue, Value};
    use serde_json::json;

    let doc = json!({ "a": 1, "b": [true, null] });
    let value = doc.clone().into_value();
    assert_eq!(value.kind(), "json");
    assert!(matches!(value, Value::Json(_)));
    assert_eq!(serde_json::Value::parse_value(&value).unwrap(), doc);
}
