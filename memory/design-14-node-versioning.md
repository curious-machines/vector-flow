# Design 14: Node Versioning

## Problem

When a node's definition changes (ports added/removed/reordered, behavior changed), saved projects still contain the old definition. On load, the old `NodeDef` is deserialized as-is with no reconciliation against the current catalog. This causes:

- **Silent wrong results**: `evaluate_node` indexes inputs by position; if ports shift, values are misread
- **Missing ports**: new ports won't appear on old nodes
- **Potential panics**: if code expects more inputs than the saved node provides

Currently the only fix is for the user to notice something looks wrong, delete the old node, and create a new one manually.

## Design Options

### Option A: Version Detection (MVP — implement first)

- Add `version: u32` to `NodeDef` (serde default = 0 for backward compat with existing files)
- Each catalog factory function sets the current version for that `NodeOp`
- On project load, compare each node's saved version against the current catalog version
- Mismatched nodes get a visual indicator (warning icon, tinted border, tooltip)
- User manually replaces outdated nodes

**Pros**: Minimal implementation effort, solves the "silent breakage" problem by making it visible.
**Cons**: No automatic repair — user must manually recreate nodes.

### Option B: Version + Automatic Migration (target for post-alpha)

- Same version integer as Option A
- Register migration functions per NodeOp: `(old_version, new_version) -> fn(NodeDef) -> NodeDef`
- On project load, run migrations sequentially (v1 -> v2 -> v3)
- Migrations can add ports with defaults, remove ports, reorder ports, update connections

**Breaking vs. non-breaking changes**:
- Non-breaking: adding a port at the end with a sensible default (existing connections unaffected since they're by index)
- Breaking: removing a port, reordering ports, changing a port's type (may need to disconnect wires)

**Migration function responsibilities**:
- Transform the `inputs`/`outputs` Vec<PortDef> to match the new definition
- Preserve `ParamValue` literals and `expression` strings where ports survive
- For removed ports that had connections, collect warnings to show the user
- For added ports, insert with default values

**On-load flow**:
1. Deserialize project as normal
2. Walk all nodes, check version against current catalog
3. For each outdated node, look up migration chain and apply
4. Collect any warnings (broken connections, removed ports)
5. Show summary dialog to user if there were breaking migrations

### Option C: Keep Old Implementations (Houdini-style, not planned)

- Maintain old `evaluate_node` code paths per version
- Runtime dispatch based on node version
- Allows old projects to render identically forever

**Not pursuing**: Too much complexity for current project stage. Would require versioned compute dispatch, growing code surface indefinitely.

## Current Implementation Plan

Implement **Option A** now. This gives us:
1. Visibility into which nodes are outdated
2. A `version` field already in the serialized format, so Option B can build on it later without another migration
3. No extra maintenance burden during rapid development — we're not writing migration functions for APIs that are still changing

Move to **Option B** once the node API surface stabilizes (post-alpha).

## Implementation Notes for Option A

- `NodeDef.version` field with `#[serde(default)]` for backward compat
- Need a way to look up "current version" for a given `NodeOp` — could be a function on `NodeOp` or a lookup table
- On load, iterate `graph.nodes` and compare versions
- Store mismatch state somewhere accessible to the UI (e.g., on `UiNode` or a separate set)
- Visual treatment: warning badge in node header, tooltip explaining the mismatch
- Consider: should outdated nodes still evaluate? Probably yes (best-effort), but with the warning visible

## Open Questions

- Should version be on `NodeOp` (variant-level) or `NodeDef` (instance-level)? Variant-level makes more sense since the version tracks the definition schema, not per-instance state.
- How to handle DslCode/Map/Generate nodes whose ports are user-defined? These probably don't need versioning in the same way — their "schema" is stored per-instance.
- Should we bump version for behavior-only changes (same ports, different computation)? Probably yes for Option B, but for Option A it's less clear since the node would still "work" with old ports.
