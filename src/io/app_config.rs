use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE_NAME: &str = "merged_lands.toml";

#[derive(Debug, Default, Deserialize)]
pub struct MergedLandsConfig {
    #[serde(default, alias = "default_output_file_dir")]
    output_file_dir: Option<String>,
}

impl MergedLandsConfig {
    /// Loads `merged_lands.toml` from `merged_lands_dir`, if present.
    pub fn load(merged_lands_dir: &Path) -> Result<Option<Self>> {
        let config_path = merged_lands_dir.join(CONFIG_FILE_NAME);

        match fs::read_to_string(&config_path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| anyhow!("Unable to parse {}", config_path.to_string_lossy()))
                .map(Some),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error)
                .with_context(|| anyhow!("Unable to read {}", config_path.to_string_lossy())),
        }
    }

    /// Returns the configured output directory, resolving relative paths against
    /// the directory that contains `merged_lands.toml`.
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
}

#[cfg(test)]
mod tests {
    use super::MergedLandsConfig;
    use std::path::Path;

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
}
