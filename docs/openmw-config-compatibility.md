# OpenMW configuration compatibility

Audit date: 2026-10-01. The executable oracle was Linux OpenMW **0.51.0**, revision
**f4bec41444**. The current upstream configuration manager was also reviewed. Windows,
macOS, Android, and distribution-specific OpenMW builds were not executable-tested.

Primary references:

- [Configuration traversal, composing values, replacement, tokens, and path quoting](https://github.com/OpenMW/openmw/blob/master/components/files/configurationmanager.cpp)
- [Engine content handling and data-local insertion](https://github.com/OpenMW/openmw/blob/master/apps/openmw/main.cpp)
- [Fixed token directories](https://github.com/OpenMW/openmw/blob/master/components/files/fixedpath.hpp)
- [Linux path defaults and build-defined global directories](https://github.com/OpenMW/openmw/blob/master/components/files/linuxpath.cpp)
- [Installed base config template](https://github.com/OpenMW/openmw/blob/master/files/openmw.cfg)

The full-modlist benchmark also checked TES3 dependency handling against the
installed engine commit: [master validation](https://github.com/OpenMW/openmw/blob/f4bec41444214a7903bebd178389ca22ca13f646/apps/openmw/mwworld/esmloader.cpp#L46-L55)
rejects missing masters and masters loaded later. The engine does not reorder
`content=` entries to satisfy those dependencies. Our validation retains those
checks, but applies them to filtered merge inputs; excluding an active master
can therefore report it as missing even when the engine's unfiltered order has it.
- [Official path/profile documentation](https://openmw.readthedocs.io/en/latest/reference/modding/paths.html)

The loader implements the LAND-relevant options: `config`, `data`, `content`, `data-local`,
`resources`, and `replace`. It follows the selected root's config chain. Automatic discovery
prefers an installed engine-local config, then the stock Unix `/etc/openmw/openmw.cfg`.
The base config itself must name any user/profile config it wants to include. Its default
template names `?userconfig?` and supplies the standard `data-local` and resources settings.

## Verified behavior and corrections

| Case | Engine behavior and repository coverage |
| --- | --- |
| Nested siblings and descendants | `root`, `first`, `first/nested`, `second`; later sources have higher priority. Existing ordering regression retained; actual engine load messages confirm depth-first order. The prose documentation currently gives a different example; the source and executable are authoritative for this audit. |
| `replace=config` | Discards already loaded intermediate layers, preserves the original base, and continues pending siblings. Existing regression retained. |
| `replace=data` / `replace=content` | Removes lower-priority composing values, including when the replacing layer supplies no values. Existing regressions retained. |
| `replace=replace` | Cancels lower layers' replacement directives. Changed to the engine's reverse merge; new regression restores root lists after an intermediate replacement. |
| Singleton path options | `data-local` or `resources` repeated in one file is an error; different layers can override the value. Corrected and covered by source-location tests. |
| Quoted content | `content="Quoted.esp"` keeps the quote characters. Removed the tool's former quote-stripping extension and updated fixtures. |
| Quoted paths | Ampersand escapes the next character; backslashes are literal; trailing text after a closing quote is ignored. Existing quoting tests cover these rules. |
| Empty paths | `data=` and `data-local=` resolve to their declaring directory. New regression distinguishes this from an unknown closed token, which clears the value. |
| Tokens | `?userconfig?mods` appends `mods`; on Unix `?userconfig?/mods` resolves to `/mods`, and a backslash suffix remains literal. Corrected and covered. `?unterminated` stays unchanged; an unknown closed token clears the path. |
| BOM and section prefixes | A BOM remains part of an option name; `[.]` is a nonempty section prefix. Corrected: BOM-prefixed `data` is ignored and section options are not mistaken for root options. |
| Symlink-relative paths | A symlinked config resolves relative data/config paths from the spelling used to reach it. Corrected canonicalization of the declaring file; new tests also preserve distinct sibling symlink aliases. |
| Optional configs | Missing config files, directories named `openmw.cfg`, and non-directory config paths are skipped. The selected root must be a file. |
| Data resolution | Later data directories win across case variants, including nested components. New resolver regressions cover cold and indexed directory lookup and coexisting exact-case spellings. |

An unknown `resources` token clears the resource path. Consequently its engine VFS suffix
is `vfs` relative to the process working directory; the loader includes it if that directory
exists. An unknown `data-local` token clears the inherited value and contributes no data
directory. Tokens reference fixed platform paths, independently of `user-data` and the active
profile. Global config paths and the `?global?` data token use separate directories.

## Reproducing the Linux oracle

Create a temporary config directory and run this command from a terminal. Duplicate sentinel
content makes OpenMW finish option processing before starting the game or graphics. Put all
XDG paths and `--user-data` under the same temporary directory so the oracle's logs and
screenshots directory are isolated.

```sh
XDG_CONFIG_HOME=/tmp/openmw-audit/xdg-config \
XDG_DATA_HOME=/tmp/openmw-audit/xdg-data \
XDG_CACHE_HOME=/tmp/openmw-audit/cache \
OPENMW_DISABLE_CRASH_CATCHER=1 \
openmw --replace config --config /tmp/openmw-audit/config \
  --user-data /tmp/openmw-audit/user-data \
  --content Sentinel.esp --content Sentinel.esp
```

Representative inputs and observations:

```text
data-local=one
data-local=two
=> option 'data-local' cannot be specified more than once

data=?userconfig?/probe
=> No such dir: /probe

data=?userconfig?probe
=> No such dir: <XDG_CONFIG_HOME>/openmw/probe

data=?unterminated
=> No such dir: ?unterminated

content="Quoted.esp"
content="Quoted.esp"
=> Content file specified more than once: "Quoted.esp"
```

For the replacement test, root declares `content=Root.esm`, `data=RootData`, and `config=first`;
first declares replacements for content/data, its own values, and `config=nested`; nested
declares `replace=replace` and its own values. Missing-directory warnings include RootData,
FirstData, and LastData in that order. Append `--content Root.esm` to the oracle command to
confirm that root content was restored: the first duplicate reported is Root.esm.

Repository verification: `cargo test --locked io::openmw_cfg` and
`cargo test --locked io::openmw_paths` cover the loader and platform discovery fixtures.

## Deliberate scope and remaining differences

- Explicit `--openmw-cfg`, saved paths, and environment overrides select a standalone root;
  they do not implicitly preload engine config as OpenMW's own `--config` argument does.
  `OPENMW_CONFIG`, `OPENMW_CONFIG_DIR`, tilde expansion, and the existing Flatpak/Alpha3
  discovery behavior are tool extensions.
- Without an installed engine base config, the tool accepts a standalone user config and
  supplies the standard data-local fallback. OpenMW itself requires a local/global base.
- The tool skips ancestor cycles by canonical identity while keeping distinct sibling aliases.
  This bounds cycles containing `..` or symlinks that the engine's lexical directory set may
  otherwise revisit. Invalid unknown config tokens are skipped rather than attempting a
  config lookup in the current directory.
- Only the relevant options are interpreted. The tool does not duplicate OpenMW's validation
  of unrelated settings, game startup checks, builtin script insertion, or content-duplicate
  rejection. Unreadable existing configs produce an actionable error rather than silently
  omitting their settings.
- Global directories in OpenMW are downstream build choices. `/etc/openmw` and
  `/usr/share/games/openmw` are stock Unix assumptions; explicit paths can be used for custom
  builds. Other platform defaults have fixture coverage, not a native executable oracle.
- Parsing is read-only. The tool filters nonexistent data directories, including resource VFS
  paths; it does not create OpenMW's config or screenshot directories while reading settings.

These checks establish compatibility for the documented option subset and nested profiles;
they do not claim equivalence with the entire OpenMW configuration/runtime system.
