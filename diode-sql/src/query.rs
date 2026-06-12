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
    /// A column reference, optionally qualified by a table name or alias.
    Column { table: Option<String>, name: String },
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
    /// `CASE WHEN ... THEN ... [ELSE ...] END`.
    Case {
        whens: Vec<(Expr, Expr)>,
        else_: Option<Box<Expr>>,
    },
    /// `CAST(expr AS ty)`.
    Cast { expr: Box<Expr>, ty: String },
    /// A scalar subquery `(SELECT ...)`.
    Subquery(Box<Select>),
    /// `[NOT] EXISTS (SELECT ...)`.
    Exists { negated: bool, select: Box<Select> },
    /// `expr [NOT] IN (SELECT ...)`.
    InSubquery {
        expr: Box<Expr>,
        negated: bool,
        select: Box<Select>,
    },
    /// `lhs IS [NOT] DISTINCT FROM rhs` (null-safe comparison).
    DistinctFrom {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        negated: bool,
    },
}

impl Expr {
    /// A column reference.
    pub fn col(name: impl Into<String>) -> Self {
        Expr::Column {
            table: None,
            name: name.into(),
        }
    }

    /// A column reference qualified by a table name or alias (`table.name`).
    pub fn col_at(table: impl Into<String>, name: impl Into<String>) -> Self {
        Expr::Column {
            table: Some(table.into()),
            name: name.into(),
        }
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

    /// Starts a `CASE` expression; finish it with [`Case::end`].
    pub fn case() -> Case {
        Case {
            whens: Vec::new(),
            else_: None,
        }
    }

    /// A scalar subquery `(SELECT ...)`.
    pub fn subquery(select: Select) -> Self {
        Expr::Subquery(Box::new(select))
    }

    /// `EXISTS (SELECT ...)`.
    pub fn exists(select: Select) -> Self {
        Expr::Exists {
            negated: false,
            select: Box::new(select),
        }
    }

    /// `NOT EXISTS (SELECT ...)`.
    pub fn not_exists(select: Select) -> Self {
        Expr::Exists {
            negated: true,
            select: Box::new(select),
        }
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

    /// `self IN (SELECT ...)`.
    pub fn in_subquery(self, select: Select) -> Expr {
        Expr::InSubquery {
            expr: Box::new(self),
            negated: false,
            select: Box::new(select),
        }
    }

    /// `self NOT IN (SELECT ...)`.
    pub fn not_in_subquery(self, select: Select) -> Expr {
        Expr::InSubquery {
            expr: Box::new(self),
            negated: true,
            select: Box::new(select),
        }
    }

    /// `CAST(self AS ty)`.
    pub fn cast(self, ty: impl Into<String>) -> Expr {
        Expr::Cast {
            expr: Box::new(self),
            ty: ty.into(),
        }
    }

    /// `self IS DISTINCT FROM rhs` (null-safe inequality).
    pub fn is_distinct_from(self, rhs: impl Into<Expr>) -> Expr {
        Expr::DistinctFrom {
            lhs: Box::new(self),
            rhs: Box::new(rhs.into()),
            negated: false,
        }
    }

    /// `self IS NOT DISTINCT FROM rhs` (null-safe equality).
    pub fn is_not_distinct_from(self, rhs: impl Into<Expr>) -> Expr {
        Expr::DistinctFrom {
            lhs: Box::new(self),
            rhs: Box::new(rhs.into()),
            negated: true,
        }
    }
}

/// Builder for a `CASE WHEN ... END` expression (see [`Expr::case`]).
pub struct Case {
    whens: Vec<(Expr, Expr)>,
    else_: Option<Box<Expr>>,
}

impl Case {
    /// Adds a `WHEN cond THEN result` branch.
    pub fn when(mut self, cond: Expr, result: impl Into<Expr>) -> Self {
        self.whens.push((cond, result.into()));
        self
    }

    /// Sets the `ELSE` result.
    pub fn otherwise(mut self, result: impl Into<Expr>) -> Self {
        self.else_ = Some(Box::new(result.into()));
        self
    }

    /// Finishes the `CASE` expression.
    pub fn end(self) -> Expr {
        Expr::Case {
            whens: self.whens,
            else_: self.else_,
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
        Expr::Column { .. }
        | Expr::Value(_)
        | Expr::Func { .. }
        | Expr::Raw { .. }
        | Expr::Case { .. }
        | Expr::Cast { .. }
        | Expr::Subquery(_) => 100,
        Expr::Unary { op: UnOp::Neg, .. } => 8,
        Expr::Unary { op: UnOp::Not, .. } => 3,
        Expr::In { .. }
        | Expr::Between { .. }
        | Expr::IsNull { .. }
        | Expr::Exists { .. }
        | Expr::InSubquery { .. }
        | Expr::DistinctFrom { .. } => 4,
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

    fn source(&mut self, source: &Source) {
        match source {
            Source::Table { name, alias } => {
                self.ident(name);
                if let Some(alias) = alias {
                    self.sql.push_str(" AS ");
                    self.ident(alias);
                }
            }
            Source::Subquery { select, alias } => {
                self.sql.push('(');
                select.write(self);
                self.sql.push_str(") AS ");
                self.ident(alias);
            }
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
            Expr::Column { table, name } => {
                if let Some(table) = table {
                    self.ident(table);
                    self.sql.push('.');
                }
                self.ident(name);
            }
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
            Expr::Case { whens, else_ } => {
                self.sql.push_str("CASE");
                for (cond, result) in whens {
                    self.sql.push_str(" WHEN ");
                    self.expr(cond);
                    self.sql.push_str(" THEN ");
                    self.expr(result);
                }
                if let Some(else_) = else_ {
                    self.sql.push_str(" ELSE ");
                    self.expr(else_);
                }
                self.sql.push_str(" END");
            }
            Expr::Cast { expr, ty } => {
                self.sql.push_str("CAST(");
                self.expr(expr);
                self.sql.push_str(" AS ");
                self.sql.push_str(ty);
                self.sql.push(')');
            }
            Expr::Subquery(select) => {
                self.sql.push('(');
                select.write(self);
                self.sql.push(')');
            }
            Expr::Exists { negated, select } => {
                self.sql
                    .push_str(if *negated { "NOT EXISTS (" } else { "EXISTS (" });
                select.write(self);
                self.sql.push(')');
            }
            Expr::InSubquery {
                expr,
                negated,
                select,
            } => {
                self.operand(expr, 4);
                self.sql
                    .push_str(if *negated { " NOT IN (" } else { " IN (" });
                select.write(self);
                self.sql.push(')');
            }
            Expr::DistinctFrom { lhs, rhs, negated } => {
                if self.dialect == Dialect::Mysql {
                    // MySQL spells null-safe equality `a <=> b`; `IS DISTINCT
                    // FROM` (negated == false) is its negation.
                    if *negated {
                        self.operand(lhs, 4);
                        self.sql.push_str(" <=> ");
                        self.operand(rhs, 4);
                    } else {
                        self.sql.push_str("NOT (");
                        self.operand(lhs, 4);
                        self.sql.push_str(" <=> ");
                        self.operand(rhs, 4);
                        self.sql.push(')');
                    }
                } else {
                    self.operand(lhs, 4);
                    self.sql.push_str(if *negated {
                        " IS NOT DISTINCT FROM "
                    } else {
                        " IS DISTINCT FROM "
                    });
                    self.operand(rhs, 4);
                }
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

/// Null ordering for [`Select::order_by_nulls`] (ignored by the MySQL dialect).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nulls {
    First,
    Last,
}

/// A table or subquery in a `FROM` or `JOIN` clause.
pub enum Source {
    Table {
        name: String,
        alias: Option<String>,
    },
    Subquery {
        select: Box<Select>,
        alias: String,
    },
}

impl Source {
    /// A table by name.
    pub fn table(name: impl Into<String>) -> Source {
        Source::Table {
            name: name.into(),
            alias: None,
        }
    }

    /// A subquery (derived table) with a mandatory alias.
    pub fn subquery(select: Select, alias: impl Into<String>) -> Source {
        Source::Subquery {
            select: Box::new(select),
            alias: alias.into(),
        }
    }

    /// Sets the alias.
    pub fn alias(mut self, alias: impl Into<String>) -> Source {
        match &mut self {
            Source::Table { alias: a, .. } => *a = Some(alias.into()),
            Source::Subquery { alias: a, .. } => *a = alias.into(),
        }
        self
    }
}

impl From<&str> for Source {
    fn from(name: &str) -> Self {
        Source::table(name)
    }
}

impl From<String> for Source {
    fn from(name: String) -> Self {
        Source::table(name)
    }
}

#[derive(Clone, Copy)]
enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

impl JoinKind {
    fn sql(self) -> &'static str {
        match self {
            JoinKind::Inner => "INNER JOIN",
            JoinKind::Left => "LEFT JOIN",
            JoinKind::Right => "RIGHT JOIN",
            JoinKind::Full => "FULL JOIN",
            JoinKind::Cross => "CROSS JOIN",
        }
    }
}

struct Join {
    kind: JoinKind,
    source: Source,
    on: Option<Expr>,
}

struct OrderTerm {
    expr: Expr,
    direction: Direction,
    nulls: Option<Nulls>,
}

#[derive(Clone, Copy)]
enum Lock {
    Update,
    Share,
}

/// What a [`Select`] returns.
enum Projection {
    All(Vec<String>),
    Items(Vec<(Expr, Option<String>)>),
}

/// A `SELECT` statement.
pub struct Select {
    distinct: bool,
    projection: Projection,
    from: Source,
    joins: Vec<Join>,
    filter: Option<Expr>,
    group: Vec<Expr>,
    having: Option<Expr>,
    order: Vec<OrderTerm>,
    limit: Option<u64>,
    offset: Option<u64>,
    lock: Option<Lock>,
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

    /// Selects only distinct rows (`SELECT DISTINCT`).
    pub fn distinct(mut self) -> Self {
        self.distinct = true;
        self
    }

    /// Replaces the `FROM` source (for an alias or a derived table).
    pub fn from(mut self, source: impl Into<Source>) -> Self {
        self.from = source.into();
        self
    }

    fn add_join(mut self, kind: JoinKind, source: impl Into<Source>, on: Option<Expr>) -> Self {
        self.joins.push(Join {
            kind,
            source: source.into(),
            on,
        });
        self
    }

    /// `INNER JOIN source ON on`.
    pub fn inner_join(self, source: impl Into<Source>, on: Expr) -> Self {
        self.add_join(JoinKind::Inner, source, Some(on))
    }

    /// `LEFT JOIN source ON on`.
    pub fn left_join(self, source: impl Into<Source>, on: Expr) -> Self {
        self.add_join(JoinKind::Left, source, Some(on))
    }

    /// `RIGHT JOIN source ON on`.
    pub fn right_join(self, source: impl Into<Source>, on: Expr) -> Self {
        self.add_join(JoinKind::Right, source, Some(on))
    }

    /// `FULL JOIN source ON on` (unsupported by MySQL and SQLite).
    pub fn full_join(self, source: impl Into<Source>, on: Expr) -> Self {
        self.add_join(JoinKind::Full, source, Some(on))
    }

    /// `CROSS JOIN source`.
    pub fn cross_join(self, source: impl Into<Source>) -> Self {
        self.add_join(JoinKind::Cross, source, None)
    }

    /// Appends an `ORDER BY` term; terms apply in the order added.
    pub fn order_by(mut self, expr: Expr, direction: Direction) -> Self {
        self.order.push(OrderTerm {
            expr,
            direction,
            nulls: None,
        });
        self
    }

    /// Appends an `ORDER BY` term with explicit null ordering (the `NULLS`
    /// clause is omitted for the MySQL dialect, which lacks it).
    pub fn order_by_nulls(mut self, expr: Expr, direction: Direction, nulls: Nulls) -> Self {
        self.order.push(OrderTerm {
            expr,
            direction,
            nulls: Some(nulls),
        });
        self
    }

    /// `FOR UPDATE` row locking (omitted for SQLite, which lacks it).
    pub fn for_update(mut self) -> Self {
        self.lock = Some(Lock::Update);
        self
    }

    /// `FOR SHARE` row locking (omitted for SQLite, which lacks it).
    pub fn for_share(mut self) -> Self {
        self.lock = Some(Lock::Share);
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
        self.write(&mut w);
        w.finish()
    }

    /// Appends this `SELECT` to `w`; used directly when embedding as a subquery.
    fn write(&self, w: &mut Writer) {
        w.sql.push_str("SELECT ");
        if self.distinct {
            w.sql.push_str("DISTINCT ");
        }
        match &self.projection {
            Projection::All(columns) => w.ident_list(columns),
            Projection::Items(items) => {
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
        w.source(&self.from);
        for join in &self.joins {
            w.sql.push(' ');
            w.sql.push_str(join.kind.sql());
            w.sql.push(' ');
            w.source(&join.source);
            if let Some(on) = &join.on {
                w.sql.push_str(" ON ");
                w.expr(on);
            }
        }
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
            for (i, term) in self.order.iter().enumerate() {
                if i != 0 {
                    w.sql.push_str(", ");
                }
                w.expr(&term.expr);
                w.sql.push_str(match term.direction {
                    Direction::Asc => " ASC",
                    Direction::Desc => " DESC",
                });
                // NULLS FIRST/LAST is unsupported by MySQL.
                if let Some(nulls) = term.nulls
                    && w.dialect != Dialect::Mysql
                {
                    w.sql.push_str(match nulls {
                        Nulls::First => " NULLS FIRST",
                        Nulls::Last => " NULLS LAST",
                    });
                }
            }
        }
        if let Some(limit) = self.limit {
            let _ = write!(w.sql, " LIMIT {limit}");
        }
        if let Some(offset) = self.offset {
            let _ = write!(w.sql, " OFFSET {offset}");
        }
        // SQLite has no row-level locking clause.
        if let Some(lock) = self.lock
            && w.dialect != Dialect::Sqlite
        {
            w.sql.push_str(match lock {
                Lock::Update => " FOR UPDATE",
                Lock::Share => " FOR SHARE",
            });
        }
    }
}

/// Conflict handling for an [`Insert`] (upsert).
enum OnConflict {
    DoNothing { target: Vec<String> },
    DoUpdate { target: Vec<String> },
}

/// An `INSERT` statement.
pub struct Insert {
    table: &'static str,
    columns: Vec<String>,
    values: Values,
    on_conflict: Option<OnConflict>,
    returning: Vec<String>,
}

impl Insert {
    /// `ON CONFLICT (target) DO NOTHING`: skip the row if it already exists.
    /// An empty `target` matches any conflict (Postgres / SQLite). The MySQL
    /// dialect renders a no-op `ON DUPLICATE KEY UPDATE col = col` and cannot
    /// express a target (any unique key conflicts).
    pub fn on_conflict_do_nothing(
        mut self,
        target: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.on_conflict = Some(OnConflict::DoNothing {
            target: target.into_iter().map(Into::into).collect(),
        });
        self
    }

    /// `ON CONFLICT (target) DO UPDATE SET ...`: insert the row or, when it
    /// already exists, update every inserted non-target column to the new value.
    /// Degrades to `DO NOTHING` when there is nothing left to set. The MySQL
    /// dialect renders `ON DUPLICATE KEY UPDATE col = VALUES(col)` and cannot
    /// express a target (any unique key conflicts).
    ///
    /// # Panics
    ///
    /// Rendering panics on the Postgres / SQLite dialects if `target` is empty
    /// (`DO UPDATE` requires a conflict target there).
    pub fn on_conflict_do_update(
        mut self,
        target: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.on_conflict = Some(OnConflict::DoUpdate {
            target: target.into_iter().map(Into::into).collect(),
        });
        self
    }

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
        match &self.on_conflict {
            None => {}
            Some(OnConflict::DoNothing { target }) => match dialect {
                Dialect::Mysql => mysql_noop_update(&mut w, &self.columns),
                _ => do_nothing_clause(&mut w, target),
            },
            Some(OnConflict::DoUpdate { target }) => {
                // Update every inserted column that is not part of the target.
                let updates: Vec<&String> =
                    self.columns.iter().filter(|c| !target.contains(c)).collect();
                match dialect {
                    Dialect::Mysql => {
                        if updates.is_empty() {
                            mysql_noop_update(&mut w, &self.columns);
                        } else {
                            w.sql.push_str(" ON DUPLICATE KEY UPDATE ");
                            for (i, column) in updates.iter().enumerate() {
                                if i != 0 {
                                    w.sql.push_str(", ");
                                }
                                w.ident(column);
                                w.sql.push_str(" = VALUES(");
                                w.ident(column);
                                w.sql.push(')');
                            }
                        }
                    }
                    _ => {
                        if updates.is_empty() {
                            do_nothing_clause(&mut w, target);
                        } else {
                            assert!(
                                !target.is_empty(),
                                "ON CONFLICT DO UPDATE requires target columns"
                            );
                            w.sql.push_str(" ON CONFLICT (");
                            w.ident_list(target);
                            w.sql.push_str(") DO UPDATE SET ");
                            for (i, column) in updates.iter().enumerate() {
                                if i != 0 {
                                    w.sql.push_str(", ");
                                }
                                w.ident(column);
                                w.sql.push_str(" = excluded.");
                                w.ident(column);
                            }
                        }
                    }
                }
            }
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

/// `ON CONFLICT [(target)] DO NOTHING` (Postgres / SQLite).
fn do_nothing_clause(w: &mut Writer, target: &[String]) {
    w.sql.push_str(" ON CONFLICT");
    if !target.is_empty() {
        w.sql.push_str(" (");
        w.ident_list(target);
        w.sql.push(')');
    }
    w.sql.push_str(" DO NOTHING");
}

/// MySQL's spelling of "do nothing": assign some column to itself.
fn mysql_noop_update(w: &mut Writer, columns: &[String]) {
    let column = columns
        .first()
        .expect("an upsert needs at least one inserted column");
    w.sql.push_str(" ON DUPLICATE KEY UPDATE ");
    w.ident(column);
    w.sql.push_str(" = ");
    w.ident(column);
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
    Select(Box<Select>),
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
        Statement::Select(Box::new(s))
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
            distinct: false,
            projection: Projection::All(Self::columns().names().to_vec()),
            from: Source::table(Self::TABLE_NAME),
            joins: Vec::new(),
            filter: None,
            group: Vec::new(),
            having: None,
            order: Vec::new(),
            limit: None,
            offset: None,
            lock: None,
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
            on_conflict: None,
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

    /// `INSERT ... ON CONFLICT (<key columns>) DO UPDATE`: insert the row or
    /// update its non-key columns if it already exists. The idiomatic write for
    /// a natural (non-generated) key, where "new or existing" is only knowable
    /// by the database.
    fn upsert(&self) -> Insert {
        <Self as QueryObject>::insert(self).on_conflict_do_update(Self::KEY_COLUMNS.iter().copied())
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
