use crate::span::Span;

#[derive(Debug, Clone, thiserror::Error)]
pub enum DslError {
    #[error("lex error at {line}:{col}: {message}")]
    LexError {
        span: Span,
        line: u32,
        col: u32,
        message: String,
    },

    #[error("parse error at {line}:{col}: {message}")]
    ParseError {
        span: Span,
        line: u32,
        col: u32,
        message: String,
    },

    #[error("type error at {line}:{col}: {message}")]
    TypeError {
        span: Span,
        line: u32,
        col: u32,
        message: String,
    },

    #[error("codegen error: {message}")]
    CodegenError {
        message: String,
    },
}

impl DslError {
    pub fn lex(span: Span, message: impl Into<String>) -> Self {
        Self::LexError { span, line: span.line, col: span.col, message: message.into() }
    }

    pub fn parse(span: Span, message: impl Into<String>) -> Self {
        Self::ParseError { span, line: span.line, col: span.col, message: message.into() }
    }

    pub fn type_err(span: Span, message: impl Into<String>) -> Self {
        Self::TypeError { span, line: span.line, col: span.col, message: message.into() }
    }

    pub fn codegen(message: impl Into<String>) -> Self {
        Self::CodegenError { message: message.into() }
    }
}
