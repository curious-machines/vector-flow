# Vector Flow - Project Conventions

## Workflow

- **Do not make code changes unless explicitly asked.** When the user is discussing how things work, asking questions, or exploring design options, respond with explanations only. Wait for a clear instruction to implement before editing any files.

## Ordering

- **Menu entries** (node catalog in `ui_node.rs`) must be sorted alphabetically within each category.
- **Documentation sections** (e.g., node entries in `node-reference.md`) must be sorted alphabetically within each parent section.
- **Table of contents** entries must match the order of the sections they reference.

## Documentation

- When adding a new node, also add its entry to `docs/node-reference.md` and update `docs/app-guide.md` category lists.
- The scripting language is called **Vector Flow Script (VFS)**, not DSL. The code node is called **VFS Code**.
- The language reference is `docs/vfs-reference.md`.

## Testing

- All new features must include tests.
- All bug fixes must include a test that verifies the fix to prevent regressions.

## Design Documents

- Design documents live in the `memory/` folder and must never be deleted.
- When discussing a new design topic, save the discussion to a new file in `memory/` (e.g., `memory/design-NN-topic.md`).

## Building

- Cargo and rustc are at `~/.cargo/bin/cargo`.
- Run `~/.cargo/bin/cargo build` to build and `~/.cargo/bin/cargo test` to run tests.
- The project must have zero clippy warnings and all tests passing before committing.
