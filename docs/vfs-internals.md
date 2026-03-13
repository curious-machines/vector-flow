# VFS Internals: From Source Code to Execution

This document describes the complete Vector Flow Script (VFS) processing pipeline — how user-written script source code is lexed into tokens, parsed into an abstract syntax tree, type-checked, compiled to native machine code via Cranelift, and executed at runtime. It covers every stage in enough detail to reimplement the pipeline from scratch.

## Table of Contents

- [Architecture Overview](#architecture-overview)
- [Stage 1: Lexing](#stage-1-lexing)
- [Stage 2: Parsing](#stage-2-parsing)
- [Stage 3: Type Checking](#stage-3-type-checking)
- [Stage 4: Code Generation](#stage-4-code-generation)
- [Stage 5: Runtime Execution](#stage-5-runtime-execution)
- [Compilation Caching](#compilation-caching)
- [Integration Points](#integration-points)

## Architecture Overview

VFS is a statically-typed expression language designed for real-time evaluation inside a node graph. The pipeline follows a classical compiler structure:

```
Source Text
    │
    ▼
┌──────────┐   Vec<Token>   ┌──────────┐   AST (Expr/Block/FunctionDef)
│  Lexer   │ ────────────►  │  Parser  │ ────────────────────────────►
└──────────┘                 └──────────┘
                                              │
                                              ▼
                              ┌────────────────────┐   Vec<DslType> (type table)
                              │   Type Checker      │ ──────────────────────────►
                              └────────────────────┘
                                              │
                                              ▼
                              ┌────────────────────┐   Native function pointer
                              │  Cranelift Codegen  │ ──────────────────────────►
                              └────────────────────┘
                                              │
                                              ▼
                              ┌────────────────────┐
                              │  Runtime Execution  │  ← DslContext (slots + time)
                              └────────────────────┘
```

Three compilation entry points exist, corresponding to three ways VFS code appears in the application:

| Entry Point | Use Case | Input | Parser |
|---|---|---|---|
| `compile_expression` | Port expressions (e.g., `sin(time * 2.0) * 50.0`) | Single expression | `parse_expression()` |
| `compile_program` | Full function definitions | `fn name(...) -> Type { ... }` | `parse_program()` |
| `compile_node_script` | VFS Code node bodies, Map node bodies | Bare statements + tail expression | `parse_script()` |

All three produce the same output type: an `extern "C" fn(*mut DslContext) -> f64` function pointer.

**Crate location:** `crates/vector-flow-dsl/`

**Key files:**

| File | Purpose |
|---|---|
| `token.rs` | Token types (TokenKind enum) |
| `span.rs` | Source positions (Span, Spanned wrapper) |
| `lexer.rs` | Tokenizer |
| `ast.rs` | AST node definitions |
| `parser.rs` | Recursive descent parser |
| `type_check.rs` | Type checker with built-in function registry |
| `codegen.rs` | Cranelift JIT code generator |
| `runtime.rs` | Runtime intrinsic functions (extern "C") |
| `cache.rs` | Thread-safe compilation cache |
| `error.rs` | Error types |

---

## Stage 1: Lexing

**File:** `lexer.rs`, `token.rs`, `span.rs`

The lexer converts source text into a flat sequence of tokens. It is a single-pass, character-at-a-time state machine operating on the source bytes.

### Token Types

```rust
pub enum TokenKind {
    // Literals
    IntLit(i64),        // 42, -7
    FloatLit(f64),      // 3.14, 1.5e-3
    BoolLit(bool),      // true, false

    // Identifier
    Ident(String),      // x, my_var, sin

    // Keywords
    Fn, Let, For, In, If, Else, Return, As,

    // Operators
    Plus, Minus, Star, Slash, Percent,  // + - * / %
    Eq, Ne, Lt, Le, Gt, Ge,            // == != < <= > >=
    And, Or, Not,                       // && || !
    Assign,                             // =

    // Delimiters
    LParen, RParen,     // ( )
    LBrace, RBrace,     // { }
    LBracket, RBracket, // [ ]

    // Punctuation
    Comma, Colon, Semicolon,  // , : ;
    Arrow,     // ->
    DotDot,    // ..
    Dot,       // .

    Eof,
}
```

### Source Position Tracking

Every token carries a `Span` recording its position in source:

```rust
pub struct Span {
    pub start: usize,  // byte offset (inclusive)
    pub end: usize,    // byte offset (exclusive)
    pub line: u32,     // 1-based line number
    pub col: u32,      // 1-based column number
}
```

Spans can be merged to cover a range of tokens (e.g., an entire binary expression). A `Span::dummy()` constructor exists for synthesized nodes that have no source position.

### Lexer Implementation

The `Lexer` struct holds:
- `src: &[u8]` — source bytes
- `pos: usize` — current byte offset
- `line: u32` — current line (1-based)
- `col: u32` — current column (1-based)

The public entry point is `tokenize(source: &str) -> Result<Vec<Token>, DslError>`, which calls `next_token()` in a loop until `Eof`.

**Token recognition rules:**

1. **Whitespace and comments** are skipped before each token. Comments use `//` line comments.
2. **Numbers**: If the character is a digit, the lexer reads all digits. It then checks for a `.` followed by another digit to distinguish floats from the `..` range operator. Scientific notation (`e`/`E` with optional `+`/`-`) is supported for floats.
3. **Identifiers and keywords**: Alphabetic or `_` characters start an identifier. The completed identifier string is checked against a keyword table (`keyword_lookup()`). `true`/`false` are recognized as `BoolLit` keywords.
4. **Multi-character operators**: `==`, `!=`, `<=`, `>=`, `&&`, `||`, `->`, `..` are recognized by peeking at the next character after consuming the first.
5. **Single-character tokens**: All remaining operators and delimiters are single characters.

**Keyword lookup:**

```rust
pub fn keyword_lookup(ident: &str) -> Option<TokenKind> {
    match ident {
        "fn" => Fn, "let" => Let, "for" => For, "in" => In,
        "if" => If, "else" => Else, "return" => Return, "as" => As,
        "true" => BoolLit(true), "false" => BoolLit(false),
        _ => None,
    }
}
```

### Spanned Wrapper

AST nodes are wrapped in `Spanned<T>`, which carries the span plus a unique integer ID:

```rust
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
    pub id: u32,  // assigned during parsing, used by type checker
}
```

The `id` is allocated sequentially by the parser (`next_id` counter) and used by the type checker to build a type table indexed by expression ID.

---

## Stage 2: Parsing

**File:** `parser.rs`, `ast.rs`

The parser is a hand-written recursive descent parser. It consumes the token vector produced by the lexer and builds a typed AST.

### Type System

```rust
pub enum DslType {
    Scalar,   // f64
    Int,      // i64
    Bool,     // bool (i8 in codegen)
    Vec2,     // (reserved, not yet fully implemented)
    Points,   // (reserved)
    Path,     // (reserved)
    Color,    // 4×f64 (RGBA) stored in DslContext slots
    Unknown,  // placeholder before type resolution
}
```

### AST Node Types

**Expressions:**

```rust
pub enum Expr {
    Literal(Literal),                         // 42, 3.14, true
    Variable(String),                         // x, time
    BinaryOp { op: BinOp, left, right },      // a + b
    UnaryOp { op: UnaryOp, operand },         // -x, !flag
    Call { name: String, args: Vec<...> },     // sin(x)
    Index { collection, index },              // arr[i]
    FieldAccess { object, field: String },    // v.x
    Cast { expr, target: DslType },           // x as Int
    If { condition, then_branch, else_branch }, // if c { a } else { b }
}
```

**Statements:**

```rust
pub enum Statement {
    Let { name, type_annotation: Option<DslType>, value },  // let x: Scalar = 1.0;
    Assign { target: AssignTarget, value },                  // x = 2.0;
    For { var, start, end, body: Block },                    // for i in 0..10 { ... }
    If { condition, then_branch, else_branch },              // if c { ... } else { ... }
    Return(Expr),                                            // return x;
    Expr(Expr),                                              // sin(x);
}
```

**Blocks:**

```rust
pub struct Block {
    pub statements: Vec<Spanned<Statement>>,
    pub tail_expr: Option<Spanned<Expr>>,  // last expression without semicolon
}
```

A block's value is its tail expression. For example, in `{ let x = 1.0; x + 1.0 }`, the block evaluates to `x + 1.0`. If the last statement ends with a semicolon, there is no tail expression and the block has no value.

**Function definitions:**

```rust
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<Param>,       // (name, type) pairs
    pub return_type: DslType,
    pub body: Block,
    pub span: Span,
}
```

### Three Entry Points

1. **`parse_expression(source)`**: Tokenizes the source, then parses a single expression. Used for node port input fields (e.g., `sin(time * 2.0) * 50.0`).

2. **`parse_program(source)`**: Tokenizes, then parses `fn name(params) -> ReturnType { body }`. Used for full function definitions.

3. **`parse_script(source)`**: Tokenizes, then parses bare statements with an optional tail expression (no `fn` wrapper). Used for VFS Code node bodies and Map node scripts. The parser detects a tail expression by checking if `Eof` follows the last expression (no semicolon required at the end).

### Operator Precedence

Operator precedence is implemented through a chain of parsing functions, each calling the next-higher-precedence function:

| Precedence | Operators | Parser function |
|---|---|---|
| 1 (lowest) | `\|\|` | `parse_or_expr()` |
| 2 | `&&` | `parse_and_expr()` |
| 3 | `==` `!=` | `parse_equality_expr()` |
| 4 | `<` `<=` `>` `>=` | `parse_comparison_expr()` |
| 5 | `+` `-` | `parse_additive_expr()` |
| 6 | `*` `/` `%` | `parse_multiplicative_expr()` |
| 7 | `-` (negate) `!` (not) | `parse_unary_expr()` |
| 8 | `as` (cast) | `parse_cast_expr()` |
| 9 (highest) | `()` call, `[]` index, `.` field | `parse_postfix_expr()` |

Each level parses its operator in a left-associative loop: parse the higher-precedence operand, then while the next token is a matching operator, consume it and parse another higher-precedence operand.

### Parser State

```
pos: usize      // index into token array
next_id: u32    // unique ID counter for Spanned nodes
tokens: Vec<Token>
```

The parser uses single-token lookahead via `peek()` and `peek_span()`. The grammar is LL(1) — one token of lookahead is always sufficient.

### Statement Parsing Details

- **Let**: `let name [: Type] = expr ;`
- **Assign**: `name = expr ;` or `name[index].field = expr ;`
- **For**: `for var in start..end { body }`
- **If**: `if condition { body } [else { body }]`
- **Return**: `return expr ;`
- **Expression statement**: Any expression followed by `;`

The parser distinguishes `Let` from `Assign` by checking for the `let` keyword. It distinguishes `Assign` from an expression statement by looking for `=` after the identifier (being careful not to confuse with `==`).

---

## Stage 3: Type Checking

**File:** `type_check.rs`

The type checker walks the AST, resolves expression types, validates operations, and produces a type table. It does NOT modify the AST — it produces a parallel `Vec<DslType>` indexed by the `id` field on each `Spanned` node.

### TypeChecker Structure

```rust
pub struct TypeChecker {
    builtins: HashMap<&'static str, BuiltinSig>,  // function registry
    scopes: Vec<HashMap<String, DslType>>,         // scope stack
    types: Vec<DslType>,                            // indexed by expr ID
    errors: Vec<DslError>,
}
```

The scope stack supports nested scopes (pushed for `for` loop bodies and `if` blocks). Variable lookup walks from innermost to outermost scope.

### Three Check Methods

1. **`check_expression(expr)`**: For standalone port expressions. Adds built-in time constants (`time: Scalar`, `frame: Int`, `fps: Scalar`, `PI`, `TAU`, `E`) to the global scope.

2. **`check_function(func_def)`**: For full programs. Loads function parameters into the initial scope. Validates that the body's return type matches the declared return type.

3. **`check_script(block, inputs, outputs)`**: For VFS Code node bodies. Pre-declares input variables as read-only and output variables as assignable. Returns the type table.

### Built-in Functions

The type checker maintains a registry of all built-in functions with their parameter types and return types:

**Math (Scalar → Scalar):**
- 1-arg: `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `sqrt`, `abs`, `floor`, `ceil`, `round`, `fract`, `exp`, `ln`, `sign`
- 2-arg: `min`, `max`, `pow`, `atan2`, `step`, `fmod`
- 3-arg: `lerp`, `clamp`, `smoothstep`

**Procedural:**
- `rand(Int) -> Scalar` — deterministic hash-based random
- `noise(Scalar, Scalar) -> Scalar` — 2D value noise

**Integer:**
- `iabs(Int) -> Int`, `imin(Int, Int) -> Int`, `imax(Int, Int) -> Int`

**Color construction:**
- `rgb(Scalar, Scalar, Scalar) -> Color` — r, g, b in [0, 1]
- `rgba(Scalar, Scalar, Scalar, Scalar) -> Color` — r, g, b, a in [0, 1]
- `hsl(Scalar, Scalar, Scalar) -> Color` — h in [0, 360], s in [0, 100], l in [0, 100]
- `hsla(Scalar, Scalar, Scalar, Scalar) -> Color` — h, s, l, a

**Color component extractors (Color → Scalar):**
- `color_r`, `color_g`, `color_b`, `color_a` — direct RGBA channel access
- `color_hue`, `color_sat`, `color_light` — convert to HSL internally

**Color modification (Color, Scalar → Color):**
- `set_lightness`, `set_saturation`, `set_hue`, `set_alpha_color`

**Built-in constants:**
- `time` (Scalar) — current time in seconds
- `frame` (Int) — current frame number
- `fps` (Scalar) — playback rate
- `PI`, `TAU`, `E` (Scalar)

### Type Rules

| Operation | Types | Result |
|---|---|---|
| `+` `-` `*` `/` `%` | Scalar × Scalar | Scalar |
| `+` `-` `*` `/` `%` | Int × Int | Int |
| `+` `-` `*` `/` `%` | Int × Scalar or Scalar × Int | Scalar (Int promoted) |
| `<` `<=` `>` `>=` `==` `!=` | Scalar/Int × Scalar/Int | Bool |
| `&&` `\|\|` | Bool × Bool | Bool |
| `!` | Bool | Bool |
| `-` (negate) | Scalar | Scalar |
| `-` (negate) | Int | Int |
| `as Int` | Scalar | Int |
| `as Scalar` | Int | Scalar |
| Function call | (checked against registry) | (per function) |
| If expression | then: T, else: T | T (must match) |

**Type promotion**: `Int` is automatically promoted to `Scalar` when a function expects `Scalar` or in mixed binary operations. This is the only implicit conversion.

### Error Handling

Type errors are collected (not immediately fatal) and reported after the full walk. This allows the checker to report multiple errors at once. Errors include the `Span` of the offending expression for diagnostic display.

---

## Stage 4: Code Generation

**File:** `codegen.rs`

The code generator translates the type-checked AST into Cranelift IR, which is then JIT-compiled to native machine code. Every compiled function has the same calling convention:

```rust
pub type ExprFnPtr = unsafe extern "C" fn(*mut DslContext) -> f64;
```

The single parameter is a pointer to `DslContext` (the slot-based I/O mechanism). The return value is `f64` — for expressions this is the result; for scripts it is always `0.0` (outputs are written to slots as a side effect).

### DslCompiler Structure

```rust
pub struct DslCompiler {
    module: JITModule,                          // Cranelift JIT module
    ctx: Context,                               // Cranelift compilation context
    func_builder_ctx: FunctionBuilderContext,    // reusable builder context
    func_counter: usize,                        // unique function name counter
    runtime_funcs: HashMap<String, FuncId>,     // cached runtime function declarations
}
```

**Initialization:**
1. Create a Cranelift ISA (instruction set architecture) targeting the host CPU with `opt_level = speed` and `is_pic = false`.
2. Build a `JITBuilder` and register all runtime symbols (math intrinsics, color functions) by name and function pointer.
3. Create the `JITModule` from the builder.

**Calling convention:** `SystemV` on Unix, `WindowsFastcall` on Windows.

### CodegenCtx (Per-Function State)

Each function compilation creates a temporary `CodegenCtx`:

```rust
struct CodegenCtx<'a> {
    builder: &'a mut FunctionBuilder<'a>,     // Cranelift IR builder
    module: &'a mut JITModule,                // for declaring function refs
    ctx_ptr: Value,                            // pointer to DslContext (parameter)
    variables: HashMap<String, (Variable, DslType)>,  // scalar/int/bool vars
    color_vars: HashMap<String, u32>,          // color var name → slot index
    next_color_slot: u32,                      // allocator for color temporaries
    last_color_slot: Option<u32>,              // side-channel for color expressions
    runtime_funcs: &'a mut HashMap<String, FuncId>,  // cached function IDs
    type_table: &'a [DslType],                 // from type checker
}
```

### Type Mapping

| VFS Type | Cranelift Type | Storage |
|---|---|---|
| Scalar | `F64` | Cranelift `Variable` |
| Int | `I64` | Cranelift `Variable` |
| Bool | `I8` | Cranelift `Variable` |
| Color | 4 × `F64` | 4 consecutive slots in `DslContext.slots[]` |

Colors are special: they are NOT represented as Cranelift variables. Instead, each color value occupies 4 consecutive `f64` slots in the `DslContext.slots` array. The `color_vars` map tracks which slot index each color variable starts at. The `next_color_slot` counter allocates new 4-slot groups for temporaries.

### DslContext Memory Layout

```rust
pub struct DslContext {
    pub slots: [f64; 16],          // general-purpose f64 array
    pub overflow_ptr: *mut f64,    // heap storage for >16 locals
    pub overflow_len: u32,
    pub _pad0: u32,
    pub frame: u64,                // current frame number
    pub time_secs: f32,            // elapsed time in seconds
    pub fps: f32,                  // playback rate
}
```

**Slot allocation convention for node scripts:**
- Slots `0..n_inputs` — input values (loaded at function entry)
- Slots `n_inputs..n_inputs+n_outputs` — output values (written before return)
- Slots `n_inputs+n_outputs..16` — temporary values (color intermediates)

Color values consume 4 consecutive slots each (R, G, B, A as f64).

### Expression Code Generation

The `emit_expr()` method recursively generates Cranelift IR for each AST node:

**Literals:**
- `Float(f)` → `f64const(f)`
- `Int(i)` → `iconst(I64, i)`
- `Bool(b)` → `iconst(I8, b as i64)`

**Variables:**
- Scalar/Int/Bool: `use_var(variable)` (Cranelift SSA variable read)
- Color: return a dummy `f64const(0.0)` and set `last_color_slot` to the variable's slot index (side-channel mechanism)

**Binary operations:**
- Type-check both sides. If mixed Int/Scalar, promote Int via `fcvt_from_sint`.
- Scalar ops use `fadd`, `fsub`, `fmul`, `fdiv`; Int ops use `iadd`, `isub`, `imul`, `sdiv`.
- Comparisons use `fcmp`/`icmp` with appropriate condition codes, producing `I8` (0 or 1).
- Logical `&&`/`||` use `band`/`bor` on `I8` values.

**Unary operations:**
- Negate: `fneg` (Scalar) or `ineg` (Int)
- Not: `bxor` with 1 (`I8`)

**Function calls:**
1. Look up the function name in `runtime_funcs` cache. If not found, declare it in the JIT module with the appropriate signature and cache the `FuncId`.
2. Create a `FuncRef` via `module.declare_func_in_func()`.
3. Emit `call(func_ref, args)`.
4. For color-returning functions (rgb, hsl, etc.), the call writes to slots instead of returning a value — the destination slot index is passed as an extra argument.

**Color handling (the side-channel pattern):**
- Color constructors (`rgb`, `hsl`, etc.) allocate a 4-slot temporary, pass the slot index to the runtime function, and record it in `last_color_slot`.
- Color variable reads set `last_color_slot` to the variable's slot index.
- Color extractors (`color_r`, etc.) read `last_color_slot` to know which slots to read from, then emit a call to the corresponding runtime extractor function.
- Color modifiers (`set_lightness`, etc.) allocate a new 4-slot temporary, pass both source and destination slot indices to the runtime function.

**Cast expressions:**
- `Scalar → Int`: `fcvt_to_sint_sat(I64, value)` (saturating conversion)
- `Int → Scalar`: `fcvt_from_sint(F64, value)`

**If expressions:**
- Create three basic blocks: `then_block`, `else_block`, `merge_block`.
- Branch on condition. Each branch emits its body.
- `merge_block` takes a block argument (Cranelift's version of a phi node) to unify the two results.

### Statement Code Generation

**Let:**
1. Evaluate the RHS expression.
2. Declare a new Cranelift `Variable` with the appropriate type.
3. For Color, register in `color_vars` and copy via the color copy runtime function.
4. For other types, `def_var(variable, value)`.

**Assign:**
1. Evaluate the RHS.
2. For Color targets, emit a color copy from the source slot to the target slot.
3. For other types, `def_var(existing_variable, new_value)`.

**For loop:**
1. Create blocks: `header`, `body`, `exit`.
2. `header`: block argument for loop variable (phi). Compare against end value; branch to body or exit.
3. `body`: push new scope, bind loop variable, emit body statements, increment loop variable, jump back to header.
4. `exit`: continue.

**Return:**
1. Evaluate the return expression.
2. Coerce to `f64` if needed (Int → fcvt_from_sint, Bool → uextend + fcvt_from_sint).
3. Emit `return_(&[value])`.

### Script Compilation (compile_script_to_fn)

This is the most common path (VFS Code and Map nodes):

1. **Create function**: Declare a new function with signature `(ptr) -> f64`, create entry block, get `ctx_ptr` from block parameters.

2. **Compute slot layout**: Count how many slots each input and output needs (1 for Scalar/Int/Bool, 4 for Color). Set `first_temp_slot` after all I/O slots.

3. **Load inputs**: For each input:
   - Color: register in `color_vars` at the current slot index (data already in slots, written by caller).
   - Other: load `f64` from `ctx.slots[i]` via a memory load instruction. For Int, convert via `fcvt_to_sint_sat`. Declare as a Cranelift variable.

4. **Pre-declare outputs**: For each output:
   - Color: register in `color_vars` at the current slot index.
   - Other: declare Cranelift variable, initialize to 0.

5. **Emit body**: Call `emit_block()` on the script's block.

6. **Handle tail expression**: If the block has a tail expression, assign its value to the first output variable. For Color, this means a color copy from the expression's slot to the output's slot.

7. **Write outputs back**: For each non-Color output, load the Cranelift variable, convert to `f64` (Int → `fcvt_from_sint`, Bool → `uextend` + `fcvt_from_sint`), and store to `ctx.slots[output_slot]` via a memory store instruction.

8. **Return**: Emit `return_(&[f64const(0.0)])`.

9. **Finalize**: Call `builder.finalize()`, `module.define_function()`, `module.clear_context()`, `module.finalize_definitions()`. Return the function pointer via `module.get_finalized_function()`.

### Time Variable Access

Built-in time variables (`time`, `frame`, `fps`) are accessed by loading directly from `DslContext` fields at known offsets using `std::mem::offset_of!`. The `time` and `fps` values are loaded as `f32` and promoted to `f64` via `fpromote`. The `frame` value is loaded as `u64` (`i64` in VFS).

---

## Stage 5: Runtime Execution

**File:** `runtime.rs`

Runtime functions are plain Rust functions with `extern "C"` ABI. They are registered as symbols in the JIT module at compiler initialization and called from generated code via Cranelift function references.

### Symbol Registration

```rust
pub fn runtime_symbols() -> Vec<(&'static str, *const u8)> {
    vec![
        ("vf_sin", vf_sin as *const u8),
        ("vf_cos", vf_cos as *const u8),
        // ... all runtime functions
    ]
}
```

These symbols are passed to `JITBuilder::symbol()` during `DslCompiler::new()`.

### Math Intrinsics

All math functions delegate to Rust's standard library:

```rust
// 1-arg: f64 → f64
pub extern "C" fn vf_sin(x: f64) -> f64 { x.sin() }
pub extern "C" fn vf_cos(x: f64) -> f64 { x.cos() }
// ... 15 total 1-arg functions

// 2-arg: (f64, f64) → f64
pub extern "C" fn vf_min(a: f64, b: f64) -> f64 { a.min(b) }
pub extern "C" fn vf_pow(base: f64, exp: f64) -> f64 { base.powf(exp) }
// ... 6 total 2-arg functions

// 3-arg: (f64, f64, f64) → f64
pub extern "C" fn vf_lerp(a: f64, b: f64, t: f64) -> f64 { a + (b - a) * t }
pub extern "C" fn vf_clamp(x: f64, lo: f64, hi: f64) -> f64 { x.max(lo).min(hi) }
pub extern "C" fn vf_smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)  // Hermite interpolation
}
```

### Procedural Functions

**`vf_rand(seed: u64) -> f64`**: Deterministic hash-based random. Uses xorshift-style mixing (multiply and shift) to map any integer seed to a value in [0, 1). The same seed always produces the same result, making animations reproducible.

**`vf_noise(x: f64, y: f64) -> f64`**: 2D value noise using a Perlin-style interpolation. Computes integer grid cell coordinates, hashes the four corners, and bilinearly interpolates with smoothstep weighting.

### Color Runtime Functions

Color functions receive a `slots_ptr` (pointer to `DslContext.slots`) and slot indices, reading/writing color components directly in the slots array.

**Construction functions** (write 4 f64 values to consecutive slots):
```rust
pub extern "C" fn vf_rgb(slots_ptr: *mut f64, r: f64, g: f64, b: f64, dest_slot: u32)
pub extern "C" fn vf_rgba(slots_ptr: *mut f64, r: f64, g: f64, b: f64, a: f64, dest_slot: u32)
pub extern "C" fn vf_hsl(slots_ptr: *mut f64, h: f64, s: f64, l: f64, dest_slot: u32)
pub extern "C" fn vf_hsla(slots_ptr: *mut f64, h: f64, s: f64, l: f64, a: f64, dest_slot: u32)
```

HSL functions accept h in [0, 360], s and l in [0, 100]. They normalize to [0, 1] internally, convert via `hsl_to_rgb_f64()`, and write RGBA to the destination slots.

**Extractor functions** (read one component from a slot group):
```rust
pub extern "C" fn vf_color_r(slots_ptr: *const f64, src_slot: u32) -> f64  // slots[src]
pub extern "C" fn vf_color_g(slots_ptr: *const f64, src_slot: u32) -> f64  // slots[src+1]
pub extern "C" fn vf_color_b(slots_ptr: *const f64, src_slot: u32) -> f64  // slots[src+2]
pub extern "C" fn vf_color_a(slots_ptr: *const f64, src_slot: u32) -> f64  // slots[src+3]
```

HSL extractors (`vf_color_hue`, `vf_color_sat`, `vf_color_light`) read the RGB values from slots, convert to HSL via `rgb_to_hsl_f64()`, and return the requested component.

**Modifier functions** (read source color, modify one HSL component, write to destination):
```rust
pub extern "C" fn vf_set_lightness(slots_ptr: *mut f64, src_slot: u32, val: f64, dest_slot: u32)
```
These read RGBA from `src_slot`, convert RGB to HSL, replace the target component, convert back to RGB, preserve original alpha, and write to `dest_slot`.

**Copy function:**
```rust
pub extern "C" fn vf_color_copy(slots_ptr: *mut f64, src_slot: u32, dest_slot: u32)
```
Copies 4 consecutive f64 values from source to destination slots.

### Execution Flow

To execute a compiled function:

```rust
// 1. Create context with current time info
let mut ctx = DslContext::new(&eval_context);

// 2. For node scripts, load inputs into slots
ctx.slots[0] = input_x;  // first input
ctx.slots[1] = input_y;  // second input

// 3. Call the compiled function
let func: ExprFnPtr = unsafe { std::mem::transmute(compiled_ptr) };
let result = unsafe { func(&mut ctx) };

// 4. For expressions, `result` is the value
// For node scripts, read outputs from slots
let output = ctx.slots[2];  // first output (after inputs)
```

---

## Compilation Caching

**File:** `cache.rs`

The `DslFunctionCache` prevents recompilation of unchanged source code. This is critical because the graph is re-evaluated every frame — without caching, every VFS expression would be recompiled 30+ times per second.

```rust
pub struct DslFunctionCache {
    entries: DashMap<u64, CacheEntry>,  // thread-safe concurrent map
}

enum CacheEntry {
    Ok(*const u8),   // compiled function pointer
    Err(DslError),   // cached compilation error
}
```

**Cache key computation:**
- For expressions and programs: `DefaultHasher` hash of the source string.
- For node scripts: hash of source string + all input names/types + all output names/types. This ensures that changing a port definition invalidates the cache even if the source is unchanged.

**Error caching:** Compilation errors are also cached. This prevents re-parsing invalid source every frame (which would be expensive and produce the same error).

**Thread safety:** Uses `DashMap` (concurrent hash map) for lock-free reads and minimal contention on writes.

Three methods mirror the three compilation entry points:
- `get_or_compile_expr(source, compiler)`
- `get_or_compile_program(source, compiler)`
- `get_or_compile_node_script(source, inputs, outputs, compiler)`

---

## Integration Points

### Port Expressions

Any node input port can contain a VFS expression instead of a literal value. During graph evaluation, the compute backend:
1. Detects that the port value is an expression string.
2. Calls `cache.get_or_compile_expr(source, compiler)`.
3. Creates a `DslContext` with the current `EvalContext` (frame, time, fps).
4. Calls the compiled function pointer.
5. Uses the returned `f64` as the port value.

### VFS Code Nodes

The VFS Code node (`NodeOp::DslCode`) has:
- `source: String` — the script body
- `script_inputs: Vec<(String, DataType)>` — user-defined input ports
- `script_outputs: Vec<(String, DataType)>` — user-defined output ports

During evaluation:
1. Map `DataType` to `DslType` for each port.
2. Call `cache.get_or_compile_node_script(source, inputs, outputs, compiler)`.
3. Create `DslContext`, load input values into slots.
4. Call the compiled function.
5. Read output values from slots, convert back to `NodeData`.

### Map Nodes

The Map node (`NodeOp::Map`) iterates over a batch input and runs a script per element. It has built-in script inputs:
- `element` (Any) — the current batch element
- `index` (Int) — current iteration index
- `count` (Int) — total batch size

Plus any user-defined extra inputs. For each element:
1. Load `element`, `index`, `count`, and extra inputs into `DslContext` slots.
2. Call the compiled script function.
3. Read outputs from slots.
4. Collect outputs into batch results.
