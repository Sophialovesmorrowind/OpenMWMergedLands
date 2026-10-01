//! Read the settings needed for LAND merging from an `OpenMW` configuration chain.
//!
//! Semantics are based on `OpenMW`'s components/files/configurationmanager.cpp and
//! configfileparser.cpp. This is an independent implementation, not an INI parser.

use super::openmw_paths::{OpenMWPaths, config_file_path};
use anyhow::{Context, Result, anyhow, bail};
use log::{debug, warn};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub enum OpenMWCfgSource {
    /// Environment overrides, then engine base config, then the standard user location.
    Default,
    /// An explicit config file or directory containing openmw.cfg.
    Path(PathBuf),
}

pub struct OpenMWConfig {
    pub root_config_file: PathBuf,
    pub data_directories: Vec<PathBuf>,
    pub content: Vec<String>,
    pub data_local: PathBuf,
}

#[derive(Default)]
struct Layer {
    data: Vec<PathBuf>,
    content: Vec<String>,
    configs: Vec<PathBuf>,
    replace: HashSet<String>,
    data_local: Option<PathBuf>,
    resources: Option<PathBuf>,
}

impl OpenMWConfig {
    pub fn load(source: OpenMWCfgSource) -> Result<Self> {
        let paths = OpenMWPaths::from_env()?;
        let root = match source {
            OpenMWCfgSource::Default => paths.discover(&|name| std::env::var_os(name)),
            OpenMWCfgSource::Path(path) => paths.expand_home(&path),
        };
        Self::load_with_paths(&root, &paths)
    }

    fn load_with_paths(root: &Path, paths: &OpenMWPaths) -> Result<Self> {
        let root = config_file_path(root)?;
        let mut pending = vec![(root.clone(), Vec::new())];
        let mut visited = HashSet::new();
        let mut layers = Vec::new();
        while let Some((file, mut ancestors)) = pending.pop() {
            // Config directories may exist without an openmw.cfg. The selected root must exist.
            let identity = match fs::canonicalize(&file) {
                Ok(identity) => identity,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) && file != root =>
                {
                    debug!("Skipping missing config {}", file.display());
                    continue;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| anyhow!("Unable to resolve {}", file.display()));
                }
            };
            // Keep each branch's path spelling for relative paths, including symlink aliases.
            // Canonical identities are only used to stop actual ancestor cycles safely.
            if ancestors.contains(&identity) || !visited.insert(file.clone()) {
                warn!("Skipping repeated OpenMW config {}", file.display());
                continue;
            }
            if !identity.is_file() && file != root {
                debug!("Skipping non-file config {}", file.display());
                continue;
            }
            ancestors.push(identity);
            debug!("Reading OpenMW config {}", file.display());
            let text = fs::read_to_string(&file)
                .with_context(|| anyhow!("Unable to read {}", file.display()))?;
            let layer = parse_layer(&text, &file, paths)?;
            if layer.replace.contains("config") && layers.len() > 1 {
                layers.truncate(1);
            }
            // OpenMW pushes directories in reverse, so the first listed directory and its
            // descendants are visited before the next sibling. Higher layers append last.
            pending.extend(
                layer
                    .configs
                    .iter()
                    .rev()
                    .map(|dir| (dir.join("openmw.cfg"), ancestors.clone())),
            );
            layers.push(layer);
        }

        // OpenMW merges from high to low priority. The accumulated replace options belong
        // to higher layers, and replace=replace prevents lower replacement directives from
        // being inherited. A forward clear-and-append pass misses that interaction.
        let mut merged = Layer::default();
        for mut layer in layers.into_iter().rev() {
            if !merged.replace.contains("data") {
                layer.data.append(&mut merged.data);
                merged.data = layer.data;
            }
            if !merged.replace.contains("content") {
                layer.content.append(&mut merged.content);
                merged.content = layer.content;
            }
            merged.data_local = merged.data_local.or(layer.data_local);
            merged.resources = merged.resources.or(layer.resources);
            if !merged.replace.contains("replace") {
                merged.replace.extend(layer.replace);
            }
        }
        let mut config = Self {
            root_config_file: root,
            data_directories: merged.data,
            content: merged.content,
            data_local: merged
                .data_local
                .unwrap_or_else(|| paths.user_data.join("data")),
        };
        if let Some(resources) = merged.resources {
            config.data_directories.insert(0, resources.join("vfs"));
        }
        if !config.data_local.as_os_str().is_empty() {
            config.data_directories.push(config.data_local.clone());
        }
        config.data_directories.retain(|dir| {
            if dir.is_dir() {
                true
            } else {
                debug!("Skipping missing OpenMW data directory {}", dir.display());
                false
            }
        });
        Ok(config)
    }
}

fn parse_layer(text: &str, file: &Path, paths: &OpenMWPaths) -> Result<Layer> {
    let mut layer = Layer::default();
    let mut in_section = false;
    let mut singletons = HashSet::new();
    let base = file
        .parent()
        .context("Config file has no parent directory")?;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|name| name.strip_suffix(']'))
        {
            in_section = !name.is_empty();
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .with_context(|| anyhow!("{}:{}: expected key=value", file.display(), index + 1))?;
        // Section prefixes are part of option names in OpenMW; none of our options have one.
        if in_section {
            continue;
        }
        let key = key.trim();
        let value = value.trim();
        if matches!(key, "data-local" | "resources") && !singletons.insert(key) {
            bail!(
                "{}:{}: option '{key}' cannot be specified more than once",
                file.display(),
                index + 1
            );
        }
        let parsed = match key {
            "data" | "data-local" | "config" | "resources" => {
                let value = parse_path_value(value).with_context(|| {
                    anyhow!("{}:{}: invalid {key} path", file.display(), index + 1)
                })?;
                resolve_path(&value, base, paths)
            }
            _ => None,
        };
        match key {
            "data" => layer.data.extend(parsed),
            "config" => layer.configs.extend(parsed),
            "data-local" => layer.data_local = Some(parsed.unwrap_or_default()),
            "resources" => layer.resources = Some(parsed.unwrap_or_default()),
            "content" => layer.content.push(value.to_string()),
            "replace" => {
                layer.replace.insert(value.to_string());
            }
            _ => {}
        }
    }
    Ok(layer)
}

fn parse_path_value(value: &str) -> Result<String> {
    let Some(quoted) = value.strip_prefix('"') else {
        return Ok(value.to_string());
    };
    let mut result = String::new();
    let mut chars = quoted.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Ok(result), // OpenMW discards text after a closing path quote.
            '&' => result.push(chars.next().context("Unfinished ampersand escape")?),
            _ => result.push(ch),
        }
    }
    bail!("Unterminated quoted path")
}

fn resolve_path(value: &str, base: &Path, paths: &OpenMWPaths) -> Option<PathBuf> {
    for (token, directory) in [
        ("?local?", Some(&paths.local)),
        ("?userconfig?", Some(&paths.user_config)),
        ("?userdata?", Some(&paths.user_data)),
        ("?global?", paths.global.as_ref()),
    ] {
        if let Some(suffix) = value.strip_prefix(token) {
            return directory.map(|directory| directory.join(suffix));
        }
    }
    if let Some(token_suffix) = value.strip_prefix('?') {
        if token_suffix.contains('?') {
            warn!("Ignoring OpenMW path with unknown token: {value}");
            return None;
        }
        // The engine leaves a path with no closing token marker unchanged.
        return Some(PathBuf::from(value));
    }
    let path = Path::new(value);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Fixture {
        root: PathBuf,
        paths: OpenMWPaths,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "merged_lands_cfg_{}_{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("time")
                    .as_nanos()
            ));
            fs::create_dir_all(&root).expect("directory");
            let root = fs::canonicalize(root).expect("canonical root");
            let paths = OpenMWPaths::fixture(&root);
            Self { root, paths }
        }

        fn write(&self, name: &str, text: &str) {
            let file = self.root.join(name);
            fs::create_dir_all(file.parent().expect("parent")).expect("directory");
            fs::write(file, text).expect("write config");
        }

        fn load(&self) -> OpenMWConfig {
            OpenMWConfig::load_with_paths(&self.root, &self.paths).expect("load config")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).expect("cleanup");
        }
    }

    #[test]
    fn path_quoting_preserves_hashes_backslashes_and_whitespace() {
        for (text, expected) in [
            (r"C:\Games\Morrowind #1", r"C:\Games\Morrowind #1"),
            (
                r#"" C:\Mods\A && B &"quoted&" " trailing text"#,
                " C:\\Mods\\A & B \"quoted\" ",
            ),
            (r#""a&b""#, "ab"),
            ("", ""),
        ] {
            assert_eq!(parse_path_value(text).expect("path"), expected);
        }
        for value in ["\"unfinished", "\"unfinished&"] {
            assert!(parse_path_value(value).is_err());
        }
    }

    #[test]
    fn parse_errors_include_filename_and_line_and_other_settings_are_ignored() {
        let fixture = Fixture::new();
        let file = fixture.root.join("openmw.cfg");
        let error = parse_layer("# comment\ndata=\"bad\n", &file, &fixture.paths)
            .err()
            .expect("error");
        assert!(format!("{error:#}").contains("openmw.cfg:2"));
        assert!(parse_layer("not an option", &file, &fixture.paths).is_err());
        let layer = parse_layer(
            "  # comment\r\nunknown=anything\ncontent=Name #1.esp\n[other]\ncontent=not-a-plugin\n",
            &file,
            &fixture.paths,
        )
        .expect("layer");
        assert_eq!(layer.content, ["Name #1.esp"]);
    }

    #[test]
    fn content_quotes_bom_and_section_prefixes_remain_literal() {
        let fixture = Fixture::new();
        let file = fixture.root.join("openmw.cfg");
        let layer = parse_layer(
            "\u{feff}data=ignored\ncontent=\"Quoted.esp\"\n[.]\ncontent=ignored.esp\n[]\ncontent=Raw.esp\n",
            &file,
            &fixture.paths,
        )
        .expect("layer");
        assert_eq!(layer.data, Vec::<PathBuf>::new());
        assert_eq!(layer.content, ["\"Quoted.esp\"", "Raw.esp"]);
    }

    #[test]
    fn duplicate_singleton_paths_are_rejected_with_source_location() {
        let fixture = Fixture::new();
        let file = fixture.root.join("openmw.cfg");
        for key in ["data-local", "resources"] {
            let error = parse_layer(
                &format!("{key}=first\n{key}=second\n"),
                &file,
                &fixture.paths,
            )
            .err()
            .expect("duplicate singleton error");
            let error = format!("{error:#}");
            assert!(error.contains("openmw.cfg:2"));
            assert!(error.contains("cannot be specified more than once"));
        }
    }

    #[test]
    fn nested_configs_follow_engine_stack_order_and_skip_cycles_and_missing_files() {
        let fixture = Fixture::new();
        fixture.write(
            "openmw.cfg",
            "content=Root.esm\nconfig=first\nconfig=second\nconfig=missing\nconfig=not-directory\nconfig=not-file\n",
        );
        fixture.write("not-directory", "");
        fs::create_dir_all(fixture.root.join("not-file/openmw.cfg")).expect("non-file config");
        fixture.write("first/openmw.cfg", "content=First.esp\nconfig=nested\n");
        fixture.write(
            "first/nested/openmw.cfg",
            "content=Nested.esp\nconfig=../..\n",
        );
        fixture.write(
            "second/openmw.cfg",
            &format!(
                "content=Second.esp\nconfig={}\n",
                fixture.root.join("first").display()
            ),
        );
        assert_eq!(
            fixture.load().content,
            ["Root.esm", "First.esp", "Nested.esp", "Second.esp"]
        );
    }

    #[test]
    fn replace_config_discards_intermediate_layers_but_keeps_root_and_pending_siblings() {
        let fixture = Fixture::new();
        fixture.write(
            "openmw.cfg",
            "content=Root.esm\nconfig=first\nconfig=second\n",
        );
        fixture.write("first/openmw.cfg", "content=Discard.esp\nconfig=reset\n");
        fixture.write(
            "first/reset/openmw.cfg",
            "replace=config\ncontent=Reset.esp\n",
        );
        fixture.write("second/openmw.cfg", "content=Second.esp\n");
        assert_eq!(
            fixture.load().content,
            ["Root.esm", "Reset.esp", "Second.esp"]
        );
    }

    #[test]
    fn replace_lists_and_singleton_winners_preserve_vfs_priority() {
        let fixture = Fixture::new();
        fixture.write("openmw.cfg", "data=old\ncontent=Old.esm\ndata-local=old-local\nresources=old-resources\nconfig=profile\n");
        fixture.write("profile/openmw.cfg", "replace=data\nreplace=content\ndata=one\ndata=two\ncontent=New.esm\ncontent=Later.esp\ndata-local=local\nresources=resources\n");
        for dir in [
            "profile/one",
            "profile/two",
            "profile/local",
            "profile/resources/vfs",
        ] {
            fs::create_dir_all(fixture.root.join(dir)).expect("data directory");
        }
        let loaded = fixture.load();
        assert_eq!(loaded.content, ["New.esm", "Later.esp"]);
        assert_eq!(loaded.data_local, fixture.root.join("profile/local"));
        assert_eq!(
            loaded.data_directories,
            [
                "profile/resources/vfs",
                "profile/one",
                "profile/two",
                "profile/local"
            ]
            .map(|dir| fixture.root.join(dir))
        );
    }

    #[test]
    fn empty_replace_clears_inherited_content_and_data() {
        let fixture = Fixture::new();
        fixture.write("openmw.cfg", "content=Old.esm\ndata=.\nconfig=profile\n");
        fixture.write("profile/openmw.cfg", "replace=data\nreplace=content\n");
        let loaded = fixture.load();
        assert_eq!(loaded.content, Vec::<String>::new());
        assert_eq!(loaded.data_directories, Vec::<PathBuf>::new());
        assert_eq!(loaded.data_local, fixture.paths.user_data.join("data"));
        assert!(
            !loaded.data_local.exists(),
            "parsing must not create directories"
        );
    }

    #[test]
    fn replacing_replace_restores_lists_from_lower_layers() {
        let fixture = Fixture::new();
        fixture.write("openmw.cfg", "content=Root.esm\ndata=old\nconfig=first\n");
        fixture.write(
            "first/openmw.cfg",
            "replace=content\nreplace=data\ncontent=First.esp\ndata=data\nconfig=nested\n",
        );
        fixture.write(
            "first/nested/openmw.cfg",
            "replace=replace\ncontent=Nested.esp\ndata=data\n",
        );
        for name in ["old", "first/data", "first/nested/data"] {
            fs::create_dir_all(fixture.root.join(name)).expect("data directory");
        }
        let loaded = fixture.load();
        assert_eq!(loaded.content, ["Root.esm", "First.esp", "Nested.esp"]);
        assert_eq!(
            loaded.data_directories,
            ["old", "first/data", "first/nested/data"].map(|name| fixture.root.join(name))
        );
    }

    #[test]
    fn empty_paths_use_the_declaring_directory_and_unknown_singletons_clear_inherited_values() {
        let fixture = Fixture::new();
        let file = fixture.root.join("openmw.cfg");
        let layer =
            parse_layer("data=\ndata-local=\n", &file, &fixture.paths).expect("empty paths");
        assert_eq!(layer.data, std::slice::from_ref(&fixture.root));
        assert_eq!(layer.data_local, Some(fixture.root.clone()));

        fixture.write("openmw.cfg", "data-local=.\nresources=.\nconfig=profile\n");
        fixture.write(
            "profile/openmw.cfg",
            "data-local=?unknown?\nresources=?unknown?\n",
        );
        assert!(fixture.load().data_local.as_os_str().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_root_uses_its_declared_location_for_relative_data_and_configs() {
        let fixture = Fixture::new();
        fixture.write("real/openmw.cfg", "data=mods\nconfig=child\n");
        fixture.write("alias/child/openmw.cfg", "content=Alias.esp\n");
        fixture.write("real/child/openmw.cfg", "content=Real.esp\n");
        fs::create_dir_all(fixture.root.join("alias/mods")).expect("data directory");
        let alias = fixture.root.join("alias/openmw.cfg");
        std::os::unix::fs::symlink(fixture.root.join("real/openmw.cfg"), &alias).expect("symlink");
        let loaded = OpenMWConfig::load_with_paths(&alias, &fixture.paths).expect("alias config");
        assert_eq!(loaded.root_config_file, alias);
        assert_eq!(loaded.content, ["Alias.esp"]);
        assert_eq!(loaded.data_directories, [fixture.root.join("alias/mods")]);
    }

    #[cfg(unix)]
    #[test]
    fn sibling_symlink_aliases_remain_distinct_config_sources() {
        let fixture = Fixture::new();
        fixture.write("openmw.cfg", "config=first\nconfig=second\n");
        fixture.write("shared/openmw.cfg", "data=mods\n");
        for name in ["first", "second"] {
            fs::create_dir_all(fixture.root.join(name).join("mods")).expect("data directory");
            std::os::unix::fs::symlink(
                fixture.root.join("shared/openmw.cfg"),
                fixture.root.join(name).join("openmw.cfg"),
            )
            .expect("symlink");
        }
        assert_eq!(
            fixture.load().data_directories,
            ["first/mods", "second/mods"].map(|name| fixture.root.join(name))
        );
    }

    #[test]
    fn tokens_use_fixed_platform_paths_and_relative_paths_use_their_own_config() {
        let fixture = Fixture::new();
        let base = fixture.root.join("profile");
        for (value, expected) in [
            ("?local?resources", fixture.paths.local.join("resources")),
            ("?userconfig?mods", fixture.paths.user_config.join("mods")),
            ("?userdata?data", fixture.paths.user_data.join("data")),
            (
                "?global?resources",
                fixture
                    .paths
                    .global
                    .as_ref()
                    .expect("global")
                    .join("resources"),
            ),
            ("relative", base.join("relative")),
        ] {
            assert_eq!(resolve_path(value, &base, &fixture.paths), Some(expected));
        }
        assert!(resolve_path("?unknown?data", &base, &fixture.paths).is_none());
        assert_eq!(
            resolve_path("?unterminated", &base, &fixture.paths),
            Some(PathBuf::from("?unterminated"))
        );
        #[cfg(unix)]
        {
            assert_eq!(
                resolve_path("?userconfig?/mods", &base, &fixture.paths),
                Some(PathBuf::from("/mods"))
            );
            assert_eq!(
                resolve_path("?userconfig?\\mods", &base, &fixture.paths),
                Some(fixture.paths.user_config.join(Path::new("\\mods")))
            );
        }
    }

    #[test]
    fn automatic_engine_base_inherits_user_profiles_and_does_not_force_a_user_layer() {
        let fixture = Fixture::new();
        fixture.write(
            "engine/openmw.cfg",
            "data-local=?userdata?data\nconfig=?userconfig?\ncontent=Engine.esm\n",
        );
        fixture.write(
            "userconfig/openmw.cfg",
            "content=User.esp\nconfig=profile\n",
        );
        fixture.write("userconfig/profile/openmw.cfg", "content=Profile.esp\n");
        let root = fixture.paths.discover(&|_| None);
        let loaded = OpenMWConfig::load_with_paths(&root, &fixture.paths).expect("automatic chain");
        assert_eq!(
            loaded.root_config_file,
            fixture.paths.local.join("openmw.cfg")
        );
        assert_eq!(loaded.content, ["Engine.esm", "User.esp", "Profile.esp"]);
        assert_eq!(loaded.data_local, fixture.paths.user_data.join("data"));

        fixture.write("engine/openmw.cfg", "content=Engine.esm\n");
        let loaded = OpenMWConfig::load_with_paths(&root, &fixture.paths).expect("engine only");
        assert_eq!(loaded.content, ["Engine.esm"]);
    }
}
