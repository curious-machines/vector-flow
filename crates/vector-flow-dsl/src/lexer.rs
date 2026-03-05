use crate::error::DslError;
use crate::span::Span;
use crate::token::{keyword_lookup, Token, TokenKind};

/// Tokenize the entire source string into a Vec of tokens (ending with Eof).
pub fn tokenize(source: &str) -> Result<Vec<Token>, DslError> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let tok = lexer.next_token()?;
        let is_eof = tok.kind == TokenKind::Eof;
        tokens.push(tok);
        if is_eof {
            break;
        }
    }
    Ok(tokens)
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            src: source.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<u8> {
        self.src.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> u8 {
        let ch = self.src[self.pos];
        self.pos += 1;
        if ch == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    fn span_from(&self, start: usize, start_line: u32, start_col: u32) -> Span {
        Span::new(start, self.pos, start_line, start_col)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip whitespace
            while let Some(ch) = self.peek() {
                if ch == b' ' || ch == b'\t' || ch == b'\r' || ch == b'\n' {
                    self.advance();
                } else {
                    break;
                }
            }
            // Skip line comments
            if self.peek() == Some(b'/') && self.peek2() == Some(b'/') {
                while let Some(ch) = self.peek() {
                    if ch == b'\n' {
                        break;
                    }
                    self.advance();
                }
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, DslError> {
        self.skip_whitespace_and_comments();

        let start = self.pos;
        let start_line = self.line;
        let start_col = self.col;

        let ch = match self.peek() {
            Some(ch) => ch,
            None => {
                return Ok(Token::new(
                    TokenKind::Eof,
                    self.span_from(start, start_line, start_col),
                ));
            }
        };

        // Single character tokens and two-character tokens
        match ch {
            b'+' => { self.advance(); Ok(Token::new(TokenKind::Plus, self.span_from(start, start_line, start_col))) }
            b'*' => { self.advance(); Ok(Token::new(TokenKind::Star, self.span_from(start, start_line, start_col))) }
            b'%' => { self.advance(); Ok(Token::new(TokenKind::Percent, self.span_from(start, start_line, start_col))) }
            b'(' => { self.advance(); Ok(Token::new(TokenKind::LParen, self.span_from(start, start_line, start_col))) }
            b')' => { self.advance(); Ok(Token::new(TokenKind::RParen, self.span_from(start, start_line, start_col))) }
            b'{' => { self.advance(); Ok(Token::new(TokenKind::LBrace, self.span_from(start, start_line, start_col))) }
            b'}' => { self.advance(); Ok(Token::new(TokenKind::RBrace, self.span_from(start, start_line, start_col))) }
            b'[' => { self.advance(); Ok(Token::new(TokenKind::LBracket, self.span_from(start, start_line, start_col))) }
            b']' => { self.advance(); Ok(Token::new(TokenKind::RBracket, self.span_from(start, start_line, start_col))) }
            b',' => { self.advance(); Ok(Token::new(TokenKind::Comma, self.span_from(start, start_line, start_col))) }
            b':' => { self.advance(); Ok(Token::new(TokenKind::Colon, self.span_from(start, start_line, start_col))) }
            b';' => { self.advance(); Ok(Token::new(TokenKind::Semicolon, self.span_from(start, start_line, start_col))) }

            // Two-char operators
            b'-' => {
                self.advance();
                if self.peek() == Some(b'>') {
                    self.advance();
                    Ok(Token::new(TokenKind::Arrow, self.span_from(start, start_line, start_col)))
                } else {
                    Ok(Token::new(TokenKind::Minus, self.span_from(start, start_line, start_col)))
                }
            }
            b'=' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    Ok(Token::new(TokenKind::Eq, self.span_from(start, start_line, start_col)))
                } else {
                    Ok(Token::new(TokenKind::Assign, self.span_from(start, start_line, start_col)))
                }
            }
            b'!' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    Ok(Token::new(TokenKind::Ne, self.span_from(start, start_line, start_col)))
                } else {
                    Ok(Token::new(TokenKind::Not, self.span_from(start, start_line, start_col)))
                }
            }
            b'<' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    Ok(Token::new(TokenKind::Le, self.span_from(start, start_line, start_col)))
                } else {
                    Ok(Token::new(TokenKind::Lt, self.span_from(start, start_line, start_col)))
                }
            }
            b'>' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    Ok(Token::new(TokenKind::Ge, self.span_from(start, start_line, start_col)))
                } else {
                    Ok(Token::new(TokenKind::Gt, self.span_from(start, start_line, start_col)))
                }
            }
            b'&' => {
                self.advance();
                if self.peek() == Some(b'&') {
                    self.advance();
                    Ok(Token::new(TokenKind::And, self.span_from(start, start_line, start_col)))
                } else {
                    Err(DslError::lex(
                        self.span_from(start, start_line, start_col),
                        "expected '&&', got single '&'",
                    ))
                }
            }
            b'|' => {
                self.advance();
                if self.peek() == Some(b'|') {
                    self.advance();
                    Ok(Token::new(TokenKind::Or, self.span_from(start, start_line, start_col)))
                } else {
                    Err(DslError::lex(
                        self.span_from(start, start_line, start_col),
                        "expected '||', got single '|'",
                    ))
                }
            }
            b'/' => {
                // Not a comment (already handled), so it's division
                self.advance();
                Ok(Token::new(TokenKind::Slash, self.span_from(start, start_line, start_col)))
            }
            b'.' => {
                self.advance();
                if self.peek() == Some(b'.') {
                    self.advance();
                    Ok(Token::new(TokenKind::DotDot, self.span_from(start, start_line, start_col)))
                } else {
                    Ok(Token::new(TokenKind::Dot, self.span_from(start, start_line, start_col)))
                }
            }

            // Numbers
            b'0'..=b'9' => self.lex_number(start, start_line, start_col),

            // Identifiers / keywords
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_ident(start, start_line, start_col),

            _ => {
                self.advance();
                Err(DslError::lex(
                    self.span_from(start, start_line, start_col),
                    format!("unexpected character '{}'", ch as char),
                ))
            }
        }
    }

    fn lex_number(&mut self, start: usize, start_line: u32, start_col: u32) -> Result<Token, DslError> {
        // Consume integer part
        while let Some(b'0'..=b'9') = self.peek() {
            self.advance();
        }

        // Check for decimal point (must be followed by a digit, not ..)
        let is_float = self.peek() == Some(b'.')
            && self.peek2().is_some_and(|c| c.is_ascii_digit());

        if is_float {
            self.advance(); // consume '.'
            while let Some(b'0'..=b'9') = self.peek() {
                self.advance();
            }
            // Scientific notation
            if matches!(self.peek(), Some(b'e' | b'E')) {
                self.advance();
                if matches!(self.peek(), Some(b'+' | b'-')) {
                    self.advance();
                }
                while let Some(b'0'..=b'9') = self.peek() {
                    self.advance();
                }
            }
            let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
            let value: f64 = text.parse().map_err(|_| {
                DslError::lex(self.span_from(start, start_line, start_col), format!("invalid float literal '{text}'"))
            })?;
            Ok(Token::new(TokenKind::FloatLit(value), self.span_from(start, start_line, start_col)))
        } else {
            let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
            let value: i64 = text.parse().map_err(|_| {
                DslError::lex(self.span_from(start, start_line, start_col), format!("invalid integer literal '{text}'"))
            })?;
            Ok(Token::new(TokenKind::IntLit(value), self.span_from(start, start_line, start_col)))
        }
    }

    fn lex_ident(&mut self, start: usize, start_line: u32, start_col: u32) -> Result<Token, DslError> {
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                self.advance();
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        let kind = keyword_lookup(text).unwrap_or_else(|| TokenKind::Ident(text.to_string()));
        Ok(Token::new(kind, self.span_from(start, start_line, start_col)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_expression() {
        let tokens = tokenize("2.0 + 3.0").unwrap();
        assert_eq!(tokens.len(), 4); // FloatLit Plus FloatLit Eof
        assert!(matches!(tokens[0].kind, TokenKind::FloatLit(f) if (f - 2.0).abs() < 1e-10));
        assert_eq!(tokens[1].kind, TokenKind::Plus);
        assert!(matches!(tokens[2].kind, TokenKind::FloatLit(f) if (f - 3.0).abs() < 1e-10));
        assert_eq!(tokens[3].kind, TokenKind::Eof);
    }

    #[test]
    fn keywords_and_operators() {
        let tokens = tokenize("fn let for in if else return as true false").unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(kinds[0], &TokenKind::Fn);
        assert_eq!(kinds[1], &TokenKind::Let);
        assert_eq!(kinds[2], &TokenKind::For);
        assert_eq!(kinds[3], &TokenKind::In);
        assert_eq!(kinds[4], &TokenKind::If);
        assert_eq!(kinds[5], &TokenKind::Else);
        assert_eq!(kinds[6], &TokenKind::Return);
        assert_eq!(kinds[7], &TokenKind::As);
        assert_eq!(kinds[8], &TokenKind::BoolLit(true));
        assert_eq!(kinds[9], &TokenKind::BoolLit(false));
    }

    #[test]
    fn two_char_operators() {
        let tokens = tokenize("== != <= >= && || -> ..").unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(kinds[0], &TokenKind::Eq);
        assert_eq!(kinds[1], &TokenKind::Ne);
        assert_eq!(kinds[2], &TokenKind::Le);
        assert_eq!(kinds[3], &TokenKind::Ge);
        assert_eq!(kinds[4], &TokenKind::And);
        assert_eq!(kinds[5], &TokenKind::Or);
        assert_eq!(kinds[6], &TokenKind::Arrow);
        assert_eq!(kinds[7], &TokenKind::DotDot);
    }

    #[test]
    fn line_comments() {
        let tokens = tokenize("1 + // this is a comment\n2").unwrap();
        assert_eq!(tokens.len(), 4);
        assert!(matches!(tokens[0].kind, TokenKind::IntLit(1)));
        assert_eq!(tokens[1].kind, TokenKind::Plus);
        assert!(matches!(tokens[2].kind, TokenKind::IntLit(2)));
    }

    #[test]
    fn integer_vs_float() {
        let tokens = tokenize("42 3.14 5..10").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::IntLit(42)));
        assert!(matches!(tokens[1].kind, TokenKind::FloatLit(f) if (f - 3.14).abs() < 1e-10));
        // 5..10 should be Int(5) DotDot Int(10)
        assert!(matches!(tokens[2].kind, TokenKind::IntLit(5)));
        assert_eq!(tokens[3].kind, TokenKind::DotDot);
        assert!(matches!(tokens[4].kind, TokenKind::IntLit(10)));
    }

    #[test]
    fn function_tokens() {
        let tokens = tokenize("fn foo(x: Scalar) -> Scalar { x * 2.0 }").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Fn);
        assert_eq!(tokens[1].kind, TokenKind::Ident("foo".into()));
        assert_eq!(tokens[2].kind, TokenKind::LParen);
    }

    #[test]
    fn span_tracking() {
        let tokens = tokenize("ab cd").unwrap();
        assert_eq!(tokens[0].span.col, 1);
        assert_eq!(tokens[1].span.col, 4);
    }

    #[test]
    fn error_single_ampersand() {
        let result = tokenize("a & b");
        assert!(result.is_err());
    }
}
