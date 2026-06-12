use std::num::NonZeroU64;

use diode_sql::{
    Delete, Dialect, Direction, Expr, Nulls, Object, QueryKeyed, QueryObject, Source, Update, Value,
};

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
fn select_find_postgres() {
    let key = NonZeroU64::new(7).unwrap();
    let (sql, params) = User::find(key).render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"SELECT "id", "name", "email_address", "active" FROM "users" WHERE "id" = $1"#
    );
    assert_eq!(params.as_slice(), &[Value::I64(7)]);
}

#[test]
fn select_uses_sqlite_placeholder() {
    let key = NonZeroU64::new(7).unwrap();
    let (sql, _) = User::find(key).render(Dialect::Sqlite);
    assert!(sql.ends_with(r#"WHERE "id" = ?"#), "{sql}");
}

#[test]
fn insert_omits_unset_auto_key() {
    let (sql, params) = user(None).insert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "users" ("name", "email_address", "active") VALUES ($1, $2, $3)"#
    );
    assert_eq!(params.len(), 3);
}

#[test]
fn insert_includes_set_key() {
    let (sql, params) = user(Some(7)).insert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "users" ("id", "name", "email_address", "active") VALUES ($1, $2, $3, $4)"#
    );
    assert_eq!(params.as_slice()[0], Value::I64(7));
}

#[test]
fn insert_with_returning() {
    let (sql, _) = user(None).insert().returning(["id"]).render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "users" ("name", "email_address", "active") VALUES ($1, $2, $3) RETURNING "id""#
    );
}

#[test]
fn update_sets_non_key_columns_where_key() {
    let (sql, params) = user(Some(7)).update().unwrap().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"UPDATE "users" SET "name" = $1, "email_address" = $2, "active" = $3 WHERE "id" = $4"#
    );
    assert_eq!(params.len(), 4);
    assert_eq!(params.as_slice()[3], Value::I64(7));
}

#[test]
fn update_without_key_is_none() {
    assert!(user(None).update().is_none());
}

#[test]
fn delete_by_key() {
    let key = NonZeroU64::new(7).unwrap();
    let (sql, _) = User::delete(key).render(Dialect::Postgres);
    assert_eq!(sql, r#"DELETE FROM "users" WHERE "id" = $1"#);
}

// --- predicates ---

#[test]
fn comparison_operator_renders() {
    let (sql, params) = User::select()
        .filter(Expr::col("id").ge(5i64))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE "id" >= $1"#), "{sql}");
    assert_eq!(params.as_slice(), &[Value::I64(5)]);
}

#[test]
fn eq_null_renders_is_null() {
    let (sql, params) = User::select()
        .filter(Expr::col("active").eq(Option::<bool>::None))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE "active" IS NULL"#), "{sql}");
    assert!(params.is_empty());
}

#[test]
fn ne_null_renders_is_not_null() {
    let (sql, params) = User::select()
        .filter(Expr::col("name").ne(Option::<String>::None))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE "name" IS NOT NULL"#), "{sql}");
    assert!(params.is_empty());
}

#[test]
fn like_renders() {
    let (sql, params) = User::select()
        .filter(Expr::col("name").like("a%"))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE "name" LIKE $1"#), "{sql}");
    assert_eq!(params.as_slice(), &[Value::Text("a%".to_string())]);
}

#[test]
fn between_renders() {
    let (sql, params) = User::select()
        .filter(Expr::col("id").between(1i64, 10i64))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE "id" BETWEEN $1 AND $2"#), "{sql}");
    assert_eq!(params.len(), 2);
}

#[test]
fn or_and_not_compose() {
    let (sql, params) = User::select()
        .filter(!(Expr::col("active").eq(true).or(Expr::col("name").eq("bob"))))
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(r#"WHERE NOT ("active" = $1 OR "name" = $2)"#),
        "{sql}"
    );
    assert_eq!(params.len(), 2);
}

#[test]
fn and_or_precedence_parenthesizes() {
    // OR binds looser than AND, so the OR child of an AND needs parentheses.
    let (sql, _) = User::select()
        .filter(
            Expr::col("a")
                .eq(1i64)
                .or(Expr::col("b").eq(2i64))
                .and(Expr::col("c").eq(3i64)),
        )
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(r#"WHERE ("a" = $1 OR "b" = $2) AND "c" = $3"#),
        "{sql}"
    );
}

#[test]
fn chained_filters_are_anded() {
    let (sql, params) = User::select()
        .filter(Expr::col("active").eq(true))
        .filter(Expr::col("name").eq("alice"))
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(r#"WHERE "active" = $1 AND "name" = $2"#),
        "{sql}"
    );
    assert_eq!(params.len(), 2);
}

#[test]
fn in_list_renders_placeholders() {
    let (sql, params) = User::select()
        .filter(Expr::col("id").in_([1i64, 2, 3]))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE "id" IN ($1, $2, $3)"#), "{sql}");
    assert_eq!(params.len(), 3);
}

#[test]
fn in_with_null_also_matches_null() {
    let (sql, params) = User::select()
        .filter(Expr::col("id").in_([Some(1i64), None, Some(2i64)]))
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(r#"WHERE ("id" IN ($1, $2) OR "id" IS NULL)"#),
        "{sql}"
    );
    assert_eq!(params.as_slice(), &[Value::I64(1), Value::I64(2)]);
}

#[test]
fn in_only_null_is_is_null() {
    let (sql, params) = User::select()
        .filter(Expr::col("id").in_([Option::<i64>::None]))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE "id" IS NULL"#), "{sql}");
    assert!(params.is_empty());
}

#[test]
fn empty_in_is_always_false() {
    let (sql, params) = User::select()
        .filter(Expr::col("id").in_(Vec::<i64>::new()))
        .render(Dialect::Postgres);
    assert!(sql.ends_with("WHERE FALSE"), "{sql}");
    assert!(params.is_empty());
}

// --- raw fragments (backend-agnostic placeholders) ---

#[test]
fn raw_fragment_translates_placeholders() {
    let (sql, params) = User::select()
        .filter(Expr::raw(r#""age" > $1"#, [18i64]))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE "age" > $1"#), "{sql}");
    assert_eq!(params.as_slice(), &[Value::I64(18)]);
}

#[test]
fn raw_fragment_sqlite_uses_question_mark() {
    let (sql, _) = User::select()
        .filter(Expr::raw(r#""age" > $1"#, [18i64]))
        .render(Dialect::Sqlite);
    assert!(sql.ends_with(r#"WHERE "age" > ?"#), "{sql}");
}

#[test]
fn raw_fragment_reorders_by_position() {
    // Numbers are fragment-local; the driver always gets positional params.
    let (sql, params) = User::select()
        .filter(Expr::raw(r#"$2 < "id" AND "id" < $1"#, [10i64, 1i64]))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE $1 < "id" AND "id" < $2"#), "{sql}");
    assert_eq!(params.as_slice(), &[Value::I64(1), Value::I64(10)]);
}

#[test]
fn raw_fragment_reuses_param() {
    let (sql, params) = User::select()
        .filter(Expr::raw(r#""lo" <= $1 AND $1 <= "hi""#, [5i64]))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE "lo" <= $1 AND $2 <= "hi""#), "{sql}");
    assert_eq!(params.as_slice(), &[Value::I64(5), Value::I64(5)]);
}

// --- projection, aggregates, grouping ---

#[test]
fn projection_subset() {
    let (sql, _) = User::select()
        .columns(["id", "name"])
        .render(Dialect::Postgres);
    assert_eq!(sql, r#"SELECT "id", "name" FROM "users""#);
}

#[test]
fn count_star() {
    let (sql, params) = User::select()
        .count()
        .filter(Expr::col("active").eq(true))
        .render(Dialect::Postgres);
    assert_eq!(sql, r#"SELECT count(*) FROM "users" WHERE "active" = $1"#);
    assert_eq!(params.len(), 1);
}

#[test]
fn aggregate_projection_group_having() {
    let (sql, params) = User::select()
        .select(Expr::col("active"), None)
        .select(Expr::count_star(), Some("n"))
        .group_by(Expr::col("active"))
        .having(Expr::count_star().gt(1i64))
        .order_by(Expr::col("n"), Direction::Desc)
        .render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"SELECT "active", count(*) AS "n" FROM "users" GROUP BY "active" HAVING count(*) > $1 ORDER BY "n" DESC"#
    );
    assert_eq!(params.as_slice(), &[Value::I64(1)]);
}

#[test]
fn order_limit_offset_render() {
    let (sql, _) = User::select()
        .order_by(Expr::col("name"), Direction::Asc)
        .order_by(Expr::col("id"), Direction::Desc)
        .limit(10)
        .offset(20)
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(r#"ORDER BY "name" ASC, "id" DESC LIMIT 10 OFFSET 20"#),
        "{sql}"
    );
}

// --- ad-hoc write ---

#[test]
fn ad_hoc_delete_by_filter() {
    let (sql, params) = Delete::table::<User>()
        .filter(Expr::col("active").eq(false))
        .render(Dialect::Postgres);
    assert_eq!(sql, r#"DELETE FROM "users" WHERE "active" = $1"#);
    assert_eq!(params.as_slice(), &[Value::Bool(false)]);
}

#[test]
fn ad_hoc_update_by_filter() {
    let (sql, params) = Update::table::<User>()
        .set("active", false)
        .set("name", "x")
        .filter(Expr::col("id").eq(1i64))
        .render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"UPDATE "users" SET "active" = $1, "name" = $2 WHERE "id" = $3"#
    );
    assert_eq!(params.len(), 3);
}

#[test]
fn update_set_with_expression() {
    let (sql, params) = Update::table::<User>()
        .set("id", Expr::col("id").add(1i64))
        .render(Dialect::Postgres);
    assert_eq!(sql, r#"UPDATE "users" SET "id" = "id" + $1"#);
    assert_eq!(params.as_slice(), &[Value::I64(1)]);
}

// --- qualified columns, case, cast, distinct-from ---

#[test]
fn qualified_column() {
    let (pg, _) = User::select()
        .filter(Expr::col_at("u", "id").eq(1i64))
        .render(Dialect::Postgres);
    assert!(pg.ends_with(r#"WHERE "u"."id" = $1"#), "{pg}");

    let (my, _) = User::select()
        .filter(Expr::col_at("u", "id").eq(1i64))
        .render(Dialect::Mysql);
    assert!(my.ends_with("WHERE `u`.`id` = ?"), "{my}");
}

#[test]
fn case_expression() {
    let label = Expr::case()
        .when(Expr::col("active").eq(true), "on")
        .otherwise("off")
        .end();
    let (sql, params) = User::select()
        .select(Expr::col("id"), None)
        .select(label, Some("state"))
        .render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"SELECT "id", CASE WHEN "active" = $1 THEN $2 ELSE $3 END AS "state" FROM "users""#
    );
    assert_eq!(params.len(), 3);
}

#[test]
fn cast_expression() {
    let (sql, _) = User::select()
        .filter(Expr::col("id").cast("text").eq("5"))
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"WHERE CAST("id" AS text) = $1"#), "{sql}");
}

#[test]
fn is_distinct_from_dialects() {
    let pg = User::select()
        .filter(Expr::col("a").is_distinct_from(Expr::col("b")))
        .render(Dialect::Postgres)
        .0;
    assert!(pg.ends_with(r#"WHERE "a" IS DISTINCT FROM "b""#), "{pg}");

    let my = User::select()
        .filter(Expr::col("a").is_distinct_from(Expr::col("b")))
        .render(Dialect::Mysql)
        .0;
    assert!(my.ends_with("WHERE NOT (`a` <=> `b`)"), "{my}");

    let my2 = User::select()
        .filter(Expr::col("a").is_not_distinct_from(Expr::col("b")))
        .render(Dialect::Mysql)
        .0;
    assert!(my2.ends_with("WHERE `a` <=> `b`"), "{my2}");
}

// --- subquery expressions ---

#[test]
fn scalar_subquery() {
    let (sql, _) = User::select()
        .filter(Expr::col("id").eq(Expr::subquery(User::select().count())))
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(r#"WHERE "id" = (SELECT count(*) FROM "users")"#),
        "{sql}"
    );
}

#[test]
fn exists_subquery() {
    let (sql, params) = User::select()
        .filter(Expr::exists(
            User::select().filter(Expr::col("active").eq(true)),
        ))
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(
            r#"WHERE EXISTS (SELECT "id", "name", "email_address", "active" FROM "users" WHERE "active" = $1)"#
        ),
        "{sql}"
    );
    assert_eq!(params.as_slice(), &[Value::Bool(true)]);
}

#[test]
fn in_subquery_threads_parameters() {
    let inner = User::select()
        .columns(["id"])
        .filter(Expr::col("active").eq(true));
    let (sql, params) = User::select()
        .filter(Expr::col("name").eq("x").and(Expr::col("id").in_subquery(inner)))
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(
            r#"WHERE "name" = $1 AND "id" IN (SELECT "id" FROM "users" WHERE "active" = $2)"#
        ),
        "{sql}"
    );
    assert_eq!(
        params.as_slice(),
        &[Value::Text("x".to_string()), Value::Bool(true)]
    );
}

// --- mysql dialect ---

#[test]
fn mysql_uses_backticks_and_question_marks() {
    let key = NonZeroU64::new(7).unwrap();
    let (sql, params) = User::find(key).render(Dialect::Mysql);
    assert_eq!(
        sql,
        "SELECT `id`, `name`, `email_address`, `active` FROM `users` WHERE `id` = ?"
    );
    assert_eq!(params.as_slice(), &[Value::I64(7)]);
}

#[test]
fn mysql_concat_uses_function() {
    let (sql, params) = Update::table::<User>()
        .set("name", Expr::col("name").concat("!"))
        .render(Dialect::Mysql);
    assert_eq!(sql, "UPDATE `users` SET `name` = CONCAT(`name`, ?)");
    assert_eq!(params.as_slice(), &[Value::Text("!".to_string())]);
}

#[test]
fn postgres_concat_uses_pipes() {
    let (sql, _) = Update::table::<User>()
        .set("name", Expr::col("name").concat("!"))
        .render(Dialect::Postgres);
    assert_eq!(sql, r#"UPDATE "users" SET "name" = "name" || $1"#);
}

#[test]
fn ilike_native_on_postgres_emulated_elsewhere() {
    let pg = User::select()
        .filter(Expr::col("name").ilike("a%"))
        .render(Dialect::Postgres)
        .0;
    assert!(pg.ends_with(r#"WHERE "name" ILIKE $1"#), "{pg}");

    let my = User::select()
        .filter(Expr::col("name").ilike("a%"))
        .render(Dialect::Mysql)
        .0;
    assert!(my.ends_with("WHERE LOWER(`name`) LIKE LOWER(?)"), "{my}");

    let lite = User::select()
        .filter(Expr::col("name").ilike("a%"))
        .render(Dialect::Sqlite)
        .0;
    assert!(lite.ends_with(r#"WHERE LOWER("name") LIKE LOWER(?)"#), "{lite}");
}

#[test]
#[should_panic(expected = "RETURNING")]
fn mysql_returning_panics() {
    let _ = user(None).insert().returning(["id"]).render(Dialect::Mysql);
}

// --- phase 2: from / joins / distinct / order / locking ---

#[test]
fn inner_join_renders() {
    let (sql, _) = User::select()
        .inner_join(
            "orders",
            Expr::col_at("orders", "user_id").eq(Expr::col_at("users", "id")),
        )
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(
            r#"FROM "users" INNER JOIN "orders" ON "orders"."user_id" = "users"."id""#
        ),
        "{sql}"
    );
}

#[test]
fn left_join_with_alias() {
    let (sql, _) = User::select()
        .left_join(
            Source::table("orders").alias("o"),
            Expr::col_at("o", "user_id").eq(Expr::col_at("users", "id")),
        )
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(r#"LEFT JOIN "orders" AS "o" ON "o"."user_id" = "users"."id""#),
        "{sql}"
    );
}

#[test]
fn cross_join_has_no_on() {
    let (sql, _) = User::select().cross_join("nums").render(Dialect::Postgres);
    assert!(sql.ends_with(r#"FROM "users" CROSS JOIN "nums""#), "{sql}");
}

#[test]
fn distinct_renders() {
    let (sql, _) = User::select()
        .distinct()
        .columns(["name"])
        .render(Dialect::Postgres);
    assert_eq!(sql, r#"SELECT DISTINCT "name" FROM "users""#);
}

#[test]
fn from_derived_table_threads_params() {
    let inner = User::select()
        .columns(["id"])
        .filter(Expr::col("active").eq(true));
    let (sql, params) = User::select()
        .from(Source::subquery(inner, "u"))
        .render(Dialect::Postgres);
    assert!(
        sql.ends_with(r#"FROM (SELECT "id" FROM "users" WHERE "active" = $1) AS "u""#),
        "{sql}"
    );
    assert_eq!(params.as_slice(), &[Value::Bool(true)]);
}

#[test]
fn order_by_nulls_dialects() {
    let pg = User::select()
        .order_by_nulls(Expr::col("name"), Direction::Asc, Nulls::Last)
        .render(Dialect::Postgres)
        .0;
    assert!(pg.ends_with(r#"ORDER BY "name" ASC NULLS LAST"#), "{pg}");

    // MySQL drops the NULLS clause.
    let my = User::select()
        .order_by_nulls(Expr::col("name"), Direction::Asc, Nulls::Last)
        .render(Dialect::Mysql)
        .0;
    assert!(my.ends_with("ORDER BY `name` ASC"), "{my}");
}

#[test]
fn for_update_dialects() {
    let pg = User::select()
        .filter(Expr::col("id").eq(1i64))
        .for_update()
        .render(Dialect::Postgres)
        .0;
    assert!(pg.ends_with(r#"WHERE "id" = $1 FOR UPDATE"#), "{pg}");

    // SQLite has no locking clause.
    let lite = User::select()
        .filter(Expr::col("id").eq(1i64))
        .for_update()
        .render(Dialect::Sqlite)
        .0;
    assert!(lite.ends_with(r#"WHERE "id" = ?"#), "{lite}");
}

// --- upsert ---

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

#[test]
fn upsert_natural_key_postgres() {
    let (sql, params) = setting().upsert().render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "settings" ("name", "value") VALUES ($1, $2) ON CONFLICT ("name") DO UPDATE SET "value" = excluded."value""#
    );
    assert_eq!(params.len(), 2);
}

#[test]
fn upsert_mysql_on_duplicate_key() {
    let (sql, _) = setting().upsert().render(Dialect::Mysql);
    assert_eq!(
        sql,
        "INSERT INTO `settings` (`name`, `value`) VALUES (?, ?) ON DUPLICATE KEY UPDATE `value` = VALUES(`value`)"
    );
}

#[test]
fn upsert_auto_key_updates_non_key_columns() {
    let (sql, _) = user(Some(7)).upsert().render(Dialect::Postgres);
    assert!(
        sql.ends_with(
            r#"ON CONFLICT ("id") DO UPDATE SET "name" = excluded."name", "email_address" = excluded."email_address", "active" = excluded."active""#
        ),
        "{sql}"
    );
}

#[test]
fn on_conflict_do_nothing_with_target() {
    let (sql, _) = setting()
        .insert()
        .on_conflict_do_nothing(["name"])
        .render(Dialect::Postgres);
    assert!(sql.ends_with(r#"ON CONFLICT ("name") DO NOTHING"#), "{sql}");
}

#[test]
fn on_conflict_do_nothing_any_target() {
    let (sql, _) = setting()
        .insert()
        .on_conflict_do_nothing(Vec::<String>::new())
        .render(Dialect::Sqlite);
    assert!(sql.ends_with("ON CONFLICT DO NOTHING"), "{sql}");
}

#[test]
fn mysql_do_nothing_assigns_column_to_itself() {
    let (sql, _) = setting()
        .insert()
        .on_conflict_do_nothing(["name"])
        .render(Dialect::Mysql);
    assert!(
        sql.ends_with("ON DUPLICATE KEY UPDATE `name` = `name`"),
        "{sql}"
    );
}

#[test]
fn upsert_all_key_table_degrades_to_do_nothing() {
    #[derive(Object, Debug, Clone, PartialEq)]
    #[object(table = "follows")]
    struct Follow {
        #[column(primary_key)]
        follower_id: NonZeroU64,
        #[column(primary_key)]
        followee_id: NonZeroU64,
    }

    let follow = Follow {
        follower_id: NonZeroU64::new(1).unwrap(),
        followee_id: NonZeroU64::new(2).unwrap(),
    };
    let (sql, _) = follow.upsert().render(Dialect::Postgres);
    assert!(
        sql.ends_with(r#"ON CONFLICT ("follower_id", "followee_id") DO NOTHING"#),
        "{sql}"
    );
}

#[test]
fn upsert_with_returning_renders_in_order() {
    let (sql, _) = user(Some(7))
        .upsert()
        .returning(["id"])
        .render(Dialect::Postgres);
    assert!(
        sql.contains("ON CONFLICT") && sql.ends_with(r#"RETURNING "id""#),
        "{sql}"
    );
}

#[test]
#[should_panic(expected = "target")]
fn do_update_without_target_panics_on_postgres() {
    let _ = setting()
        .insert()
        .on_conflict_do_update(Vec::<String>::new())
        .render(Dialect::Postgres);
}

// --- composite key ---

#[derive(Object)]
#[object(table = "user_roles")]
struct UserRole {
    #[column(primary_key)]
    user_id: NonZeroU64,
    #[column(primary_key)]
    role_id: NonZeroU64,
    granted: bool,
}

#[test]
fn composite_key_find_ands_columns() {
    let key = (NonZeroU64::new(3).unwrap(), NonZeroU64::new(9).unwrap());
    let (sql, params) = UserRole::find(key).render(Dialect::Postgres);
    assert_eq!(
        sql,
        r#"SELECT "user_id", "role_id", "granted" FROM "user_roles" WHERE "user_id" = $1 AND "role_id" = $2"#
    );
    assert_eq!(params.as_slice(), &[Value::I64(3), Value::I64(9)]);
}
