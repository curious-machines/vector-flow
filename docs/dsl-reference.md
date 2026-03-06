# Vector Flow DSL Reference

The Vector Flow DSL is a small, statically-typed scripting language designed for writing custom node logic. It compiles to native machine code via Cranelift JIT for fast evaluation every frame.

## Table of Contents

- [Overview](#overview)
- [Lexical Elements](#lexical-elements)
  - [Comments](#comments)
  - [Identifiers](#identifiers)
  - [Keywords](#keywords)
  - [Literals](#literals)
  - [Operators](#operators)
  - [Punctuation and Delimiters](#punctuation-and-delimiters)
- [Types](#types)
  - [Type Promotion](#type-promotion)
  - [Type Casting](#type-casting)
- [Expressions](#expressions)
  - [Operator Precedence](#operator-precedence)
  - [Arithmetic Operators](#arithmetic-operators)
  - [Comparison Operators](#comparison-operators)
  - [Logical Operators](#logical-operators)
  - [Unary Operators](#unary-operators)
  - [If Expressions](#if-expressions)
  - [Function Calls](#function-calls)
  - [Indexing and Field Access](#indexing-and-field-access)
- [Statements](#statements)
  - [Let Bindings](#let-bindings)
  - [Assignment](#assignment)
  - [For Loops](#for-loops)
  - [If Statements](#if-statements)
  - [Return Statements](#return-statements)
  - [Expression Statements](#expression-statements)
- [Blocks and Tail Expressions](#blocks-and-tail-expressions)
- [Built-in Constants](#built-in-constants)
- [Global Variables](#global-variables)
- [Built-in Functions](#built-in-functions)
  - [Trigonometric Functions](#trigonometric-functions)
  - [Power and Exponential Functions](#power-and-exponential-functions)
  - [Rounding Functions](#rounding-functions)
  - [Clamping and Interpolation Functions](#clamping-and-interpolation-functions)
  - [Other Math Functions](#other-math-functions)
  - [Integer Functions](#integer-functions)
  - [Procedural Functions](#procedural-functions)
- [Entry Points](#entry-points)
  - [Expression Mode](#expression-mode)
  - [Program Mode](#program-mode)
  - [Script Mode (DSL Node)](#script-mode-dsl-node)
- [Grammar (BNF)](#grammar-bnf)

---

## Overview

The DSL is used in two main contexts within Vector Flow:

1. **Port expressions** -- standalone expressions that compute a value for a single node input (e.g., `sin(time * 2.0) * 50.0`).
2. **DSL Code nodes** -- multi-statement scripts with user-defined inputs and outputs, written in the properties panel.

The language supports integers, floating-point scalars, and booleans, with arithmetic, comparison, logical operators, for-loops, if/else branching, and a library of built-in math functions.

---

## Lexical Elements

### Comments

Line comments begin with `//` and extend to the end of the line. There are no block comments.

```
// This is a comment
let x = 5.0; // inline comment
```

### Identifiers

Identifiers begin with a letter or underscore, followed by letters, digits, or underscores.

```
x
my_var
_temp
value2
```

### Keywords

The following words are reserved and cannot be used as identifiers:

| Keyword  | Purpose                    |
|----------|----------------------------|
| `fn`     | Function definition        |
| `let`    | Variable binding           |
| `for`    | Loop                       |
| `in`     | Loop range                 |
| `if`     | Conditional branch         |
| `else`   | Alternative branch         |
| `return` | Early return               |
| `as`     | Type cast                  |
| `true`   | Boolean literal            |
| `false`  | Boolean literal            |

### Literals

**Integer literals** are sequences of digits, optionally preceded by a minus sign:
```
0
42
-10
```

**Float literals** include a decimal point and/or scientific notation:
```
3.14
1.0
2.5e-3
1e5
```

**Boolean literals** are the keywords `true` and `false`.

### Operators

| Symbol | Name                |
|--------|---------------------|
| `+`    | Addition            |
| `-`    | Subtraction / Negate|
| `*`    | Multiplication      |
| `/`    | Division            |
| `%`    | Modulo              |
| `==`   | Equal               |
| `!=`   | Not equal           |
| `<`    | Less than           |
| `<=`   | Less or equal       |
| `>`    | Greater than        |
| `>=`   | Greater or equal    |
| `&&`   | Logical AND         |
| <code>&#124;&#124;</code> | Logical OR          |
| `!`    | Logical NOT         |
| `=`    | Assignment          |

### Punctuation and Delimiters

| Symbol | Purpose                          |
|--------|----------------------------------|
| `(`    | Open parenthesis                 |
| `)`    | Close parenthesis                |
| `{`    | Open brace (block start)         |
| `}`    | Close brace (block end)          |
| `[`    | Open bracket (indexing)          |
| `]`    | Close bracket                    |
| `,`    | Argument / parameter separator   |
| `:`    | Type annotation separator        |
| `;`    | Statement terminator             |
| `->`   | Return type arrow                |
| `..`   | Range operator (for loops)       |
| `.`    | Field access                     |

---

## Types

The DSL has three core types:

| Type     | Description                    | Example Values        |
|----------|--------------------------------|-----------------------|
| `Scalar` | 64-bit floating-point number   | `3.14`, `0.0`, `-1.5` |
| `Int`    | 64-bit signed integer          | `0`, `42`, `-10`       |
| `Bool`   | Boolean                        | `true`, `false`        |

### Type Promotion

In arithmetic operations, `Int` is automatically promoted to `Scalar` when the other operand is a `Scalar`:

```
let x: Scalar = 2.5;
let n: Int = 3;
let result = x * n;  // n promoted to Scalar, result is Scalar
```

No other implicit promotions occur.

### Type Casting

Use the `as` keyword for explicit type conversion:

```
let x: Scalar = 3.7;
let n: Int = x as Int;      // truncates to 3
let y: Scalar = n as Scalar; // converts to 3.0
```

---

## Expressions

### Operator Precedence

From lowest to highest precedence:

| Precedence | Operators       | Associativity | Description          |
|------------|-----------------|---------------|----------------------|
| 1          | <code>&#124;&#124;</code> | Left          | Logical OR           |
| 2          | `&&`            | Left          | Logical AND          |
| 3          | `==` `!=`       | Left          | Equality             |
| 4          | `<` `<=` `>` `>=` | Left       | Comparison           |
| 5          | `+` `-`         | Left          | Addition/Subtraction |
| 6          | `*` `/` `%`     | Left          | Multiply/Divide/Mod  |
| 7          | `-` `!`         | Prefix        | Unary negate/not     |
| 8          | `as`            | Left          | Type cast            |
| 9          | `()` `[]` `.`   | Postfix       | Call, index, field   |

Parentheses can override precedence:

```
(1.0 + 2.0) * 3.0   // 9.0, not 7.0
```

### Arithmetic Operators

| Operator | Operation      | Operand Types           | Result Type |
|----------|----------------|-------------------------|-------------|
| `+`      | Addition       | Scalar+Scalar, Int+Int  | Same type   |
| `-`      | Subtraction    | Scalar-Scalar, Int-Int  | Same type   |
| `*`      | Multiplication | Scalar*Scalar, Int*Int  | Same type   |
| `/`      | Division       | Scalar/Scalar, Int/Int  | Same type   |
| `%`      | Modulo         | Scalar%Scalar, Int%Int  | Same type   |

When one operand is Int and the other is Scalar, the Int is promoted to Scalar and the result is Scalar.

```
let a = 10.0 + 5.0;   // 15.0
let b = 10 / 3;       // 3 (integer division)
let c = 10.0 / 3;     // 3.333... (3 promoted to Scalar)
let d = 7 % 3;        // 1
let e = 7.5 % 2.0;    // 1.5
```

### Comparison Operators

All comparison operators return `Bool`:

| Operator | Meaning          |
|----------|------------------|
| `==`     | Equal            |
| `!=`     | Not equal        |
| `<`      | Less than        |
| `<=`     | Less or equal    |
| `>`      | Greater than     |
| `>=`     | Greater or equal |

```
let is_positive = x > 0.0;       // Bool
let in_range = x >= 0.0 && x <= 1.0;
```

### Logical Operators

| Operator | Operation   | Operand Types | Result |
|----------|-------------|---------------|--------|
| `&&`     | Logical AND | Bool, Bool    | Bool   |
| <code>&#124;&#124;</code> | Logical OR  | Bool, Bool    | Bool   |
| `!`      | Logical NOT | Bool          | Bool   |

```
let valid = x > 0.0 && x < 100.0;
let either = a || b;
let inverted = !flag;
```

### Unary Operators

| Operator | Operation | Operand Types    |
|----------|-----------|------------------|
| `-`      | Negate    | Scalar or Int    |
| `!`      | Not       | Bool             |

```
let neg = -x;
let opposite = !condition;
```

### If Expressions

`if` can be used as an expression that returns a value. Both branches must have the same type:

```
let sign = if x > 0.0 { 1.0 } else { -1.0 };
```

`else if` chains are supported:

```
let category = if x < 0.0 {
    -1.0
} else if x == 0.0 {
    0.0
} else {
    1.0
};
```

### Function Calls

Call built-in functions by name with parenthesized arguments:

```
sin(x)
lerp(a, b, 0.5)
clamp(value, 0.0, 1.0)
```

See [Built-in Functions](#built-in-functions) for the full list.

### Indexing and Field Access

Indexing uses brackets, field access uses dot notation:

```
points[i].x
collection[0].field
```

These are primarily used in assignment targets for structured data.

---

## Statements

### Let Bindings

Declare a new variable with `let`. Type annotations are optional -- the type is inferred from the initializer when omitted:

```
let x = 5.0;              // inferred as Scalar
let n: Int = 10;           // explicit type
let flag: Bool = true;     // explicit Bool
```

Variables are scoped to the enclosing block.

### Assignment

Assign a new value to an existing variable:

```
let x = 0.0;
x = 5.0;
```

Indexed field assignment is also supported:

```
data[i].value = 42.0;
```

### For Loops

Iterate over an exclusive integer range with `for..in`:

```
for i in 0..10 {
    // i goes from 0 to 9
}
```

The loop variable is implicitly typed as `Int`. The range endpoint is **exclusive** (the loop runs from `start` up to but not including `end`).

```
// Sum integers from 0 to n-1
let total: Scalar = 0.0;
for i in 0..n {
    total = total + i as Scalar;
}
```

### If Statements

`if` can also be used as a statement (without producing a value):

```
if x > threshold {
    result = 1.0;
} else {
    result = 0.0;
}
```

`else if` chains work as expected:

```
if x < 0.0 {
    category = -1;
} else if x == 0.0 {
    category = 0;
} else {
    category = 1;
}
```

### Return Statements

Exit a function early and return a value:

```
fn safe_divide(a: Scalar, b: Scalar) -> Scalar {
    if b == 0.0 {
        return 0.0;
    }
    a / b
}
```

### Expression Statements

Any expression followed by a semicolon is an expression statement. The value is discarded:

```
sin(x);  // evaluated but result discarded
```

---

## Blocks and Tail Expressions

A block is a sequence of statements enclosed in braces `{ }`. The last expression in a block, if it does **not** end with a semicolon, becomes the block's **tail expression** -- the value the block evaluates to.

```
let result = {
    let a = 3.0;
    let b = 4.0;
    sqrt(a * a + b * b)   // no semicolon: this is the tail expression
};
// result == 5.0
```

If a block has no tail expression (all statements end with `;`), it evaluates to `0.0` (Scalar) by default.

This tail expression rule is how DSL Code nodes return their output -- the last expression in the script becomes the value of the first output port.

---

## Built-in Constants

These constants are always available:

| Name  | Value                  | Type   |
|-------|------------------------|--------|
| `PI`  | 3.14159265358979...    | Scalar |
| `TAU` | 6.28318530717959...    | Scalar |
| `E`   | 2.71828182845905...    | Scalar |

```
let circumference = TAU * radius;
let half_turn = PI;
```

## Global Variables

These variables are automatically available and reflect the current animation state:

| Name    | Type   | Description                    |
|---------|--------|--------------------------------|
| `time`  | Scalar | Current time in seconds        |
| `frame` | Int    | Current frame number           |
| `fps`   | Scalar | Frames per second              |

```
let angle = time * 360.0;          // full rotation per second
let pulse = sin(time * TAU);       // oscillation
let phase = frame as Scalar / fps; // same as time
```

---

## Built-in Functions

### Trigonometric Functions

All angles are in **radians**.

| Function    | Signature              | Description                          |
|-------------|------------------------|--------------------------------------|
| `sin(x)`    | Scalar -> Scalar       | Sine                                 |
| `cos(x)`    | Scalar -> Scalar       | Cosine                               |
| `tan(x)`    | Scalar -> Scalar       | Tangent                              |
| `asin(x)`   | Scalar -> Scalar       | Inverse sine (result in radians)     |
| `acos(x)`   | Scalar -> Scalar       | Inverse cosine                       |
| `atan(x)`   | Scalar -> Scalar       | Inverse tangent                      |
| `atan2(y,x)`| Scalar, Scalar -> Scalar | Two-argument arctangent            |

```
let y = sin(time * TAU);            // oscillate -1..1
let angle = atan2(dy, dx);          // angle from deltas
let radians = acos(0.5);            // PI/3
```

### Power and Exponential Functions

| Function    | Signature                | Description               |
|-------------|--------------------------|---------------------------|
| `sqrt(x)`   | Scalar -> Scalar         | Square root               |
| `pow(x, y)` | Scalar, Scalar -> Scalar | x raised to the power y   |
| `exp(x)`    | Scalar -> Scalar         | e^x                       |
| `ln(x)`     | Scalar -> Scalar         | Natural logarithm         |

```
let dist = sqrt(x * x + y * y);
let cubed = pow(x, 3.0);
let growth = exp(rate * time);
```

### Rounding Functions

| Function    | Signature        | Description                                |
|-------------|------------------|--------------------------------------------|
| `floor(x)`  | Scalar -> Scalar | Round down to nearest integer              |
| `ceil(x)`   | Scalar -> Scalar | Round up to nearest integer                |
| `round(x)`  | Scalar -> Scalar | Round to nearest integer                   |
| `fract(x)`  | Scalar -> Scalar | Fractional part (x - floor(x))            |

```
let snapped = floor(x / grid) * grid;   // snap to grid
let t = fract(time);                     // repeating 0..1 ramp
```

### Clamping and Interpolation Functions

| Function                  | Signature                          | Description                              |
|---------------------------|------------------------------------|------------------------------------------|
| `min(a, b)`               | Scalar, Scalar -> Scalar           | Minimum of two values                    |
| `max(a, b)`               | Scalar, Scalar -> Scalar           | Maximum of two values                    |
| `clamp(x, lo, hi)`        | Scalar, Scalar, Scalar -> Scalar   | Clamp x to range [lo, hi]               |
| `lerp(a, b, t)`           | Scalar, Scalar, Scalar -> Scalar   | Linear interpolation: a + (b-a)*t       |
| `smoothstep(e0, e1, x)`   | Scalar, Scalar, Scalar -> Scalar   | Hermite interpolation (smooth 0..1)     |
| `step(edge, x)`           | Scalar, Scalar -> Scalar           | 0.0 if x < edge, else 1.0              |

```
let bounded = clamp(value, 0.0, 1.0);
let mid = lerp(start, end, 0.5);
let smooth = smoothstep(0.0, 1.0, t);   // smooth ease-in/out
let on_off = step(0.5, t);              // hard threshold
```

### Other Math Functions

| Function    | Signature                | Description                            |
|-------------|--------------------------|----------------------------------------|
| `abs(x)`    | Scalar -> Scalar         | Absolute value                         |
| `sign(x)`   | Scalar -> Scalar         | -1.0, 0.0, or 1.0                     |
| `fmod(x,y)` | Scalar, Scalar -> Scalar | Floating-point remainder               |

```
let magnitude = abs(x);
let direction = sign(velocity);
let wrapped = fmod(angle, TAU);
```

### Integer Functions

| Function       | Signature          | Description          |
|----------------|--------------------|----------------------|
| `iabs(x)`      | Int -> Int         | Absolute value       |
| `imin(a, b)`   | Int, Int -> Int    | Minimum              |
| `imax(a, b)`   | Int, Int -> Int    | Maximum              |

```
let positive_n = iabs(n);
let capped = imin(count, 100);
```

### Procedural Functions

| Function         | Signature                | Description                                 |
|------------------|--------------------------|---------------------------------------------|
| `rand(seed)`     | Int -> Scalar            | Deterministic random value in [0, 1)        |
| `noise(x, y)`   | Scalar, Scalar -> Scalar | 2D value noise (smooth, deterministic)      |

```
let r = rand(frame);                     // different each frame, repeatable
let n = noise(x * 0.01, y * 0.01);      // smooth spatial noise
let animated = noise(x * 0.01, time);    // animated noise
```

`rand` uses a hash-based PRNG -- the same seed always produces the same value. Use `frame` or varying integers for animation.

`noise` produces smooth, continuous values suitable for organic motion and textures.

---

## Entry Points

The DSL has three parsing modes, each suited to different contexts.

### Expression Mode

Parses a single expression. Used for port expressions on node inputs.

```
sin(time * 2.0) * 50.0
```

```
if frame % 2 == 0 { 1.0 } else { 0.0 }
```

### Program Mode

Parses a full function definition with typed parameters and return type:

```
fn wave(freq: Scalar, amp: Scalar) -> Scalar {
    sin(time * freq * TAU) * amp
}
```

### Script Mode (DSL Node)

Parses bare statements without a function wrapper. This is the mode used by the **DSL Code** node. Inputs and outputs are defined externally in the node's port editor.

Inputs are available as pre-declared variables. Outputs are assigned by name, or the tail expression is assigned to the first output.

**Example -- single output via tail expression:**

Inputs: `freq` (Scalar), `amp` (Scalar)
Outputs: `result` (Scalar)

```
sin(time * freq * TAU) * amp
```

The tail expression (no semicolon) is automatically assigned to the first output (`result`).

**Example -- explicit assignment to output:**

Inputs: `x` (Scalar)
Outputs: `doubled` (Scalar), `tripled` (Scalar)

```
doubled = x * 2.0;
tripled = x * 3.0;
```

**Example -- accumulator pattern:**

Inputs: `n` (Int)
Outputs: `total` (Scalar)

```
let sum: Scalar = 0.0;
for i in 0..n {
    sum = sum + i as Scalar;
}
total = sum;
```

**Example -- conditional logic:**

Inputs: `value` (Scalar), `threshold` (Scalar)
Outputs: `result` (Scalar)

```
if value > threshold {
    result = 1.0;
} else {
    result = 0.0;
}
```

---

## Grammar (BNF)

The following is the formal grammar of the Vector Flow DSL in BNF notation.

### Top-Level

```bnf
<program>    ::= "fn" <ident> "(" <param_list> ")" "->" <type> <block>

<script>     ::= <stmt>* <expr>?

<expression> ::= <expr>
```

### Parameters and Types

```bnf
<param_list> ::= <param> ("," <param>)* | <empty>

<param>      ::= <ident> ":" <type>

<type>       ::= "Scalar" | "Int" | "Bool"
```

### Blocks and Statements

```bnf
<block>      ::= "{" <stmt>* <expr>? "}"

<stmt>       ::= <let_stmt>
               | <assign_stmt>
               | <for_stmt>
               | <if_stmt>
               | <return_stmt>
               | <expr_stmt>

<let_stmt>   ::= "let" <ident> (":" <type>)? "=" <expr> ";"

<assign_stmt>::= <assign_target> "=" <expr> ";"

<assign_target> ::= <ident>
                   | <ident> "[" <expr> "]" "." <ident>

<for_stmt>   ::= "for" <ident> "in" <expr> ".." <expr> <block>

<if_stmt>    ::= "if" <expr> <block> ("else" (<if_stmt> | <block>))?

<return_stmt>::= "return" <expr> ";"

<expr_stmt>  ::= <expr> ";"
```

### Expressions

```bnf
<expr>       ::= <or_expr>

<or_expr>    ::= <and_expr> ("||" <and_expr>)*

<and_expr>   ::= <eq_expr> ("&&" <eq_expr>)*

<eq_expr>    ::= <cmp_expr> (("==" | "!=") <cmp_expr>)*

<cmp_expr>   ::= <add_expr> (("<" | "<=" | ">" | ">=") <add_expr>)*

<add_expr>   ::= <mul_expr> (("+" | "-") <mul_expr>)*

<mul_expr>   ::= <unary_expr> (("*" | "/" | "%") <unary_expr>)*

<unary_expr> ::= ("-" | "!") <unary_expr>
               | <cast_expr>

<cast_expr>  ::= <postfix_expr> ("as" <type>)*

<postfix_expr> ::= <primary_expr> <postfix_op>*

<postfix_op> ::= "(" <arg_list> ")"
               | "[" <expr> "]"
               | "." <ident>

<primary_expr> ::= <int_lit>
                 | <float_lit>
                 | <bool_lit>
                 | <ident>
                 | "if" <expr> <block> "else" (<if_expr> | <block>)
                 | "(" <expr> ")"

<arg_list>   ::= <expr> ("," <expr>)* | <empty>
```

### Lexical Elements

```bnf
<ident>      ::= [a-zA-Z_] [a-zA-Z0-9_]*

<int_lit>    ::= [0-9]+

<float_lit>  ::= [0-9]+ "." [0-9]*
               | [0-9]+ ("." [0-9]*)? [eE] [+-]? [0-9]+

<bool_lit>   ::= "true" | "false"

<comment>    ::= "//" .* <newline>
```
