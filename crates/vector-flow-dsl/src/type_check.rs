use std::collections::HashMap;

use crate::ast::*;
use crate::error::DslError;
use crate::span::Spanned;

/// Built-in function signature: (param_types, return_type).
struct BuiltinSig {
    params: &'static [DslType],
    ret: DslType,
}

const S1: &[DslType] = &[DslType::Scalar];
const S2: &[DslType] = &[DslType::Scalar, DslType::Scalar];
const S3: &[DslType] = &[DslType::Scalar, DslType::Scalar, DslType::Scalar];
const I1: &[DslType] = &[DslType::Int];
const I2: &[DslType] = &[DslType::Int, DslType::Int];

fn builtin_functions() -> HashMap<&'static str, BuiltinSig> {
    let mut m = HashMap::new();

    // 1-arg scalar → scalar
    for name in ["sin", "cos", "tan", "asin", "acos", "atan", "sqrt", "abs",
                  "floor", "ceil", "round", "fract", "exp", "ln", "sign"] {
        m.insert(name, BuiltinSig { params: S1, ret: DslType::Scalar });
    }

    // 2-arg scalar → scalar
    for name in ["min", "max", "pow", "atan2", "step", "fmod"] {
        m.insert(name, BuiltinSig { params: S2, ret: DslType::Scalar });
    }

    // 3-arg scalar → scalar
    for name in ["lerp", "clamp", "smoothstep"] {
        m.insert(name, BuiltinSig { params: S3, ret: DslType::Scalar });
    }

    // Procedural
    m.insert("rand", BuiltinSig { params: I1, ret: DslType::Scalar });
    m.insert("noise", BuiltinSig { params: S2, ret: DslType::Scalar });

    // Int ops
    m.insert("iabs", BuiltinSig { params: I1, ret: DslType::Int });
    m.insert("imin", BuiltinSig { params: I2, ret: DslType::Int });
    m.insert("imax", BuiltinSig { params: I2, ret: DslType::Int });

    m
}

/// Type checker: walks the AST, resolves types, and validates operations.
/// Returns a type table mapping expression IDs → DslType.
pub struct TypeChecker {
    builtins: HashMap<&'static str, BuiltinSig>,
    /// Stack of scopes (innermost last). Each scope maps variable names to types.
    scopes: Vec<HashMap<String, DslType>>,
    /// Resolved types indexed by expression ID.
    types: Vec<DslType>,
    errors: Vec<DslError>,
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            builtins: builtin_functions(),
            scopes: vec![HashMap::new()],
            types: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Type-check a standalone expression. Returns the type table on success.
    pub fn check_expression(&mut self, expr: &Spanned<Expr>) -> Result<Vec<DslType>, DslError> {
        // Add time builtins to global scope
        self.define_time_builtins();
        self.resolve_expr(expr)?;
        if let Some(err) = self.errors.pop() {
            return Err(err);
        }
        Ok(std::mem::take(&mut self.types))
    }

    /// Type-check a full function definition. Returns the type table on success.
    pub fn check_function(&mut self, func: &FunctionDef) -> Result<Vec<DslType>, DslError> {
        self.define_time_builtins();

        // Add function params to scope
        for (i, param) in func.params.iter().enumerate() {
            self.scopes.last_mut().unwrap().insert(
                param.name.clone(),
                param.param_type,
            );
            // Also expose as slot index (for codegen reference)
            let _ = i; // index used in codegen, not type check
        }

        self.resolve_block(&func.body, func.return_type)?;
        if let Some(err) = self.errors.pop() {
            return Err(err);
        }
        Ok(std::mem::take(&mut self.types))
    }

    /// Type-check a bare script block with externally-defined input/output variables.
    pub fn check_script(
        &mut self,
        block: &Block,
        inputs: &[(String, DslType)],
        outputs: &[(String, DslType)],
    ) -> Result<Vec<DslType>, DslError> {
        self.define_time_builtins();

        // Add input variables to scope.
        for (name, ty) in inputs {
            self.scopes.last_mut().unwrap().insert(name.clone(), *ty);
        }

        // Add output variables to scope (pre-declared, assignable).
        for (name, ty) in outputs {
            self.scopes.last_mut().unwrap().insert(name.clone(), *ty);
        }

        self.resolve_block(block, DslType::Scalar)?;
        if let Some(err) = self.errors.pop() {
            return Err(err);
        }
        Ok(std::mem::take(&mut self.types))
    }

    fn define_time_builtins(&mut self) {
        let scope = self.scopes.last_mut().unwrap();
        scope.insert("time".to_string(), DslType::Scalar);
        scope.insert("frame".to_string(), DslType::Int);
        scope.insert("fps".to_string(), DslType::Scalar);
        scope.insert("PI".to_string(), DslType::Scalar);
        scope.insert("TAU".to_string(), DslType::Scalar);
        scope.insert("E".to_string(), DslType::Scalar);
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn lookup_var(&self, name: &str) -> Option<DslType> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(*ty);
            }
        }
        None
    }

    fn define_var(&mut self, name: String, ty: DslType) {
        self.scopes.last_mut().unwrap().insert(name, ty);
    }

    /// Ensure the type table has enough entries for this expression ID.
    fn set_type(&mut self, id: u32, ty: DslType) {
        let idx = id as usize;
        if idx >= self.types.len() {
            self.types.resize(idx + 1, DslType::Unknown);
        }
        self.types[idx] = ty;
    }

    fn resolve_expr(&mut self, expr: &Spanned<Expr>) -> Result<DslType, DslError> {
        let ty = match &expr.node {
            Expr::Literal(lit) => match lit {
                Literal::Int(_) => DslType::Int,
                Literal::Float(_) => DslType::Scalar,
                Literal::Bool(_) => DslType::Bool,
            },

            Expr::Variable(name) => {
                self.lookup_var(name).ok_or_else(|| {
                    DslError::type_err(expr.span, format!("undefined variable '{name}'"))
                })?
            }

            Expr::BinaryOp { op, left, right } => {
                let lt = self.resolve_expr(left)?;
                let rt = self.resolve_expr(right)?;
                self.check_binary_op(*op, lt, rt, expr.span)?
            }

            Expr::UnaryOp { op, operand } => {
                let ot = self.resolve_expr(operand)?;
                match op {
                    UnaryOp::Neg => {
                        if ot != DslType::Scalar && ot != DslType::Int {
                            return Err(DslError::type_err(expr.span, format!("cannot negate {ot}")));
                        }
                        ot
                    }
                    UnaryOp::Not => {
                        if ot != DslType::Bool {
                            return Err(DslError::type_err(expr.span, format!("cannot apply ! to {ot}")));
                        }
                        DslType::Bool
                    }
                }
            }

            Expr::Call { name, args } => {
                let arg_types: Vec<DslType> = args
                    .iter()
                    .map(|a| self.resolve_expr(a))
                    .collect::<Result<_, _>>()?;

                if let Some(sig) = self.builtins.get(name.as_str()) {
                    if sig.params.len() != arg_types.len() {
                        return Err(DslError::type_err(
                            expr.span,
                            format!("{}() takes {} args, got {}", name, sig.params.len(), arg_types.len()),
                        ));
                    }
                    for (i, (&expected, &actual)) in sig.params.iter().zip(arg_types.iter()).enumerate() {
                        let actual_promoted = self.maybe_promote(actual, expected);
                        if actual_promoted != expected {
                            return Err(DslError::type_err(
                                args[i].span,
                                format!("{}() arg {}: expected {expected}, got {actual}", name, i + 1),
                            ));
                        }
                    }
                    sig.ret
                } else {
                    return Err(DslError::type_err(expr.span, format!("unknown function '{name}'")));
                }
            }

            Expr::Index { collection, index } => {
                let _ct = self.resolve_expr(collection)?;
                let it = self.resolve_expr(index)?;
                if it != DslType::Int {
                    return Err(DslError::type_err(expr.span, format!("index must be Int, got {it}")));
                }
                // For now, indexing is deferred — return Scalar as placeholder
                DslType::Scalar
            }

            Expr::FieldAccess { object, field } => {
                let _ot = self.resolve_expr(object)?;
                // Field access types depend on object type. For now, assume Scalar.
                let _ = field;
                DslType::Scalar
            }

            Expr::Cast { expr: inner, target } => {
                let from = self.resolve_expr(inner)?;
                match (from, *target) {
                    (DslType::Int, DslType::Scalar) | (DslType::Scalar, DslType::Int) => *target,
                    (a, b) if a == b => a,
                    _ => {
                        return Err(DslError::type_err(
                            expr.span,
                            format!("cannot cast {from} to {target}"),
                        ));
                    }
                }
            }

            Expr::If { condition, then_branch, else_branch } => {
                let ct = self.resolve_expr(condition)?;
                if ct != DslType::Bool {
                    return Err(DslError::type_err(expr.span, format!("if condition must be Bool, got {ct}")));
                }
                let then_ty = self.resolve_block(then_branch, DslType::Unknown)?;
                if let Some(else_block) = else_branch {
                    let else_ty = self.resolve_block(else_block, DslType::Unknown)?;
                    let unified = self.unify(then_ty, else_ty);
                    if unified == DslType::Unknown && then_ty != DslType::Unknown {
                        return Err(DslError::type_err(
                            expr.span,
                            format!("if branches have incompatible types: {then_ty} vs {else_ty}"),
                        ));
                    }
                    unified
                } else {
                    then_ty
                }
            }
        };

        self.set_type(expr.id, ty);
        Ok(ty)
    }

    fn resolve_block(&mut self, block: &Block, _expected: DslType) -> Result<DslType, DslError> {
        self.push_scope();
        for stmt in &block.statements {
            self.resolve_statement(stmt)?;
        }
        let ty = if let Some(ref tail) = block.tail_expr {
            self.resolve_expr(tail)?
        } else {
            DslType::Scalar // default: blocks without tail return Scalar (0.0)
        };
        self.pop_scope();
        Ok(ty)
    }

    fn resolve_statement(&mut self, stmt: &Spanned<Statement>) -> Result<(), DslError> {
        match &stmt.node {
            Statement::Let { name, type_annotation, value } => {
                let vt = self.resolve_expr(value)?;
                let ty = if let Some(ann) = type_annotation {
                    let promoted = self.maybe_promote(vt, *ann);
                    if promoted != *ann {
                        return Err(DslError::type_err(
                            stmt.span,
                            format!("cannot assign {vt} to {ann}"),
                        ));
                    }
                    *ann
                } else {
                    vt
                };
                self.define_var(name.clone(), ty);
            }

            Statement::Assign { target, value } => {
                let vt = self.resolve_expr(value)?;
                match target {
                    AssignTarget::Variable(name) => {
                        let existing = self.lookup_var(name).ok_or_else(|| {
                            DslError::type_err(stmt.span, format!("undefined variable '{name}'"))
                        })?;
                        let promoted = self.maybe_promote(vt, existing);
                        if promoted != existing {
                            return Err(DslError::type_err(
                                stmt.span,
                                format!("cannot assign {vt} to {existing}"),
                            ));
                        }
                    }
                    AssignTarget::IndexField { collection, index, field: _ } => {
                        let _ct = self.lookup_var(collection).ok_or_else(|| {
                            DslError::type_err(stmt.span, format!("undefined variable '{collection}'"))
                        })?;
                        let it = self.resolve_expr(index)?;
                        if it != DslType::Int {
                            return Err(DslError::type_err(stmt.span, "index must be Int"));
                        }
                    }
                }
            }

            Statement::For { var, start, end, body } => {
                let st = self.resolve_expr(start)?;
                let et = self.resolve_expr(end)?;
                if st != DslType::Int {
                    return Err(DslError::type_err(stmt.span, format!("for range start must be Int, got {st}")));
                }
                if et != DslType::Int {
                    return Err(DslError::type_err(stmt.span, format!("for range end must be Int, got {et}")));
                }
                self.push_scope();
                self.define_var(var.clone(), DslType::Int);
                for inner_stmt in &body.statements {
                    self.resolve_statement(inner_stmt)?;
                }
                if let Some(ref tail) = body.tail_expr {
                    self.resolve_expr(tail)?;
                }
                self.pop_scope();
            }

            Statement::If { condition, then_branch, else_branch } => {
                let ct = self.resolve_expr(condition)?;
                if ct != DslType::Bool {
                    return Err(DslError::type_err(stmt.span, format!("if condition must be Bool, got {ct}")));
                }
                self.resolve_block(then_branch, DslType::Unknown)?;
                if let Some(else_block) = else_branch {
                    self.resolve_block(else_block, DslType::Unknown)?;
                }
            }

            Statement::Return(value) => {
                self.resolve_expr(value)?;
            }

            Statement::Expr(e) => {
                self.resolve_expr(e)?;
            }
        }
        Ok(())
    }

    /// Int→Scalar promotion.
    fn maybe_promote(&self, from: DslType, to: DslType) -> DslType {
        if from == to {
            return from;
        }
        if from == DslType::Int && to == DslType::Scalar {
            return DslType::Scalar;
        }
        from
    }

    /// Unify two types (for if/else branches).
    fn unify(&self, a: DslType, b: DslType) -> DslType {
        if a == b {
            return a;
        }
        // Int + Scalar → Scalar
        if (a == DslType::Int && b == DslType::Scalar) || (a == DslType::Scalar && b == DslType::Int) {
            return DslType::Scalar;
        }
        if a == DslType::Unknown { return b; }
        if b == DslType::Unknown { return a; }
        DslType::Unknown
    }

    fn check_binary_op(&self, op: BinOp, lt: DslType, rt: DslType, span: crate::span::Span) -> Result<DslType, DslError> {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let unified = self.unify(lt, rt);
                if unified == DslType::Scalar || unified == DslType::Int {
                    Ok(unified)
                } else {
                    Err(DslError::type_err(span, format!("cannot apply arithmetic to {lt} and {rt}")))
                }
            }
            BinOp::Eq | BinOp::Ne => {
                let unified = self.unify(lt, rt);
                if unified != DslType::Unknown {
                    Ok(DslType::Bool)
                } else {
                    Err(DslError::type_err(span, format!("cannot compare {lt} and {rt}")))
                }
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let unified = self.unify(lt, rt);
                if unified == DslType::Scalar || unified == DslType::Int {
                    Ok(DslType::Bool)
                } else {
                    Err(DslError::type_err(span, format!("cannot compare {lt} and {rt}")))
                }
            }
            BinOp::And | BinOp::Or => {
                if lt == DslType::Bool && rt == DslType::Bool {
                    Ok(DslType::Bool)
                } else {
                    Err(DslError::type_err(span, format!("logical operators require Bool, got {lt} and {rt}")))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_expression;

    #[test]
    fn check_simple_arithmetic() {
        let expr = parse_expression("2.0 + 3.0").unwrap();
        let mut tc = TypeChecker::new();
        let types = tc.check_expression(&expr).unwrap();
        // Root expression should be Scalar
        assert_eq!(types[expr.id as usize], DslType::Scalar);
    }

    #[test]
    fn check_time_builtin() {
        let expr = parse_expression("time * 2.0").unwrap();
        let mut tc = TypeChecker::new();
        let types = tc.check_expression(&expr).unwrap();
        assert_eq!(types[expr.id as usize], DslType::Scalar);
    }

    #[test]
    fn check_function_call() {
        let expr = parse_expression("sin(1.0)").unwrap();
        let mut tc = TypeChecker::new();
        let types = tc.check_expression(&expr).unwrap();
        assert_eq!(types[expr.id as usize], DslType::Scalar);
    }

    #[test]
    fn check_int_to_scalar_promotion() {
        let expr = parse_expression("1 + 2.0").unwrap();
        let mut tc = TypeChecker::new();
        let types = tc.check_expression(&expr).unwrap();
        assert_eq!(types[expr.id as usize], DslType::Scalar);
    }

    #[test]
    fn check_comparison_returns_bool() {
        let expr = parse_expression("1.0 > 0.0").unwrap();
        let mut tc = TypeChecker::new();
        let types = tc.check_expression(&expr).unwrap();
        assert_eq!(types[expr.id as usize], DslType::Bool);
    }

    #[test]
    fn check_undefined_variable_error() {
        let expr = parse_expression("undefined_var + 1.0").unwrap();
        let mut tc = TypeChecker::new();
        assert!(tc.check_expression(&expr).is_err());
    }

    #[test]
    fn check_wrong_arg_count() {
        let expr = parse_expression("sin(1.0, 2.0)").unwrap();
        let mut tc = TypeChecker::new();
        assert!(tc.check_expression(&expr).is_err());
    }

    #[test]
    fn check_if_expression() {
        let expr = parse_expression("if true { 1.0 } else { 2.0 }").unwrap();
        let mut tc = TypeChecker::new();
        let types = tc.check_expression(&expr).unwrap();
        assert_eq!(types[expr.id as usize], DslType::Scalar);
    }

    #[test]
    fn check_cast() {
        let expr = parse_expression("42 as Scalar").unwrap();
        let mut tc = TypeChecker::new();
        let types = tc.check_expression(&expr).unwrap();
        assert_eq!(types[expr.id as usize], DslType::Scalar);
    }

    #[test]
    fn check_constants() {
        let expr = parse_expression("PI * 2.0").unwrap();
        let mut tc = TypeChecker::new();
        let types = tc.check_expression(&expr).unwrap();
        assert_eq!(types[expr.id as usize], DslType::Scalar);
    }
}
