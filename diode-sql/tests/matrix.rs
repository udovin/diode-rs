//! Executable documentation of the persistence matrix: how insert / update /
//! delete / find / upsert render for an auto-generated `Option` primary key
//! (absent and set) versus a natural (non-Option) key. Every assertion matches
//! the generated SQL exactly, character for character (Postgres dialect).

use std::num::NonZeroU64;

use diode_sql::{Dialect, Object, QueryKeyed, QueryObject, Value};

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

#[derive(Object, Debug, Clone, PartialEq)]
#[object(table = "settings")]
struct Setting {
    #[column(primary_key)]
    name: String,
    value: String,
}

fn setting() -> Setting {
    Setting {
        name: "theme".to_string(),
        value: "dark".to_string(),
    }
}

// --- auto-generated key, not set yet (new object) ---

#[test]
fn auto_key_none_insert_omits_key() {
    let (sql, params) = user(None).insert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "users" ("name", "email_address", "active") VALUES ($1, $2, $3)"#
    );
    assert_eq!(
        params.as_slice(),
        &[
            Value::Text("alice".to_string()),
            Value::Text("alice@example.com".to_string()),
            Value::Bool(true),
        ]
    );
}

#[test]
fn auto_key_none_insert_returning_fetches_key() {
    let (sql, _) = user(None).insert().returning(["id"]).render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "users" ("name", "email_address", "active") VALUES ($1, $2, $3) RETURNING "id""#
    );
}

#[test]
fn auto_key_none_update_is_none() {
    // No key - nothing to address; the caller must handle this.
    assert!(user(None).update().is_none());
}

// Note: delete-by-key for an unsaved object is unrepresentable - `delete`
// takes a `Key` argument, and there is none to pass.

#[test]
fn auto_key_none_upsert_degenerates_to_insert() {
    // The key is omitted, so a conflict on it cannot happen: the ON CONFLICT
    // clause is dead weight and the statement behaves as a plain insert.
    let (sql, _) = user(None).upsert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "users" ("name", "email_address", "active") VALUES ($1, $2, $3) ON CONFLICT ("id") DO UPDATE SET "name" = excluded."name", "email_address" = excluded."email_address", "active" = excluded."active""#
    );
}

// --- auto-generated key, set (persisted object) ---

#[test]
fn auto_key_some_insert_includes_key() {
    let (sql, params) = user(Some(7)).insert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "users" ("id", "name", "email_address", "active") VALUES ($1, $2, $3, $4)"#
    );
    assert_eq!(
        params.as_slice(),
        &[
            Value::I64(7),
            Value::Text("alice".to_string()),
            Value::Text("alice@example.com".to_string()),
            Value::Bool(true),
        ]
    );
}

#[test]
fn auto_key_some_find() {
    let key = NonZeroU64::new(7).unwrap();
    let (sql, params) = User::find(key).render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"SELECT "id", "name", "email_address", "active" FROM "users" WHERE "id" = $1"#
    );
    assert_eq!(params.as_slice(), &[Value::I64(7)]);
}

#[test]
fn auto_key_some_update_sets_non_key_where_key() {
    // Affected rows must be checked by the executor: 0 means the row vanished.
    let (sql, params) = user(Some(7)).update().unwrap().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"UPDATE "users" SET "name" = $1, "email_address" = $2, "active" = $3 WHERE "id" = $4"#
    );
    assert_eq!(
        params.as_slice(),
        &[
            Value::Text("alice".to_string()),
            Value::Text("alice@example.com".to_string()),
            Value::Bool(true),
            Value::I64(7),
        ]
    );
}

#[test]
fn auto_key_some_delete() {
    let key = NonZeroU64::new(7).unwrap();
    let (sql, params) = User::delete(key).render(Dialect::Postgres);
    assert_eq!(sql, r#"DELETE FROM "users" WHERE "id" = $1"#);
    assert_eq!(params.as_slice(), &[Value::I64(7)]);
}

#[test]
fn auto_key_some_upsert_inserts_or_updates_by_key() {
    let (sql, params) = user(Some(7)).upsert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "users" ("id", "name", "email_address", "active") VALUES ($1, $2, $3, $4) ON CONFLICT ("id") DO UPDATE SET "name" = excluded."name", "email_address" = excluded."email_address", "active" = excluded."active""#
    );
    assert_eq!(params.len(), 4);
}

// --- natural (non-Option) key: always present, never database-generated ---

#[test]
fn natural_key_insert_always_includes_key() {
    let (sql, params) = setting().insert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "settings" ("name", "value") VALUES ($1, $2)"#
    );
    assert_eq!(
        params.as_slice(),
        &[
            Value::Text("theme".to_string()),
            Value::Text("dark".to_string()),
        ]
    );
}

#[test]
fn natural_key_find() {
    let (sql, params) = Setting::find("theme".to_string()).render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"SELECT "name", "value" FROM "settings" WHERE "name" = $1"#
    );
    assert_eq!(params.as_slice(), &[Value::Text("theme".to_string())]);
}

#[test]
fn natural_key_update_is_always_some() {
    // The key cannot rename itself here: key columns are excluded from SET.
    let (sql, params) = setting().update().unwrap().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"UPDATE "settings" SET "value" = $1 WHERE "name" = $2"#
    );
    assert_eq!(
        params.as_slice(),
        &[
            Value::Text("dark".to_string()),
            Value::Text("theme".to_string()),
        ]
    );
}

#[test]
fn natural_key_delete() {
    let (sql, params) = Setting::delete("theme".to_string()).render(Dialect::Postgres);
    assert_eq!(sql, r#"DELETE FROM "settings" WHERE "name" = $1"#);
    assert_eq!(params.as_slice(), &[Value::Text("theme".to_string())]);
}

#[test]
fn natural_key_upsert_is_the_idiomatic_write() {
    // "New or existing" is only knowable by the database for a natural key.
    let (sql, params) = setting().upsert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "settings" ("name", "value") VALUES ($1, $2) ON CONFLICT ("name") DO UPDATE SET "value" = excluded."value""#
    );
    assert_eq!(
        params.as_slice(),
        &[
            Value::Text("theme".to_string()),
            Value::Text("dark".to_string()),
        ]
    );
}

// --- composite natural key ---

#[derive(Object, Debug, Clone, PartialEq)]
#[object(table = "user_roles")]
struct UserRole {
    #[column(primary_key)]
    user_id: NonZeroU64,
    #[column(primary_key)]
    role_id: NonZeroU64,
    granted: bool,
}

fn user_role() -> UserRole {
    UserRole {
        user_id: NonZeroU64::new(3).unwrap(),
        role_id: NonZeroU64::new(9).unwrap(),
        granted: true,
    }
}

#[test]
fn composite_key_insert() {
    let (sql, params) = user_role().insert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "user_roles" ("user_id", "role_id", "granted") VALUES ($1, $2, $3)"#
    );
    assert_eq!(
        params.as_slice(),
        &[Value::I64(3), Value::I64(9), Value::Bool(true)]
    );
}

#[test]
fn composite_key_update() {
    let (sql, params) = user_role().update().unwrap().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"UPDATE "user_roles" SET "granted" = $1 WHERE "user_id" = $2 AND "role_id" = $3"#
    );
    assert_eq!(
        params.as_slice(),
        &[Value::Bool(true), Value::I64(3), Value::I64(9)]
    );
}

#[test]
fn composite_key_delete() {
    let key = (NonZeroU64::new(3).unwrap(), NonZeroU64::new(9).unwrap());
    let (sql, params) = UserRole::delete(key).render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"DELETE FROM "user_roles" WHERE "user_id" = $1 AND "role_id" = $2"#
    );
    assert_eq!(params.as_slice(), &[Value::I64(3), Value::I64(9)]);
}

#[test]
fn composite_key_upsert() {
    let (sql, params) = user_role().upsert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "user_roles" ("user_id", "role_id", "granted") VALUES ($1, $2, $3) ON CONFLICT ("user_id", "role_id") DO UPDATE SET "granted" = excluded."granted""#
    );
    assert_eq!(params.len(), 3);
}
