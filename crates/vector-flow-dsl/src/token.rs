use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),

    // Identifier
    Ident(String),

    // Keywords
    Fn,
    Let,
    For,
    In,
    If,
    Else,
    Return,
    As,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,      // ==
    Ne,      // !=
    Lt,      // <
    Le,      // <=
    Gt,      // >
    Ge,      // >=
    And,     // &&
    Or,      // ||
    Not,     // !
    Assign,  // =

    // Delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    // Punctuation
    Comma,
    Colon,
    Semicolon,
    Arrow,   // ->
    DotDot,  // ..
    Dot,

    // End of file
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Look up whether an identifier is a keyword.
pub fn keyword_lookup(ident: &str) -> Option<TokenKind> {
    match ident {
        "fn" => Some(TokenKind::Fn),
        "let" => Some(TokenKind::Let),
        "for" => Some(TokenKind::For),
        "in" => Some(TokenKind::In),
        "if" => Some(TokenKind::If),
        "else" => Some(TokenKind::Else),
        "return" => Some(TokenKind::Return),
        "as" => Some(TokenKind::As),
        "true" => Some(TokenKind::BoolLit(true)),
        "false" => Some(TokenKind::BoolLit(false)),
        _ => None,
    }
}
