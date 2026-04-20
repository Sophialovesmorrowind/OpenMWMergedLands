# AGENTS

## Repo Reality Check (verify first)
- This snapshot is a single Rust crate (`Cargo.toml`) named `merged_lands`.
- `rust-toolchain.toml` pins `stable`; `Cargo.toml` requires Rust `1.88.0`.
- Build uses `build.rs` + `shadow-rs` for build metadata.
- `Cargo.toml` uses `openmw-config = "1.0.1"` from crates.io.
- Current tracked tree does **not** include `src/` (confirm with `git ls-tree --name-only -r HEAD`), so normal `cargo build/test/run` will fail until sources are restored.

## Commands Agents Should Use
- Quick capability check once sources are restored: `cargo metadata --format-version 1`.
- If sources are restored: `cargo build` then `cargo run -- --help`.
- Runtime mode is OpenMW by default; use `--vanilla` to switch to classic Morrowind behavior.

## Runtime/Filesystem Quirks
- Tool expects/creates working-dir artifacts: `Conflicts/` and `merged_lands.log`.
- `.gitignore` excludes: `target/*`, `Data Files/*`, `Maps/*`, `Conflicts/*`, `*.ini`, `merged_lands.log`.
- Default output file differs by mode:
  - OpenMW mode: `Merged Lands.omwaddon` in OpenMW `data-local`.
  - `--vanilla` mode: `Merged Lands.esp` in `Data Files` (or explicit `--output-file-dir`).

## Domain-Specific Behavior Worth Remembering
- OpenMW mode reads `openmw.cfg` (respects `OPENMW_CONFIG` / `OPENMW_CONFIG_DIR`).
- OpenMW load order comes from `content=` in `openmw.cfg` (no mtime sorting).
- `.mergedlands.toml` sidecar files next to plugins control merge inclusion/conflict strategy.

## Missing Repo Automation
- No `.github/workflows/` found.
- No existing repo instruction files found (`AGENTS.md`, `CLAUDE.md`, `.cursorrules`, `.cursor/rules`, `.github/copilot-instructions.md`, `opencode.json`).
