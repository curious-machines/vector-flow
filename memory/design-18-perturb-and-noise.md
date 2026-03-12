# Design 18: Perturb Points Node + Standalone Noise Node

## Overview

Two new nodes that share a noise evaluation module:
- **Perturb Points** (Path Ops) — displaces geometry points by random/noise offsets
- **Noise** (Generators) — samples a noise field at point positions, outputs scalar values

## Design Decisions

### Perturb Points

- **Three perturbation methods** (combobox stored as i32): 0=Uniform, 1=Gaussian, 2=Noise
- **Four handle target modes** (combobox stored as i32): 0=Anchors Only, 1=Handles Only, 2=Both, 3=Anchors + Coherent Handles
- **Radial vs per-axis toggle** (`per_axis: bool`): single `amount` or `amount_x`/`amount_y`
- **Preserve smoothness** (`preserve_smoothness: bool`): constrains handle perturbation to length-only
- **Programmatic port visibility**: hide/show ports based on mode selections
- **Operates on**: PathData, Paths, PointBatch, Shape, Shapes

### Handle Logic

| Target mode | Anchor behavior | Handle behavior |
|---|---|---|
| 0: Anchors Only | Perturbed | Dragged by same delta (offset preserved) |
| 1: Handles Only | Unchanged | Perturbed independently |
| 2: Both | Perturbed | Perturbed independently (separate seed offsets) |

### Preserve Smoothness (modes 1, 2, 3)

- Compute direction and length from anchor to handle
- Use only scalar magnitude of displacement to adjust length
- New handle = anchor + direction * (length + scalar_perturbation)

### Noise Node

- Takes PointBatch input, samples FBM noise at each point
- Parameters: seed, frequency, octaves, lacunarity, amplitude, offset_x, offset_y
- Outputs: Scalars (one value per point)

### Shared Module

`cpu/noise.rs` contains:
- PRNG functions (splitmix64, rand_pair, box_muller)
- Displacement functions (uniform/gaussian × radial/per-axis)
- Noise displacement via `noise` crate (Fbm<OpenSimplex>)
- Batch noise sampling for standalone Noise node

### Dependencies

- `noise = "0.9"` added to vector-flow-compute
