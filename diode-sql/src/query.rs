use std::fmt::Write as _;
use std::num::{NonZeroU32, NonZeroU64};

use crate::{IntoValue, Keyed, Object, Value, Values};

/// SQL dialect controlling placeholder syntax and identifier quoting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Dialect {
    /// Numbered placeholders (`$1`, `$2`, ...), double-quoted identifiers.
    Postgres,
    /// Anonymous placeholders (`?`), double-quoted identifiers.
    Sqlite,
    /// Anonymous placeholders (`?`), backtick-quoted identifiers. `||` renders as
    /// `CONCAT(...)`, `ILIKE` is emulated with `LOWER(...) LIKE LOWER(...)`, and
    /// `RETURNING` is unsupported (panics).
    Mysql,
}

impl Dialect {
    fn quote(self) -> char {
        match self {
            Dialect::Mysql => '`',
            Dialect::Postgres | Dialect::Sqlite => '"',
        }
    }
}

/// A binary operator in an [`Expr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
    Concat,
    Like,
    ILike,
}

impl BinOp {
    fn sql(self) -> &'static str {
        match self {
            BinOp::Eq => "=",
            BinOp::Ne => "<>",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "AND",
            BinOp::Or => "OR",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Concat => "||",
            BinOp::Like => "LIKE",
            BinOp::ILike => "ILIKE",
        }
    }

    fn precedence(self) -> u8 {
        match self {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Like
            | BinOp::ILike => 4,
            BinOp::Concat => 5,
            BinOp::Add | BinOp::Sub => 6,
            BinOp::Mul | BinOp::Div => 7,
        }
    }
}

/// A unary operator in an [`Expr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
}

/// A SQL scalar or boolean expression: the building block of `WHERE`, `HAVING`,
/// projections, `GROUP BY` and `SET`.
///
/// Build leaves with [`Expr::col`] / [`Expr::val`] / [`Expr::raw`] and compose
/// with the combinator methods ([`eq`](Expr::eq), [`and`](Expr::and),
/// [`add`](Expr::add), ...). Rust values become bound literals through
/// [`IntoValue`], so `expr.eq(5)` binds a parameter while `expr.eq(Expr::col(c))`
/// compares two columns.
pub enum Expr {
    /// A column reference.
    Column(String),
    /// A bound literal, rendered as a placeholder.
    Value(Value),
    /// `name([DISTINCT] args)`.
    Func {
        name: String,
        distinct: bool,
        args: Vec<Expr>,
    },
    /// `lhs <op> rhs`.
    Binary {
        lhs: Box<Expr>,
        op: BinOp,
        rhs: Box<Expr>,
    },
    /// `<op> operand`.
    Unary { op: UnOp, operand: Box<Expr> },
    /// `expr IN (values)`. An empty set renders as `FALSE`; a null among the
    /// values adds `OR expr IS NULL`, since `IN` never matches a null on its own.
    In { expr: Box<Expr>, values: Vec<Value> },
    /// `expr BETWEEN low AND high`.
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
    },
    /// `expr IS [NOT] NULL`.
    IsNull { expr: Box<Expr>, negated: bool },
    /// A backend-agnostic raw fragment: `$1`, `$2`, ... mark holes filled from
    /// `params` in order (a number may repeat to reuse a value). The driver
    /// always receives positional parameters; the caller is responsible for the
    /// fragment's safety (never build it from untrusted input).
    Raw { sql: String, params: Vec<Value> },
}

impl Expr {
    /// A column reference.
    pub fn col(name: impl Into<String>) -> Self {
        Expr::Column(name.into())
    }

    /// A bound literal value.
    pub fn val(value: impl IntoValue) -> Self {
        Expr::Value(value.into_value())
    }

    /// A raw fragment with `$1`, `$2`, ... holes filled from `params`. See
    /// [`Expr::Raw`].
    pub fn raw(sql: impl Into<String>, params: impl IntoIterator<Item = impl IntoValue>) -> Self {
        Expr::Raw {
            sql: sql.into(),
            params: params.into_iter().map(IntoValue::into_value).collect(),
        }
    }

    /// A function call `name(args)`.
    pub fn func(name: impl Into<String>, args: impl IntoIterator<Item = Expr>) -> Self {
        Expr::Func {
            name: name.into(),
            distinct: false,
            args: args.into_iter().collect(),
        }
    }

    /// `count(*)`.
    pub fn count_star() -> Self {
        Expr::Raw {
            sql: "count(*)".to_string(),
            params: Vec::new(),
        }
    }

    /// `count(expr)`.
    pub fn count(expr: Expr) -> Self {
        Expr::func("count", [expr])
    }

    /// `count(DISTINCT expr)`.
    pub fn count_distinct(expr: Expr) -> Self {
        Expr::Func {
            name: "count".to_string(),
            distinct: true,
            args: vec![expr],
        }
    }

    /// `sum(expr)`.
    pub fn sum(expr: Expr) -> Self {
        Expr::func("sum", [expr])
    }

    /// `min(expr)`.
    pub fn min(expr: Expr) -> Self {
        Expr::func("min", [expr])
    }

    /// `max(expr)`.
    pub fn max(expr: Expr) -> Self {
        Expr::func("max", [expr])
    }

    /// `avg(expr)`.
    pub fn avg(expr: Expr) -> Self {
        Expr::func("avg", [expr])
    }
}

#[allow(clippy::should_implement_trait, clippy::wrong_self_convention)]
impl Expr {
    fn binary(self, op: BinOp, rhs: impl Into<Expr>) -> Expr {
        Expr::Binary {
            lhs: Box::new(self),
            op,
            rhs: Box::new(rhs.into()),
        }
    }

    /// `self = rhs` (or `IS NULL` for a null literal).
    pub fn eq(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Eq, rhs)
    }

    /// `self <> rhs` (or `IS NOT NULL` for a null literal).
    pub fn ne(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Ne, rhs)
    }

    /// `self < rhs`.
    pub fn lt(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Lt, rhs)
    }

    /// `self <= rhs`.
    pub fn le(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Le, rhs)
    }

    /// `self > rhs`.
    pub fn gt(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Gt, rhs)
    }

    /// `self >= rhs`.
    pub fn ge(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Ge, rhs)
    }

    /// `self AND rhs`.
    pub fn and(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::And, rhs)
    }

    /// `self OR rhs`.
    pub fn or(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Or, rhs)
    }

    /// `self + rhs`.
    pub fn add(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Add, rhs)
    }

    /// `self - rhs`.
    pub fn sub(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Sub, rhs)
    }

    /// `self * rhs`.
    pub fn mul(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Mul, rhs)
    }

    /// `self / rhs`.
    pub fn div(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Div, rhs)
    }

    /// `self || rhs` (string concatenation).
    pub fn concat(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Concat, rhs)
    }

    /// `self LIKE rhs`.
    pub fn like(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::Like, rhs)
    }

    /// `self ILIKE rhs`.
    pub fn ilike(self, rhs: impl Into<Expr>) -> Expr {
        self.binary(BinOp::ILike, rhs)
    }

    /// `self BETWEEN low AND high`.
    pub fn between(self, low: impl Into<Expr>, high: impl Into<Expr>) -> Expr {
        Expr::Between {
            expr: Box::new(self),
            low: Box::new(low.into()),
            high: Box::new(high.into()),
        }
    }

    /// `self IS NULL`.
    pub fn is_null(self) -> Expr {
        Expr::IsNull {
            expr: Box::new(self),
            negated: false,
        }
    }

    /// `self IS NOT NULL`.
    pub fn is_not_null(self) -> Expr {
        Expr::IsNull {
            expr: Box::new(self),
            negated: true,
        }
    }

    /// `self IN (values)`.
    pub fn in_(self, values: impl IntoIterator<Item = impl IntoValue>) -> Expr {
        Expr::In {
            expr: Box::new(self),
            values: values.into_iter().map(IntoValue::into_value).collect(),
        }
    }
}

impl std::ops::Not for Expr {
    type Output = Expr;

    /// `NOT self` (also usable as `!expr`).
    fn not(self) -> Expr {
        Expr::Unary {
            op: UnOp::Not,
            operand: Box::new(self),
        }
    }
}

impl std::ops::Neg for Expr {
    type Output = Expr;

    /// `-self` (also usable as `-expr`).
    fn neg(self) -> Expr {
        Expr::Unary {
            op: UnOp::Neg,
            operand: Box::new(self),
        }
    }
}

impl From<Value> for Expr {
    fn from(value: Value) -> Self {
        Expr::Value(value)
    }
}

impl From<&str> for Expr {
    fn from(value: &str) -> Self {
        Expr::Value(value.into_value())
    }
}

impl<T: IntoValue> From<Option<T>> for Expr {
    fn from(value: Option<T>) -> Self {
        Expr::Value(value.into_value())
    }
}

macro_rules! expr_from {
    ($($t:ty),* $(,)?) => {$(
        impl From<$t> for Expr {
            fn from(value: $t) -> Self {
                Expr::Value(value.into_value())
            }
        }
    )*};
}

expr_from!(
    bool, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, String, Vec<u8>, NonZeroU64, NonZeroU32,
);

fn precedence(expr: &Expr) -> u8 {
    match expr {
        Expr::Column(_) | Expr::Value(_) | Expr::Func { .. } | Expr::Raw { .. } => 100,
        Expr::Unary { op: UnOp::Neg, .. } => 8,
        Expr::Unary { op: UnOp::Not, .. } => 3,
        Expr::In { .. } | Expr::Between { .. } | Expr::IsNull { .. } => 4,
        Expr::Binary { op, .. } => op.precedence(),
    }
}

/// Accumulates rendered SQL text and its positional parameters.
struct Writer {
    dialect: Dialect,
    sql: String,
    params: Values,
}

impl Writer {
    fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            sql: String::new(),
            params: Values::default(),
        }
    }

    fn ident(&mut self, name: &str) {
        let quote = self.dialect.quote();
        self.sql.push(quote);
        for ch in name.chars() {
            if ch == quote {
                self.sql.push(quote);
            }
            self.sql.push(ch);
        }
        self.sql.push(quote);
    }

    fn ident_list(&mut self, names: &[String]) {
        for (i, name) in names.iter().enumerate() {
            if i != 0 {
                self.sql.push_str(", ");
            }
            self.ident(name);
        }
    }

    fn placeholder(&mut self, value: Value) {
        self.params.push(value);
        match self.dialect {
            Dialect::Postgres => {
                let _ = write!(self.sql, "${}", self.params.len());
            }
            Dialect::Sqlite | Dialect::Mysql => self.sql.push('?'),
        }
    }

    /// Expands a raw fragment: each `$n` binds `params[n-1]` at the current
    /// position, so reuse duplicates the value and the driver always sees
    /// positional parameters in textual order.
    fn raw(&mut self, sql: &str, params: &[Value]) {
        let mut chars = sql.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek().is_some_and(char::is_ascii_digit) {
                let mut n = 0usize;
                while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                    n = n * 10 + d as usize;
                    chars.next();
                }
                assert!(
                    n >= 1 && n <= params.len(),
                    "raw fragment references ${n} but {} parameters were provided",
                    params.len()
                );
                self.placeholder(params[n - 1].clone());
            } else {
                self.sql.push(ch);
            }
        }
    }

    /// Renders `expr`, wrapping it in parentheses if its precedence is below
    /// `min_prec`.
    fn operand(&mut self, expr: &Expr, min_prec: u8) {
        if precedence(expr) < min_prec {
            self.sql.push('(');
            self.expr(expr);
            self.sql.push(')');
        } else {
            self.expr(expr);
        }
    }

    fn in_list(&mut self, expr: &Expr, values: &[Value]) {
        self.operand(expr, 4);
        self.sql.push_str(" IN (");
        for (i, value) in values.iter().enumerate() {
            if i != 0 {
                self.sql.push_str(", ");
            }
            self.placeholder(value.clone());
        }
        self.sql.push(')');
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Column(name) => self.ident(name),
            Expr::Value(value) => self.placeholder(value.clone()),
            Expr::Raw { sql, params } => self.raw(sql, params),
            Expr::Func {
                name,
                distinct,
                args,
            } => {
                self.sql.push_str(name);
                self.sql.push('(');
                if *distinct {
                    self.sql.push_str("DISTINCT ");
                }
                for (i, arg) in args.iter().enumerate() {
                    if i != 0 {
                        self.sql.push_str(", ");
                    }
                    self.expr(arg);
                }
                self.sql.push(')');
            }
            Expr::Binary { lhs, op, rhs } => {
                // `= null` / `<> null` specialize to IS [NOT] NULL.
                if let Expr::Value(v) = rhs.as_ref()
                    && v.is_null()
                    && matches!(op, BinOp::Eq | BinOp::Ne)
                {
                    self.operand(lhs, 4);
                    self.sql
                        .push_str(if matches!(op, BinOp::Eq) { " IS NULL" } else { " IS NOT NULL" });
                    return;
                }
                // MySQL has no `||` string concat operator (it means OR there).
                if matches!(op, BinOp::Concat) && self.dialect == Dialect::Mysql {
                    self.sql.push_str("CONCAT(");
                    self.expr(lhs);
                    self.sql.push_str(", ");
                    self.expr(rhs);
                    self.sql.push(')');
                    return;
                }
                // ILIKE is Postgres-only; emulate it elsewhere.
                if matches!(op, BinOp::ILike) && self.dialect != Dialect::Postgres {
                    self.sql.push_str("LOWER(");
                    self.expr(lhs);
                    self.sql.push_str(") LIKE LOWER(");
                    self.expr(rhs);
                    self.sql.push(')');
                    return;
                }
                let p = op.precedence();
                self.operand(lhs, p);
                self.sql.push(' ');
                self.sql.push_str(op.sql());
                self.sql.push(' ');
                let right = if matches!(op, BinOp::Sub | BinOp::Div) { p + 1 } else { p };
                self.operand(rhs, right);
            }
            Expr::Unary { op, operand } => match op {
                UnOp::Not => {
                    self.sql.push_str("NOT ");
                    self.operand(operand, 3);
                }
                UnOp::Neg => {
                    self.sql.push('-');
                    self.operand(operand, 8);
                }
            },
            Expr::In { expr, values } => {
                let has_null = values.iter().any(Value::is_null);
                let present: Vec<Value> = values.iter().filter(|v| !v.is_null()).cloned().collect();
                match (present.is_empty(), has_null) {
                    (true, false) => self.sql.push_str("FALSE"),
                    (true, true) => {
                        self.operand(expr, 4);
                        self.sql.push_str(" IS NULL");
                    }
                    (false, false) => self.in_list(expr, &present),
                    (false, true) => {
                        self.sql.push('(');
                        self.in_list(expr, &present);
                        self.sql.push_str(" OR ");
                        self.operand(expr, 4);
                        self.sql.push_str(" IS NULL)");
                    }
                }
            }
            Expr::Between { expr, low, high } => {
                self.operand(expr, 4);
                self.sql.push_str(" BETWEEN ");
                self.operand(low, 4);
                self.sql.push_str(" AND ");
                self.operand(high, 4);
            }
            Expr::IsNull { expr, negated } => {
                self.operand(expr, 4);
                self.sql
                    .push_str(if *negated { " IS NOT NULL" } else { " IS NULL" });
            }
        }
    }

    fn where_clause(&mut self, filter: &Option<Expr>) {
        if let Some(expr) = filter {
            self.sql.push_str(" WHERE ");
            self.expr(expr);
        }
    }

    fn finish(self) -> (String, Values) {
        (self.sql, self.params)
    }
}

fn combine(slot: &mut Option<Expr>, expr: Expr) {
    *slot = Some(match slot.take() {
        Some(existing) => existing.and(expr),
        None => expr,
    });
}

/// Builds the `WHERE` predicate matching `key` across its key columns.
fn key_filter<T: Keyed>(key: &T::Key) -> Expr {
    let values = T::key_values(key);
    let mut filter = None;
    for (column, value) in T::KEY_COLUMNS.iter().zip(values.as_slice()) {
        combine(&mut filter, Expr::col(*column).eq(value.clone()));
    }
    filter.expect("a key has at least one column")
}

/// Sort direction for [`Select::order_by`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

/// What a [`Select`] returns.
enum Projection {
    All(Vec<String>),
    Items(Vec<(Expr, Option<String>)>),
}

/// A `SELECT` statement.
pub struct Select {
    table: &'static str,
    projection: Projection,
    filter: Option<Expr>,
    group: Vec<Expr>,
    having: Option<Expr>,
    order: Vec<(String, Direction)>,
    limit: Option<u64>,
    offset: Option<u64>,
}

impl Select {
    /// Adds a `WHERE` predicate, `AND`-combined with any previous one.
    pub fn filter(mut self, expr: Expr) -> Self {
        combine(&mut self.filter, expr);
        self
    }

    /// Restricts the selection to named columns (the default is every column).
    /// Note a projected row no longer round-trips through `Fields::parse`.
    pub fn columns(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.projection = Projection::All(columns.into_iter().map(Into::into).collect());
        self
    }

    /// Selects `count(*)`.
    pub fn count(mut self) -> Self {
        self.projection = Projection::Items(vec![(Expr::count_star(), None)]);
        self
    }

    /// Adds a projection expression with an optional alias. The first call
    /// replaces the default all-columns selection.
    pub fn select(mut self, expr: Expr, alias: Option<&str>) -> Self {
        let mut items = match self.projection {
            Projection::Items(items) => items,
            Projection::All(_) => Vec::new(),
        };
        items.push((expr, alias.map(str::to_string)));
        self.projection = Projection::Items(items);
        self
    }

    /// Appends a `GROUP BY` term.
    pub fn group_by(mut self, expr: Expr) -> Self {
        self.group.push(expr);
        self
    }

    /// Adds a `HAVING` predicate, `AND`-combined with any previous one.
    pub fn having(mut self, expr: Expr) -> Self {
        combine(&mut self.having, expr);
        self
    }

    /// Appends an `ORDER BY` term; terms apply in the order added.
    pub fn order_by(mut self, column: impl Into<String>, direction: Direction) -> Self {
        self.order.push((column.into(), direction));
        self
    }

    /// Sets the row `LIMIT`.
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Sets the row `OFFSET`.
    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Renders to SQL text and its positional parameters.
    pub fn render(&self, dialect: Dialect) -> (String, Values) {
        let mut w = Writer::new(dialect);
        match &self.projection {
            Projection::All(columns) => {
                w.sql.push_str("SELECT ");
                w.ident_list(columns);
            }
            Projection::Items(items) => {
                w.sql.push_str("SELECT ");
                for (i, (expr, alias)) in items.iter().enumerate() {
                    if i != 0 {
                        w.sql.push_str(", ");
                    }
                    w.expr(expr);
                    if let Some(alias) = alias {
                        w.sql.push_str(" AS ");
                        w.ident(alias);
                    }
                }
            }
        }
        w.sql.push_str(" FROM ");
        w.ident(self.table);
        w.where_clause(&self.filter);
        if !self.group.is_empty() {
            w.sql.push_str(" GROUP BY ");
            for (i, expr) in self.group.iter().enumerate() {
                if i != 0 {
                    w.sql.push_str(", ");
                }
                w.expr(expr);
            }
        }
        if let Some(having) = &self.having {
            w.sql.push_str(" HAVING ");
            w.expr(having);
        }
        if !self.order.is_empty() {
            w.sql.push_str(" ORDER BY ");
            for (i, (column, direction)) in self.order.iter().enumerate() {
                if i != 0 {
                    w.sql.push_str(", ");
                }
                w.ident(column);
                w.sql.push_str(match direction {
                    Direction::Asc => " ASC",
                    Direction::Desc => " DESC",
                });
            }
        }
        if let Some(limit) = self.limit {
            let _ = write!(w.sql, " LIMIT {limit}");
        }
        if let Some(offset) = self.offset {
            let _ = write!(w.sql, " OFFSET {offset}");
        }
        w.finish()
    }
}

/// An `INSERT` statement.
pub struct Insert {
    table: &'static str,
    columns: Vec<String>,
    values: Values,
    returning: Vec<String>,
}

impl Insert {
    /// Appends a `RETURNING` clause (for example the generated key). Supported by
    /// Postgres and SQLite 3.35+.
    pub fn returning(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.returning = columns.into_iter().map(Into::into).collect();
        self
    }

    /// Renders to SQL text and its positional parameters.
    pub fn render(&self, dialect: Dialect) -> (String, Values) {
        let mut w = Writer::new(dialect);
        w.sql.push_str("INSERT INTO ");
        w.ident(self.table);
        if self.columns.is_empty() {
            w.sql.push_str(match dialect {
                Dialect::Mysql => " () VALUES ()",
                _ => " DEFAULT VALUES",
            });
        } else {
            w.sql.push_str(" (");
            w.ident_list(&self.columns);
            w.sql.push_str(") VALUES (");
            for (i, value) in self.values.as_slice().iter().enumerate() {
                if i != 0 {
                    w.sql.push_str(", ");
                }
                w.placeholder(value.clone());
            }
            w.sql.push(')');
        }
        if !self.returning.is_empty() {
            assert!(
                dialect != Dialect::Mysql,
                "RETURNING is not supported by the MySQL dialect"
            );
            w.sql.push_str(" RETURNING ");
            w.ident_list(&self.returning);
        }
        w.finish()
    }
}

/// An `UPDATE` statement.
pub struct Update {
    table: &'static str,
    assignments: Vec<(String, Expr)>,
    filter: Option<Expr>,
}

impl Update {
    /// Starts an `UPDATE <table>` with no assignments or filter; use
    /// [`set`](Update::set) and [`filter`](Update::filter) to build it.
    pub fn table<T: Object>() -> Update {
        Update {
            table: T::TABLE_NAME,
            assignments: Vec::new(),
            filter: None,
        }
    }

    /// Appends a `column = value` assignment. `value` may be any [`Expr`], so a
    /// Rust value binds a literal while `Expr::col("n").add(1)` updates in place.
    pub fn set(mut self, column: impl Into<String>, value: impl Into<Expr>) -> Self {
        self.assignments.push((column.into(), value.into()));
        self
    }

    /// Adds a `WHERE` predicate, `AND`-combined with any previous one.
    pub fn filter(mut self, expr: Expr) -> Self {
        combine(&mut self.filter, expr);
        self
    }

    /// Renders to SQL text and its positional parameters.
    pub fn render(&self, dialect: Dialect) -> (String, Values) {
        let mut w = Writer::new(dialect);
        w.sql.push_str("UPDATE ");
        w.ident(self.table);
        w.sql.push_str(" SET ");
        for (i, (column, value)) in self.assignments.iter().enumerate() {
            if i != 0 {
                w.sql.push_str(", ");
            }
            w.ident(column);
            w.sql.push_str(" = ");
            w.expr(value);
        }
        w.where_clause(&self.filter);
        w.finish()
    }
}

/// A `DELETE` statement.
pub struct Delete {
    table: &'static str,
    filter: Option<Expr>,
}

impl Delete {
    /// Starts a `DELETE FROM <table>` with no filter (which would delete every
    /// row); narrow it with [`filter`](Delete::filter).
    pub fn table<T: Object>() -> Delete {
        Delete {
            table: T::TABLE_NAME,
            filter: None,
        }
    }

    /// Adds a `WHERE` predicate, `AND`-combined with any previous one.
    pub fn filter(mut self, expr: Expr) -> Self {
        combine(&mut self.filter, expr);
        self
    }

    /// Renders to SQL text and its positional parameters.
    pub fn render(&self, dialect: Dialect) -> (String, Values) {
        let mut w = Writer::new(dialect);
        w.sql.push_str("DELETE FROM ");
        w.ident(self.table);
        w.where_clause(&self.filter);
        w.finish()
    }
}

/// Any one of the SQL statements this crate builds.
pub enum Statement {
    Select(Select),
    Insert(Insert),
    Update(Update),
    Delete(Delete),
}

impl Statement {
    /// Renders to SQL text and its positional parameters.
    pub fn render(&self, dialect: Dialect) -> (String, Values) {
        match self {
            Statement::Select(s) => s.render(dialect),
            Statement::Insert(s) => s.render(dialect),
            Statement::Update(s) => s.render(dialect),
            Statement::Delete(s) => s.render(dialect),
        }
    }
}

impl From<Select> for Statement {
    fn from(s: Select) -> Self {
        Statement::Select(s)
    }
}

impl From<Insert> for Statement {
    fn from(s: Insert) -> Self {
        Statement::Insert(s)
    }
}

impl From<Update> for Statement {
    fn from(s: Update) -> Self {
        Statement::Update(s)
    }
}

impl From<Delete> for Statement {
    fn from(s: Delete) -> Self {
        Statement::Delete(s)
    }
}

/// Statement builders available on any [`Object`].
pub trait QueryObject: Object {
    /// `SELECT <columns> FROM <table>`, with no filter.
    fn select() -> Select {
        Select {
            table: Self::TABLE_NAME,
            projection: Projection::All(Self::columns().names().to_vec()),
            filter: None,
            group: Vec::new(),
            having: None,
            order: Vec::new(),
            limit: None,
            offset: None,
        }
    }

    /// `INSERT INTO <table> (...) VALUES (...)` for this row.
    ///
    /// [`GENERATED_COLUMNS`](Object::GENERATED_COLUMNS) whose value is null are
    /// omitted so the database can assign them (an unset auto-increment key).
    fn insert(&self) -> Insert {
        let columns = Self::columns();
        let row = self.values(columns);
        let mut names = Vec::new();
        let mut values = Values::default();
        for name in columns.names() {
            let value = columns
                .get_value(&row, name)
                .expect("row has every column");
            if value.is_null() && Self::GENERATED_COLUMNS.contains(&name.as_str()) {
                continue;
            }
            names.push(name.clone());
            values.push(value.clone());
        }
        Insert {
            table: Self::TABLE_NAME,
            columns: names,
            values,
            returning: Vec::new(),
        }
    }
}

impl<T: Object> QueryObject for T {}

/// Statement builders available on any [`Keyed`] object.
pub trait QueryKeyed: Keyed {
    /// `SELECT ... WHERE <key>` for the row with `key`.
    fn find(key: Self::Key) -> Select {
        <Self as QueryObject>::select().filter(key_filter::<Self>(&key))
    }

    /// `DELETE FROM <table> WHERE <key>`.
    fn delete(key: Self::Key) -> Delete {
        Delete {
            table: Self::TABLE_NAME,
            filter: Some(key_filter::<Self>(&key)),
        }
    }

    /// `UPDATE <table> SET <non-key columns> WHERE <key>`, or `None` if this row
    /// has no key yet (nothing to match on).
    fn update(&self) -> Option<Update> {
        let key = self.key()?;
        let columns = Self::columns();
        let row = self.values(columns);
        let mut assignments = Vec::new();
        for name in columns.names() {
            if Self::KEY_COLUMNS.contains(&name.as_str()) {
                continue;
            }
            let value = columns
                .get_value(&row, name)
                .expect("row has every column");
            assignments.push((name.clone(), Expr::Value(value.clone())));
        }
        Some(Update {
            table: Self::TABLE_NAME,
            assignments,
            filter: Some(key_filter::<Self>(&key)),
        })
    }
}

impl<T: Keyed> QueryKeyed for T {}
