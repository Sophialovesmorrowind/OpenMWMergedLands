# AGENTS

## Repo Reality Check (verify first)
- This snapshot is a single Rust crate (`Cargo.toml`) named `merged_lands`.
- `rust-toolchain.toml` pins Rust `1.99.0` with rustfmt and Clippy for local work and CI; `Cargo.toml` requires Rust `1.99.0`.
- OpenMW config parsing and discovery live in `src/io/openmw_cfg.rs` and `src/io/openmw_paths.rs`; there is no external config parser dependency.
- Current tracked tree includes `src/`, so normal Rust checks should work.
- The current checkout is OpenMW-first: default mode reads `openmw.cfg`; use `--vanilla` for classic Morrowind behavior.

## Commands Agents Should Use
- Quick capability check: `cargo metadata --format-version 1`.
- Core verification: `cargo fmt --check`, `cargo test --locked`, and `cargo clippy --workspace --all-targets --all-features -- -W clippy::pedantic -D warnings`.
- CLI smoke check: `cargo run -- --help`.
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

## Repo Automation
- `.github/workflows/release.yaml` owns checks, native desktop builds/tests, Android cross-builds, and tagged GitHub releases.
- Package checks: `python3 -m unittest discover -s ci -p 'test_*.py'`.
- Packaging: `python3 ci/package.py <platform> <rust-target> --version <version>` after a release build for that target.
