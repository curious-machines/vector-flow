/// Byte-offset span in source code with line:col for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Byte offset of the start of the span.
    pub start: usize,
    /// Byte offset of the end of the span (exclusive).
    pub end: usize,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number.
    pub col: u32,
}

impl Span {
    pub fn new(start: usize, end: usize, line: u32, col: u32) -> Self {
        Self { start, end, line, col }
    }

    /// A dummy span for synthesized nodes.
    pub fn dummy() -> Self {
        Self { start: 0, end: 0, line: 0, col: 0 }
    }

    /// Merge two spans into one covering both.
    pub fn merge(self, other: Span) -> Span {
        let start = self.start.min(other.start);
        let end = self.end.max(other.end);
        // Use the earlier span's line/col
        if self.start <= other.start {
            Span { start, end, line: self.line, col: self.col }
        } else {
            Span { start, end, line: other.line, col: other.col }
        }
    }
}

/// An AST node wrapped with its source span.
#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
    /// Unique ID assigned during parsing, used by the type checker.
    pub id: u32,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span, id: u32) -> Self {
        Self { node, span, id }
    }
}
