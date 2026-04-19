# Merged Lands

`merged_lands.exe` is a tool for merging land in TES3 mods.

The output of the tool is a plugin called `Merged Lands.omwaddon` in OpenMW mode, or
`Merged Lands.esp` in classic `--vanilla` mode. It should go at the end of your load order.
Yes, that includes after `Merged Objects.esp` if you're using `TES3Merge`.

The plugin contains a merged representation of any `LAND`, `LTEX`, and `CELL` records edited by mods.

The tool works with both OpenMW (reading `openmw.cfg`) and the original Morrowind engine
(reading `Morrowind.ini`). OpenMW is the default mode. See [OpenMW Support](#openmw-support)
below for details.

## How?

1. The tool builds a "reference" landmass by merging all `.ESM` plugins using a similar algorithm as Morrowind.
2. The tool calculates a "difference" landmass for each mod _with respect to the reference landmass_.
3. The tool copies the "reference" landmass into a new "merged" landmass.
4. For each "difference" landmass from a plugin, the tool merges it into the "merged" landmass. If mods do not overlap with their changes, the resulting terrain will perfectly match both mods' intended changes. If there _is_ overlap, the tool will attempt to resolve the conflicts in an intelligent manner.
5. The "merged" landmass is checked for seams and repaired if necessary.
6. The "merged" landmass is converted into the `TES3` format and saved as a plugin.

## Limitations

- The tool does NOT move entities within the cell. This may result in floating or buried objects. This may include grass from any grass mods, or similar landscape detailing.
- The tool does NOT perform magic. If one mod puts a hill in the exact same spot another mod tries to put a valley, the resulting land will likely be less than appealing.

## Installation & Usage

1. Create a folder for the tool's executable, e.g. `merged_lands_bin`.
2. Create a directory in that folder called `Conflicts`.
3. Place the executable in the `merged_lands_bin` folder.

You should have a directory tree that looks like the following:

```
merged_lands_bin\
    merged_lands.exe
    Conflicts\
```

To run the tool, open a terminal (e.g. `cmd`) in the `merged_lands` directory.

By default, the tool reads your OpenMW configuration from the platform-default `openmw.cfg`
location (respecting `OPENMW_CONFIG` / `OPENMW_CONFIG_DIR`).

```bash
# Default OpenMW behavior
merged_lands_bin> .\merged_lands.exe

# Classic Morrowind / vanilla behavior
merged_lands_bin> .\merged_lands.exe --vanilla --data-files-dir "C:\Program Files (x86)\Steam\steamapps\common\Morrowind\Data Files"
```

An example configuration for `MO2` is shown below.

![example MO2 config](./docs/images/mo2_config.png)

### Outputs

By default, the tool will save the output `Merged Lands.omwaddon` in OpenMW's `data-local`
directory. If `openmw.cfg` sets `data-local=`, that exact path is used; otherwise the tool falls
back to the platform-default OpenMW `data-local` path for the current OS.

In classic Morrowind mode (`--vanilla`), it instead defaults to `Merged Lands.esp` in the
`Data Files` directory.

This can be changed with the `--output-file-dir` and `--output-file` arguments.

If you want a persistent custom output directory without passing `--output-file-dir` every time,
create a file named `merged_lands.toml` inside `--merged-lands-dir` and set:

```toml
output_file_dir = "/absolute/path/to/your/output"
```

Relative paths are also supported and are resolved relative to `--merged-lands-dir`.

### Troubleshooting Merges

The tool will save the log file to the `--merged-lands-dir`. This defaults to `.`, or "the current directory".

The tool will save images to a folder `Conflicts` in the `--merged-lands-dir`.

```
merged_lands_bin\
    merged_lands.exe
    merged_lands.log   <-- Log file.
    Conflicts\
        ...            <-- Images of conflicts.
```

A conflict image shows `green` where changes were merged without any conflicts, whereas `yellow` means a minor conflict occurred, and `red` means a major conflict occurred. 
In addition, the tool creates `MERGED` map showing the final result.

**Note:** Each conflict image is created relative to a specific plugin. This makes it easier to understand how the final land differs from the expectation of each plugin.

![conflict_image](./docs/images/conflict_images.png)

In addition, the tool can be run with the `--add-debug-vertex-colors` switch to color the actual `LAND` records saved in the output file.
This feature can help with understanding where a conflict shown in the `Conflicts` folder actually exists in-game and the severity of it with respect to the world.

![conflict_colors](./docs/images/conflict_vertex_colors.png)

### Other Configuration

Run the tool with `--help` to see a full list of supported arguments.

## OpenMW Support

The tool can discover plugins and data directories by reading an `openmw.cfg` file instead of
`Morrowind.ini`. This is useful if you manage your mods with OpenMW or a mod manager that writes
to `openmw.cfg` (e.g., OpenMW Launcher, MO2 with an OpenMW configuration, Portmod).

### OpenMW is the default

If you do not pass any mode flag, the tool loads the platform-default `openmw.cfg`. The
`OPENMW_CONFIG` and `OPENMW_CONFIG_DIR` environment variables are respected, as they are by
OpenMW itself.

You can still override the config path with `--openmw-cfg <PATH>`, where `<PATH>` may be either
a directory containing `openmw.cfg` or a direct path to the file. Example:

```bash
# Use the platform default (or the OPENMW_CONFIG env var if set)
merged_lands

# Explicit path
merged_lands --openmw-cfg "/home/me/.config/openmw"
merged_lands --openmw-cfg "/home/me/.config/openmw/openmw.cfg"
```

To use classic Morrowind behavior instead, pass `--vanilla`.

### What changes in OpenMW mode

- **Data directories.** Plugins and their `.mergedlands.toml` meta files are searched across every
  `data=` entry from the config chain, in reverse priority order (OpenMW's VFS rule — later `data=`
  lines win). This includes any engine-added entries such as the resources VFS and `data-local`.
- **Load order.** The tool uses the `content=` order from `openmw.cfg` verbatim. No mtime sorting
  is applied, because the cfg's ordering is already the user's authoritative load order.
- **Output location.** If `--output-file-dir` is not set, `Merged Lands.omwaddon` is written to the
  OpenMW `data-local` directory. If `openmw.cfg` omits `data-local=`, the tool uses the
  platform-default OpenMW `data-local` path instead. Classic mode (`--vanilla`) still defaults
  to writing `Merged Lands.esp` in `--data-files-dir`.
- **`--data-files-dir` is only used for plugin discovery in `--vanilla` mode.**

### Persistent output override

If you keep the tool in a dedicated folder, you can put a `merged_lands.toml` file in
`--merged-lands-dir` to set a persistent output directory:

```toml
output_file_dir = "Merged Output"
```

This is only used when `--output-file-dir` is not passed. Relative paths are resolved relative to
`--merged-lands-dir`.

You can still override either source by passing an explicit plugin list on the command line; that
list wins over whatever `openmw.cfg` says.

### ESP-as-master handling

Regardless of mode, the tool inspects each plugin's TES3 header and treats any plugin declared as
a master by another plugin as part of the reference landmass — even if its extension is `.esp`.
This is the correct behavior for mods that ship a parent ESP plus dependent patch ESPs (common
with OpenMW-centric mod compilations), and it matches how OpenMW itself resolves dependencies.

A message is logged at `debug` level when a plugin is promoted from a plugin to a master this way.

## Supporting Patches

The tool will automatically read `.mergedlands.toml` files from the `Data Files` directory.

```bash
Data Files\
    Cantons_on_the_Global_Map_v1.1.esp
    Cantons_on_the_Global_Map_v1.1.mergedlands.toml
```

These files are used to control the tool's behavior.

### Example 1. `Cantons_on_the_Global_Map_v1.1.mergedlands.toml`

This patch file would instruct the tool to ignore all changes made by the mod except for those related to `world_map_data`.
Then, for those changes only, the mod would resolve any conflicts with other mods by using the changes from `Cantons on the Global Map` instead.

```toml
version = "0"
meta_type = "Patch"

[height_map]
included = false

[vertex_colors]
included = false

[texture_indices]
included = false

[world_map_data]
conflict_strategy = "Overwrite"
```

### Example 2. `BCOM_Suran Expansion.mergedlands.toml`

The Beautiful Cities of Morrowind Suran Expansion mod should load after `BCoM`. It modifies the same land, and we would like to prefer the changes from Suran Expanson over the normal `BCoM` edits. We can set each field to `"Overwrite"`.

```toml
version = "0"
meta_type = "Patch"

[height_map]
conflict_strategy = "Overwrite"

[vertex_colors]
conflict_strategy = "Overwrite"

[texture_indices]
conflict_strategy = "Overwrite"

[world_map_data]
conflict_strategy = "Overwrite"
```

The example conflict shown above in [Troubleshooting Merges](#troubleshooting-merges) is now fixed.

![conflict_colors](./docs/images/conflict_vertex_colors_resolved.png)

### Example 3. Ignoring Changes

If we'd like a mod to load after another mod and _not_ try to merge changes where those mods conflict, we can use the `"Ignore"` setting.
For example, if we knew that some mod would overwrite texture changes from an earlier mod, and we wanted to prevent that, we could do the following:

```toml
version = "0"
meta_type = "Patch"

[texture_indices]
conflict_strategy = "Ignore"
```

### Defaults

Each type of `LAND` record is `included = true` and `conflict_strategy = "Auto"` by default. `"Auto"` allows the tool to determine an "optimal" way to resolve conflicts -- whether that means merging, overwriting, or even ignoring the conflict.
You should not write a `.mergedlands.toml` file until it is known to be necessary.
