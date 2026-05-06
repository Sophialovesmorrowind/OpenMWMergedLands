use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CONFIG_FILE_NAME: &str = "merged_lands.toml";
const DEFAULT_GENERATED_OUTPUT_DIR: &str = "default_data_local";
const DEFAULT_IGNORED_PLUGINS: [&str; 6] = [
    "delta-merged.omwaddon",
    "deleted_groundcover.omwaddon",
    "S3LightFixes.omwaddon",
    "OMWLLFMod.omwaddon",
    "merged.omwaddon",
    "Merged Objects.esp",
];

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AppConfigSource {
    Explicit,
    OpenMWConfig,
    ExecutableDir,
}

pub struct AppConfigLocation {
    dir: PathBuf,
    source: AppConfigSource,
}

impl AppConfigLocation {
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    #[must_use]
    pub fn source(&self) -> AppConfigSource {
        self.source
    }
}

pub struct LoadedMergedLandsConfig {
    pub config: MergedLandsConfig,
    pub created: bool,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct MergedLandsConfig {
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    openmw_cfg: Option<String>,
    #[serde(default, alias = "default_output_file_dir")]
    #[serde(skip_serializing_if = "Option::is_none")]
    output_file_dir: Option<String>,
    #[serde(default)]
    ignore_plugins: Vec<String>,
    #[serde(default, alias = "ignore_plugins_from_paths", alias = "ignore_paths")]
    ignore_plugins_from_path: Vec<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    generated_output_dir: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    generated_output_files: Vec<String>,
}

impl MergedLandsConfig {
    /// Resolves the directory containing `merged_lands.toml`.
    ///
    /// Explicit `--config-dir` values must be writable. Without an explicit override, the
    /// preferred `OpenMW` config directory is used when writable, otherwise the executable's
    /// directory is used as a local fallback.
    pub fn resolve_location(
        explicit_dir: Option<PathBuf>,
        openmw_config_dir: Option<PathBuf>,
    ) -> Result<AppConfigLocation> {
        if let Some(dir) = explicit_dir {
            prepare_config_dir(&dir)
                .with_context(|| anyhow!("Unable to use explicit config directory"))?;
            return Ok(AppConfigLocation {
                dir,
                source: AppConfigSource::Explicit,
            });
        }

        if let Some(dir) = openmw_config_dir
            && prepare_config_dir(&dir).is_ok()
        {
            return Ok(AppConfigLocation {
                dir,
                source: AppConfigSource::OpenMWConfig,
            });
        }

        let dir = executable_dir()?;
        prepare_config_dir(&dir)
            .with_context(|| anyhow!("Unable to use executable directory for app config"))?;
        Ok(AppConfigLocation {
            dir,
            source: AppConfigSource::ExecutableDir,
        })
    }

    /// Loads `merged_lands.toml` from `config_dir`, if present.
    pub fn load(config_dir: &Path) -> Result<Option<Self>> {
        let config_path = config_dir.join(CONFIG_FILE_NAME);

        match fs::read_to_string(&config_path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| anyhow!("Unable to parse {}", config_path.to_string_lossy()))
                .map(Some),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error)
                .with_context(|| anyhow!("Unable to read {}", config_path.to_string_lossy())),
        }
    }

    /// Loads `merged_lands.toml` from `config_dir`, or creates a default config if none exists.
    pub fn load_or_create(config_dir: &Path) -> Result<LoadedMergedLandsConfig> {
        if let Some(config) = Self::load(config_dir)? {
            Ok(LoadedMergedLandsConfig {
                config,
                created: false,
            })
        } else {
            let config = Self::with_default_ignored_plugins();
            config.save(config_dir)?;
            Ok(LoadedMergedLandsConfig {
                config,
                created: true,
            })
        }
    }

    /// Creates a default config seeded with plugins that are not useful merge inputs.
    #[must_use]
    pub fn with_default_ignored_plugins() -> Self {
        Self {
            generated_output_dir: Some(DEFAULT_GENERATED_OUTPUT_DIR.to_string()),
            ignore_plugins: DEFAULT_IGNORED_PLUGINS
                .iter()
                .map(ToString::to_string)
                .collect(),
            ..Self::default()
        }
    }

    /// Saves `merged_lands.toml` to `config_dir`.
    pub fn save(&self, config_dir: &Path) -> Result<()> {
        let config_path = config_dir.join(CONFIG_FILE_NAME);
        fs::write(&config_path, toml::to_string_pretty(self).expect("safe"))
            .with_context(|| anyhow!("Unable to write {}", config_path.to_string_lossy()))
    }

    /// Returns the configured output directory, resolving relative paths against
    /// the directory that contains `merged_lands.toml`.
    #[must_use]
    pub fn output_file_dir(&self, merged_lands_dir: &Path) -> Option<PathBuf> {
        self.output_file_dir.as_ref().map(|dir| {
            let path = PathBuf::from(dir);
            if path.is_absolute() {
                path
            } else {
                merged_lands_dir.join(path)
            }
        })
    }

    /// Returns the configured `openmw.cfg` path, if one has been saved.
    #[must_use]
    pub fn openmw_cfg(&self) -> Option<&str> {
        self.openmw_cfg.as_deref()
    }

    /// Records the root `openmw.cfg` path to use for future `OpenMW` runs.
    pub fn set_openmw_cfg(&mut self, path: &Path) {
        self.openmw_cfg = Some(path.to_string_lossy().into_owned());
    }

    /// Records a generated output plugin name and directory for future self-output filtering.
    pub fn record_generated_output(&mut self, output_file_dir: &Path, output_file_name: &str) {
        self.generated_output_dir = Some(output_file_dir.to_string_lossy().into_owned());

        if !self
            .generated_output_files
            .iter()
            .any(|file_name| file_name.eq_ignore_ascii_case(output_file_name))
        {
            self.generated_output_files
                .push(output_file_name.to_string());
        }
    }

    /// Returns the generated output directory, resolving relative paths against the app config dir.
    #[must_use]
    pub fn generated_output_dir(&self, config_dir: &Path) -> Option<PathBuf> {
        self.generated_output_dir.as_ref().and_then(|dir| {
            if dir == DEFAULT_GENERATED_OUTPUT_DIR {
                return None;
            }

            let path = PathBuf::from(dir);
            Some(if path.is_absolute() {
                path
            } else {
                config_dir.join(path)
            })
        })
    }

    /// Returns generated output plugin names that exist in the recorded generated output
    /// directory, falling back to the current output directory for older configs.
    #[must_use]
    pub fn generated_output_files_that_exist(
        &self,
        current_output_file_dir: &Path,
        config_dir: &Path,
    ) -> Vec<String> {
        let output_file_dir = self
            .generated_output_dir(config_dir)
            .unwrap_or_else(|| current_output_file_dir.to_path_buf());

        self.generated_output_files
            .iter()
            .filter(|file_name| output_file_dir.join(file_name).is_file())
            .cloned()
            .collect()
    }

    /// Returns plugin names to ignore before parsing.
    #[must_use]
    pub fn ignore_plugins(&self) -> &[String] {
        &self.ignore_plugins
    }

    /// Returns ignored plugin paths, resolving relative paths against the app config directory.
    #[must_use]
    pub fn ignore_plugins_from_path(&self, config_dir: &Path) -> Vec<PathBuf> {
        self.ignore_plugins_from_path
            .iter()
            .map(|path| {
                let path = PathBuf::from(path);
                if path.is_absolute() {
                    path
                } else {
                    config_dir.join(path)
                }
            })
            .collect()
    }
}

fn prepare_config_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| {
        anyhow!(
            "Unable to create config directory {}",
            dir.to_string_lossy()
        )
    })?;

    let probe = dir.join(format!(
        ".merged_lands_write_test_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    ));

    fs::write(&probe, []).with_context(|| {
        anyhow!(
            "Unable to write to config directory {}",
            dir.to_string_lossy()
        )
    })?;
    match fs::remove_file(&probe) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            anyhow!(
                "Unable to clean up write test file {}",
                probe.to_string_lossy()
            )
        }),
    }
}

fn executable_dir() -> Result<PathBuf> {
    let executable =
        std::env::current_exe().with_context(|| anyhow!("Unable to determine executable path"))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .with_context(|| anyhow!("Unable to determine executable directory"))
}

#[cfg(test)]
mod tests {
    use super::{
        AppConfigSource, CONFIG_FILE_NAME, DEFAULT_GENERATED_OUTPUT_DIR, MergedLandsConfig,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "{}_{}_{}",
            name,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before unix epoch")
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn output_file_dir_resolves_relative_paths_against_merged_lands_dir() {
        let config: MergedLandsConfig =
            toml::from_str("output_file_dir = \"Merged Output\"").expect("config should parse");

        let resolved = config
            .output_file_dir(Path::new("/tmp/merged_lands"))
            .expect("output path should be present");

        assert_eq!(resolved, Path::new("/tmp/merged_lands/Merged Output"));
    }

    #[test]
    fn output_file_dir_keeps_absolute_paths() {
        let config: MergedLandsConfig =
            toml::from_str("output_file_dir = \"/var/tmp/out\"").expect("config should parse");

        let resolved = config
            .output_file_dir(Path::new("/tmp/ignored"))
            .expect("output path should be present");

        assert_eq!(resolved, Path::new("/var/tmp/out"));
    }

    #[test]
    fn generated_output_files_falls_back_to_current_output_dir_for_older_configs() {
        let root = unique_temp_dir("app_config_generated_outputs");
        let output_dir = root.join("Output");
        fs::create_dir_all(&output_dir).expect("create output dir");
        fs::write(output_dir.join("Merged Lands.omwaddon"), []).expect("write generated output");

        let config: MergedLandsConfig = toml::from_str(
            "generated_output_files = [\"Merged Lands.omwaddon\", \"Old Output.esp\"]",
        )
        .expect("config should parse");

        assert_eq!(
            config.generated_output_files_that_exist(&output_dir, &root),
            vec!["Merged Lands.omwaddon"]
        );

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn generated_output_files_prefers_recorded_generated_output_dir() {
        let root = unique_temp_dir("app_config_generated_output_dir");
        let current_output_dir = root.join("CurrentOutput");
        let generated_output_dir = root.join("GeneratedOutput");
        fs::create_dir_all(&current_output_dir).expect("create current output dir");
        fs::create_dir_all(&generated_output_dir).expect("create generated output dir");
        fs::write(generated_output_dir.join("Merged Lands.omwaddon"), [])
            .expect("write generated output");

        let config = MergedLandsConfig {
            generated_output_dir: Some(generated_output_dir.to_string_lossy().into_owned()),
            generated_output_files: vec![
                "Merged Lands.omwaddon".to_string(),
                "Old Output.esp".to_string(),
            ],
            ..MergedLandsConfig::default()
        };

        assert_eq!(
            config.generated_output_files_that_exist(&current_output_dir, &root),
            vec!["Merged Lands.omwaddon"]
        );

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn generated_output_dir_default_label_resolves_to_none() {
        let config: MergedLandsConfig =
            toml::from_str("generated_output_dir = \"default_data_local\"")
                .expect("config should parse");

        assert_eq!(config.generated_output_dir(Path::new("/tmp/config")), None);
    }

    #[test]
    fn record_generated_output_deduplicates_case_insensitively() {
        let mut config = MergedLandsConfig::default();

        config.record_generated_output(Path::new("/tmp/output"), "Merged Lands.omwaddon");
        config.record_generated_output(Path::new("/tmp/output"), "merged lands.OMWADDON");

        assert_eq!(config.generated_output_files.len(), 1);
    }

    #[test]
    fn record_generated_output_records_output_dir() {
        let mut config = MergedLandsConfig::default();

        config.record_generated_output(Path::new("/tmp/generated"), "Merged Lands.omwaddon");

        assert_eq!(
            config.generated_output_dir(Path::new("/tmp/config")),
            Some(PathBuf::from("/tmp/generated"))
        );
    }

    #[test]
    fn save_roundtrips_generated_output_files() {
        let root = unique_temp_dir("app_config_save");
        let output_dir = root.join("Output");
        let mut config = MergedLandsConfig::default();
        config.record_generated_output(&output_dir, "Merged Lands.omwaddon");

        config.save(&root).expect("save config");
        let loaded = MergedLandsConfig::load(&root)
            .expect("load config")
            .expect("config exists");

        assert_eq!(loaded.generated_output_files, vec!["Merged Lands.omwaddon"]);
        assert_eq!(loaded.generated_output_dir(&root), Some(output_dir));

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn save_roundtrips_openmw_cfg_path() {
        let root = unique_temp_dir("app_config_openmw_cfg");
        let openmw_cfg = root.join("openmw.cfg");
        let mut config = MergedLandsConfig::default();
        config.set_openmw_cfg(&openmw_cfg);

        config.save(&root).expect("save config");
        let loaded = MergedLandsConfig::load(&root)
            .expect("load config")
            .expect("config exists");

        assert_eq!(
            loaded.openmw_cfg().map(std::path::PathBuf::from),
            Some(openmw_cfg)
        );

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn load_or_create_seeds_default_ignored_plugins_only_for_new_config() {
        let root = unique_temp_dir("app_config_defaults");

        let created = MergedLandsConfig::load_or_create(&root).expect("create config");
        assert!(created.created);
        assert!(
            created
                .config
                .ignore_plugins()
                .iter()
                .any(|plugin| plugin == "delta-merged.omwaddon")
        );
        assert!(
            created
                .config
                .ignore_plugins()
                .iter()
                .any(|plugin| plugin == "Merged Objects.esp")
        );
        assert_eq!(
            created.config.generated_output_dir.as_deref(),
            Some(DEFAULT_GENERATED_OUTPUT_DIR)
        );

        fs::write(
            root.join(CONFIG_FILE_NAME),
            "ignore_plugins = [\"Custom.esp\"]\n",
        )
        .expect("overwrite config");
        let loaded = MergedLandsConfig::load_or_create(&root).expect("load config");

        assert!(!loaded.created);
        assert_eq!(loaded.config.ignore_plugins(), &["Custom.esp"]);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn resolve_location_uses_explicit_config_dir() {
        let root = unique_temp_dir("app_config_explicit");
        let explicit = root.join("Explicit");

        let location =
            MergedLandsConfig::resolve_location(Some(explicit.clone()), None).expect("resolve");

        assert_eq!(location.dir(), explicit);
        assert_eq!(location.source(), AppConfigSource::Explicit);
        assert!(explicit.is_dir());

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn resolve_location_prefers_openmw_config_dir() {
        let root = unique_temp_dir("app_config_openmw");
        let openmw = root.join("openmw");

        let location =
            MergedLandsConfig::resolve_location(None, Some(openmw.clone())).expect("resolve");

        assert_eq!(location.dir(), openmw);
        assert_eq!(location.source(), AppConfigSource::OpenMWConfig);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn ignored_plugin_paths_resolve_relative_to_config_dir() {
        let config: MergedLandsConfig = toml::from_str("ignore_plugins_from_path = [\"ignored\"]")
            .expect("config should parse");

        assert_eq!(
            config.ignore_plugins_from_path(Path::new("/tmp/config")),
            vec![Path::new("/tmp/config/ignored")]
        );
    }
}
