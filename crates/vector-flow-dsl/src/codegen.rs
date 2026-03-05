use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types;
use cranelift_codegen::ir::{AbiParam, BlockArg, InstBuilder, UserFuncName};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use vector_flow_core::compute::DslContext;

use crate::ast::*;
use crate::error::DslError;
use crate::parser;
use crate::runtime;
use crate::span::Spanned;
use crate::type_check::TypeChecker;

/// Compiled DSL function pointer type for expressions.
/// Signature: `extern "C" fn(ctx: *mut DslContext) -> f64`
pub type ExprFnPtr = unsafe extern "C" fn(*mut DslContext) -> f64;

/// The JIT compiler for DSL expressions and programs.
pub struct DslCompiler {
    module: JITModule,
    ctx: Context,
    func_builder_ctx: FunctionBuilderContext,
    func_counter: usize,
    /// Cached declarations of runtime functions (by symbol name -> FuncId).
    runtime_funcs: HashMap<String, FuncId>,
}

impl DslCompiler {
    pub fn new() -> Result<Self, DslError> {
        let isa = {
            let mut flag_builder = settings::builder();
            flag_builder.set("is_pic", "false").map_err(|e| DslError::codegen(e.to_string()))?;
            flag_builder.set("opt_level", "speed").map_err(|e| DslError::codegen(e.to_string()))?;
            let flags = settings::Flags::new(flag_builder);
            cranelift_native::builder()
                .map_err(DslError::codegen)?
                .finish(flags)
                .map_err(|e| DslError::codegen(e.to_string()))?
        };

        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

        for (name, ptr) in runtime::runtime_symbols() {
            builder.symbol(name, ptr);
        }

        let module = JITModule::new(builder);
        let ctx = module.make_context();

        Ok(Self {
            module,
            ctx,
            func_builder_ctx: FunctionBuilderContext::new(),
            func_counter: 0,
            runtime_funcs: HashMap::new(),
        })
    }

    /// Compile a standalone expression to a function pointer.
    /// Signature: `extern "C" fn(ctx: *mut DslContext) -> f64`
    pub fn compile_expression(&mut self, source: &str) -> Result<*const u8, DslError> {
        let expr = parser::parse_expression(source)?;
        let mut tc = TypeChecker::new();
        let type_table = tc.check_expression(&expr)?;
        self.compile_expr_to_fn(&expr, &type_table)
    }

    /// Compile a full DSL program to a function pointer.
    /// Signature: `extern "C" fn(ctx: *mut DslContext) -> f64`
    pub fn compile_program(&mut self, source: &str) -> Result<*const u8, DslError> {
        let func_def = parser::parse_program(source)?;
        let mut tc = TypeChecker::new();
        let type_table = tc.check_function(&func_def)?;
        self.compile_program_to_fn(&func_def, &type_table)
    }

    fn next_func_name(&mut self) -> String {
        let name = format!("dsl_func_{}", self.func_counter);
        self.func_counter += 1;
        name
    }

    fn compile_expr_to_fn(
        &mut self,
        expr: &Spanned<Expr>,
        type_table: &[DslType],
    ) -> Result<*const u8, DslError> {
        let name = self.next_func_name();

        let mut sig = self.module.make_signature();
        sig.call_conv = CallConv::SystemV;
        sig.params.push(AbiParam::new(self.module.target_config().pointer_type()));
        sig.returns.push(AbiParam::new(types::F64));

        let func_id = self.module
            .declare_function(&name, Linkage::Local, &sig)
            .map_err(|e| DslError::codegen(e.to_string()))?;

        self.ctx.func.signature = sig;
        self.ctx.func.name = UserFuncName::user(0, func_id.as_u32());

        {
            let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.func_builder_ctx);
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let ctx_ptr = builder.block_params(entry_block)[0];

            // Inner scope: CodegenCtx borrows builder mutably, is dropped before finalize
            {
                let mut cg = CodegenCtx {
                    builder: &mut builder,
                    module: &mut self.module,
                    ctx_ptr,
                    variables: HashMap::new(),
                    runtime_funcs: &mut self.runtime_funcs,
                    type_table,
                };

                let result = cg.emit_expr(expr)?;
                let result_f64 = cg.ensure_f64(result, expr.id);
                cg.builder.ins().return_(&[result_f64]);
            }
            // cg dropped, builder borrow released
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut self.ctx)
            .map_err(|e| DslError::codegen(e.to_string()))?;

        self.module.clear_context(&mut self.ctx);
        self.module.finalize_definitions()
            .map_err(|e| DslError::codegen(e.to_string()))?;

        Ok(self.module.get_finalized_function(func_id))
    }

    fn compile_program_to_fn(
        &mut self,
        func_def: &FunctionDef,
        type_table: &[DslType],
    ) -> Result<*const u8, DslError> {
        let name = self.next_func_name();

        let mut sig = self.module.make_signature();
        sig.call_conv = CallConv::SystemV;
        sig.params.push(AbiParam::new(self.module.target_config().pointer_type()));
        sig.returns.push(AbiParam::new(types::F64));

        let func_id = self.module
            .declare_function(&name, Linkage::Local, &sig)
            .map_err(|e| DslError::codegen(e.to_string()))?;

        self.ctx.func.signature = sig;
        self.ctx.func.name = UserFuncName::user(0, func_id.as_u32());

        {
            let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.func_builder_ctx);
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let ctx_ptr = builder.block_params(entry_block)[0];

            {
                let mut cg = CodegenCtx {
                    builder: &mut builder,
                    module: &mut self.module,
                    ctx_ptr,
                    variables: HashMap::new(),
                    runtime_funcs: &mut self.runtime_funcs,
                    type_table,
                };

                // Load function params from ctx.slots[]
                for (i, param) in func_def.params.iter().enumerate() {
                    let offset = std::mem::offset_of!(DslContext, slots) + i * 8;
                    let val = cg.builder.ins().load(
                        types::F64,
                        cranelift_codegen::ir::MemFlags::trusted(),
                        ctx_ptr,
                        offset as i32,
                    );
                    let var = cg.declare_variable(&param.name, param.param_type);
                    let typed_val = match param.param_type {
                        DslType::Int => cg.builder.ins().fcvt_to_sint_sat(types::I64, val),
                        _ => val,
                    };
                    cg.builder.def_var(var, typed_val);
                }

                let result = cg.emit_block(&func_def.body)?;
                let result_f64 = match result {
                    Some(v) => cg.coerce_to_f64(v),
                    None => cg.builder.ins().f64const(0.0),
                };
                cg.builder.ins().return_(&[result_f64]);
            }
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut self.ctx)
            .map_err(|e| DslError::codegen(e.to_string()))?;

        self.module.clear_context(&mut self.ctx);
        self.module.finalize_definitions()
            .map_err(|e| DslError::codegen(e.to_string()))?;

        Ok(self.module.get_finalized_function(func_id))
    }
}

// ---------------------------------------------------------------------------
// Internal codegen context
// ---------------------------------------------------------------------------

struct CodegenCtx<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    module: &'a mut JITModule,
    ctx_ptr: cranelift_codegen::ir::Value,
    variables: HashMap<String, (Variable, DslType)>,
    runtime_funcs: &'a mut HashMap<String, FuncId>,
    type_table: &'a [DslType],
}

impl CodegenCtx<'_, '_> {
    fn declare_variable(&mut self, name: &str, ty: DslType) -> Variable {
        let var = self.builder.declare_var(dsl_type_to_cl(ty));
        self.variables.insert(name.to_string(), (var, ty));
        var
    }

    fn lookup_runtime_func(
        &mut self,
        func_name: &str,
        param_types: &[cranelift_codegen::ir::Type],
        ret_type: cranelift_codegen::ir::Type,
    ) -> Result<cranelift_codegen::ir::FuncRef, DslError> {
        let symbol = format!("vf_{func_name}");

        let func_id = if let Some(&id) = self.runtime_funcs.get(&symbol) {
            id
        } else {
            let mut sig = self.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            for &pt in param_types {
                sig.params.push(AbiParam::new(pt));
            }
            sig.returns.push(AbiParam::new(ret_type));

            let id = self.module
                .declare_function(&symbol, Linkage::Import, &sig)
                .map_err(|e| DslError::codegen(e.to_string()))?;
            self.runtime_funcs.insert(symbol.clone(), id);
            id
        };

        Ok(self.module.declare_func_in_func(func_id, self.builder.func))
    }

    fn expr_type(&self, id: u32) -> DslType {
        self.type_table.get(id as usize).copied().unwrap_or(DslType::Scalar)
    }

    fn is_int_expr(&self, id: u32) -> bool {
        self.expr_type(id) == DslType::Int
    }

    // -----------------------------------------------------------------
    // Expression codegen
    // -----------------------------------------------------------------

    fn emit_expr(&mut self, expr: &Spanned<Expr>) -> Result<cranelift_codegen::ir::Value, DslError> {
        match &expr.node {
            Expr::Literal(lit) => self.emit_literal(lit),
            Expr::Variable(name) => self.emit_variable(name, expr),
            Expr::BinaryOp { op, left, right } => {
                let lv = self.emit_expr(left)?;
                let rv = self.emit_expr(right)?;
                self.emit_binary_op(*op, lv, rv, left.id, right.id)
            }
            Expr::UnaryOp { op, operand } => {
                let v = self.emit_expr(operand)?;
                self.emit_unary_op(*op, v, operand.id)
            }
            Expr::Call { name, args } => self.emit_call(name, args),
            Expr::Cast { expr: inner, target } => {
                let v = self.emit_expr(inner)?;
                self.emit_cast(v, inner.id, *target)
            }
            Expr::If { condition, then_branch, else_branch } => {
                self.emit_if_expr(condition, then_branch, else_branch.as_deref())
            }
            Expr::Index { .. } | Expr::FieldAccess { .. } => {
                // Deferred to follow-up (point access)
                Ok(self.builder.ins().f64const(0.0))
            }
        }
    }

    fn emit_literal(&mut self, lit: &Literal) -> Result<cranelift_codegen::ir::Value, DslError> {
        match lit {
            Literal::Float(f) => Ok(self.builder.ins().f64const(*f)),
            Literal::Int(i) => Ok(self.builder.ins().iconst(types::I64, *i)),
            Literal::Bool(b) => Ok(self.builder.ins().iconst(types::I8, *b as i64)),
        }
    }

    fn emit_variable(
        &mut self,
        name: &str,
        expr: &Spanned<Expr>,
    ) -> Result<cranelift_codegen::ir::Value, DslError> {
        match name {
            "time" => {
                let offset = std::mem::offset_of!(DslContext, time_secs) as i32;
                let f32_val = self.builder.ins().load(
                    types::F32, cranelift_codegen::ir::MemFlags::trusted(),
                    self.ctx_ptr, offset,
                );
                Ok(self.builder.ins().fpromote(types::F64, f32_val))
            }
            "frame" => {
                let offset = std::mem::offset_of!(DslContext, frame) as i32;
                Ok(self.builder.ins().load(
                    types::I64, cranelift_codegen::ir::MemFlags::trusted(),
                    self.ctx_ptr, offset,
                ))
            }
            "fps" => {
                let offset = std::mem::offset_of!(DslContext, fps) as i32;
                let f32_val = self.builder.ins().load(
                    types::F32, cranelift_codegen::ir::MemFlags::trusted(),
                    self.ctx_ptr, offset,
                );
                Ok(self.builder.ins().fpromote(types::F64, f32_val))
            }
            "PI" => Ok(self.builder.ins().f64const(std::f64::consts::PI)),
            "TAU" => Ok(self.builder.ins().f64const(std::f64::consts::TAU)),
            "E" => Ok(self.builder.ins().f64const(std::f64::consts::E)),
            _ => {
                if let Some(&(var, _ty)) = self.variables.get(name) {
                    Ok(self.builder.use_var(var))
                } else {
                    Err(DslError::codegen(format!(
                        "undefined variable '{}' at {}:{}",
                        name, expr.span.line, expr.span.col
                    )))
                }
            }
        }
    }

    fn emit_binary_op(
        &mut self,
        op: BinOp,
        mut lv: cranelift_codegen::ir::Value,
        mut rv: cranelift_codegen::ir::Value,
        left_id: u32,
        right_id: u32,
    ) -> Result<cranelift_codegen::ir::Value, DslError> {
        let lt = self.expr_type(left_id);
        let rt = self.expr_type(right_id);

        // Promote Int -> Scalar if mixed
        if lt == DslType::Int && rt == DslType::Scalar {
            lv = self.builder.ins().fcvt_from_sint(types::F64, lv);
        } else if lt == DslType::Scalar && rt == DslType::Int {
            rv = self.builder.ins().fcvt_from_sint(types::F64, rv);
        }

        let use_int = lt == DslType::Int && rt == DslType::Int;

        match op {
            BinOp::Add if use_int => Ok(self.builder.ins().iadd(lv, rv)),
            BinOp::Add => Ok(self.builder.ins().fadd(lv, rv)),
            BinOp::Sub if use_int => Ok(self.builder.ins().isub(lv, rv)),
            BinOp::Sub => Ok(self.builder.ins().fsub(lv, rv)),
            BinOp::Mul if use_int => Ok(self.builder.ins().imul(lv, rv)),
            BinOp::Mul => Ok(self.builder.ins().fmul(lv, rv)),
            BinOp::Div if use_int => Ok(self.builder.ins().sdiv(lv, rv)),
            BinOp::Div => Ok(self.builder.ins().fdiv(lv, rv)),
            BinOp::Mod if use_int => {
                let quot = self.builder.ins().sdiv(lv, rv);
                let prod = self.builder.ins().imul(quot, rv);
                Ok(self.builder.ins().isub(lv, prod))
            }
            BinOp::Mod => {
                let func_ref = self.lookup_runtime_func("fmod", &[types::F64, types::F64], types::F64)?;
                let call = self.builder.ins().call(func_ref, &[lv, rv]);
                Ok(self.builder.inst_results(call)[0])
            }
            BinOp::Eq if use_int => Ok(self.builder.ins().icmp(IntCC::Equal, lv, rv)),
            BinOp::Eq => Ok(self.builder.ins().fcmp(FloatCC::Equal, lv, rv)),
            BinOp::Ne if use_int => Ok(self.builder.ins().icmp(IntCC::NotEqual, lv, rv)),
            BinOp::Ne => Ok(self.builder.ins().fcmp(FloatCC::NotEqual, lv, rv)),
            BinOp::Lt if use_int => Ok(self.builder.ins().icmp(IntCC::SignedLessThan, lv, rv)),
            BinOp::Lt => Ok(self.builder.ins().fcmp(FloatCC::LessThan, lv, rv)),
            BinOp::Le if use_int => Ok(self.builder.ins().icmp(IntCC::SignedLessThanOrEqual, lv, rv)),
            BinOp::Le => Ok(self.builder.ins().fcmp(FloatCC::LessThanOrEqual, lv, rv)),
            BinOp::Gt if use_int => Ok(self.builder.ins().icmp(IntCC::SignedGreaterThan, lv, rv)),
            BinOp::Gt => Ok(self.builder.ins().fcmp(FloatCC::GreaterThan, lv, rv)),
            BinOp::Ge if use_int => Ok(self.builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, lv, rv)),
            BinOp::Ge => Ok(self.builder.ins().fcmp(FloatCC::GreaterThanOrEqual, lv, rv)),
            BinOp::And => Ok(self.builder.ins().band(lv, rv)),
            BinOp::Or => Ok(self.builder.ins().bor(lv, rv)),
        }
    }

    fn emit_unary_op(
        &mut self,
        op: UnaryOp,
        v: cranelift_codegen::ir::Value,
        operand_id: u32,
    ) -> Result<cranelift_codegen::ir::Value, DslError> {
        match op {
            UnaryOp::Neg => {
                if self.is_int_expr(operand_id) {
                    Ok(self.builder.ins().ineg(v))
                } else {
                    Ok(self.builder.ins().fneg(v))
                }
            }
            UnaryOp::Not => {
                let one = self.builder.ins().iconst(types::I8, 1);
                Ok(self.builder.ins().bxor(v, one))
            }
        }
    }

    fn emit_call(
        &mut self,
        name: &str,
        args: &[Spanned<Expr>],
    ) -> Result<cranelift_codegen::ir::Value, DslError> {
        let (param_cl_types, ret_cl_type) = match name {
            "iabs" => (vec![types::I64], types::I64),
            "imin" | "imax" => (vec![types::I64, types::I64], types::I64),
            "rand" => (vec![types::I64], types::F64),
            _ => {
                let pts: Vec<_> = args.iter().map(|_| types::F64).collect();
                (pts, types::F64)
            }
        };

        let mut arg_values = Vec::with_capacity(args.len());
        for (i, arg) in args.iter().enumerate() {
            let mut v = self.emit_expr(arg)?;
            let expected = param_cl_types[i];
            let actual_ty = self.expr_type(arg.id);

            if actual_ty == DslType::Int && expected == types::F64 {
                v = self.builder.ins().fcvt_from_sint(types::F64, v);
            }
            arg_values.push(v);
        }

        let func_ref = self.lookup_runtime_func(name, &param_cl_types, ret_cl_type)?;
        let call = self.builder.ins().call(func_ref, &arg_values);
        Ok(self.builder.inst_results(call)[0])
    }

    fn emit_cast(
        &mut self,
        v: cranelift_codegen::ir::Value,
        from_id: u32,
        target: DslType,
    ) -> Result<cranelift_codegen::ir::Value, DslError> {
        let from = self.expr_type(from_id);
        match (from, target) {
            (DslType::Int, DslType::Scalar) => Ok(self.builder.ins().fcvt_from_sint(types::F64, v)),
            (DslType::Scalar, DslType::Int) => Ok(self.builder.ins().fcvt_to_sint_sat(types::I64, v)),
            _ => Ok(v),
        }
    }

    // -----------------------------------------------------------------
    // If expression (returns a value)
    // -----------------------------------------------------------------

    fn emit_if_expr(
        &mut self,
        condition: &Spanned<Expr>,
        then_branch: &Block,
        else_branch: Option<&Block>,
    ) -> Result<cranelift_codegen::ir::Value, DslError> {
        let cond_val = self.emit_expr(condition)?;

        let then_block = self.builder.create_block();
        let else_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::F64);

        self.builder.ins().brif(cond_val, then_block, &[] as &[BlockArg], else_block, &[] as &[BlockArg]);

        self.builder.switch_to_block(then_block);
        self.builder.seal_block(then_block);
        let then_val = self.emit_block_value(then_branch)?;
        let then_val = self.coerce_to_f64(then_val);
        self.builder.ins().jump(merge_block, &[BlockArg::Value(then_val)]);

        self.builder.switch_to_block(else_block);
        self.builder.seal_block(else_block);
        let else_val = if let Some(eb) = else_branch {
            let v = self.emit_block_value(eb)?;
            self.coerce_to_f64(v)
        } else {
            self.builder.ins().f64const(0.0)
        };
        self.builder.ins().jump(merge_block, &[BlockArg::Value(else_val)]);

        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);

        Ok(self.builder.block_params(merge_block)[0])
    }

    // -----------------------------------------------------------------
    // Block / statement codegen
    // -----------------------------------------------------------------

    fn emit_block(&mut self, block: &Block) -> Result<Option<cranelift_codegen::ir::Value>, DslError> {
        for stmt in &block.statements {
            self.emit_statement(stmt)?;
        }
        if let Some(ref tail) = block.tail_expr {
            Ok(Some(self.emit_expr(tail)?))
        } else {
            Ok(None)
        }
    }

    fn emit_block_value(&mut self, block: &Block) -> Result<cranelift_codegen::ir::Value, DslError> {
        match self.emit_block(block)? {
            Some(v) => Ok(v),
            None => Ok(self.builder.ins().f64const(0.0)),
        }
    }

    fn emit_statement(&mut self, stmt: &Spanned<Statement>) -> Result<(), DslError> {
        match &stmt.node {
            Statement::Let { name, type_annotation, value } => {
                let val = self.emit_expr(value)?;
                let ty = type_annotation.unwrap_or_else(|| self.expr_type(value.id));
                let var = self.declare_variable(name, ty);

                if ty == DslType::Scalar && self.expr_type(value.id) == DslType::Int {
                    let converted = self.builder.ins().fcvt_from_sint(types::F64, val);
                    self.builder.def_var(var, converted);
                } else if ty == DslType::Int && self.expr_type(value.id) == DslType::Scalar {
                    let converted = self.builder.ins().fcvt_to_sint_sat(types::I64, val);
                    self.builder.def_var(var, converted);
                } else {
                    self.builder.def_var(var, val);
                }
            }

            Statement::Assign { target, value } => {
                let val = self.emit_expr(value)?;
                match target {
                    AssignTarget::Variable(name) => {
                        if let Some(&(var, var_ty)) = self.variables.get(name.as_str()) {
                            let converted = if var_ty == DslType::Scalar && self.expr_type(value.id) == DslType::Int {
                                self.builder.ins().fcvt_from_sint(types::F64, val)
                            } else {
                                val
                            };
                            self.builder.def_var(var, converted);
                        } else {
                            return Err(DslError::codegen(format!("undefined variable '{name}'")));
                        }
                    }
                    AssignTarget::IndexField { .. } => { /* deferred */ }
                }
            }

            Statement::For { var, start, end, body } => {
                self.emit_for_loop(var, start, end, body)?;
            }

            Statement::If { condition, then_branch, else_branch } => {
                self.emit_if_stmt(condition, then_branch, else_branch.as_ref())?;
            }

            Statement::Return(value) => {
                let val = self.emit_expr(value)?;
                let f64_val = self.coerce_to_f64(val);
                self.builder.ins().return_(&[f64_val]);
            }

            Statement::Expr(e) => {
                self.emit_expr(e)?;
            }
        }
        Ok(())
    }

    fn emit_for_loop(
        &mut self,
        var_name: &str,
        start: &Spanned<Expr>,
        end: &Spanned<Expr>,
        body: &Block,
    ) -> Result<(), DslError> {
        let start_val = self.emit_expr(start)?;
        let end_val = self.emit_expr(end)?;

        let start_i64 = if self.expr_type(start.id) == DslType::Scalar {
            self.builder.ins().fcvt_to_sint_sat(types::I64, start_val)
        } else {
            start_val
        };
        let end_i64 = if self.expr_type(end.id) == DslType::Scalar {
            self.builder.ins().fcvt_to_sint_sat(types::I64, end_val)
        } else {
            end_val
        };

        let loop_var = self.declare_variable(var_name, DslType::Int);
        self.builder.def_var(loop_var, start_i64);

        let header_block = self.builder.create_block();
        let body_block = self.builder.create_block();
        let exit_block = self.builder.create_block();

        self.builder.ins().jump(header_block, &[] as &[BlockArg]);

        // Header: check i < end
        self.builder.switch_to_block(header_block);
        let current = self.builder.use_var(loop_var);
        let cmp = self.builder.ins().icmp(IntCC::SignedLessThan, current, end_i64);
        self.builder.ins().brif(cmp, body_block, &[] as &[BlockArg], exit_block, &[] as &[BlockArg]);

        // Body
        self.builder.switch_to_block(body_block);
        self.builder.seal_block(body_block);
        for s in &body.statements {
            self.emit_statement(s)?;
        }
        if let Some(ref tail) = body.tail_expr {
            self.emit_expr(tail)?;
        }
        let current = self.builder.use_var(loop_var);
        let one = self.builder.ins().iconst(types::I64, 1);
        let next = self.builder.ins().iadd(current, one);
        self.builder.def_var(loop_var, next);
        self.builder.ins().jump(header_block, &[] as &[BlockArg]);

        // Seal header after both predecessors are defined
        self.builder.seal_block(header_block);

        self.builder.switch_to_block(exit_block);
        self.builder.seal_block(exit_block);

        Ok(())
    }

    fn emit_if_stmt(
        &mut self,
        condition: &Spanned<Expr>,
        then_branch: &Block,
        else_branch: Option<&Block>,
    ) -> Result<(), DslError> {
        let cond_val = self.emit_expr(condition)?;

        let then_block = self.builder.create_block();
        let else_block = self.builder.create_block();
        let merge_block = self.builder.create_block();

        self.builder.ins().brif(cond_val, then_block, &[] as &[BlockArg], else_block, &[] as &[BlockArg]);

        self.builder.switch_to_block(then_block);
        self.builder.seal_block(then_block);
        self.emit_block(then_branch)?;
        self.builder.ins().jump(merge_block, &[] as &[BlockArg]);

        self.builder.switch_to_block(else_block);
        self.builder.seal_block(else_block);
        if let Some(eb) = else_branch {
            self.emit_block(eb)?;
        }
        self.builder.ins().jump(merge_block, &[] as &[BlockArg]);

        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);

        Ok(())
    }

    // -----------------------------------------------------------------
    // Type coercion helpers
    // -----------------------------------------------------------------

    fn ensure_f64(&mut self, val: cranelift_codegen::ir::Value, expr_id: u32) -> cranelift_codegen::ir::Value {
        match self.expr_type(expr_id) {
            DslType::Int => self.builder.ins().fcvt_from_sint(types::F64, val),
            DslType::Bool => {
                let i64_val = self.builder.ins().uextend(types::I64, val);
                self.builder.ins().fcvt_from_sint(types::F64, i64_val)
            }
            _ => val,
        }
    }

    fn coerce_to_f64(&mut self, val: cranelift_codegen::ir::Value) -> cranelift_codegen::ir::Value {
        let val_type = self.builder.func.dfg.value_type(val);
        if val_type == types::F64 {
            val
        } else if val_type == types::I64 {
            self.builder.ins().fcvt_from_sint(types::F64, val)
        } else if val_type == types::I8 {
            let i64_val = self.builder.ins().uextend(types::I64, val);
            self.builder.ins().fcvt_from_sint(types::F64, i64_val)
        } else if val_type == types::F32 {
            self.builder.ins().fpromote(types::F64, val)
        } else {
            val
        }
    }
}

fn dsl_type_to_cl(ty: DslType) -> cranelift_codegen::ir::Type {
    match ty {
        DslType::Scalar => types::F64,
        DslType::Int => types::I64,
        DslType::Bool => types::I8,
        _ => types::F64,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::compute::DslContext;
    use vector_flow_core::types::TimeContext;

    fn eval_expr(source: &str, time: f32) -> f64 {
        let mut compiler = DslCompiler::new().unwrap();
        let ptr = compiler.compile_expression(source).unwrap();
        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = TimeContext { frame: 0, time_secs: time, fps: 30.0 };
        let mut ctx = DslContext::new(&tc);
        unsafe { func(&mut ctx) }
    }

    #[test]
    fn codegen_constant() {
        let result = eval_expr("2.0 + 3.0", 0.0);
        assert!((result - 5.0).abs() < 1e-10);
    }

    #[test]
    fn codegen_sin_zero() {
        let result = eval_expr("sin(0.0)", 0.0);
        assert!(result.abs() < 1e-10);
    }

    #[test]
    fn codegen_time_expr() {
        let result = eval_expr("sin(time * 2.0) * 50.0", 0.0);
        assert!(result.abs() < 1e-10, "sin(0)*50 should be 0, got {result}");

        let result = eval_expr(
            "sin(time * 2.0) * 50.0",
            std::f32::consts::FRAC_PI_4,
        );
        assert!((result - 50.0).abs() < 0.01, "expected ~50.0, got {result}");
    }

    #[test]
    fn codegen_pi_constant() {
        let result = eval_expr("PI * 2.0", 0.0);
        assert!((result - std::f64::consts::TAU).abs() < 1e-10);
    }

    #[test]
    fn codegen_int_arithmetic() {
        let result = eval_expr("(3 + 4) as Scalar", 0.0);
        assert!((result - 7.0).abs() < 1e-10);
    }

    #[test]
    fn codegen_if_expression() {
        let result = eval_expr("if true { 42.0 } else { 0.0 }", 0.0);
        assert!((result - 42.0).abs() < 1e-10);
    }

    #[test]
    fn codegen_nested_calls() {
        let result = eval_expr("abs(sin(0.0))", 0.0);
        assert!(result.abs() < 1e-10);
    }

    #[test]
    fn codegen_comparison() {
        let result = eval_expr("if 1.0 > 0.0 { 1.0 } else { 0.0 }", 0.0);
        assert!((result - 1.0).abs() < 1e-10);
    }

    #[test]
    fn codegen_unary_neg() {
        let result = eval_expr("-5.0", 0.0);
        assert!((result - (-5.0)).abs() < 1e-10);
    }

    #[test]
    fn codegen_program_simple() {
        let mut compiler = DslCompiler::new().unwrap();
        let ptr = compiler.compile_program(
            "fn test(x: Scalar) -> Scalar { let y = x * 2.0; y }"
        ).unwrap();
        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = TimeContext { frame: 0, time_secs: 0.0, fps: 30.0 };
        let mut ctx = DslContext::new(&tc);
        ctx.slots[0] = 5.0;
        let result = unsafe { func(&mut ctx) };
        assert!((result - 10.0).abs() < 1e-10);
    }

    #[test]
    fn codegen_program_for_loop() {
        let mut compiler = DslCompiler::new().unwrap();
        let ptr = compiler.compile_program(
            "fn sum(n: Int) -> Scalar { let total: Scalar = 0.0; for i in 0..n { total = total + 1.0; } total }"
        ).unwrap();
        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = TimeContext { frame: 0, time_secs: 0.0, fps: 30.0 };
        let mut ctx = DslContext::new(&tc);
        ctx.slots[0] = 10.0; // n = 10 (loaded as f64, converted to i64)
        let result = unsafe { func(&mut ctx) };
        assert!((result - 10.0).abs() < 1e-10, "expected 10.0, got {result}");
    }

    #[test]
    fn codegen_program_if_else() {
        let mut compiler = DslCompiler::new().unwrap();
        let ptr = compiler.compile_program(
            "fn pick(x: Scalar) -> Scalar { if x > 0.0 { x } else { -x } }"
        ).unwrap();
        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = TimeContext { frame: 0, time_secs: 0.0, fps: 30.0 };

        let mut ctx = DslContext::new(&tc);
        ctx.slots[0] = 5.0;
        let result = unsafe { func(&mut ctx) };
        assert!((result - 5.0).abs() < 1e-10);

        let mut ctx = DslContext::new(&tc);
        ctx.slots[0] = -3.0;
        let result = unsafe { func(&mut ctx) };
        assert!((result - 3.0).abs() < 1e-10);
    }

    #[test]
    fn codegen_lerp() {
        let result = eval_expr("lerp(0.0, 10.0, 0.5)", 0.0);
        assert!((result - 5.0).abs() < 1e-10);
    }

    #[test]
    fn codegen_frame_builtin() {
        let mut compiler = DslCompiler::new().unwrap();
        let ptr = compiler.compile_expression("frame as Scalar").unwrap();
        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = TimeContext { frame: 42, time_secs: 1.4, fps: 30.0 };
        let mut ctx = DslContext::new(&tc);
        let result = unsafe { func(&mut ctx) };
        assert!((result - 42.0).abs() < 1e-10);
    }
}
