use crate::ast::*;
use crate::error::DslError;
use crate::lexer::tokenize;
use crate::span::{Span, Spanned};
use crate::token::{Token, TokenKind};

/// Parse a standalone expression (for port expressions like `sin(time * 2.0) * 50.0`).
pub fn parse_expression(source: &str) -> Result<Spanned<Expr>, DslError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expr()?;
    parser.expect(TokenKind::Eof)?;
    Ok(expr)
}

/// Parse a full DSL program (`fn name(params) -> Type { body }`).
pub fn parse_program(source: &str) -> Result<FunctionDef, DslError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser::new(tokens);
    let func = parser.parse_function_def()?;
    parser.expect(TokenKind::Eof)?;
    Ok(func)
}

/// Parse a bare script (statements and optional tail expression, no `fn` wrapper).
/// Used for DSL node bodies where inputs/outputs are defined externally.
pub fn parse_script(source: &str) -> Result<Block, DslError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser::new(tokens);
    let mut statements = Vec::new();
    let mut tail_expr = None;

    while *parser.peek() != TokenKind::Eof {
        let stmt_or_expr = parser.parse_statement_or_tail()?;
        match stmt_or_expr {
            StmtOrTail::Stmt(s) => statements.push(*s),
            StmtOrTail::Tail(e) => {
                tail_expr = Some(e);
                break;
            }
        }
    }
    parser.expect(TokenKind::Eof)?;
    Ok(Block { statements, tail_expr })
}

enum StmtOrTail {
    Stmt(Box<Spanned<Statement>>),
    Tail(Spanned<Expr>),
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    next_id: u32,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0, next_id: 0 }
    }

    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: TokenKind) -> Result<Span, DslError> {
        let tok = &self.tokens[self.pos];
        if std::mem::discriminant(&tok.kind) == std::mem::discriminant(&expected) {
            let span = tok.span;
            self.advance();
            Ok(span)
        } else {
            Err(DslError::parse(
                tok.span,
                format!("expected {:?}, got {:?}", expected, tok.kind),
            ))
        }
    }

    fn expect_ident(&mut self) -> Result<(String, Span), DslError> {
        let tok = &self.tokens[self.pos];
        if let TokenKind::Ident(name) = &tok.kind {
            let name = name.clone();
            let span = tok.span;
            self.advance();
            Ok((name, span))
        } else {
            Err(DslError::parse(tok.span, format!("expected identifier, got {:?}", tok.kind)))
        }
    }

    // -----------------------------------------------------------------
    // Top-level: function definition
    // -----------------------------------------------------------------

    fn parse_function_def(&mut self) -> Result<FunctionDef, DslError> {
        let start_span = self.expect(TokenKind::Fn)?;
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::LParen)?;

        let mut params = Vec::new();
        while *self.peek() != TokenKind::RParen {
            if !params.is_empty() {
                self.expect(TokenKind::Comma)?;
                // Allow trailing comma
                if *self.peek() == TokenKind::RParen {
                    break;
                }
            }
            let (pname, _) = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let ptype = self.parse_type()?;
            params.push(Param { name: pname, param_type: ptype });
        }
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let return_type = self.parse_type()?;

        let body = self.parse_block()?;
        let span = start_span.merge(self.tokens[self.pos.saturating_sub(1)].span);

        Ok(FunctionDef { name, params, return_type, body, span })
    }

    fn parse_type(&mut self) -> Result<DslType, DslError> {
        let (name, span) = self.expect_ident()?;
        match name.as_str() {
            "Scalar" => Ok(DslType::Scalar),
            "Int" => Ok(DslType::Int),
            "Bool" => Ok(DslType::Bool),
            "Vec2" => Ok(DslType::Vec2),
            "Points" => Ok(DslType::Points),
            "Path" => Ok(DslType::Path),
            "Color" => Ok(DslType::Color),
            _ => Err(DslError::parse(span, format!("unknown type '{name}'"))),
        }
    }

    // -----------------------------------------------------------------
    // Block
    // -----------------------------------------------------------------

    fn parse_block(&mut self) -> Result<Block, DslError> {
        self.expect(TokenKind::LBrace)?;
        let mut statements = Vec::new();
        let mut tail_expr = None;

        while *self.peek() != TokenKind::RBrace {
            // Try to parse a statement
            let stmt_or_expr = self.parse_statement_or_tail()?;
            match stmt_or_expr {
                StmtOrTail::Stmt(s) => statements.push(*s),
                StmtOrTail::Tail(e) => {
                    tail_expr = Some(e);
                    break;
                }
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(Block { statements, tail_expr })
    }

    /// Distinguishes between statements (followed by ; or are blocks like if/for)
    /// and a trailing expression (the block's return value).
    fn parse_statement_or_tail(&mut self) -> Result<StmtOrTail, DslError> {
        match self.peek().clone() {
            TokenKind::Let => {
                let stmt = self.parse_let()?;
                Ok(StmtOrTail::Stmt(Box::new(stmt)))
            }
            TokenKind::For => {
                let stmt = self.parse_for()?;
                Ok(StmtOrTail::Stmt(Box::new(stmt)))
            }
            TokenKind::Return => {
                let stmt = self.parse_return()?;
                Ok(StmtOrTail::Stmt(Box::new(stmt)))
            }
            _ => {
                // Could be: if statement, assignment, or tail expression
                let start = self.peek_span();
                let expr = self.parse_expr()?;

                // Check if this is an assignment
                if *self.peek() == TokenKind::Assign {
                    self.advance();
                    let target = expr_to_assign_target(&expr)?;
                    let value = self.parse_expr()?;
                    self.expect(TokenKind::Semicolon)?;
                    let span = start.merge(self.tokens[self.pos.saturating_sub(1)].span);
                    let id = self.alloc_id();
                    Ok(StmtOrTail::Stmt(Box::new(Spanned::new(
                        Statement::Assign { target, value },
                        span,
                        id,
                    ))))
                } else if *self.peek() == TokenKind::Semicolon {
                    // Expression statement
                    self.advance();

                    // Check if this is an if expression turned into a statement
                    if let Expr::If { condition, then_branch, else_branch } = expr.node.clone() {
                        let id = self.alloc_id();
                        Ok(StmtOrTail::Stmt(Box::new(Spanned::new(
                            Statement::If {
                                condition: *condition,
                                then_branch: *then_branch,
                                else_branch: else_branch.map(|b| *b),
                            },
                            expr.span,
                            id,
                        ))))
                    } else {
                        let id = self.alloc_id();
                        Ok(StmtOrTail::Stmt(Box::new(Spanned::new(
                            Statement::Expr(expr),
                            start.merge(self.tokens[self.pos.saturating_sub(1)].span),
                            id,
                        ))))
                    }
                } else if *self.peek() == TokenKind::RBrace || *self.peek() == TokenKind::Eof {
                    // Tail expression (no semicolon before closing brace or end of script)
                    Ok(StmtOrTail::Tail(expr))
                } else {
                    // If expressions and if statements don't need semicolons
                    if let Expr::If { condition, then_branch, else_branch } = expr.node.clone() {
                        let id = self.alloc_id();
                        Ok(StmtOrTail::Stmt(Box::new(Spanned::new(
                            Statement::If {
                                condition: *condition,
                                then_branch: *then_branch,
                                else_branch: else_branch.map(|b| *b),
                            },
                            expr.span,
                            id,
                        ))))
                    } else {
                        Err(DslError::parse(
                            self.peek_span(),
                            format!("expected ';' or '}}', got {:?}", self.peek()),
                        ))
                    }
                }
            }
        }
    }

    fn parse_let(&mut self) -> Result<Spanned<Statement>, DslError> {
        let start = self.expect(TokenKind::Let)?;
        let (name, _) = self.expect_ident()?;
        let type_annotation = if *self.peek() == TokenKind::Colon {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(TokenKind::Assign)?;
        let value = self.parse_expr()?;
        self.expect(TokenKind::Semicolon)?;
        let span = start.merge(self.tokens[self.pos.saturating_sub(1)].span);
        let id = self.alloc_id();
        Ok(Spanned::new(Statement::Let { name, type_annotation, value }, span, id))
    }

    fn parse_for(&mut self) -> Result<Spanned<Statement>, DslError> {
        let start = self.expect(TokenKind::For)?;
        let (var, _) = self.expect_ident()?;
        self.expect(TokenKind::In)?;
        let range_start = self.parse_expr()?;
        self.expect(TokenKind::DotDot)?;
        let range_end = self.parse_expr()?;
        let body = self.parse_block()?;
        let span = start.merge(self.tokens[self.pos.saturating_sub(1)].span);
        let id = self.alloc_id();
        Ok(Spanned::new(
            Statement::For { var, start: range_start, end: range_end, body },
            span,
            id,
        ))
    }

    fn parse_return(&mut self) -> Result<Spanned<Statement>, DslError> {
        let start = self.expect(TokenKind::Return)?;
        let value = self.parse_expr()?;
        self.expect(TokenKind::Semicolon)?;
        let span = start.merge(self.tokens[self.pos.saturating_sub(1)].span);
        let id = self.alloc_id();
        Ok(Spanned::new(Statement::Return(value), span, id))
    }

    // -----------------------------------------------------------------
    // Expression parsing: precedence climbing
    // -----------------------------------------------------------------

    fn parse_expr(&mut self) -> Result<Spanned<Expr>, DslError> {
        self.parse_or()
    }

    // Precedence 1: ||
    fn parse_or(&mut self) -> Result<Spanned<Expr>, DslError> {
        let mut left = self.parse_and()?;
        while *self.peek() == TokenKind::Or {
            self.advance();
            let right = self.parse_and()?;
            let span = left.span.merge(right.span);
            let id = self.alloc_id();
            left = Spanned::new(
                Expr::BinaryOp { op: BinOp::Or, left: Box::new(left), right: Box::new(right) },
                span, id,
            );
        }
        Ok(left)
    }

    // Precedence 2: &&
    fn parse_and(&mut self) -> Result<Spanned<Expr>, DslError> {
        let mut left = self.parse_equality()?;
        while *self.peek() == TokenKind::And {
            self.advance();
            let right = self.parse_equality()?;
            let span = left.span.merge(right.span);
            let id = self.alloc_id();
            left = Spanned::new(
                Expr::BinaryOp { op: BinOp::And, left: Box::new(left), right: Box::new(right) },
                span, id,
            );
        }
        Ok(left)
    }

    // Precedence 3: == !=
    fn parse_equality(&mut self) -> Result<Spanned<Expr>, DslError> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                TokenKind::Eq => BinOp::Eq,
                TokenKind::Ne => BinOp::Ne,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
            let span = left.span.merge(right.span);
            let id = self.alloc_id();
            left = Spanned::new(
                Expr::BinaryOp { op, left: Box::new(left), right: Box::new(right) },
                span, id,
            );
        }
        Ok(left)
    }

    // Precedence 4: < <= > >=
    fn parse_comparison(&mut self) -> Result<Spanned<Expr>, DslError> {
        let mut left = self.parse_addition()?;
        loop {
            let op = match self.peek() {
                TokenKind::Lt => BinOp::Lt,
                TokenKind::Le => BinOp::Le,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::Ge => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let right = self.parse_addition()?;
            let span = left.span.merge(right.span);
            let id = self.alloc_id();
            left = Spanned::new(
                Expr::BinaryOp { op, left: Box::new(left), right: Box::new(right) },
                span, id,
            );
        }
        Ok(left)
    }

    // Precedence 5: + -
    fn parse_addition(&mut self) -> Result<Spanned<Expr>, DslError> {
        let mut left = self.parse_multiplication()?;
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplication()?;
            let span = left.span.merge(right.span);
            let id = self.alloc_id();
            left = Spanned::new(
                Expr::BinaryOp { op, left: Box::new(left), right: Box::new(right) },
                span, id,
            );
        }
        Ok(left)
    }

    // Precedence 6: * / %
    fn parse_multiplication(&mut self) -> Result<Spanned<Expr>, DslError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            let span = left.span.merge(right.span);
            let id = self.alloc_id();
            left = Spanned::new(
                Expr::BinaryOp { op, left: Box::new(left), right: Box::new(right) },
                span, id,
            );
        }
        Ok(left)
    }

    // Precedence 7: Unary - !
    fn parse_unary(&mut self) -> Result<Spanned<Expr>, DslError> {
        match self.peek().clone() {
            TokenKind::Minus => {
                let start = self.advance().span;
                let operand = self.parse_unary()?;
                let span = start.merge(operand.span);
                let id = self.alloc_id();
                Ok(Spanned::new(
                    Expr::UnaryOp { op: UnaryOp::Neg, operand: Box::new(operand) },
                    span, id,
                ))
            }
            TokenKind::Not => {
                let start = self.advance().span;
                let operand = self.parse_unary()?;
                let span = start.merge(operand.span);
                let id = self.alloc_id();
                Ok(Spanned::new(
                    Expr::UnaryOp { op: UnaryOp::Not, operand: Box::new(operand) },
                    span, id,
                ))
            }
            _ => self.parse_cast(),
        }
    }

    // Precedence 8: `as` cast
    fn parse_cast(&mut self) -> Result<Spanned<Expr>, DslError> {
        let mut expr = self.parse_postfix()?;
        while *self.peek() == TokenKind::As {
            self.advance();
            let target = self.parse_type()?;
            let span = expr.span.merge(self.tokens[self.pos.saturating_sub(1)].span);
            let id = self.alloc_id();
            expr = Spanned::new(
                Expr::Cast { expr: Box::new(expr), target },
                span, id,
            );
        }
        Ok(expr)
    }

    // Precedence 9: Postfix — call, index, field access
    fn parse_postfix(&mut self) -> Result<Spanned<Expr>, DslError> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek().clone() {
                TokenKind::LParen => {
                    // Function call — only if the expr is a variable
                    if let Expr::Variable(name) = &expr.node {
                        let name = name.clone();
                        self.advance(); // consume '('
                        let mut args = Vec::new();
                        while *self.peek() != TokenKind::RParen {
                            if !args.is_empty() {
                                self.expect(TokenKind::Comma)?;
                            }
                            args.push(self.parse_expr()?);
                        }
                        let end = self.expect(TokenKind::RParen)?;
                        let span = expr.span.merge(end);
                        let id = self.alloc_id();
                        expr = Spanned::new(Expr::Call { name, args }, span, id);
                    } else {
                        break;
                    }
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    let end = self.expect(TokenKind::RBracket)?;
                    let span = expr.span.merge(end);
                    let id = self.alloc_id();
                    expr = Spanned::new(
                        Expr::Index { collection: Box::new(expr), index: Box::new(index) },
                        span, id,
                    );
                }
                TokenKind::Dot => {
                    self.advance();
                    let (field, field_span) = self.expect_ident()?;
                    let span = expr.span.merge(field_span);
                    let id = self.alloc_id();
                    expr = Spanned::new(
                        Expr::FieldAccess { object: Box::new(expr), field },
                        span, id,
                    );
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    // Primary: literals, variables, parenthesized, if expression
    fn parse_primary(&mut self) -> Result<Spanned<Expr>, DslError> {
        let tok = self.tokens[self.pos].clone();
        match tok.kind {
            TokenKind::IntLit(v) => {
                self.advance();
                let id = self.alloc_id();
                Ok(Spanned::new(Expr::Literal(Literal::Int(v)), tok.span, id))
            }
            TokenKind::FloatLit(v) => {
                self.advance();
                let id = self.alloc_id();
                Ok(Spanned::new(Expr::Literal(Literal::Float(v)), tok.span, id))
            }
            TokenKind::BoolLit(v) => {
                self.advance();
                let id = self.alloc_id();
                Ok(Spanned::new(Expr::Literal(Literal::Bool(v)), tok.span, id))
            }
            TokenKind::Ident(ref name) => {
                let name = name.clone();
                self.advance();
                let id = self.alloc_id();
                Ok(Spanned::new(Expr::Variable(name), tok.span, id))
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                Ok(expr)
            }
            TokenKind::If => {
                self.parse_if_expr()
            }
            _ => {
                Err(DslError::parse(tok.span, format!("expected expression, got {:?}", tok.kind)))
            }
        }
    }

    fn parse_if_expr(&mut self) -> Result<Spanned<Expr>, DslError> {
        let start = self.expect(TokenKind::If)?;
        let condition = self.parse_expr()?;
        let then_block = self.parse_block()?;
        let else_block = if *self.peek() == TokenKind::Else {
            self.advance();
            if *self.peek() == TokenKind::If {
                // else if → wrap in a block with single if expression as tail
                let if_expr = self.parse_if_expr()?;
                Some(Block {
                    statements: Vec::new(),
                    tail_expr: Some(if_expr),
                })
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };
        let end_span = self.tokens[self.pos.saturating_sub(1)].span;
        let span = start.merge(end_span);
        let id = self.alloc_id();
        Ok(Spanned::new(
            Expr::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_block),
                else_branch: else_block.map(Box::new),
            },
            span,
            id,
        ))
    }
}

/// Convert an expression (parsed before we knew it was an assignment target) into an AssignTarget.
fn expr_to_assign_target(expr: &Spanned<Expr>) -> Result<AssignTarget, DslError> {
    match &expr.node {
        Expr::Variable(name) => Ok(AssignTarget::Variable(name.clone())),
        Expr::FieldAccess { object, field } => {
            // object[index].field pattern
            if let Expr::Index { collection, index } = &object.node {
                if let Expr::Variable(name) = &collection.node {
                    return Ok(AssignTarget::IndexField {
                        collection: name.clone(),
                        index: Box::new((**index).clone()),
                        field: field.clone(),
                    });
                }
            }
            Err(DslError::parse(expr.span, "invalid assignment target"))
        }
        _ => Err(DslError::parse(expr.span, "invalid assignment target")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_addition() {
        let expr = parse_expression("2.0 + 3.0").unwrap();
        match &expr.node {
            Expr::BinaryOp { op: BinOp::Add, .. } => {}
            other => panic!("expected BinaryOp::Add, got {:?}", other),
        }
    }

    #[test]
    fn operator_precedence() {
        // 1.0 + 2.0 * 3.0 should be 1.0 + (2.0 * 3.0)
        let expr = parse_expression("1.0 + 2.0 * 3.0").unwrap();
        match &expr.node {
            Expr::BinaryOp { op: BinOp::Add, right, .. } => {
                assert!(matches!(&right.node, Expr::BinaryOp { op: BinOp::Mul, .. }));
            }
            other => panic!("expected Add(_, Mul(_, _)), got {:?}", other),
        }
    }

    #[test]
    fn parse_function_call() {
        let expr = parse_expression("sin(time * 2.0)").unwrap();
        match &expr.node {
            Expr::Call { name, args } => {
                assert_eq!(name, "sin");
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected Call, got {:?}", other),
        }
    }

    #[test]
    fn parse_unary_neg() {
        let expr = parse_expression("-x").unwrap();
        assert!(matches!(&expr.node, Expr::UnaryOp { op: UnaryOp::Neg, .. }));
    }

    #[test]
    fn parse_cast() {
        let expr = parse_expression("x as Scalar").unwrap();
        match &expr.node {
            Expr::Cast { target: DslType::Scalar, .. } => {}
            other => panic!("expected Cast to Scalar, got {:?}", other),
        }
    }

    #[test]
    fn parse_if_expression() {
        let expr = parse_expression("if x > 0.0 { 1.0 } else { 0.0 }").unwrap();
        assert!(matches!(&expr.node, Expr::If { .. }));
    }

    #[test]
    fn parse_program_simple() {
        let func = parse_program("fn add(x: Scalar, y: Scalar) -> Scalar { x + y }").unwrap();
        assert_eq!(func.name, "add");
        assert_eq!(func.params.len(), 2);
        assert_eq!(func.return_type, DslType::Scalar);
    }

    #[test]
    fn parse_let_and_for() {
        let func = parse_program(
            "fn sum(n: Int) -> Scalar { let total: Scalar = 0.0; for i in 0..n { total = total + 1.0; } total }"
        ).unwrap();
        assert_eq!(func.name, "sum");
        assert_eq!(func.body.statements.len(), 2); // let, for
        assert!(func.body.tail_expr.is_some()); // total
    }

    #[test]
    fn parse_nested_calls() {
        let expr = parse_expression("sin(cos(x))").unwrap();
        match &expr.node {
            Expr::Call { name, args } => {
                assert_eq!(name, "sin");
                assert!(matches!(&args[0].node, Expr::Call { name, .. } if name == "cos"));
            }
            other => panic!("expected nested Call, got {:?}", other),
        }
    }

    #[test]
    fn parse_comparison_chain() {
        // x > 0.0 && x < 10.0
        let expr = parse_expression("x > 0.0 && x < 10.0").unwrap();
        assert!(matches!(&expr.node, Expr::BinaryOp { op: BinOp::And, .. }));
    }
}
