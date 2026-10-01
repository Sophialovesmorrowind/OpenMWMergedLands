# README

Merged Lands combines terrain changes from your Morrowind mods and repairs gaps
between landscape cells. It reads OpenMW's load order automatically and writes
one `Merged Lands.omwaddon` plugin. Classic Morrowind is also supported with
`--vanilla`.

## Installation

Choose the download button for your operating system and architecture. Extract
the ZIP and open its `merged_lands` folder. Each archive contains the executable,
README, license, build information, and a `Conflicts` folder.

- **Windows:** `merged_lands.exe`
- **Linux, macOS, and Android:** `merged_lands`

Use **macOS ARM64** for Apple Silicon Macs and **macOS Intel** for Intel Macs.
Each release has SHA-256 and SHA-512 checksum files for all six platform ZIPs.
Dev builds come from successful GitHub Actions runs and require GitHub sign-in.
The development artifact contains the platform ZIP and its checksums.

On Linux or macOS, make the executable runnable:

```sh
chmod +x merged_lands
./merged_lands --help
```

On Windows, open PowerShell in the extracted folder:

```powershell
.\merged_lands.exe --help
```

The Android build runs in a terminal environment with access to your OpenMW
files. Keep the executable in an executable app-private directory, such as
Termux's home directory.

## Usage

Run Merged Lands after installing your mods and finalizing their load order.
No options are needed for a standard OpenMW installation.

Linux, macOS, or Android:

```sh
./merged_lands
```

Windows (PowerShell):

```powershell
.\merged_lands.exe
```

The tool discovers OpenMW's engine configuration, user configuration, and nested
`config=` profiles. It follows `content=` load order and supports
`OPENMW_CONFIG` and `OPENMW_CONFIG_DIR` overrides.

The generated plugin goes into OpenMW's `data-local` directory. Add it to the
**end of your content load order**, including after `Merged Objects.esp` if you
use TES3Merge:

```ini
content=Merged Lands.omwaddon
```

Rerun Merged Lands whenever your mods or load order change. Previously generated
Merged Lands plugins are excluded from merge inputs.

For a custom OpenMW profile or output directory:

```sh
./merged_lands --openmw-cfg /path/to/openmw.cfg --output-file-dir /path/to/output
```

Windows:

```powershell
.\merged_lands.exe --openmw-cfg 'C:\Games\OpenMW Profile\openmw.cfg' --output-file-dir 'C:\Games\Merged Lands\output'
```

For classic Morrowind, use `--vanilla` and your `Data Files` directory. The
default output filename becomes `Merged Lands.esp`:

```powershell
.\merged_lands.exe --vanilla --data-files-dir 'C:\Games\Morrowind\Data Files'
```

## Configuration

On first run, Merged Lands creates `merged_lands.toml` in OpenMW's configuration
directory. Existing settings are preserved. If that directory cannot be used,
the tool falls back to the executable's directory; `--config-dir` selects a
custom configuration directory.

Default locations:

- **Linux:** `$XDG_CONFIG_HOME/openmw` or `~/.config/openmw`
- **macOS:** `~/Library/Preferences/openmw`
- **Windows:** `Documents\My Games\OpenMW`
- **Android:** `/storage/emulated/0/Alpha3/config`

For persistent custom paths, edit the TOML. Use single-quoted strings for
Windows paths:

```toml
openmw_cfg = '/path/to/openmw.cfg'
output_file_dir = '/path/to/output'
```

First-run defaults exclude these generated or unrelated plugins:

```toml
ignore_plugins = [
    "delta-merged.omwaddon",
    "deleted_groundcover.omwaddon",
    "groundcover.omwaddon",
    "S3LightFixes.omwaddon",
    "OMWLLFMod.omwaddon",
    "merged.omwaddon",
    "Merged Objects.esp",
]
```

New configurations also skip TR's four Hunza Camp terrain cells before merging
and seam repair. Existing configurations can add the same entry, after all
top-level settings:

```toml
[cell_ignore_by_plugin]
"TR_Mainland.esm" = [[-9, -46], [-9, -47], [-10, -46], [-10, -47]]
```

This exception leaves TR's original terrain in place when it is the only
height-map contributor. Other plugins can still contribute to these coordinates.
Use `cell_ignore` when an entire cell must be excluded from generated output.
See the [Hunza Camp investigation](https://github.com/Sophialovesmorrowind/OpenMWMergedLands/blob/main/docs/hunza-camp.md)
for the source comparison and scope of the exception.

Per-plugin `.mergedlands.toml` sidecars control which terrain fields participate
and how conflicts are resolved. See the
[full project README](https://github.com/Sophialovesmorrowind/OpenMWMergedLands/blob/main/README.md)
for all options, examples, and precedence rules.

## How it works

Merged Lands compares each plugin's terrain with its reference, combines the
changes in load order, repairs mismatched boundaries, and removes unnecessary
overrides. The result contains `LAND` records and the `LTEX` records they need.
It does not emit `CELL` records.

Mods that place a hill and a valley in the same location can still conflict.
The tool changes terrain; it does not move statics, NPCs, grass, or other placed
objects to match the new surface. Review the generated conflict images and
check affected areas in-game.

## Development

Build from source with the Rust version pinned in `rust-toolchain.toml`:

```sh
cargo build --release --locked
cargo test --locked
cargo clippy --workspace --all-targets --all-features -- -W clippy::pedantic -D warnings
```

The release pipeline builds Linux x86-64 and ARM64, macOS Intel and Apple
Silicon, Windows x86-64, and Android ARM64. Each platform archive has both
SHA-256 and SHA-512 checksums.

See the [performance notes](https://github.com/Sophialovesmorrowind/OpenMWMergedLands/blob/main/docs/performance.md)
and [OpenMW configuration compatibility notes](https://github.com/Sophialovesmorrowind/OpenMWMergedLands/blob/main/docs/openmw-config-compatibility.md).

## Credits

Merged Lands is MIT-licensed. Thanks to the original Merged Lands authors, the
OpenMW project, the TES3 library contributors, and Modding-OpenMW.

This website follows the layout and styling of
[GOCoverify](https://modding-openmw.gitlab.io/gocoverify/), using Soupault,
Sakura, and the MOMW Mod Template. See the [website licenses](/licenses/).
