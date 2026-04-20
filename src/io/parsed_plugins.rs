use crate::cli::SortOrder;
use crate::io::meta_schema::{PluginMeta, VersionedPluginMeta};
use crate::term_style::{bold, bold_red, yellow};
use anyhow::{anyhow, bail, Context, Result};
use log::{debug, error, info, trace, warn};
use openmw_config::{default_data_local_path, OpenMWConfiguration};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Lines};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tes3::esp::{Cell, Header, Landscape, LandscapeTexture, Plugin, TES3Object};

// -------------------------------------------------------------------------------------------------
// DataDirs
// -------------------------------------------------------------------------------------------------

/// A set of directories in which plugin files and their meta files can be found.
///
/// For classic Morrowind, this is a single `Data Files` directory. For `OpenMW` it is the
/// ordered list of `data=` entries from `openmw.cfg`, plus any engine-added entries such as
/// the resources VFS and `data-local`.
///
/// Within the vector, later entries have higher priority (`OpenMW`'s VFS last-wins rule).
/// [`DataDirs::resolve`] walks the list in reverse to locate a file.
#[derive(Debug, Clone)]
pub struct DataDirs {
    dirs: Vec<PathBuf>,
}

impl DataDirs {
    /// Creates a [`DataDirs`] containing a single directory (classic Morrowind layout).
    pub fn single(dir: PathBuf) -> Self {
        Self { dirs: vec![dir] }
    }

    /// Creates a [`DataDirs`] from an ordered list. The first entry is the lowest priority,
    /// the last entry is the highest. Must not be empty.
    pub fn from_ordered(dirs: Vec<PathBuf>) -> Result<Self> {
        if dirs.is_empty() {
            bail!("DataDirs must contain at least one directory");
        }
        Ok(Self { dirs })
    }

    /// The highest-priority data directory. This is used for last-wins data resolution and as the
    /// starting point for finding `Morrowind.ini` in classic mode.
    pub fn primary(&self) -> &Path {
        self.dirs
            .last()
            .expect("DataDirs is non-empty by construction")
            .as_path()
    }

    /// Searches for `name` in every data directory, highest priority first. Returns the resolved
    /// filesystem path if the file exists, else [`None`].
    ///
    /// Note: matching is case-sensitive on case-sensitive filesystems (i.e., Linux) because
    /// [`Path::is_file`] is case-sensitive there. On Windows and case-insensitive macOS
    /// filesystems the underlying OS resolves the cases for us.
    pub fn resolve(&self, name: &str) -> Option<PathBuf> {
        for dir in self.dirs.iter().rev() {
            let candidate: PathBuf = [dir.as_path(), Path::new(name)].iter().collect();
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// Iterates over the directories in priority order (lowest first, highest last).
    pub fn iter(&self) -> impl Iterator<Item = &Path> {
        self.dirs.iter().map(std::path::PathBuf::as_path)
    }
}

// -------------------------------------------------------------------------------------------------
// PluginListSource
// -------------------------------------------------------------------------------------------------

/// How to obtain the ordered list of plugins to process.
pub enum PluginListSource {
    /// Read the list from `Morrowind.ini` in the parent of the primary data dir
    /// (classic Morrowind behavior).
    MorrowindIni,
    /// Use this explicit list verbatim (from CLI args or from an already-parsed `openmw.cfg`).
    Explicit(Vec<String>),
}

// -------------------------------------------------------------------------------------------------
// OpenMW cfg loading
// -------------------------------------------------------------------------------------------------

/// Where to load the `OpenMW` configuration from.
pub enum OpenMWCfgSource {
    /// Use [`OpenMWConfiguration::from_env`] — respects `OPENMW_CONFIG` / `OPENMW_CONFIG_DIR`
    /// and then falls back to the platform-default location.
    Default,
    /// Load from the given file or directory path.
    Path(PathBuf),
}

pub struct LoadedOpenMWConfig {
    /// The ordered data directories used for plugin and meta discovery.
    pub data_dirs: DataDirs,
    /// The ordered `content=` entries from `openmw.cfg`.
    pub plugins: Vec<String>,
    /// The directory to use as the default output location in `OpenMW` mode.
    pub data_local: PathBuf,
}

/// Loads an `OpenMW` configuration and extracts the list of data directories, the ordered list of
/// `content=` entries, and the resolved `data-local` output directory.
///
/// The returned [`DataDirs`] will include any entries the `openmw-config` crate injects for the
/// engine resources VFS and `data-local`; this matches what `OpenMW` itself sees at runtime.
pub fn load_openmw_cfg(source: OpenMWCfgSource) -> Result<LoadedOpenMWConfig> {
    let config = match source {
        OpenMWCfgSource::Default => {
            info!("Loading OpenMW configuration from default location");
            OpenMWConfiguration::from_env()
        }
        OpenMWCfgSource::Path(path) => {
            info!(
                "Loading OpenMW configuration from {}",
                path.to_string_lossy()
            );
            OpenMWConfiguration::new(Some(path))
        }
    }
    .map_err(|e| anyhow!("Failed to load openmw.cfg: {e:?}"))?;

    debug!(
        "Using root openmw.cfg at {}",
        config.root_config_file().to_string_lossy()
    );

    let dirs: Vec<PathBuf> = config
        .data_directories_iter()
        .map(|d| d.parsed().to_path_buf())
        .collect();

    if dirs.is_empty() {
        bail!("openmw.cfg contains no `data=` directories; cannot discover plugins");
    }

    for dir in &dirs {
        trace!("data dir: {}", dir.to_string_lossy());
    }

    let plugins: Vec<String> = config
        .content_files_iter()
        .map(|f| f.value().clone())
        .collect();

    debug!(
        "Parsed {} data directories and {} content files from openmw.cfg",
        dirs.len(),
        plugins.len()
    );

    let data_local = config
        .data_local()
        .map_or_else(default_data_local_path, |dir| dir.parsed().to_path_buf());

    debug!(
        "Resolved OpenMW data-local output directory to {}",
        data_local.to_string_lossy()
    );

    let data_dirs = DataDirs::from_ordered(dirs)?;
    Ok(LoadedOpenMWConfig {
        data_dirs,
        plugins,
        data_local,
    })
}

// -------------------------------------------------------------------------------------------------
// Plugin parsing helpers
// -------------------------------------------------------------------------------------------------

/// Parse a [`Plugin`] named `plugin_name`, resolving it through `data_dirs`.
fn parse_records(data_dirs: &DataDirs, plugin_name: &str) -> Result<Plugin> {
    let file_path = data_dirs.resolve(plugin_name).with_context(|| {
        anyhow!("Unable to find plugin {plugin_name} in any configured data directory")
    })?;

    let mut plugin = Plugin::new();
    plugin
        .load_path_filtered(file_path, |tag| {
            matches!(
                &tag,
                Header::TAG | LandscapeTexture::TAG | Landscape::TAG | Cell::TAG
            )
        })
        .with_context(|| anyhow!("Failed to load records from plugin {plugin_name}"))?;

    plugin.objects.retain(|object| match object {
        TES3Object::Cell(cell) => cell.is_exterior(),
        _ => true,
    });

    Ok(plugin)
}

/// Opens `filename` and returns an iterator for the lines in the file.
fn read_lines(filename: &Path) -> Result<Lines<BufReader<File>>> {
    let file = File::open(filename).with_context(|| {
        anyhow!(
            "Unable to open file {} for reading",
            filename.to_string_lossy()
        )
    })?;
    Ok(BufReader::new(file).lines())
}

/// Returns `true` if `path` ends with `.esm`, ignoring case.
pub fn is_esm(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("esm"))
}

/// Returns `true` if `path` ends with `.esp`, ignoring case.
pub fn is_esp(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("esp"))
}

/// Sorts `plugin_list` by last modified time, with `.esm` files given priority.
pub fn sort_plugins(
    data_dirs: &DataDirs,
    plugin_list: &mut [String],
    sort_order: SortOrder,
) -> Result<()> {
    if matches!(sort_order, SortOrder::None) {
        return Ok(());
    }

    // Resolve and read metadata once per plugin; sorting then reuses cached keys.
    let mut cached_order = HashMap::with_capacity(plugin_list.len());
    for plugin_name in plugin_list.iter() {
        let path = data_dirs.resolve(plugin_name).with_context(|| {
            anyhow!("Unable to find plugin {plugin_name} when computing load order")
        })?;
        let last_modified_time = path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .with_context(|| anyhow!("Unable to read metadata for plugin {plugin_name}"))?;

        cached_order.insert(
            plugin_name.clone(),
            (!is_esm(plugin_name), last_modified_time),
        );
    }

    plugin_list.sort_by(|a, b| {
        cached_order
            .get(a)
            .expect("validated above")
            .cmp(cached_order.get(b).expect("validated above"))
    });

    Ok(())
}

/// Returns a `name` describing a meta file by replacing the extension with `.mergedlands.toml`.
pub fn meta_name(name: &str) -> String {
    let file_name_without_extension = Path::new(&name).file_stem().unwrap().to_string_lossy();
    format!("{file_name_without_extension}.mergedlands.toml")
}

// -------------------------------------------------------------------------------------------------
// ParsedPlugin / ParsedPlugins
// -------------------------------------------------------------------------------------------------

/// A [`ParsedPlugin`] is the `name`, [Plugin] records, and any [`PluginMeta`] data.
pub struct ParsedPlugin {
    /// The `name` of the plugin.
    pub name: String,
    /// The parsed [Plugin] records.
    pub records: Plugin,
    /// The parsed [`PluginMeta`], or a default if no meta file was found.
    pub meta: PluginMeta,
}

const QUOTE_CHARS: [char; 2] = ['\'', '"'];

impl ParsedPlugin {
    /// Returns an empty [`ParsedPlugin`] with the provided `name`.
    pub fn empty(name: &str) -> Self {
        Self {
            name: name.to_string(),
            records: Plugin::new(),
            meta: PluginMeta::default(),
        }
    }

    /// Creates a [`ParsedPlugin`]. If `meta` is [None], a default [`PluginMeta`] is created.
    fn from(name: &str, records: Plugin, meta: Option<PluginMeta>) -> Self {
        Self {
            name: name.to_string(),
            records,
            meta: meta.unwrap_or_default(),
        }
    }

    /// Returns the list of plugins this plugin declares as masters in its TES3 header.
    /// Each entry is `(master_filename, master_file_size_in_bytes)`.
    pub fn header_masters(&self) -> &[(String, u64)] {
        self.records
            .objects_of_type::<Header>()
            .next()
            .and_then(|h| h.masters.as_deref())
            .unwrap_or(&[])
    }
}

impl Eq for ParsedPlugin {}

impl PartialEq<Self> for ParsedPlugin {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Hash for ParsedPlugin {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

/// All [`ParsedPlugin`] organized for downstream merge logic.
///
/// In classic mode, plugins are split into "master-like" vs "plugin-like".
/// In `OpenMW` mode, `masters` is empty and `plugins` preserves the exact content order.
pub struct ParsedPlugins {
    /// The ordered list of master-like plugins.
    /// These are used for creating the reference [`crate::Landmass`].
    pub masters: Vec<Arc<ParsedPlugin>>,
    /// The ordered list of plugin-like plugins.
    /// These are used for creating each [`crate::LandmassDiff`].
    pub plugins: Vec<Arc<ParsedPlugin>>,
}

/// Returns a [Vec] of plugin names by reading the `.ini` file at `path`. Each plugin name is
/// checked for existence in `data_dirs`.
fn read_ini_file(data_dirs: &DataDirs, path: &Path) -> Result<Vec<String>> {
    let lines = read_lines(path).with_context(|| anyhow!("Unable to read Morrowind.ini"))?;

    let mut all_plugins = Vec::new();

    let mut is_game_files = false;
    for line in lines
        .map_while(Result::ok)
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty() && !line.starts_with(';'))
    {
        if line == "[Game Files]" {
            is_game_files = true;
        } else if line.starts_with('[') {
            is_game_files = false;
        } else if is_game_files {
            let Some((key, value)) = line.split_once('=') else {
                warn!(
                    "{}",
                    yellow(format!(
                        "Found junk in [Game Files] section: {}",
                        bold(&line)
                    ))
                );
                continue;
            };

            let Some(index) = key.strip_prefix("GameFile") else {
                warn!(
                    "{}",
                    yellow(format!(
                        "Found junk in [Game Files] section: {}",
                        bold(&line)
                    ))
                );
                continue;
            };

            if index.is_empty() || !index.chars().all(|ch| ch.is_ascii_digit()) {
                warn!(
                    "{}",
                    yellow(format!(
                        "Found junk in [Game Files] section: {}",
                        bold(&line)
                    ))
                );
                continue;
            }

            let plugin_name = value
                .trim()
                .trim_start_matches(QUOTE_CHARS)
                .trim_end_matches(QUOTE_CHARS);

            match data_dirs.resolve(plugin_name) {
                Some(_) => all_plugins.push(plugin_name.to_string()),
                None => error!(
                    "{}",
                    bold_red(format!(
                        "Plugin {} does not exist in any configured data directory",
                        bold(plugin_name)
                    ))
                ),
            }
        }
    }

    Ok(all_plugins)
}

impl ParsedPlugins {
    /// Helper function for returning an `Err` if `dir` does not exist or is otherwise inaccessible.
    pub fn check_dir_exists(dir: impl AsRef<Path>) -> Result<()> {
        let path = dir.as_ref();
        let exists = path
            .try_exists()
            .with_context(|| anyhow!("Unable to find `{}` directory", path.to_string_lossy()))?;

        if !exists {
            bail!("The `{}` directory does not exist", path.to_string_lossy());
        }

        Ok(())
    }

    /// Creates a new [`ParsedPlugins`].
    ///
    /// - `data_dirs` is the set of search directories (one entry for classic Morrowind, many for
    ///   `OpenMW`).
    /// - `source` determines where the plugin list comes from.
    /// - `sort_order` is applied after the list is gathered; pass [`SortOrder::None`] to preserve
    ///   the list's existing order (recommended when the list came from `openmw.cfg`, which is
    ///   the user's authoritative load order).
    /// - `is_openmw_mode` switches from classic global master/plugin bucketing to exact ordered
    ///   content processing with per-plugin dependency validation.
    pub fn new(
        data_dirs: &DataDirs,
        source: PluginListSource,
        sort_order: SortOrder,
        is_openmw_mode: bool,
    ) -> Result<Self> {
        for dir in data_dirs.iter() {
            ParsedPlugins::check_dir_exists(dir)
                .with_context(|| anyhow!("Invalid data directory"))?;
        }

        let mut all_plugins = match source {
            PluginListSource::Explicit(list) => {
                trace!("Using {} plugins provided explicitly", list.len());
                list
            }
            PluginListSource::MorrowindIni => {
                trace!("Parsing Morrowind.ini for plugins");

                let parent_directory = data_dirs.primary().parent().with_context(|| {
                    anyhow!(
                        "Unable to find parent of `{}` directory",
                        data_dirs.primary().to_string_lossy()
                    )
                })?;

                let file_path: PathBuf = [parent_directory, Path::new("Morrowind.ini")]
                    .iter()
                    .collect();

                let plugin_names = read_ini_file(data_dirs, &file_path)
                    .with_context(|| anyhow!("Unable to parse plugins from Morrowind.ini"))?;

                trace!(
                    "Using {} plugins parsed from Morrowind.ini",
                    plugin_names.len()
                );

                plugin_names
            }
        };

        sort_plugins(data_dirs, &mut all_plugins, sort_order)
            .with_context(|| anyhow!("Unknown load order for plugins"))?;

        // Parse every plugin first, preserving load order. We need the full set before we can
        // decide which plugins are "master-like" via header cross-references.
        let mut parsed: Vec<Arc<ParsedPlugin>> = Vec::with_capacity(all_plugins.len());

        for plugin_name in all_plugins {
            match parse_records(data_dirs, &plugin_name) {
                Ok(records) => {
                    let meta = Self::load_meta_for(data_dirs, &plugin_name);
                    parsed.push(Arc::new(ParsedPlugin::from(&plugin_name, records, meta)));
                }
                Err(e) => {
                    error!(
                        "{} {}",
                        bold_red(format!("Failed to parse plugin {}", bold(&plugin_name))),
                        bold_red(format!("due to: {:?}", bold(format!("{e:?}"))))
                    );
                }
            }
        }

        if is_openmw_mode {
            Self::validate_openmw_load_order(&parsed)?;
            return Ok(Self {
                masters: Vec::new(),
                plugins: parsed,
            });
        }

        // Build a case-insensitive set of every plugin name that appears as a master in some
        // other plugin's header. OpenMW does not require these to have a `.esm` extension, and
        // the classic engine also tolerates ESP-as-master for plugins that declare it explicitly.
        let mut referenced_as_master: HashSet<String> = HashSet::new();
        for parsed_plugin in &parsed {
            for (master_name, _size) in parsed_plugin.header_masters() {
                referenced_as_master.insert(master_name.to_ascii_lowercase());
            }
        }

        // Split into master-like and plugin-like, preserving the overall load order within each.
        let mut masters = Vec::new();
        let mut plugins = Vec::new();
        for parsed_plugin in parsed {
            let name_lc = parsed_plugin.name.to_ascii_lowercase();
            let promoted_to_master =
                !is_esm(&parsed_plugin.name) && referenced_as_master.contains(&name_lc);

            if is_esm(&parsed_plugin.name) || promoted_to_master {
                if promoted_to_master {
                    debug!(
                        "Treating {} as a master because another plugin declares it as such",
                        parsed_plugin.name
                    );
                }
                masters.push(parsed_plugin);
            } else {
                plugins.push(parsed_plugin);
            }
        }

        Ok(Self { masters, plugins })
    }

    /// Validates that `OpenMW` `content=` order respects each plugin's TES3 header masters.
    ///
    /// `OpenMW` loads content in the order given. A plugin may depend on an earlier plugin, but if
    /// one of its declared masters is missing or appears later in the list, we treat that as a
    /// user load-order problem and stop cleanly.
    fn validate_openmw_load_order(parsed: &[Arc<ParsedPlugin>]) -> Result<()> {
        let positions: HashMap<String, usize> = parsed
            .iter()
            .enumerate()
            .map(|(idx, plugin)| (plugin.name.to_ascii_lowercase(), idx))
            .collect();

        let mut found_invalid_dependency_order = false;

        for (plugin_idx, plugin) in parsed.iter().enumerate() {
            for (master_name, _size) in plugin.header_masters() {
                let master_name_lc = master_name.to_ascii_lowercase();
                match positions.get(&master_name_lc).copied() {
                    Some(master_idx) if master_idx < plugin_idx => {}
                    Some(master_idx) => {
                        warn!(
                            "OpenMW load order problem: {} declares master {} but loads before it (positions {} and {})",
                            plugin.name,
                            master_name,
                            plugin_idx + 1,
                            master_idx + 1
                        );
                        found_invalid_dependency_order = true;
                    }
                    None => {
                        warn!(
                            "OpenMW load order problem: {} declares missing master {}",
                            plugin.name, master_name
                        );
                        found_invalid_dependency_order = true;
                    }
                }
            }
        }

        if found_invalid_dependency_order {
            bail!("OpenMW load order contains missing or out-of-order masters; see warnings above");
        }

        Ok(())
    }

    /// Attempts to load the `.mergedlands.toml` meta file for `plugin_name`, searching across
    /// every configured data directory. Returns [`None`] if no valid meta file was found
    /// (including the common case where one simply doesn't exist).
    fn load_meta_for(data_dirs: &DataDirs, plugin_name: &str) -> Option<PluginMeta> {
        let meta_file_name = meta_name(plugin_name);
        let meta_file_path = data_dirs.resolve(&meta_file_name)?;

        let data = fs::read_to_string(&meta_file_path)
            .with_context(|| anyhow!("Failed to read meta file."))
            .and_then(|text| {
                toml::from_str::<VersionedPluginMeta>(&text)
                    .with_context(|| anyhow!("Failed to parse meta file contents."))
            });

        match data {
            Ok(VersionedPluginMeta::V0(meta)) => {
                trace!("Parsed meta file {meta_file_name}");
                Some(meta)
            }
            Ok(VersionedPluginMeta::Unsupported) => {
                error!(
                    "{}",
                    bold_red(format!(
                        "Unsupported plugin meta file {}",
                        bold(&meta_file_name)
                    ))
                );
                None
            }
            // TODO(dvd): #refactor Is there a TOML error we could be printing here?
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_esm, is_esp, meta_name, sort_plugins, DataDirs};
    use crate::cli::SortOrder;
    use std::fs;
    use std::path::Path;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn create_temp_data_dir() -> std::path::PathBuf {
        let unique = format!(
            "merged_lands_test_{}_{}",
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

    fn create_empty_file(path: &Path) {
        fs::write(path, []).expect("write empty file");
    }

    #[test]
    fn extension_helpers_are_case_insensitive() {
        assert!(is_esm("Morrowind.ESM"));
        assert!(is_esp("Patch.EsP"));

        assert!(!is_esm("mod.esp"));
        assert!(!is_esp("mod.esm"));
        assert!(!is_esp("README"));
    }

    #[test]
    fn meta_name_replaces_extension_with_sidecar_name() {
        assert_eq!(meta_name("Morrowind.esm"), "Morrowind.mergedlands.toml");
        assert_eq!(
            meta_name("Data Files/Plugin.esp"),
            "Plugin.mergedlands.toml"
        );
    }

    #[test]
    fn sort_plugins_none_keeps_order() {
        let data_dir = create_temp_data_dir();
        create_empty_file(&data_dir.join("a.esp"));
        create_empty_file(&data_dir.join("b.esp"));

        let mut plugins = vec!["b.esp".to_string(), "a.esp".to_string()];
        sort_plugins(
            &DataDirs::single(data_dir.clone()),
            &mut plugins,
            SortOrder::None,
        )
        .expect("sort should succeed");

        assert_eq!(plugins, vec!["b.esp", "a.esp"]);
        fs::remove_dir_all(data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sort_plugins_prioritizes_esm_over_esp() {
        let data_dir = create_temp_data_dir();
        create_empty_file(&data_dir.join("plugin.esp"));
        thread::sleep(Duration::from_millis(10));
        create_empty_file(&data_dir.join("master.esm"));

        let mut plugins = vec!["plugin.esp".to_string(), "master.esm".to_string()];
        sort_plugins(
            &DataDirs::single(data_dir.clone()),
            &mut plugins,
            SortOrder::Default,
        )
        .expect("sort should succeed");

        assert_eq!(plugins, vec!["master.esm", "plugin.esp"]);
        fs::remove_dir_all(data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sort_plugins_orders_by_mtime_within_same_extension() {
        let data_dir = create_temp_data_dir();
        create_empty_file(&data_dir.join("older.esp"));
        thread::sleep(Duration::from_millis(10));
        create_empty_file(&data_dir.join("newer.esp"));

        let mut plugins = vec!["newer.esp".to_string(), "older.esp".to_string()];
        sort_plugins(
            &DataDirs::single(data_dir.clone()),
            &mut plugins,
            SortOrder::Default,
        )
        .expect("sort should succeed");

        assert_eq!(plugins, vec!["older.esp", "newer.esp"]);
        fs::remove_dir_all(data_dir).expect("cleanup temp dir");
    }
}
