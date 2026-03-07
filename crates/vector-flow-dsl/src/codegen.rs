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

    /// Compile a bare script with externally-defined input/output port bindings.
    /// Inputs are loaded from `ctx.slots[0..inputs.len()]`.
    /// Outputs are written back to `ctx.slots[inputs.len()..inputs.len()+outputs.len()]`.
    /// Signature: `extern "C" fn(ctx: *mut DslContext) -> f64`
    pub fn compile_node_script(
        &mut self,
        source: &str,
        inputs: &[(String, DslType)],
        outputs: &[(String, DslType)],
    ) -> Result<*const u8, DslError> {
        let block = parser::parse_script(source)?;
        let mut tc = TypeChecker::new();
        let type_table = tc.check_script(&block, inputs, outputs)?;
        self.compile_script_to_fn(&block, &type_table, inputs, outputs)
    }

    fn compile_script_to_fn(
        &mut self,
        block: &crate::ast::Block,
        type_table: &[DslType],
        inputs: &[(String, DslType)],
        outputs: &[(String, DslType)],
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
                // Compute how many slots are used by inputs+outputs (Color takes 4 slots each).
                let input_slot_count: usize = inputs.iter().map(|(_, t)| slots_for_type(*t)).sum();
                let output_slot_count: usize = outputs.iter().map(|(_, t)| slots_for_type(*t)).sum();
                let first_temp_slot = (input_slot_count + output_slot_count) as u32;

                let mut cg = CodegenCtx {
                    builder: &mut builder,
                    module: &mut self.module,
                    ctx_ptr,
                    variables: HashMap::new(),
                    color_vars: HashMap::new(),
                    next_color_slot: first_temp_slot,
                    last_color_slot: None,
                    runtime_funcs: &mut self.runtime_funcs,
                    type_table,
                };

                // Load input variables from ctx.slots[]
                let mut input_slot_idx = 0usize;
                for (param_name, param_type) in inputs.iter() {
                    if *param_type == DslType::Color {
                        // Color input: register as color var at current slot index
                        cg.color_vars.insert(param_name.clone(), input_slot_idx as u32);
                        input_slot_idx += 4;
                    } else {
                        let offset = std::mem::offset_of!(DslContext, slots) + input_slot_idx * 8;
                        let val = cg.builder.ins().load(
                            types::F64,
                            cranelift_codegen::ir::MemFlags::trusted(),
                            ctx_ptr,
                            offset as i32,
                        );
                        let var = cg.declare_variable(param_name, *param_type);
                        let typed_val = match param_type {
                            DslType::Int => cg.builder.ins().fcvt_to_sint_sat(types::I64, val),
                            _ => val,
                        };
                        cg.builder.def_var(var, typed_val);
                        input_slot_idx += 1;
                    }
                }

                // Pre-declare output variables (initialized to 0)
                let mut output_slot_idx = input_slot_idx;
                for (out_name, out_type) in outputs.iter() {
                    if *out_type == DslType::Color {
                        // Color output: register as color var at output slot index
                        cg.color_vars.insert(out_name.clone(), output_slot_idx as u32);
                        output_slot_idx += 4;
                    } else {
                        let var = cg.declare_variable(out_name, *out_type);
                        let zero = match out_type {
                            DslType::Int => cg.builder.ins().iconst(types::I64, 0),
                            DslType::Bool => cg.builder.ins().iconst(types::I8, 0),
                            _ => cg.builder.ins().f64const(0.0),
                        };
                        cg.builder.def_var(var, zero);
                        output_slot_idx += 1;
                    }
                }

                // Emit body
                let tail_result = cg.emit_block(block)?;

                // If there's a tail expression, assign it to the first output variable
                // so the output writeback below picks it up.
                if let (Some(tail_val), Some((out_name, out_type))) = (tail_result, outputs.first()) {
                    if *out_type == DslType::Color {
                        // Color tail: last_color_slot should have been set by the expression.
                        // Copy from last_color_slot to the output color slot if different.
                        if let (Some(src), Some(&dest)) = (cg.last_color_slot, cg.color_vars.get(out_name.as_str())) {
                            if src != dest {
                                cg.emit_color_copy(src, dest);
                            }
                        }
                    } else if let Some(&(var, _)) = cg.variables.get(out_name.as_str()) {
                        let coerced = match out_type {
                            DslType::Int => cg.builder.ins().fcvt_to_sint_sat(types::I64, tail_val),
                            _ => {
                                let val_type = cg.builder.func.dfg.value_type(tail_val);
                                if val_type == types::I64 {
                                    cg.builder.ins().fcvt_from_sint(types::F64, tail_val)
                                } else {
                                    tail_val
                                }
                            }
                        };
                        cg.builder.def_var(var, coerced);
                    }
                }

                // Write all non-Color output variables back to ctx.slots[]
                // (Color outputs are already written directly to slots by the codegen.)
                let mut out_slot = input_slot_count;
                for (out_name, out_type) in outputs.iter() {
                    if *out_type == DslType::Color {
                        out_slot += 4;
                        continue;
                    }
                    if let Some(&(var, _)) = cg.variables.get(out_name.as_str()) {
                        let val = cg.builder.use_var(var);
                        let out_offset = std::mem::offset_of!(DslContext, slots)
                            + out_slot * 8;
                        let f64_val = match out_type {
                            DslType::Int => cg.builder.ins().fcvt_from_sint(types::F64, val),
                            DslType::Bool => {
                                let i64_val = cg.builder.ins().uextend(types::I64, val);
                                cg.builder.ins().fcvt_from_sint(types::F64, i64_val)
                            }
                            _ => val,
                        };
                        cg.builder.ins().store(
                            cranelift_codegen::ir::MemFlags::trusted(),
                            f64_val,
                            ctx_ptr,
                            out_offset as i32,
                        );
                    }
                    out_slot += 1;
                }

                // Return value (for compatibility, return 0)
                let ret = cg.builder.ins().f64const(0.0);
                cg.builder.ins().return_(&[ret]);
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
                    color_vars: HashMap::new(),
                    next_color_slot: 0,
                    last_color_slot: None,
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
                let param_slot_count: usize = func_def.params.iter()
                    .map(|p| slots_for_type(p.param_type)).sum();

                let mut cg = CodegenCtx {
                    builder: &mut builder,
                    module: &mut self.module,
                    ctx_ptr,
                    variables: HashMap::new(),
                    color_vars: HashMap::new(),
                    next_color_slot: param_slot_count as u32,
                    last_color_slot: None,
                    runtime_funcs: &mut self.runtime_funcs,
                    type_table,
                };

                // Load function params from ctx.slots[]
                let mut param_slot_idx = 0usize;
                for param in func_def.params.iter() {
                    if param.param_type == DslType::Color {
                        cg.color_vars.insert(param.name.clone(), param_slot_idx as u32);
                        param_slot_idx += 4;
                    } else {
                        let offset = std::mem::offset_of!(DslContext, slots) + param_slot_idx * 8;
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
                        param_slot_idx += 1;
                    }
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
    /// Scalar/Int/Bool variables: mapped to Cranelift Variables.
    variables: HashMap<String, (Variable, DslType)>,
    /// Color variables: mapped to slot indices in DslContext.slots[].
    /// A Color occupies 4 consecutive f64 slots starting at this index.
    color_vars: HashMap<String, u32>,
    /// Next available slot index for Color temporaries in DslContext.slots[].
    next_color_slot: u32,
    /// Slot index where the last Color expression result was written.
    /// Used as a side-channel since Color values don't fit in a single SSA value.
    last_color_slot: Option<u32>,
    runtime_funcs: &'a mut HashMap<String, FuncId>,
    type_table: &'a [DslType],
}

impl CodegenCtx<'_, '_> {
    fn declare_variable(&mut self, name: &str, ty: DslType) -> Variable {
        let var = self.builder.declare_var(dsl_type_to_cl(ty));
        self.variables.insert(name.to_string(), (var, ty));
        var
    }

    /// Allocate 4 consecutive temporary slots for a Color value.
    fn alloc_color_temp(&mut self) -> u32 {
        let slot = self.next_color_slot;
        self.next_color_slot += 4;
        slot
    }

    /// Declare a Color variable backed by context slots.
    fn declare_color_variable(&mut self, name: &str) -> u32 {
        let slot = self.alloc_color_temp();
        self.color_vars.insert(name.to_string(), slot);
        slot
    }

    /// Emit a call to vf_color_copy(slots_ptr, src_slot, dest_slot).
    fn emit_color_copy(&mut self, src_slot: u32, dest_slot: u32) {
        let slots_ptr = self.slots_ptr();
        let src_val = self.builder.ins().iconst(types::I32, src_slot as i64);
        let dest_val = self.builder.ins().iconst(types::I32, dest_slot as i64);
        let ptr_type = self.builder.func.dfg.value_type(self.ctx_ptr);
        let func_ref = self.lookup_runtime_func_raw(
            "vf_color_copy",
            &[ptr_type, types::I32, types::I32],
            None,
        ).unwrap();
        self.builder.ins().call(func_ref, &[slots_ptr, src_val, dest_val]);
    }

    /// Get a pointer to ctx.slots (ctx_ptr + offset_of(slots)).
    fn slots_ptr(&mut self) -> cranelift_codegen::ir::Value {
        let offset = std::mem::offset_of!(DslContext, slots) as i64;
        self.builder.ins().iadd_imm(self.ctx_ptr, offset)
    }

    /// Lookup a runtime function with optional return type (None = void).
    fn lookup_runtime_func_raw(
        &mut self,
        symbol: &str,
        param_types: &[cranelift_codegen::ir::Type],
        ret_type: Option<cranelift_codegen::ir::Type>,
    ) -> Result<cranelift_codegen::ir::FuncRef, DslError> {
        let func_id = if let Some(&id) = self.runtime_funcs.get(symbol) {
            id
        } else {
            let mut sig = self.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            for &pt in param_types {
                sig.params.push(AbiParam::new(pt));
            }
            if let Some(rt) = ret_type {
                sig.returns.push(AbiParam::new(rt));
            }
            let id = self.module
                .declare_function(symbol, Linkage::Import, &sig)
                .map_err(|e| DslError::codegen(e.to_string()))?;
            self.runtime_funcs.insert(symbol.to_string(), id);
            id
        };
        Ok(self.module.declare_func_in_func(func_id, self.builder.func))
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
                // Check Color variables first
                if let Some(&slot) = self.color_vars.get(name) {
                    self.last_color_slot = Some(slot);
                    // Return a dummy f64 — the actual color data is in slots.
                    Ok(self.builder.ins().f64const(0.0))
                } else if let Some(&(var, _ty)) = self.variables.get(name) {
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
        // ---- Color construction functions: (scalar args) -> Color in slots ----
        if matches!(name, "hsl" | "hsla" | "rgb" | "rgba") {
            return self.emit_color_constructor(name, args);
        }

        // ---- Color component extractors: (Color) -> Scalar ----
        if matches!(name, "color_r" | "color_g" | "color_b" | "color_a"
                       | "color_hue" | "color_sat" | "color_light") {
            return self.emit_color_extractor(name, args);
        }

        // ---- Color modification: (Color, Scalar) -> Color in slots ----
        if matches!(name, "set_lightness" | "set_saturation" | "set_hue" | "set_alpha_color") {
            return self.emit_color_modifier(name, args);
        }

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

    // -----------------------------------------------------------------
    // Color codegen helpers
    // -----------------------------------------------------------------

    /// Emit a Color construction call: hsl/hsla/rgb/rgba.
    /// The runtime function writes 4 f64 values to dest slots.
    fn emit_color_constructor(
        &mut self,
        name: &str,
        args: &[Spanned<Expr>],
    ) -> Result<cranelift_codegen::ir::Value, DslError> {
        // Evaluate scalar args
        let mut arg_values = Vec::with_capacity(args.len());
        for arg in args {
            let mut v = self.emit_expr(arg)?;
            if self.expr_type(arg.id) == DslType::Int {
                v = self.builder.ins().fcvt_from_sint(types::F64, v);
            }
            arg_values.push(v);
        }

        let dest_slot = self.alloc_color_temp();
        let slots_ptr = self.slots_ptr();
        let dest_val = self.builder.ins().iconst(types::I32, dest_slot as i64);

        let ptr_type = self.builder.func.dfg.value_type(self.ctx_ptr);
        let symbol = format!("vf_{name}");

        // Build param types: ptr, f64s..., i32 (dest_slot)
        let mut param_types = vec![ptr_type];
        param_types.extend(std::iter::repeat_n(types::F64, arg_values.len()));
        param_types.push(types::I32);

        let func_ref = self.lookup_runtime_func_raw(&symbol, &param_types, None)?;

        let mut call_args = vec![slots_ptr];
        call_args.extend_from_slice(&arg_values);
        call_args.push(dest_val);

        self.builder.ins().call(func_ref, &call_args);

        self.last_color_slot = Some(dest_slot);
        // Return dummy f64 — actual color is in slots
        Ok(self.builder.ins().f64const(0.0))
    }

    /// Emit a Color component extractor: color_r/g/b/a/hue/sat/light.
    /// The first arg must be a Color (resolved to a slot index via last_color_slot).
    fn emit_color_extractor(
        &mut self,
        name: &str,
        args: &[Spanned<Expr>],
    ) -> Result<cranelift_codegen::ir::Value, DslError> {
        // Evaluate the Color arg — this sets last_color_slot
        self.emit_expr(&args[0])?;
        let src_slot = self.last_color_slot.ok_or_else(|| {
            DslError::codegen(format!("{name}(): Color argument did not resolve to a slot"))
        })?;

        let slots_ptr = self.slots_ptr();
        let src_val = self.builder.ins().iconst(types::I32, src_slot as i64);

        let ptr_type = self.builder.func.dfg.value_type(self.ctx_ptr);
        let symbol = format!("vf_{name}");
        let func_ref = self.lookup_runtime_func_raw(
            &symbol,
            &[ptr_type, types::I32],
            Some(types::F64),
        )?;

        let call = self.builder.ins().call(func_ref, &[slots_ptr, src_val]);
        self.last_color_slot = None; // Result is a scalar, not a color
        Ok(self.builder.inst_results(call)[0])
    }

    /// Emit a Color modifier: set_lightness/set_saturation/set_hue/set_alpha_color.
    /// Signature: (slots_ptr, src_slot, scalar_val, dest_slot).
    fn emit_color_modifier(
        &mut self,
        name: &str,
        args: &[Spanned<Expr>],
    ) -> Result<cranelift_codegen::ir::Value, DslError> {
        // First arg is Color
        self.emit_expr(&args[0])?;
        let src_slot = self.last_color_slot.ok_or_else(|| {
            DslError::codegen(format!("{name}(): Color argument did not resolve to a slot"))
        })?;

        // Second arg is Scalar
        let mut scalar_val = self.emit_expr(&args[1])?;
        if self.expr_type(args[1].id) == DslType::Int {
            scalar_val = self.builder.ins().fcvt_from_sint(types::F64, scalar_val);
        }

        let dest_slot = self.alloc_color_temp();
        let slots_ptr = self.slots_ptr();
        let src_val = self.builder.ins().iconst(types::I32, src_slot as i64);
        let dest_val = self.builder.ins().iconst(types::I32, dest_slot as i64);

        let ptr_type = self.builder.func.dfg.value_type(self.ctx_ptr);
        let symbol = format!("vf_{name}");
        let func_ref = self.lookup_runtime_func_raw(
            &symbol,
            &[ptr_type, types::I32, types::F64, types::I32],
            None,
        )?;

        self.builder.ins().call(func_ref, &[slots_ptr, src_val, scalar_val, dest_val]);

        self.last_color_slot = Some(dest_slot);
        Ok(self.builder.ins().f64const(0.0))
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
                let ty = type_annotation.unwrap_or_else(|| self.expr_type(value.id));

                if ty == DslType::Color {
                    // Evaluate the Color expression (sets last_color_slot)
                    self.emit_expr(value)?;
                    let src = self.last_color_slot.unwrap_or(0);
                    let dest = self.declare_color_variable(name);
                    if src != dest {
                        self.emit_color_copy(src, dest);
                    }
                } else {
                    let val = self.emit_expr(value)?;
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
            }

            Statement::Assign { target, value } => {
                match target {
                    AssignTarget::Variable(name) => {
                        // Check if it's a Color variable
                        if let Some(&dest_slot) = self.color_vars.get(name.as_str()) {
                            // Evaluate Color expression (sets last_color_slot)
                            self.emit_expr(value)?;
                            let src = self.last_color_slot.unwrap_or(0);
                            if src != dest_slot {
                                self.emit_color_copy(src, dest_slot);
                            }
                        } else if let Some(&(var, var_ty)) = self.variables.get(name.as_str()) {
                            let val = self.emit_expr(value)?;
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

/// How many f64 slots a type occupies in DslContext.
fn slots_for_type(ty: DslType) -> usize {
    match ty {
        DslType::Color => 4,
        _ => 1,
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
    use vector_flow_core::types::EvalContext;

    fn eval_expr(source: &str, time: f32) -> f64 {
        let mut compiler = DslCompiler::new().unwrap();
        let ptr = compiler.compile_expression(source).unwrap();
        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext { frame: 0, time_secs: time, fps: 30.0, ..Default::default() };
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
        let tc = EvalContext { frame: 0, time_secs: 0.0, fps: 30.0, ..Default::default() };
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
        let tc = EvalContext { frame: 0, time_secs: 0.0, fps: 30.0, ..Default::default() };
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
        let tc = EvalContext { frame: 0, time_secs: 0.0, fps: 30.0, ..Default::default() };

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
        let tc = EvalContext { frame: 42, time_secs: 1.4, fps: 30.0, ..Default::default() };
        let mut ctx = DslContext::new(&tc);
        let result = unsafe { func(&mut ctx) };
        assert!((result - 42.0).abs() < 1e-10);
    }

    #[test]
    fn codegen_node_script_simple() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![("x".to_string(), DslType::Scalar)];
        let outputs = vec![("result".to_string(), DslType::Scalar)];
        let ptr = compiler.compile_node_script(
            "result = x * 2.0;",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext { frame: 0, time_secs: 0.0, fps: 30.0, ..Default::default() };
        let mut ctx = DslContext::new(&tc);
        ctx.slots[0] = 5.0; // x = 5.0

        unsafe { func(&mut ctx) };

        // Output at slots[1] (after 1 input)
        assert!((ctx.slots[1] - 10.0).abs() < 1e-10, "expected 10.0, got {}", ctx.slots[1]);
    }

    #[test]
    fn codegen_node_script_tail_expr() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![("x".to_string(), DslType::Scalar)];
        let outputs = vec![("result".to_string(), DslType::Scalar)];
        // Tail expression (no semicolon) should write to first output.
        let ptr = compiler.compile_node_script(
            "x + 100.0",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext { frame: 0, time_secs: 0.0, fps: 30.0, ..Default::default() };
        let mut ctx = DslContext::new(&tc);
        ctx.slots[0] = 7.0;

        unsafe { func(&mut ctx) };

        assert!((ctx.slots[1] - 107.0).abs() < 1e-10, "expected 107.0, got {}", ctx.slots[1]);
    }

    #[test]
    fn codegen_node_script_multiple_outputs() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![("x".to_string(), DslType::Scalar)];
        let outputs = vec![
            ("sum".to_string(), DslType::Scalar),
            ("product".to_string(), DslType::Scalar),
        ];
        let ptr = compiler.compile_node_script(
            "sum = x + 1.0;\nproduct = x * 2.0;",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext { frame: 0, time_secs: 0.0, fps: 30.0, ..Default::default() };
        let mut ctx = DslContext::new(&tc);
        ctx.slots[0] = 3.0;

        unsafe { func(&mut ctx) };

        assert!((ctx.slots[1] - 4.0).abs() < 1e-10, "sum: expected 4.0, got {}", ctx.slots[1]);
        assert!((ctx.slots[2] - 6.0).abs() < 1e-10, "product: expected 6.0, got {}", ctx.slots[2]);
    }

    #[test]
    fn codegen_node_script_time_builtin() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![];
        let outputs = vec![("result".to_string(), DslType::Scalar)];
        let ptr = compiler.compile_node_script(
            "result = sin(time);",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext { frame: 0, time_secs: std::f32::consts::FRAC_PI_2, fps: 30.0, ..Default::default() };
        let mut ctx = DslContext::new(&tc);

        unsafe { func(&mut ctx) };

        assert!((ctx.slots[0] - 1.0).abs() < 0.01, "expected ~1.0, got {}", ctx.slots[0]);
    }

    #[test]
    fn codegen_node_script_no_ports() {
        // Empty ports, expression mode fallback (should still compile as script)
        let mut compiler = DslCompiler::new().unwrap();
        let ptr = compiler.compile_node_script(
            "42.0",
            &[],
            &[],
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext { frame: 0, time_secs: 0.0, fps: 30.0, ..Default::default() };
        let mut ctx = DslContext::new(&tc);
        let result = unsafe { func(&mut ctx) };
        // Returns 0.0 since no outputs, but shouldn't crash
        assert!((result - 0.0).abs() < 1e-10);
    }

    // ---------------------------------------------------------------
    // Color codegen tests
    // ---------------------------------------------------------------

    #[test]
    fn codegen_color_rgb_constructor() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![];
        let outputs = vec![("c".to_string(), DslType::Color)];
        let ptr = compiler.compile_node_script(
            "c = rgb(1.0, 0.5, 0.25);",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext::default();
        let mut ctx = DslContext::new(&tc);
        unsafe { func(&mut ctx) };

        // Output Color at slots[0..4]
        assert!((ctx.slots[0] - 1.0).abs() < 1e-10, "r: expected 1.0, got {}", ctx.slots[0]);
        assert!((ctx.slots[1] - 0.5).abs() < 1e-10, "g: expected 0.5, got {}", ctx.slots[1]);
        assert!((ctx.slots[2] - 0.25).abs() < 1e-10, "b: expected 0.25, got {}", ctx.slots[2]);
        assert!((ctx.slots[3] - 1.0).abs() < 1e-10, "a: expected 1.0, got {}", ctx.slots[3]);
    }

    #[test]
    fn codegen_color_hsl_constructor() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![];
        let outputs = vec![("c".to_string(), DslType::Color)];
        let ptr = compiler.compile_node_script(
            "c = hsl(0.0, 100.0, 50.0);",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext::default();
        let mut ctx = DslContext::new(&tc);
        unsafe { func(&mut ctx) };

        // Pure red: hsl(0, 100%, 50%) = rgb(1, 0, 0)
        assert!((ctx.slots[0] - 1.0).abs() < 1e-4, "r: expected 1.0, got {}", ctx.slots[0]);
        assert!(ctx.slots[1].abs() < 1e-4, "g: expected 0.0, got {}", ctx.slots[1]);
        assert!(ctx.slots[2].abs() < 1e-4, "b: expected 0.0, got {}", ctx.slots[2]);
        assert!((ctx.slots[3] - 1.0).abs() < 1e-10, "a: expected 1.0, got {}", ctx.slots[3]);
    }

    #[test]
    fn codegen_color_extractor() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![];
        let outputs = vec![
            ("r_val".to_string(), DslType::Scalar),
            ("g_val".to_string(), DslType::Scalar),
        ];
        let ptr = compiler.compile_node_script(
            "let c: Color = rgb(0.8, 0.6, 0.4);\nr_val = color_r(c);\ng_val = color_g(c);",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext::default();
        let mut ctx = DslContext::new(&tc);
        unsafe { func(&mut ctx) };

        // Outputs at slots[0] and slots[1] (no inputs, 2 scalar outputs)
        assert!((ctx.slots[0] - 0.8).abs() < 1e-10, "r_val: expected 0.8, got {}", ctx.slots[0]);
        assert!((ctx.slots[1] - 0.6).abs() < 1e-10, "g_val: expected 0.6, got {}", ctx.slots[1]);
    }

    #[test]
    fn codegen_color_let_and_assign() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![];
        let outputs = vec![("result".to_string(), DslType::Color)];
        let ptr = compiler.compile_node_script(
            "let c: Color = rgb(0.1, 0.2, 0.3);\nresult = c;",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext::default();
        let mut ctx = DslContext::new(&tc);
        unsafe { func(&mut ctx) };

        assert!((ctx.slots[0] - 0.1).abs() < 1e-10, "r: got {}", ctx.slots[0]);
        assert!((ctx.slots[1] - 0.2).abs() < 1e-10, "g: got {}", ctx.slots[1]);
        assert!((ctx.slots[2] - 0.3).abs() < 1e-10, "b: got {}", ctx.slots[2]);
        assert!((ctx.slots[3] - 1.0).abs() < 1e-10, "a: got {}", ctx.slots[3]);
    }

    #[test]
    fn codegen_color_set_alpha() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![];
        let outputs = vec![("result".to_string(), DslType::Color)];
        let ptr = compiler.compile_node_script(
            "result = set_alpha_color(rgb(1.0, 0.0, 0.0), 0.5);",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext::default();
        let mut ctx = DslContext::new(&tc);
        unsafe { func(&mut ctx) };

        assert!((ctx.slots[0] - 1.0).abs() < 1e-10, "r: got {}", ctx.slots[0]);
        assert!(ctx.slots[1].abs() < 1e-10, "g: got {}", ctx.slots[1]);
        assert!(ctx.slots[2].abs() < 1e-10, "b: got {}", ctx.slots[2]);
        assert!((ctx.slots[3] - 0.5).abs() < 1e-10, "a: expected 0.5, got {}", ctx.slots[3]);
    }

    #[test]
    fn codegen_color_rgba_constructor() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![];
        let outputs = vec![("c".to_string(), DslType::Color)];
        let ptr = compiler.compile_node_script(
            "c = rgba(0.2, 0.4, 0.6, 0.8);",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext::default();
        let mut ctx = DslContext::new(&tc);
        unsafe { func(&mut ctx) };

        assert!((ctx.slots[0] - 0.2).abs() < 1e-10, "r: got {}", ctx.slots[0]);
        assert!((ctx.slots[1] - 0.4).abs() < 1e-10, "g: got {}", ctx.slots[1]);
        assert!((ctx.slots[2] - 0.6).abs() < 1e-10, "b: got {}", ctx.slots[2]);
        assert!((ctx.slots[3] - 0.8).abs() < 1e-10, "a: got {}", ctx.slots[3]);
    }

    #[test]
    fn codegen_color_input_output() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![("in_color".to_string(), DslType::Color)];
        let outputs = vec![("brightness".to_string(), DslType::Scalar)];
        let ptr = compiler.compile_node_script(
            "brightness = color_r(in_color) * 0.2126 + color_g(in_color) * 0.7152 + color_b(in_color) * 0.0722;",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext::default();
        let mut ctx = DslContext::new(&tc);
        // Input color at slots[0..4] (white)
        ctx.slots[0] = 1.0; ctx.slots[1] = 1.0; ctx.slots[2] = 1.0; ctx.slots[3] = 1.0;
        unsafe { func(&mut ctx) };

        // Output at slots[4] (after 4 color input slots)
        assert!((ctx.slots[4] - 1.0).abs() < 1e-4, "brightness of white: expected ~1.0, got {}", ctx.slots[4]);
    }

    #[test]
    fn codegen_color_hsl_extractors() {
        let mut compiler = DslCompiler::new().unwrap();
        let inputs = vec![];
        let outputs = vec![
            ("h".to_string(), DslType::Scalar),
            ("s".to_string(), DslType::Scalar),
            ("l".to_string(), DslType::Scalar),
        ];
        let ptr = compiler.compile_node_script(
            "let c: Color = hsl(120.0, 100.0, 50.0);\nh = color_hue(c);\ns = color_sat(c);\nl = color_light(c);",
            &inputs,
            &outputs,
        ).unwrap();

        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let tc = EvalContext::default();
        let mut ctx = DslContext::new(&tc);
        unsafe { func(&mut ctx) };

        assert!((ctx.slots[0] - 120.0).abs() < 0.5, "hue: expected ~120, got {}", ctx.slots[0]);
        assert!((ctx.slots[1] - 100.0).abs() < 0.5, "sat: expected ~100, got {}", ctx.slots[1]);
        assert!((ctx.slots[2] - 50.0).abs() < 0.5, "light: expected ~50, got {}", ctx.slots[2]);
    }
}
