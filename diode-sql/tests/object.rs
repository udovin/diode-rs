use std::num::NonZeroU64;

use diode_sql::{Columns, Error, Fields, Keyed, Object, Value};

#[derive(Object, Debug, Clone, PartialEq)]
#[object(table = "users")]
struct User {
    #[column(primary_key)]
    id: Option<NonZeroU64>,
    name: String,
    #[column(name = "email_address")]
    email: String,
    active: bool,
}

fn user(id: Option<u64>) -> User {
    User {
        id: id.and_then(NonZeroU64::new),
        name: "alice".to_string(),
        email: "alice@example.com".to_string(),
        active: true,
    }
}

#[test]
fn object_metadata() {
    assert_eq!(User::TABLE_NAME, "users");
    let cols: Vec<&str> = User::columns().names().iter().map(String::as_str).collect();
    assert_eq!(cols, ["id", "name", "email_address", "active"]);
}

#[test]
fn object_round_trip() {
    let u = user(Some(7));
    let columns = User::columns();
    let values = u.values(columns);
    assert_eq!(User::parse(&values, columns).unwrap(), u);
}

#[test]
fn key_metadata() {
    assert_eq!(User::KEY_COLUMNS.to_vec(), vec!["id"]);
}

#[test]
fn unsaved_key_is_none() {
    let u = user(None);
    assert_eq!(u.key(), None);
    let columns = User::columns();
    let values = u.values(columns);
    assert_eq!(columns.get_value(&values, "id"), Some(&Value::Null));
}

#[test]
fn set_key_populates() {
    let mut u = user(None);
    u.set_key(NonZeroU64::new(42).unwrap());
    assert_eq!(u.key(), NonZeroU64::new(42));
}

#[test]
fn single_key_codec() {
    let key = NonZeroU64::new(5).unwrap();
    let values = User::key_values(&key);
    assert_eq!(values.as_slice().to_vec(), vec![Value::I64(5)]);

    let columns = Columns::new(User::KEY_COLUMNS.iter().copied());
    assert_eq!(User::parse_key(&values, &columns).unwrap(), key);
}

#[test]
fn parse_is_order_independent() {
    let columns = Columns::new(["email_address", "active", "id", "name"]);
    let mut values = columns.new_values();
    columns.set_value(&mut values, "id", NonZeroU64::new(1).unwrap());
    columns.set_value(&mut values, "name", "dora".to_string());
    columns.set_value(&mut values, "email_address", "d@example.com".to_string());
    columns.set_value(&mut values, "active", true);

    let u = User::parse(&values, &columns).unwrap();
    assert_eq!(u.key(), NonZeroU64::new(1));
    assert_eq!(u.name, "dora");
}

#[test]
fn parse_missing_column_errors() {
    let columns = Columns::new(["id", "name"]);
    let mut values = columns.new_values();
    columns.set_value(&mut values, "id", NonZeroU64::new(1).unwrap());
    columns.set_value(&mut values, "name", "x".to_string());

    let err = User::parse(&values, &columns).unwrap_err();
    assert!(matches!(err, Error::UnknownColumn(c) if c == "email_address"));
}

// --- composite key ---

#[derive(Object, Debug, Clone, PartialEq)]
#[object(table = "user_roles")]
struct UserRole {
    #[column(primary_key)]
    user_id: NonZeroU64,
    #[column(primary_key)]
    role_id: NonZeroU64,
    granted: bool,
}

#[test]
fn composite_key_metadata_and_codec() {
    assert_eq!(UserRole::KEY_COLUMNS.to_vec(), vec!["user_id", "role_id"]);

    let mut ur = UserRole {
        user_id: NonZeroU64::new(3).unwrap(),
        role_id: NonZeroU64::new(9).unwrap(),
        granted: true,
    };

    // Assigned key is always present.
    let key = (NonZeroU64::new(3).unwrap(), NonZeroU64::new(9).unwrap());
    assert_eq!(ur.key(), Some(key));

    // key_values aligns with KEY_COLUMNS, and parse_key round-trips it.
    let values = UserRole::key_values(&key);
    assert_eq!(values.as_slice().to_vec(), vec![Value::I64(3), Value::I64(9)]);
    let columns = Columns::new(UserRole::KEY_COLUMNS.iter().copied());
    assert_eq!(UserRole::parse_key(&values, &columns).unwrap(), key);

    // set_key writes every key field.
    ur.set_key((NonZeroU64::new(1).unwrap(), NonZeroU64::new(2).unwrap()));
    assert_eq!(ur.user_id, NonZeroU64::new(1).unwrap());
    assert_eq!(ur.role_id, NonZeroU64::new(2).unwrap());

    // Full object still round-trips.
    let columns = UserRole::columns();
    let row = ur.values(columns);
    assert_eq!(UserRole::parse(&row, columns).unwrap(), ur);
}

// --- key-less ---

#[derive(Object, Debug, Clone, PartialEq)]
#[object(table = "audit_log")]
struct AuditEntry {
    action: String,
    detail: String,
}

#[test]
fn keyless_object_round_trips() {
    // AuditEntry implements Object but intentionally not Keyed.
    let entry = AuditEntry {
        action: "login".to_string(),
        detail: "ok".to_string(),
    };
    let columns = AuditEntry::columns();
    let values = entry.values(columns);
    assert_eq!(AuditEntry::parse(&values, columns).unwrap(), entry);
    assert_eq!(AuditEntry::TABLE_NAME, "audit_log");
}

// --- natural (non-Option) single key ---

#[derive(Object, Debug, Clone, PartialEq)]
#[object(table = "settings")]
struct Setting {
    #[column(primary_key)]
    name: String,
    value: String,
}

#[test]
fn natural_string_key() {
    let setting = Setting {
        name: "theme".to_string(),
        value: "dark".to_string(),
    };
    // A natural key is always present.
    assert_eq!(setting.key(), Some("theme".to_string()));
    assert_eq!(Setting::KEY_COLUMNS.to_vec(), vec!["name"]);
    // Not database-generated, so insert must keep the column.
    assert_eq!(Setting::GENERATED_COLUMNS, &[] as &[&str]);

    let values = Setting::key_values(&"theme".to_string());
    assert_eq!(
        values.as_slice().to_vec(),
        vec![Value::Text("theme".to_string())]
    );

    let columns = Setting::columns();
    let row = setting.values(columns);
    assert_eq!(Setting::parse(&row, columns).unwrap(), setting);
}

// --- flatten: embedded value group ---

#[derive(Fields, Debug, Clone, PartialEq)]
struct Timestamps {
    created_at: i64,
    updated_at: i64,
}

#[derive(Object, Debug, Clone, PartialEq)]
#[object(table = "posts")]
struct Post {
    #[column(primary_key)]
    id: Option<NonZeroU64>,
    title: String,
    #[column(flatten)]
    timestamps: Timestamps,
}

#[test]
fn flatten_value_group() {
    let cols: Vec<&str> = Post::columns().names().iter().map(String::as_str).collect();
    assert_eq!(cols, ["id", "title", "created_at", "updated_at"]);

    let post = Post {
        id: NonZeroU64::new(1),
        title: "hello".to_string(),
        timestamps: Timestamps {
            created_at: 100,
            updated_at: 200,
        },
    };
    let columns = Post::columns();
    let values = post.values(columns);
    assert_eq!(columns.get_value(&values, "created_at"), Some(&Value::I64(100)));
    assert_eq!(Post::parse(&values, columns).unwrap(), post);
}

// --- flatten: object inside object (no column collision) ---

#[derive(Object, Debug, Clone, PartialEq)]
#[object(table = "a")]
struct A {
    #[column(primary_key)]
    a_id: Option<NonZeroU64>,
    a_val: String,
}

#[derive(Object, Debug, Clone, PartialEq)]
#[object(table = "b")]
struct B {
    #[column(primary_key)]
    b_id: Option<NonZeroU64>,
    b_val: String,
}

#[derive(Fields, Debug, Clone, PartialEq)]
struct Joined {
    #[column(flatten)]
    a: A,
    #[column(flatten)]
    b: B,
}

#[test]
fn flatten_object_in_object() {
    let cols: Vec<&str> = Joined::columns().names().iter().map(String::as_str).collect();
    assert_eq!(cols, ["a_id", "a_val", "b_id", "b_val"]);

    let joined = Joined {
        a: A {
            a_id: NonZeroU64::new(1),
            a_val: "x".to_string(),
        },
        b: B {
            b_id: NonZeroU64::new(2),
            b_val: "y".to_string(),
        },
    };
    let columns = Joined::columns();
    let values = joined.values(columns);
    assert_eq!(Joined::parse(&values, columns).unwrap(), joined);
}

// --- flatten: colliding columns panic ---

#[derive(Fields)]
struct Collision {
    #[column(flatten)]
    first: A,
    #[column(flatten)]
    second: A,
}

#[test]
#[should_panic(expected = "duplicate column")]
fn flatten_collision_panics() {
    let _ = Collision::columns();
}
