use crate::span::{Span, Spanned};

/// DSL type system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DslType {
    Scalar,
    Int,
    Bool,
    Vec2,
    Points,
    Path,
    Color,
    /// Used internally for type checking before resolution.
    Unknown,
}

impl std::fmt::Display for DslType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DslType::Scalar => write!(f, "Scalar"),
            DslType::Int => write!(f, "Int"),
            DslType::Bool => write!(f, "Bool"),
            DslType::Vec2 => write!(f, "Vec2"),
            DslType::Points => write!(f, "Points"),
            DslType::Path => write!(f, "Path"),
            DslType::Color => write!(f, "Color"),
            DslType::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// Literal values.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// Expressions.
#[derive(Debug, Clone)]
pub enum Expr {
    Literal(Literal),
    Variable(String),
    BinaryOp {
        op: BinOp,
        left: Box<Spanned<Expr>>,
        right: Box<Spanned<Expr>>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Spanned<Expr>>,
    },
    Call {
        name: String,
        args: Vec<Spanned<Expr>>,
    },
    Index {
        collection: Box<Spanned<Expr>>,
        index: Box<Spanned<Expr>>,
    },
    FieldAccess {
        object: Box<Spanned<Expr>>,
        field: String,
    },
    Cast {
        expr: Box<Spanned<Expr>>,
        target: DslType,
    },
    If {
        condition: Box<Spanned<Expr>>,
        then_branch: Box<Block>,
        else_branch: Option<Box<Block>>,
    },
}

/// Assignment targets.
#[derive(Debug, Clone)]
pub enum AssignTarget {
    Variable(String),
    IndexField {
        collection: String,
        index: Box<Spanned<Expr>>,
        field: String,
    },
}

/// Statements.
#[derive(Debug, Clone)]
pub enum Statement {
    Let {
        name: String,
        type_annotation: Option<DslType>,
        value: Spanned<Expr>,
    },
    Assign {
        target: AssignTarget,
        value: Spanned<Expr>,
    },
    For {
        var: String,
        start: Spanned<Expr>,
        end: Spanned<Expr>,
        body: Block,
    },
    If {
        condition: Spanned<Expr>,
        then_branch: Block,
        else_branch: Option<Block>,
    },
    Return(Spanned<Expr>),
    Expr(Spanned<Expr>),
}

/// A block of statements with an optional trailing expression (the block's value).
#[derive(Debug, Clone)]
pub struct Block {
    pub statements: Vec<Spanned<Statement>>,
    pub tail_expr: Option<Spanned<Expr>>,
}

/// A parameter in a function definition.
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub param_type: DslType,
}

/// A top-level function definition.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: DslType,
    pub body: Block,
    pub span: Span,
}
