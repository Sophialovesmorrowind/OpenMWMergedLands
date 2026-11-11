# Changelog

#### Unreleased

- Omitted LAND overrides whose only change is adding white vertex colors where OpenMW already defaults to white.

#### 1.3

##### Terrain merging

- Repaired shared terrain before VHGT encoding and rejected unrepresentable height steps instead of silently clamping them.
- Preserved fractional height offsets and kept connected cells on a common height phase.
- Made cleanup compare against OpenMW's whole LAND record winner and carried untouched winning layers into generated overrides.
- Preserved present zero-valued terrain layers when later LAND records omit them.
- Preserved independent authored normals, including normal-only patches and vectors synthesized by `Resolve`, while their source geometry still applies.
- Respected case-insensitive texture IDs, LTEX-only overrides, and overrides that clear a texture filename.

##### Memory use

- Allocated optional terrain grids only when present.
- Applied and released plugin diffs as they are read. Their history remains available when conflict images or debug vertex colors require it.
- See the [terrain audit and ten-run comparison](https://github.com/Sophialovesmorrowind/OpenMWMergedLands/blob/main/docs/terrain-storage-audit.md) for measured results and validation limits.

##### Maintenance

- Updated `anyhow`, `bitflags`, `clap`, `image`, `log`, `serde`, `toml`, and `windows-sys`, plus the CI dependency cache action.
- Kept cell-coordinate validation strict with the new TOML parser and retained the required LAND flag traits after the bitflags upgrade.

#### 1.1

##### Recent changes

- Added a default, plugin-specific exception for TR's four Hunza Camp cells.
- Preserved original TR heights when earlier plugins supply texture-only LAND.
- Fixed concurrent test fixtures sharing temporary directories on macOS.
- Added Cargo dependency caching for checks and platform builds.
- Verified every release includes all six archives and both checksum files.
- Fixed cross-platform path and filename-case assertions.
- Pinned matching Rust and Clippy versions for local work and CI.

#### 1.0

##### OpenMW support

- OpenMW mode is the default; `--vanilla` selects classic Morrowind behavior.
- Configuration discovery follows OpenMW's engine and user locations.
- Nested `config=` profiles, replacement directives, and relative paths follow
  the declaring configuration.
- OpenMW load order comes from `content=` entries.
- Configuration is auto-detected without an initial mode selector.
- Generated outputs contain landscape and texture records without CELL records.
- Added global and per-plugin cell exclusions.
- Optimized plugin resolution, terrain merging, and seam repair.

For version-specific notes and downloads, see the
[GitHub releases](https://github.com/Sophialovesmorrowind/OpenMWMergedLands/releases).

[View source history](https://github.com/Sophialovesmorrowind/OpenMWMergedLands/commits/main)
