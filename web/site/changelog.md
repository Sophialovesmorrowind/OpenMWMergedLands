# Changelog

## Recent changes

- Added a default, plugin-specific exception for TR's four Hunza Camp cells.
- Preserved original TR heights when earlier plugins supply texture-only LAND.
- Fixed concurrent test fixtures sharing temporary directories on macOS.
- Added Cargo dependency caching for checks and platform builds.
- Verified every release includes all six archives and both checksum files.
- Fixed cross-platform path and filename-case assertions.
- Pinned matching Rust and Clippy versions for local work and CI.

## OpenMW support

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
