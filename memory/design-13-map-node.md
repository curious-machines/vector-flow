# Design 13: Map Node + Batch-Aware Styling

## Overview

A Map node that iterates over a batch (Vec<T>), runs embedded DSL code per element, and collects results into a new batch. Combined with batch-aware SetFill/SetStroke/SetAlpha to apply per-element styling to shapes.

## Motivation

CopyToPoints produces a Vec<Shape> where every copy has identical styling. Users need per-copy variation (e.g., random lightness per rectangle). There is no mechanism to apply different attributes to each element of a batch.

## Map Node

### Concept

Map is a node with an embedded code body (same UI as DSL Code). It takes a batch input, iterates over it, executes the code once per element with the current element and iteration metadata available, and collects the per-element output into a new batch.

From the scheduler's perspective, Map is a single node that evaluates once. All iteration is internal to its `evaluate` implementation (same pattern as CopyToPoints looping internally).

### Node Definition

```
NodeOp::Map {
    source: String,           // DSL code body
    script_inputs: Vec<(String, DataType)>,
    script_outputs: Vec<(String, DataType)>,
}
```

Catalog entry creates the node with default ports:
- **Inputs:**
  - `batch` (DataType::Any) — the batch to iterate over
  - (user can add extra input ports for parameters, e.g., base_color, seed)
- **Outputs:**
  - `result` (DataType::Any) — the collected output batch
  - (user can add extra output ports)

Default script ports (injected into the code scope, not graph ports):
  - `element` — current batch element (type matches batch element type)
  - `index` (Int) — current iteration index (0-based)
  - `count` (Int) — total batch length

`index` and `count` are default script inputs that the user can delete if not needed (to save DSL slot space). `element` is always present and cannot be removed.

### Output Type

The output type of the Map node is **declared by the user** on the output port (same as DSL Code — user picks the DataType). This avoids inference complexity. The Map node wraps per-element results into the corresponding batch type:

| Output port type | Per-element code returns | Collected into |
|-----------------|------------------------|----------------|
| Scalar          | f64                    | Scalars        |
| Int             | i64                    | Ints           |
| Color           | Color                  | Colors         |
| Scalars         | (passthrough)          | Scalars        |

Initially we support Scalar, Int, and Color outputs. Shape output (returning modified shapes) is deferred until the DSL supports struct types with property access.

### Property Panel UI

Identical to DSL Code:
- Port editor: add/remove/rename/retype input and output ports
- Code editor: multiline text field for the DSL source
- `element`, `index`, `count` shown as script inputs (removable except `element`)

### Evaluation

A single `DslContext` is allocated once and reused across all iterations. Before each iteration, the input slots are overwritten with the current element, index, and count. The compiled function pointer is the same every iteration — the context is just a scratch pad.

```rust
// Pseudocode for Map evaluation
fn evaluate_map(batch_input, extra_inputs, source, time_ctx) -> batch_output {
    let elements = unwrap_batch(batch_input);
    let count = elements.len();

    let compiled_fn = compile_or_cache(source);

    let mut ctx = DslContext::new(time_ctx);
    let _overflow = if needs_overflow { Some(ctx.alloc_overflow(n)) } else { None };

    // Load extra inputs once (they don't change per iteration)
    load_extra_inputs(&mut ctx, extra_inputs);

    let mut results = Vec::with_capacity(count);
    for (i, elem) in elements.iter().enumerate() {
        // Overwrite per-iteration slots
        load_element_into_slots(&mut ctx, elem);
        ctx.slots[index_slot] = i as f64;
        ctx.slots[count_slot] = count as f64;

        execute_dsl(compiled_fn, &mut ctx);

        results.push(read_output_from_slots(&ctx));
    }

    wrap_as_batch(results) // Scalars, Ints, Colors, etc.
}
```

The compiled DSL function is cached (same as DSL Code caching), so iteration only pays the JIT cost once. Future optimization: parallel execution with rayon, using one DslContext per thread.

### Batch Unwrapping

Map accepts any batch type as input. The element type exposed to the code depends on the batch:

| Input batch type | Element available as | Slots used |
|-----------------|---------------------|------------|
| Scalars         | Scalar (1 slot)     | 1          |
| Ints            | Int (1 slot)        | 1          |
| Colors          | Color (4 slots: r, g, b, a) | 4   |
| Shapes          | (see below)         | —          |
| Paths           | (deferred)          | —          |

For **Shapes** input: since the DSL cannot manipulate Shape structs yet, the element is not directly accessible as typed slots. Instead, the user would rely on `index` and `count` plus extra inputs to compute new values. The primary use case is generating a parallel Colors/Scalars batch from indices, not transforming shapes in-place.

Future: when the DSL gains struct property access, `element.fill.r`, `element.transform`, etc. become available.

### DslContext Slot Expansion

The fixed slot count increases from 8 to 16. This comfortably fits typical Map scripts without overflowing to the heap.

**Cost analysis:**
- Memory: DslContext struct grows from ~96 bytes to ~160 bytes. Trivial — stack-allocated, one per evaluation.
- Performance: Zero impact. Slots are a flat C array accessed by compile-time offset. Unused slots sit idle.
- Codegen: One-time update — `offset_of!` values for `frame`, `time_secs`, `fps` shift since they follow the slots array.
- Benefit: Fewer scripts need heap overflow. A typical Map with element (4 Color slots) + index (1) + count (1) + base_color (4) + result (4) = 14 slots fits within 16.

### Color in the DSL

Color support requires extending the DSL type system. A Color occupies 4 consecutive f64 slots (r, g, b, a — promoted from f32 for DSL arithmetic consistency).

**New built-in functions:**
- `hsl(h, s, l) -> Color` — create Color from HSL (h: 0-360, s: 0-100, l: 0-100), alpha defaults to 1.0
- `hsla(h, s, l, a) -> Color` — same with explicit alpha
- `rgb(r, g, b) -> Color` — create Color from RGB (0.0-1.0 range)
- `rgba(r, g, b, a) -> Color`
- `color_r(c) -> Scalar`, `color_g(c)`, `color_b(c)`, `color_a(c)` — extract components
- `color_hue(c) -> Scalar`, `color_sat(c)`, `color_light(c)` — extract HSL components
- `set_lightness(c, l) -> Color` — return new Color with adjusted lightness
- `set_saturation(c, s) -> Color`
- `set_hue(c, h) -> Color`
- `set_alpha(c, a) -> Color`

These reuse the existing `color_math.rs` functions (rgb_to_hsl, hsl_to_rgb) as runtime intrinsics.

**DslType::Color** already exists in the AST. The codegen and type checker need to handle multi-slot types:
- Variable declaration allocates 4 consecutive slots
- Assignment/load moves 4 values
- Function calls pass/return 4 values for Color args/returns

This is the most significant implementation effort in this design.

### Example: Random Lightness Per Copy

Node graph:
```
[Circle Path] -> [CopyToPoints] -> [SetFill] -> [GraphOutput]
[Rectangle]  -/        |               ^
                       |               |
                  indices (Scalars)     |
                       |               |
                  [Map] ------> colors (Colors)
```

Map node configuration:
- Input ports: `batch` (Scalars, connected to indices), `seed` (Int)
- Output ports: `result` (Color)
- Default script inputs: `element`, `index`, `count`
- Code:
```
h = 200.0
s = 80.0
l = 30.0 + 50.0 * rand(index + seed)
result = hsl(h, s, l)
```

## Batch-Aware SetFill / SetStroke / SetAlpha

### Current Behavior

SetFill takes `(geometry: Any, color: Color)` and applies the single color to all shapes uniformly.

### New Behavior

SetFill, SetStroke, and SetAlpha accept either a single value or a batch for their styling parameter:

**SetFill:**
- `color` input: accepts `Color` (single) or `Colors` (batch)
- If single Color + Shapes: apply uniformly to all (current behavior, unchanged)
- If Colors batch + Shapes: zip 1:1, each shape gets its own fill
- Size mismatch rules:
  - Colors.len() == 1: broadcast (treat as single)
  - Colors.len() == Shapes.len(): zip 1:1
  - Colors.len() < Shapes.len(): cycle the colors list
  - Colors.len() > Shapes.len(): truncate to Shapes.len()

**SetStroke:**
- `color` input: accepts `Color` or `Colors` batch
- Same mismatch rules as SetFill
- Width, cap, join, dash remain single values (applied uniformly)
- Future: could also accept Scalars for per-element width

**SetAlpha:**
- `alpha` input: accepts `Scalar` or `Scalars` batch
- Same mismatch rules

### Implementation

In `styling.rs`, the existing `set_fill` function signature changes:

```rust
// Before:
pub fn set_fill(data: &NodeData, color: Color) -> NodeData

// After — overloaded via the color input NodeData:
pub fn set_fill(data: &NodeData, color_data: &NodeData) -> NodeData
```

When `color_data` is `NodeData::Color(c)`, current behavior. When `color_data` is `NodeData::Colors(colors)`, zip with shapes using cycling.

The node's `evaluate` in `mod.rs` passes the raw color NodeData through instead of extracting a single Color.

### Port Type

The `color` input port on SetFill/SetStroke changes from `DataType::Color` to `DataType::Any` (or we add a union concept). Simplest approach: keep it as `DataType::Color` and rely on `can_promote_to` — add `(Colors, Color)` as a valid promotion so Colors batch can connect to a Color port. The compute code then checks at runtime whether it received Color or Colors.

Alternative: change the port to `DataType::Any` and validate in compute. Less type-safe but simpler.

## Implementation Plan

### Phase 1: Batch-Aware SetFill/SetStroke/SetAlpha (no dependencies)
1. Modify `set_fill()` to accept `&NodeData` for color (single or batch, with cycling)
2. Modify `set_stroke()` similarly
3. Modify `set_alpha()` similarly
4. Update `evaluate_node` calls to pass raw NodeData
5. Port type adjustment (Colors -> Color promotion)
6. Tests: single color (unchanged behavior), Colors batch, cycling, size mismatch

### Phase 2: DSL Color Type (parallel with Phase 1, no dependencies)
1. Bump DslContext slots from 8 to 16; update `offset_of!` in codegen
2. Multi-slot variable allocation in codegen (Color var = 4 consecutive slots)
3. Type checker: Color propagation, function signatures
4. Runtime intrinsics: `vf_hsl`, `vf_rgba`, `vf_color_r/g/b/a`, `vf_color_hue/sat/light`, `vf_set_lightness/saturation/hue/alpha` (wraps existing color_math.rs)
5. Tests: Color construction, extraction, manipulation, multi-slot arithmetic

### Phase 3: Map Node (depends on Phase 2)
1. `NodeOp::Map` variant in core
2. `NodeDef::map()` catalog entry with default ports
3. Evaluation in compute: batch unwrap, single DslContext reused per iteration, result collection
4. App UI: property panel (reuse DSL Code panel), catalog entry under Utility category
5. Tests: iterate Scalars, iterate Ints, iterate with Color output, empty batch, extra inputs

### Phase 4: Integration Testing (depends on Phase 1 + Phase 3)
1. CopyToPoints -> Map -> SetFill end-to-end
2. Cycling colors across shapes
3. Manual testing in app

## Deferred

- Map over Shapes with struct property access (needs DSL struct types)
- Map over Paths
- Per-element stroke width (Scalars input on SetStroke)
- Nested Map (map within map)
- Map with multiple batch inputs (zip two batches)
- Parallel execution with rayon (one DslContext per thread)
- Classes/templates for reusable Map configurations
