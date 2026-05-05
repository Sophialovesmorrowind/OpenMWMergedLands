use crate::io::app_config::{CONFIG_FILE_NAME, MergedLandsConfig};
use crate::io::meta_schema::{ConflictStrategy, MetaType};
use crate::io::parsed_plugins::{
    DataDirs, ParsedPlugin, ParsedPlugins, PluginFilter, PluginListSource, load_openmw_cfg,
};
use crate::io::save_to_image::save_landmass_images;
use crate::io::save_to_plugin::{convert_landmass_diff_to_landmass, save_plugin};
use crate::land::conversions::{coordinates, landscape_flags};
use crate::land::grid_access::{GridAccessor2D, SquareGridIterator};
use crate::land::landscape_diff::LandscapeDiff;
use crate::land::terrain_map::{LandData, Vec2};
use crate::land::textures::{IndexVTEX, KnownTextures, RemappedTextures};
use crate::merge::merge_strategy::apply_merge_strategy;
use crate::merge::relative_terrain_map::{IsModified, RelativeTerrainMap};
use crate::repair::cleaning::{clean_known_textures, clean_landmass_diff};
use crate::repair::debugging::add_debug_vertex_colors_to_landmass;
use crate::repair::seam_detection::repair_landmass_seams;
use crate::term_style::{bold, bold_red};
use anyhow::{Context, Result, anyhow};
use log::{debug, error, info, trace, warn};
use openmw_config::{OpenMWConfiguration, try_default_config_path};
use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, LevelPadding, TermLogger,
    TerminalMode, WriteLogger,
};
use std::any::Any;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{IsTerminal, Read, Write, stdin, stdout};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::Arc;
use std::time::Instant;
use tes3::esp::{Landscape, LandscapeFlags, LandscapeTexture, ObjectFlags};

mod io;
mod land;
mod merge;
mod repair;
mod term_style;

const OPENMW_LOGO_ASCII: &str = include_str!("../assets/openmw-logo-ascii.txt");
const DEFAULT_OPENMW_OUTPUT_FILE: &str = "Merged Lands.omwaddon";
const DEFAULT_VANILLA_OUTPUT_FILE: &str = "Merged Lands.esp";

/// A [Landmass] represents a collection of [Landscape] and the associated [`ParsedPlugin`].
pub struct Landmass {
    plugin: Arc<ParsedPlugin>,
    land: HashMap<Vec2<i32>, Landscape>,
    plugins: HashMap<Vec2<i32>, Arc<ParsedPlugin>>,
}

impl Landmass {
    fn new(plugin: Arc<ParsedPlugin>) -> Self {
        Self {
            plugin,
            land: HashMap::new(),
            plugins: HashMap::new(),
        }
    }

    fn insert_land(&mut self, coords: Vec2<i32>, plugin: &Arc<ParsedPlugin>, land: &Landscape) {
        self.plugins.insert(coords, plugin.clone());
        self.land.insert(coords, land.clone());
    }

    /// Returns the [Landscape] entries ordered by `x` and `y` coordinates.
    fn sorted(&self) -> Vec<(&Vec2<i32>, &Landscape)> {
        let mut entries: Vec<_> = self.land.iter().collect();
        entries.sort_by_key(|f| (f.0.x, f.0.y));
        entries
    }
}

impl Clone for Landmass {
    fn clone(&self) -> Self {
        Self {
            plugin: self.plugin.clone(),
            land: self.land.clone(),
            plugins: self.plugins.clone(),
        }
    }
}

/// A [`LandmassDiff`] represents a collection of [`LandscapeDiff`] and the associated [`ParsedPlugin`].
pub struct LandmassDiff {
    plugin: Arc<ParsedPlugin>,
    land: HashMap<Vec2<i32>, LandscapeDiff>,
}

impl LandmassDiff {
    fn new(plugin: Arc<ParsedPlugin>) -> Self {
        Self {
            plugin,
            land: HashMap::new(),
        }
    }

    /// Returns the [`LandscapeDiff`] entries ordered by `x` and `y` coordinates.
    fn sorted(&self) -> Vec<(&Vec2<i32>, &LandscapeDiff)> {
        let mut entries: Vec<_> = self.land.iter().collect();
        entries.sort_by_key(|f| (f.0.x, f.0.y));
        entries
    }
}

mod cli {
    use crate::ParsedPlugins;
    use crate::io::parsed_plugins::OpenMWCfgSource;
    use anyhow::{Context, Result, anyhow};
    use clap::{Parser, ValueEnum};
    use log::LevelFilter;
    use std::path::PathBuf;

    #[derive(Copy, PartialEq, Eq, Debug, Hash, Clone, ValueEnum)]
    pub enum CliLevelFilter {
        Off,
        Error,
        Warn,
        Info,
        Debug,
        Trace,
    }

    #[derive(Copy, PartialEq, Eq, Debug, Hash, Clone, ValueEnum)]
    pub enum SortOrder {
        Default,
        None,
    }

    impl From<CliLevelFilter> for LevelFilter {
        fn from(v: CliLevelFilter) -> Self {
            match v {
                CliLevelFilter::Off => LevelFilter::Off,
                CliLevelFilter::Error => LevelFilter::Error,
                CliLevelFilter::Warn => LevelFilter::Warn,
                CliLevelFilter::Info => LevelFilter::Info,
                CliLevelFilter::Debug => LevelFilter::Debug,
                CliLevelFilter::Trace => LevelFilter::Trace,
            }
        }
    }

    #[derive(Parser, Debug)]
    #[command(author = "DVD")]
    #[command(about = "Merges lands.")]
    #[command(version)]
    #[command(long_about = None)] // Read from `Cargo.toml`
    pub struct Cli {
        #[arg(long, default_value_t = String::from("."))]
        /// The directory containing the `Conflicts` folder.
        /// This is also where the `log_file` is stored.
        merged_lands_dir: String,

        #[arg(long)]
        /// The directory containing `merged_lands.toml`.
        /// Defaults to the `OpenMW` config directory, falling back to the executable directory.
        config_dir: Option<String>,

        #[arg(long, default_value_t = String::from("Data Files"))]
        /// The absolute or relative path to the `Data Files` folder containing plugins.
        /// Used for plugin discovery only in classic Morrowind mode (`--vanilla`).
        data_files_dir: String,

        #[arg(long, conflicts_with = "openmw_cfg")]
        /// Enables classic Morrowind mode using `Data Files` + `Morrowind.ini`.
        /// When this is not set, the tool defaults to `OpenMW` mode.
        pub vanilla: bool,

        #[arg(long, conflicts_with = "vanilla")]
        /// Uses the `openmw.cfg` at this path instead of the platform-default location.
        /// The path may be either a directory containing `openmw.cfg` or a direct path to the
        /// file. `OpenMW` mode is the default when `--vanilla` is not set.
        pub openmw_cfg: Option<String>,

        #[arg(long)]
        /// The name of the output file. This will be written to `output_file_dir`.
        /// Defaults to `Merged Lands.omwaddon` in `OpenMW` mode and `Merged Lands.esp` in
        /// `--vanilla` mode.
        pub output_file: Option<String>,

        #[arg(long)]
        /// The directory for the `output_file`.
        /// If not provided, the resolution order is:
        /// `merged_lands.toml` in `config_dir`, `OpenMW` `data-local`, then `data_files_dir` in
        /// `--vanilla` mode.
        output_file_dir: Option<String>,

        #[arg(required = false)]
        /// An ordered list of plugins.
        /// If this is not provided, the tool will use `content=` entries from `openmw.cfg` by
        /// default, or `Morrowind.ini` in `--vanilla` mode.
        input_file_names: Vec<String>,

        #[arg(long, value_enum, default_value_t = SortOrder::Default)]
        /// The method of sorting plugins.
        /// `none` is only valid if `input_file_names` are provided.
        pub sort_order: SortOrder,

        #[arg(long, default_value_t = String::from("merged_lands.log"))]
        /// The name of the log file. This will be written to `merged_lands_dir`.
        pub log_file: String,

        #[arg(long, value_enum, default_value_t = CliLevelFilter::Debug)]
        /// The level of logging.
        /// If set to Off, no log will will be written.
        pub log_level: CliLevelFilter,

        #[arg(long, default_value_t = 8)]
        /// The size of the application's stack in MB.
        stack_size_mb: u8,

        #[arg(long)]
        /// The application will color the LAND vertex colors to show conflicts.
        pub add_debug_vertex_colors: bool,

        #[arg(long)]
        /// The application will wait for the user to hit the ENTER key before closing.
        pub wait_for_exit: bool,
    }

    impl Cli {
        pub fn read_args() -> Cli {
            Cli::parse_from(wild::args())
        }

        pub fn plugins(&self) -> Option<&[String]> {
            (!self.input_file_names.is_empty()).then_some(&self.input_file_names)
        }

        pub fn should_write_log_file(&self) -> bool {
            self.log_level != CliLevelFilter::Off
        }

        pub fn merged_lands_dir(&self) -> PathBuf {
            let dir = &self.merged_lands_dir;
            PathBuf::from(dir)
        }

        pub fn config_dir(&self) -> Option<PathBuf> {
            self.config_dir.as_ref().map(PathBuf::from)
        }

        pub fn data_files_dir(&self) -> Result<PathBuf> {
            let dir = &self.data_files_dir;
            ParsedPlugins::resolve_dir(dir)
                .with_context(|| anyhow!("Invalid `Data Files` directory"))
        }

        /// Returns `true` unless classic Morrowind mode was requested explicitly.
        pub fn is_openmw_mode(&self) -> bool {
            !self.vanilla
        }

        /// Resolves the `OpenMW` config source. `OpenMW` is the default unless `--vanilla` is used.
        /// The CLI path wins, then the saved app config path, then auto-detection.
        pub fn openmw_cfg_source(
            &self,
            app_config_openmw_cfg: Option<&str>,
        ) -> Option<OpenMWCfgSource> {
            if self.vanilla {
                None
            } else if let Some(path) = &self.openmw_cfg {
                Some(OpenMWCfgSource::Path(PathBuf::from(path)))
            } else if let Some(path) = app_config_openmw_cfg {
                Some(OpenMWCfgSource::Path(PathBuf::from(path)))
            } else {
                Some(OpenMWCfgSource::Default)
            }
        }

        /// Returns the output directory specified on the CLI, if any. In `OpenMW` mode when this
        /// is unset, the caller should default to the primary (last) data directory.
        pub fn output_file_dir_override(&self) -> Option<&String> {
            self.output_file_dir.as_ref()
        }

        pub fn output_file_name(&self) -> &str {
            self.output_file
                .as_deref()
                .unwrap_or(if self.is_openmw_mode() {
                    super::DEFAULT_OPENMW_OUTPUT_FILE
                } else {
                    super::DEFAULT_VANILLA_OUTPUT_FILE
                })
        }

        pub fn output_file_dir(&self) -> Result<PathBuf> {
            let dir = self
                .output_file_dir
                .as_ref()
                .unwrap_or(&self.data_files_dir);
            ParsedPlugins::resolve_dir(dir)
                .with_context(|| anyhow!("Invalid output file directory"))
        }

        pub fn stack_size(&self) -> usize {
            (self.stack_size_mb as usize) * 1024 * 1024
        }
    }

    #[cfg(test)]
    mod tests {
        use super::Cli;
        use crate::io::parsed_plugins::OpenMWCfgSource;
        use clap::Parser;
        use std::path::Path;

        #[test]
        fn default_mode_is_openmw() {
            let cli = Cli::try_parse_from(["merged_lands"]).expect("CLI should parse");
            assert!(cli.is_openmw_mode());
            assert_eq!(cli.output_file_name(), "Merged Lands.omwaddon");
        }

        #[test]
        fn vanilla_mode_changes_default_output_name() {
            let cli = Cli::try_parse_from(["merged_lands", "--vanilla"]).expect("CLI should parse");
            assert!(!cli.is_openmw_mode());
            assert_eq!(cli.output_file_name(), "Merged Lands.esp");
        }

        #[test]
        fn explicit_output_file_name_wins() {
            let cli = Cli::try_parse_from(["merged_lands", "--output-file", "custom.esp"])
                .expect("CLI should parse");
            assert_eq!(cli.output_file_name(), "custom.esp");
        }

        #[test]
        fn vanilla_conflicts_with_openmw_cfg_flag() {
            let err = Cli::try_parse_from([
                "merged_lands",
                "--vanilla",
                "--openmw-cfg",
                "/tmp/openmw.cfg",
            ])
            .expect_err("CLI should reject conflicting flags");

            let rendered = err.to_string();
            assert!(
                rendered.contains("cannot be used with") || rendered.contains("conflicts with")
            );
        }

        #[test]
        fn openmw_cfg_source_uses_saved_app_config_path() {
            let cli = Cli::try_parse_from(["merged_lands"]).expect("CLI should parse");

            let Some(OpenMWCfgSource::Path(path)) =
                cli.openmw_cfg_source(Some("/tmp/saved/openmw.cfg"))
            else {
                panic!("saved app config path should be used");
            };

            assert_eq!(path, Path::new("/tmp/saved/openmw.cfg"));
        }

        #[test]
        fn openmw_cfg_source_prefers_cli_path_over_saved_app_config_path() {
            let cli = Cli::try_parse_from(["merged_lands", "--openmw-cfg", "/tmp/cli/openmw.cfg"])
                .expect("CLI should parse");

            let Some(OpenMWCfgSource::Path(path)) =
                cli.openmw_cfg_source(Some("/tmp/saved/openmw.cfg"))
            else {
                panic!("CLI path should be used");
            };

            assert_eq!(path, Path::new("/tmp/cli/openmw.cfg"));
        }

        #[test]
        fn vanilla_mode_ignores_saved_openmw_cfg_path() {
            let cli = Cli::try_parse_from(["merged_lands", "--vanilla"]).expect("CLI should parse");

            assert!(
                cli.openmw_cfg_source(Some("/tmp/saved/openmw.cfg"))
                    .is_none()
            );
        }
    }
}

use cli::{Cli, SortOrder};

/// Handles CLI arguments, log initialization, and the creation of a worker thread
/// for running the actual [`merge_all`] function.
fn format_thread_panic(panic: Box<dyn Any + Send + 'static>) -> String {
    match panic.downcast::<String>() {
        Ok(message) => *message,
        Err(panic) => match panic.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "unknown panic payload".to_string(),
        },
    }
}

fn ensure_output_file_dir_exists(dir: PathBuf, source: &str) -> Result<PathBuf> {
    if let Ok(existing_dir) = ParsedPlugins::resolve_dir(&dir) {
        return Ok(existing_dir);
    }

    fs::create_dir_all(&dir).with_context(|| {
        anyhow!(
            "Unable to create output file directory from {} at {}",
            source,
            dir.to_string_lossy()
        )
    })?;

    ParsedPlugins::check_dir_exists(&dir)
        .with_context(|| anyhow!("Invalid output file directory from {source}"))?;

    ParsedPlugins::resolve_dir(dir)
}

fn run_merge_on_worker_thread(cli: Cli) -> Result<()> {
    let worker_stack_size = cli.stack_size();
    let worker = std::thread::Builder::new()
        .stack_size(worker_stack_size)
        .spawn(move || merge_all(&cli))
        .with_context(|| anyhow!("unable to create worker thread"))?;

    match worker.join() {
        Ok(result) => result,
        Err(panic) => {
            let panic_message = format_thread_panic(panic);
            Err(anyhow!("Worker thread panicked: {panic_message}"))
        }
    }
}

fn main() {
    let cli = Cli::read_args();
    let wait_for_exit = cli.wait_for_exit;

    init_log(&cli);
    print_startup_banner();

    if let Err(e) = run_merge_on_worker_thread(cli) {
        error!(
            "{}",
            bold_red(format!(
                "An unexpected error occurred: {:?}",
                bold(format!("{e:?}"))
            ))
        );

        wait_for_user_exit(wait_for_exit);
        exit(1);
    }

    wait_for_user_exit(wait_for_exit);
}

fn print_startup_banner() {
    println!("{}", OPENMW_LOGO_ASCII.trim_end());
}

fn generated_output_file_names(
    cli: &Cli,
    app_config: &MergedLandsConfig,
    output_file_dir: &Path,
) -> Vec<String> {
    let mut names = vec![
        DEFAULT_OPENMW_OUTPUT_FILE.to_string(),
        DEFAULT_VANILLA_OUTPUT_FILE.to_string(),
    ];
    let output_file_name = cli.output_file_name();

    if !names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(output_file_name))
    {
        names.push(output_file_name.to_string());
    }

    for output_file_name in app_config.generated_output_files_that_exist(output_file_dir) {
        if !names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&output_file_name))
        {
            names.push(output_file_name);
        }
    }

    names
}

fn preferred_openmw_config_dir(cli: &Cli) -> Option<PathBuf> {
    cli.openmw_cfg
        .as_ref()
        .map(|path| openmw_cfg_path_to_dir(Path::new(path)))
        .or_else(|| try_default_config_path().ok())
}

fn openmw_cfg_path_to_dir(path: &Path) -> PathBuf {
    if path.file_name().is_some_and(|file_name| {
        file_name
            .to_string_lossy()
            .eq_ignore_ascii_case("openmw.cfg")
    }) {
        return path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    }

    path.to_path_buf()
}

fn should_prompt_for_openmw_cfg(
    cli: &Cli,
    app_config: &MergedLandsConfig,
    app_config_created: bool,
) -> bool {
    app_config_created
        && cli.is_openmw_mode()
        && cli.openmw_cfg.is_none()
        && app_config.openmw_cfg().is_none()
}

fn maybe_prompt_for_openmw_cfg(
    cli: &Cli,
    app_config: &mut MergedLandsConfig,
    app_config_dir: &Path,
    app_config_created: bool,
) -> Result<()> {
    if !should_prompt_for_openmw_cfg(cli, app_config, app_config_created) {
        return Ok(());
    }

    if !stdin().is_terminal() {
        debug!("Skipping first-run OpenMW config prompt because stdin is not interactive");
        return Ok(());
    }

    let openmw_cfg = prompt_for_openmw_cfg()?;
    app_config.set_openmw_cfg(&openmw_cfg);
    app_config.save(app_config_dir)?;
    let app_config_path = app_config_dir.join(CONFIG_FILE_NAME);

    println!();
    println!(
        "Saved default OpenMW configuration path to {}.",
        app_config_path.to_string_lossy()
    );
    println!("You can change it later by editing `openmw_cfg` in that file.");
    println!("Continuing with merge...");

    info!(
        "Saved OpenMW configuration path to {}",
        app_config_path.to_string_lossy()
    );

    Ok(())
}

fn prompt_for_openmw_cfg() -> Result<PathBuf> {
    loop {
        println!();
        println!("First run setup: choose OpenMW configuration source.");
        println!("1. Provide a path to openmw.cfg or a directory containing it");
        println!("2. Try OpenMW auto-detect");
        print!("Enter 1 or 2: ");
        stdout()
            .flush()
            .with_context(|| anyhow!("Unable to write OpenMW config prompt"))?;

        let mut selection = String::new();
        stdin()
            .read_line(&mut selection)
            .with_context(|| anyhow!("Unable to read OpenMW config prompt"))?;

        match selection.trim() {
            "1" => {
                if let Some(path) = prompt_for_explicit_openmw_cfg()? {
                    return Ok(path);
                }
            }
            "2" => match autodetect_openmw_cfg_path() {
                Ok(path) => return Ok(path),
                Err(error) => println!("Auto-detect could not load openmw.cfg: {error:?}"),
            },
            _ => println!("Please enter 1 or 2."),
        }
    }
}

fn prompt_for_explicit_openmw_cfg() -> Result<Option<PathBuf>> {
    print!("Path to openmw.cfg or its directory: ");
    stdout()
        .flush()
        .with_context(|| anyhow!("Unable to write OpenMW config path prompt"))?;

    let mut input = String::new();
    stdin()
        .read_line(&mut input)
        .with_context(|| anyhow!("Unable to read OpenMW config path"))?;

    let Some(path) = prompted_path(&input) else {
        println!("Path cannot be empty.");
        return Ok(None);
    };

    match explicit_openmw_cfg_path(path) {
        Ok(path) => Ok(Some(path)),
        Err(error) => {
            println!("Could not load openmw.cfg: {error:?}");
            Ok(None)
        }
    }
}

fn prompted_path(input: &str) -> Option<PathBuf> {
    let path = input.trim().trim_matches(|c| c == '"' || c == '\'');
    (!path.is_empty()).then(|| PathBuf::from(path.replace("\\ ", " ")))
}

fn explicit_openmw_cfg_path(path: PathBuf) -> Result<PathBuf> {
    OpenMWConfiguration::new(Some(path))
        .map(|config| config.root_config_file().to_path_buf())
        .map_err(|error| anyhow!("Failed to load openmw.cfg: {error:?}"))
}

fn autodetect_openmw_cfg_path() -> Result<PathBuf> {
    OpenMWConfiguration::from_env()
        .map(|config| config.root_config_file().to_path_buf())
        .map_err(|error| anyhow!("Failed to load openmw.cfg: {error:?}"))
}

fn wait_for_user_exit(wait_for_exit: bool) {
    if !wait_for_exit {
        return;
    }

    println!();
    println!("Press Enter to exit.");
    let mut buf = [0; 1];
    std::io::stdin().read_exact(&mut buf).ok();
}

/// The main function.
fn merge_all(cli: &Cli) -> Result<()> {
    let start = Instant::now();
    let mut phase_start = Instant::now();

    let mut known_textures = KnownTextures::new();

    // STEP 1:
    // For each Plugin, ordered by last modified:
    //  - Get or create reference landmass.
    //      - References are created by a list of ESMs / ESPs.
    //      - By default, the references are pulled from the TES3 header.
    //      - If the plugin has an associated `.mergedlands.meta`, read additional references from that.
    //      - Order the list by ESMs then ESPs, then within each category, order by last modified date.
    //      - [WARN] The current plugin loads before one or more of the references.
    //      - Calculate the "naive" TES3 merge of the ordered ESMs / ESPs.
    //  - Calculate diff from reference landmass.
    //  => return LandmassDiff

    // [IMPLEMENTATION NOTE] Whenever an ESM or ESP is loaded, all LTEX records are registered with
    // the KnownTextures and all texture indices in LAND records are updated accordingly.

    // [IMPLEMENTATION NOTE] Each loaded Plugin is stored in an Arc<...> with any data from the
    // optional `.mergedlands.toml` if it existed. The Arc<...> is copied into each LandscapeDiff.
    info!(":: Parsing Plugins ::");

    let app_config_location =
        MergedLandsConfig::resolve_location(cli.config_dir(), preferred_openmw_config_dir(cli))?;
    if app_config_location.source() == crate::io::app_config::AppConfigSource::ExecutableDir {
        warn!(
            "Unable to use the OpenMW config directory for {}; using {}",
            CONFIG_FILE_NAME,
            app_config_location.dir().to_string_lossy()
        );
    } else {
        debug!(
            "Using app config directory {}",
            app_config_location.dir().to_string_lossy()
        );
    }

    let app_config_dir = app_config_location.dir().to_path_buf();
    let loaded_app_config = MergedLandsConfig::load_or_create(&app_config_dir)?;
    let app_config_created = loaded_app_config.created;
    let mut app_config = loaded_app_config.config;
    maybe_prompt_for_openmw_cfg(cli, &mut app_config, &app_config_dir, app_config_created)?;

    // Determine whether we're in default OpenMW mode (`openmw.cfg`) or classic Morrowind mode
    // (`--vanilla`, using a single `Data Files` directory + Morrowind.ini). These two paths
    // differ in how data directories and the load order are discovered.
    let (data_dirs, plugin_source, effective_sort_order, default_openmw_output_dir) =
        if let Some(cfg_source) = cli.openmw_cfg_source(app_config.openmw_cfg()) {
            let openmw_config = load_openmw_cfg(cfg_source)?;
            let data_dirs = openmw_config.data_dirs;
            let cfg_content_files = openmw_config.plugins;
            let data_local = openmw_config.data_local;

            // A CLI plugin list, if given, always wins over what the cfg says.
            let (plugin_list, source_note) = match cli.plugins() {
                Some(cli_list) => (cli_list.to_vec(), "command-line arguments"),
                None => (cfg_content_files, "openmw.cfg content entries"),
            };

            debug!(
                "OpenMW mode: using {} plugins from {}",
                plugin_list.len(),
                source_note
            );

            // openmw.cfg's `content=` order is the user's authoritative load order — mtime sorting
            // would scramble it. If the user nonetheless asked for a sort, honor it; otherwise
            // preserve the list as given.
            let sort_order = if cli.plugins().is_some() {
                cli.sort_order
            } else {
                SortOrder::None
            };

            (
                data_dirs,
                PluginListSource::Explicit(plugin_list),
                sort_order,
                Some(data_local),
            )
        } else {
            let data_files = cli.data_files_dir()?;
            let data_dirs = DataDirs::single(data_files);

            let source = match cli.plugins() {
                Some(list) => PluginListSource::Explicit(list.to_vec()),
                None => PluginListSource::MorrowindIni,
            };

            (data_dirs, source, cli.sort_order, None)
        };

    // Output path precedence:
    //  1. `--output-file-dir`
    //  2. `output_file_dir` in `merged_lands.toml`
    //  3. OpenMW `data-local`
    //  4. `data_files_dir` in `--vanilla` mode
    let output_file_dir = match cli.output_file_dir_override() {
        Some(_) => cli.output_file_dir()?,
        None => match app_config.output_file_dir(&app_config_dir) {
            Some(dir) => ensure_output_file_dir_exists(dir, CONFIG_FILE_NAME)?,
            None if cli.is_openmw_mode() => ensure_output_file_dir_exists(
                default_openmw_output_dir.expect("OpenMW mode should provide data-local"),
                "openmw.cfg data-local",
            )?,
            None => cli.output_file_dir()?,
        },
    };

    let is_openmw_mode = cli.is_openmw_mode();
    let generated_output_names = generated_output_file_names(cli, &app_config, &output_file_dir);
    let plugin_filter = PluginFilter::new(
        app_config.ignore_plugins(),
        app_config.ignore_plugins_from_path(&app_config_dir),
    );
    let parsed_plugins = ParsedPlugins::new(
        &data_dirs,
        plugin_source,
        effective_sort_order,
        &generated_output_names,
        &plugin_filter,
        is_openmw_mode,
    )?;
    debug!("Parsed plugins in {:?}", phase_start.elapsed());
    phase_start = Instant::now();

    let (reference_landmass, modded_landmasses, raw_load_order_landmass) =
        create_reference_and_modded_landmasses(&parsed_plugins, &mut known_textures);
    debug!(
        "Built reference and modded landmasses in {:?}",
        phase_start.elapsed()
    );
    phase_start = Instant::now();

    debug!(
        "Found {} masters and {} plugins",
        parsed_plugins.masters.len(),
        parsed_plugins.plugins.len(),
    );
    debug!("Found {} unique LTEX records", known_textures.len());
    debug!("{} plugins contain LAND records", modded_landmasses.len());

    // STEP 2:
    // Create the MergedLands.esp:
    //  - Calculate the "naive" TES3 merge of the ordered ESMs.
    info!(":: Creating Reference Land ::");

    debug!(
        "Reference contains {} LAND records",
        reference_landmass.land.len()
    );

    let mut merged_lands = create_merged_lands_from_reference(&reference_landmass);
    debug!(
        "Created merged reference baseline in {:?}",
        phase_start.elapsed()
    );
    phase_start = Instant::now();

    // STEP 3:
    // For each LandmassDiff, [IMPLEMENTATION NOTE] same order as Plugin:
    //  - Merge into `MergedLands.esp`.
    //     - If LAND does not exist in MergedLands.esp, insert.
    //     - Else, apply merge strategies.
    //        - Each merge is applied to the result of any previous merge.
    //        - Each merge is tracked so it can be referenced in the future.
    //        - Merge strategies may use the optional `.mergedlands.toml` for conflict resolution.
    //  - Iterate through updated landmass and check for seams on any modified cell.
    info!(":: Merging Lands ::");

    for modded_landmass in &modded_landmasses {
        merge_landmass_into(&mut merged_lands, modded_landmass);
    }

    // We fix seams as a post-processing step because individual mods can introduce
    // tears into the landscape that would be fixed by subsequent mods. (e.g. patches)
    // If we try to fix the seams early, sadness results.
    repair_landmass_seams(&mut merged_lands);
    debug!(
        "Merged land diffs and repaired seams in {:?}",
        phase_start.elapsed()
    );
    phase_start = Instant::now();

    // STEP 4:
    //  - Produce images of the final merge results.
    info!(":: Summarizing Conflicts ::");

    let merged_lands_dir = cli.merged_lands_dir();
    save_landmass_images(&merged_lands_dir, &merged_lands, &modded_landmasses);
    debug!(
        "Saved conflict summary images in {:?}",
        phase_start.elapsed()
    );
    phase_start = Instant::now();

    let debug_vertex_colors = cli.add_debug_vertex_colors;
    if debug_vertex_colors {
        warn!(":: Adding Debug Colors ::");
        for modded_landmass in &modded_landmasses {
            add_debug_vertex_colors_to_landmass(&mut merged_lands, modded_landmass);
        }
    }

    // STEP 5:
    // - Iterate through cells in MergedLands.esp and drop anything that is unchanged from the
    //   reference landmass created for MergedLands.esp.
    // - Update all LandData flags to match TES3 expectations.
    // - Run a final seam detection and assert that no seams were found.
    // [IMPLEMENTATION NOTE] This is an optimization to make MergedLands.esp friendlier.
    info!(":: Cleaning Land ::");

    clean_landmass_diff(&mut merged_lands, &raw_load_order_landmass);
    debug!("Cleaned merged land diff in {:?}", phase_start.elapsed());
    phase_start = Instant::now();

    // ---------------------------------------------------------------------------------------------
    // [IMPLEMENTATION NOTE] Below this line, the merged landmass cannot be diff'd against plugins.
    // ---------------------------------------------------------------------------------------------

    // STEP 6:
    // Update LTEX records to only include textures in use in modified cells.
    info!(":: Updating LTEX Records ::");

    let remapped_textures =
        clean_known_textures(&parsed_plugins, &merged_lands, &mut known_textures);
    debug!("Updated LTEX records in {:?}", phase_start.elapsed());
    phase_start = Instant::now();

    // STEP 7:
    // Convert "height map" representation of LAND records to "xy delta + offset" representation.
    // Remap texture indices.
    info!(":: Converting to LAND Records ::");

    let landmass = convert_landmass_diff_to_landmass(&merged_lands, &remapped_textures);
    debug!(
        "Converted merged diff to LAND records in {:?}",
        phase_start.elapsed()
    );
    phase_start = Instant::now();

    // STEP 7:
    // Save to an ESP.
    //  - [IMPLEMENTATION NOTE] Reuse last modified date if the ESP already exists.
    info!(":: Saving ::");

    let file_name = cli.output_file_name();
    save_plugin(
        &data_dirs,
        &output_file_dir,
        file_name,
        cli.sort_order,
        &landmass,
        &known_textures,
    )?;
    app_config.record_generated_output(file_name);
    app_config.save(&app_config_dir)?;
    debug!("Saved plugin and app config in {:?}", phase_start.elapsed());

    info!(":: Finished ::");
    info!("Time Elapsed: {:?}", Instant::now().duration_since(start));

    Ok(())
}

/// Initializes a [`TermLogger`] and [`WriteLogger`]. If the [`WriteLogger`] cannot be initialized,
/// then the program will continue with only the [`TermLogger`].
fn init_log(cli: &Cli) -> bool {
    let config = ConfigBuilder::default()
        .set_time_level(LevelFilter::Off)
        .set_thread_level(LevelFilter::Off)
        .set_location_level(LevelFilter::Off)
        .set_target_level(LevelFilter::Off)
        .set_level_padding(LevelPadding::Right)
        .build();

    let get_log_file_path = || -> Result<PathBuf> {
        let merged_lands_dir = cli.merged_lands_dir();
        let log_file_name = &cli.log_file;
        Ok(merged_lands_dir.join(log_file_name))
    };

    let write_logger = cli.should_write_log_file().then(|| {
        let log_file_path = get_log_file_path()?;
        File::create(&log_file_path)
            .map(|file| WriteLogger::new(cli.log_level.into(), config.clone(), file))
            .with_context(|| {
                anyhow!(
                    "Unable to create log file at {}",
                    log_file_path.to_string_lossy()
                )
            })
    });

    let term_logger = TermLogger::new(
        LevelFilter::Debug,
        config,
        TerminalMode::Mixed,
        ColorChoice::Auto,
    );

    match write_logger {
        Some(Ok(write_logger)) => {
            CombinedLogger::init(vec![term_logger, write_logger]).expect("safe");
            trace!(
                "Log file will be saved to {}",
                get_log_file_path().expect("safe").to_string_lossy()
            );

            true
        }
        Some(Err(e)) => {
            CombinedLogger::init(vec![term_logger]).expect("safe");
            error!(
                "{} {}",
                bold_red(format!(
                    "Failed to create log file at {}",
                    bold(
                        get_log_file_path()
                            .unwrap_or_else(|_| PathBuf::from(&cli.log_file))
                            .to_string_lossy()
                    )
                )),
                bold_red(format!("due to: {:?}", bold(format!("{e:?}"))))
            );

            false
        }
        None => {
            trace!("No log file will be created.");
            CombinedLogger::init(vec![term_logger]).expect("safe");
            false
        }
    }
}

/// Copy [Landscape] records from `plugin` and remap the texture indices with [`RemappedTextures`].
fn try_copy_landscape_and_remap_textures(
    plugin: &Arc<ParsedPlugin>,
    remapped_textures: &RemappedTextures,
) -> Option<Landmass> {
    let mut landmass = Landmass::new(plugin.clone());

    if plugin.records.objects_of_type::<Landscape>().any(|_| true) {
        debug!("Creating landmass from {}", plugin.name);
    }

    for land in plugin.records.objects_of_type::<Landscape>() {
        let mut updated_land = land.clone();
        let coords = coordinates(land);

        if let Some(texture_indices) = updated_land.texture_indices.as_mut() {
            let fallback_texture_index = remapped_textures.fallback_texture_index();
            let mut invalid_texture_indices = 0usize;
            let mut first_invalid_texture_index = None;

            for idx in texture_indices.data.as_flattened_mut() {
                let original_index = IndexVTEX::new(*idx);
                if let Some(remapped) = remapped_textures.try_remapped_index(original_index) {
                    *idx = remapped.as_u16();
                } else {
                    invalid_texture_indices += 1;
                    first_invalid_texture_index.get_or_insert(original_index.as_u16());
                    *idx = fallback_texture_index.as_u16();
                }
            }

            if invalid_texture_indices > 0 {
                warn!(
                    "({:>4}, {:>4}) | {:<50} | Replaced {} invalid source texture indices (first VTEX index = {}) with fallback VTEX index {}",
                    coords.x,
                    coords.y,
                    plugin.name,
                    invalid_texture_indices,
                    first_invalid_texture_index
                        .expect("invalid index count implies first invalid index"),
                    fallback_texture_index.as_u16()
                );
            }
        }

        landmass.insert_land(coords, plugin, &updated_land);
    }

    if landmass.land.is_empty() {
        None
    } else {
        Some(landmass)
    }
}

/// Creates a [Landmass] from the `plugin` and updates [`KnownTextures`].
fn try_create_landmass(
    plugin: &Arc<ParsedPlugin>,
    known_textures: &mut KnownTextures,
) -> Option<Landmass> {
    if plugin
        .records
        .objects_of_type::<LandscapeTexture>()
        .any(|_| true)
    {
        debug!("Remapping textures from {}", plugin.name);
    }

    let mut remapped_textures = RemappedTextures::new(known_textures);
    for texture in plugin.records.objects_of_type::<LandscapeTexture>() {
        known_textures.add_remapped_texture(plugin, texture, &mut remapped_textures);
    }

    try_copy_landscape_and_remap_textures(plugin, &remapped_textures)
}

/// Returns a "merged" [Landscape] combining `rhs` and `lhs` by stomping over
/// any changes in `lhs` with the records from `rhs`.
fn merge_tes3_landscape(lhs: &Landscape, rhs: &Landscape) -> Landscape {
    let mut land = lhs.clone();

    let mut old_data = landscape_flags(lhs);
    let new_data = landscape_flags(rhs);

    assert_eq!(lhs.flags, rhs.flags, "expected identical LAND flags");
    assert!(
        !rhs.flags.contains(ObjectFlags::DELETED),
        "tried to add deleted LAND"
    );

    if new_data.contains(LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS) {
        if let Some(vertex_heights) = rhs.vertex_heights.as_ref() {
            old_data |= LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS;
            land.vertex_heights = Some(vertex_heights.clone());
        }
        if let Some(vertex_normals) = rhs.vertex_normals.as_ref() {
            old_data |= LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS;
            land.vertex_normals = Some(vertex_normals.clone());
        }
    }

    if new_data.contains(LandscapeFlags::USES_VERTEX_COLORS)
        && let Some(vertex_colors) = rhs.vertex_colors.as_ref()
    {
        old_data |= LandscapeFlags::USES_VERTEX_COLORS;
        land.vertex_colors = Some(vertex_colors.clone());
    }

    if new_data.contains(LandscapeFlags::USES_TEXTURES)
        && let Some(texture_indices) = rhs.texture_indices.as_ref()
    {
        old_data |= LandscapeFlags::USES_TEXTURES;
        land.texture_indices = Some(texture_indices.clone());
    }

    if new_data.uses_world_map_data()
        && let Some(world_map_data) = rhs.world_map_data.as_ref()
    {
        land.world_map_data = Some(world_map_data.clone());
    }

    land.landscape_flags = old_data;

    land
}

/// Creates a single [Landmass] by calling [`merge_tes3_landscape`] on all `landmasses`.
fn merge_tes3_landmasses(
    plugin: &Arc<ParsedPlugin>,
    landmasses: impl Iterator<Item = Landmass>,
) -> Landmass {
    let mut merged_landmass = Landmass::new(plugin.clone());

    for landmass in landmasses {
        for (coords, land) in &landmass.land {
            let merged_land = if let Some(existing) = merged_landmass.land.get(coords) {
                merge_tes3_landscape(existing, land)
            } else {
                land.clone()
            };

            merged_landmass.land.insert(*coords, merged_land);
            merged_landmass
                .plugins
                .insert(*coords, landmass.plugin.clone());
        }
    }

    merged_landmass
}

/// Given a [`ParsedPlugin`] and a specific [Landscape], returns [`LandData`] representing
/// what should be used when creating or merging a [`LandscapeDiff`].
fn find_allowed_data(plugin: &ParsedPlugin, land: &Landscape) -> LandData {
    let mut allowed_data: LandData = landscape_flags(land).into();

    if !plugin.meta.height_map.included {
        allowed_data.remove(LandData::VERTEX_HEIGHTS | LandData::VERTEX_NORMALS);
    }

    if !plugin.meta.vertex_colors.included {
        allowed_data.remove(LandData::VERTEX_COLORS);
    }

    if !plugin.meta.texture_indices.included {
        allowed_data.remove(LandData::TEXTURES);
    }

    if !plugin.meta.world_map_data.included {
        allowed_data.remove(LandData::WORLD_MAP);
    }

    allowed_data
}

fn filter_landscape_to_allowed_data(plugin: &ParsedPlugin, land: &Landscape) -> Option<Landscape> {
    let allowed_data = find_allowed_data(plugin, land);
    if allowed_data.is_empty() {
        return None;
    }

    let mut filtered = land.clone();

    if !allowed_data.contains(LandData::VERTEX_HEIGHTS) {
        filtered.vertex_heights = None;
        filtered.vertex_normals = None;
        filtered
            .landscape_flags
            .remove(LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS);
    }

    if !allowed_data.contains(LandData::VERTEX_COLORS) {
        filtered.vertex_colors = None;
        filtered
            .landscape_flags
            .remove(LandscapeFlags::USES_VERTEX_COLORS);
    }

    if !allowed_data.contains(LandData::TEXTURES) {
        filtered.texture_indices = None;
        filtered
            .landscape_flags
            .remove(LandscapeFlags::USES_TEXTURES);
    }

    if !allowed_data.contains(LandData::WORLD_MAP) {
        filtered.world_map_data = None;
    }

    Some(filtered)
}

/// Applies the winning LAND state from `next` into `merged`, updating the source plugin for
/// every cell that `next` contributes. This matches last-loaded LAND record behavior while still
/// respecting the current master-before-plugin ordering used by the tool.
fn merge_tes3_landmass_into(merged: &mut Landmass, next: &Landmass) {
    for (coords, land) in &next.land {
        let merged_land = if let Some(existing) = merged.land.get(coords) {
            merge_tes3_landscape(existing, land)
        } else {
            land.clone()
        };

        merged.land.insert(*coords, merged_land);
        merged.plugins.insert(*coords, next.plugin.clone());
    }
}

fn merge_allowed_landmass_into(merged: &mut Landmass, next: &Landmass) {
    let mut filtered = Landmass::new(next.plugin.clone());

    for (coords, land) in &next.land {
        if let Some(land) = filter_landscape_to_allowed_data(&next.plugin, land) {
            filtered.insert_land(*coords, &next.plugin, &land);
        }
    }

    merge_tes3_landmass_into(merged, &filtered);
}

/// Creates a [`LandmassDiff`] representing the set of [`LandscapeDiff`] between the
/// `landmass` and `reference` [Landmass].
fn find_landmass_diff(landmass: &Landmass, reference: &Landmass) -> LandmassDiff {
    let mut landmass_diff = LandmassDiff::new(landmass.plugin.clone());

    for (coords, land) in &landmass.land {
        let reference_land = reference.land.get(coords);
        let allowed_data = find_allowed_data(&landmass.plugin, land);
        let landscape_diff = LandscapeDiff::from_difference(land, reference_land, allowed_data);
        landmass_diff.land.insert(*coords, landscape_diff);
    }

    landmass_diff
}

/// Builds the initial reference landmass and the plugin diffs used for the final merge.
///
/// Plugin diffs are computed against the rolling winning LAND state from load order. That keeps
/// each diff local to what the plugin actually changed relative to previous winners, rather than
/// pretending there is a useful engine-specific LAND merge model. There isn't. Thankfully.
fn create_reference_and_modded_landmasses(
    parsed_plugins: &ParsedPlugins,
    known_textures: &mut KnownTextures,
) -> (Arc<Landmass>, Vec<LandmassDiff>, Arc<Landmass>) {
    let reference_landmass = create_tes3_landmass(
        "ReferenceLandmass.esp",
        parsed_plugins.masters.iter(),
        known_textures,
    );

    // TODO(dvd): #feature Support "ignored" maps for hiding differences that we don't care about.
    let mut rolling_reference = reference_landmass.clone();
    let mut raw_load_order_landmass = reference_landmass.clone();
    let mut modded_landmasses = Vec::new();

    for plugin in &parsed_plugins.plugins {
        if plugin.meta.meta_type == MetaType::MergedLands {
            trace!("Skipping {}", plugin.name);
            continue;
        }

        let Some(landmass) = try_create_landmass(plugin, known_textures) else {
            continue;
        };

        modded_landmasses.push(find_landmass_diff(&landmass, &rolling_reference));
        merge_allowed_landmass_into(&mut rolling_reference, &landmass);
        merge_tes3_landmass_into(&mut raw_load_order_landmass, &landmass);
    }

    (
        Arc::new(reference_landmass),
        modded_landmasses,
        Arc::new(raw_load_order_landmass),
    )
}

/// LAND texture indices are categorical winner data instead of numeric deltas. Load order is
/// top-to-bottom, so the newest plugin wins for the coordinates it actually changed.
fn merge_load_order_texture_indices(
    old: Option<&RelativeTerrainMap<IndexVTEX, 16>>,
    new: Option<&RelativeTerrainMap<IndexVTEX, 16>>,
) -> Option<RelativeTerrainMap<IndexVTEX, 16>> {
    let Some(new_texture_indices) = new else {
        return old.cloned();
    };

    let old_texture_indices = old.map_or(
        [[IndexVTEX::default(); 16]; 16],
        RelativeTerrainMap::to_terrain,
    );
    let mut merged_texture_indices = old_texture_indices;
    let mut changed_anything = false;

    for coords in new_texture_indices.iter_grid() {
        if !new_texture_indices.has_difference(coords) {
            continue;
        }

        let new_value = new_texture_indices.get_value(coords);
        if merged_texture_indices.get(coords) != new_value {
            *merged_texture_indices.get_mut(coords) = new_value;
            changed_anything = true;
        }
    }

    if !changed_anything {
        return old.cloned();
    }

    Some(RelativeTerrainMap::from_difference(
        &old_texture_indices,
        &merged_texture_indices,
    ))
}

/// With rolling-reference diffs, `Auto` should preserve load-order winner semantics. Explicit
/// strategies still mean exactly what the user asked for; `Auto` is the only policy default here.
fn load_order_auto_strategy(conflict_strategy: ConflictStrategy) -> ConflictStrategy {
    if matches!(conflict_strategy, ConflictStrategy::Auto) {
        ConflictStrategy::Overwrite
    } else {
        conflict_strategy
    }
}

/// Merges `old` and `new` [`LandscapeDiff`].
fn merge_landscape_diff(
    plugin: &Arc<ParsedPlugin>,
    old: &LandscapeDiff,
    new: &LandscapeDiff,
) -> LandscapeDiff {
    let mut merged = old.clone();
    merged.plugins.push((plugin.clone(), new.modified_data()));

    let coords = merged.coords;

    merged.height_map = apply_merge_strategy(
        coords,
        plugin,
        "height_map",
        old.height_map.as_ref(),
        new.height_map.as_ref(),
        load_order_auto_strategy(plugin.meta.height_map.conflict_strategy),
    );

    merged.vertex_normals = apply_merge_strategy(
        coords,
        plugin,
        "vertex_normals",
        old.vertex_normals.as_ref(),
        new.vertex_normals.as_ref(),
        load_order_auto_strategy(plugin.meta.height_map.conflict_strategy),
    );

    if let Some(vertex_normals) = merged.vertex_normals.as_ref() {
        merged.vertex_normals = Some(LandscapeDiff::apply_mask(
            vertex_normals,
            merged
                .height_map
                .as_ref()
                .map(RelativeTerrainMap::differences),
        ));
    }

    if merged.vertex_normals.is_modified() {
        assert!(merged.height_map.is_modified());
    }

    merged.world_map_data = apply_merge_strategy(
        coords,
        plugin,
        "world_map_data",
        old.world_map_data.as_ref(),
        new.world_map_data.as_ref(),
        load_order_auto_strategy(plugin.meta.world_map_data.conflict_strategy),
    );

    merged.vertex_colors = apply_merge_strategy(
        coords,
        plugin,
        "vertex_colors",
        old.vertex_colors.as_ref(),
        new.vertex_colors.as_ref(),
        load_order_auto_strategy(plugin.meta.vertex_colors.conflict_strategy),
    );

    merged.texture_indices = if matches!(
        plugin.meta.texture_indices.conflict_strategy,
        ConflictStrategy::Auto | ConflictStrategy::Overwrite
    ) {
        merge_load_order_texture_indices(old.texture_indices.as_ref(), new.texture_indices.as_ref())
    } else {
        apply_merge_strategy(
            coords,
            plugin,
            "texture_indices",
            old.texture_indices.as_ref(),
            new.texture_indices.as_ref(),
            plugin.meta.texture_indices.conflict_strategy,
        )
    };

    merged
}

/// Merges `plugin` [`LandmassDiff`] into `merged` [`LandmassDiff`].
fn merge_landmass_into(merged: &mut LandmassDiff, plugin: &LandmassDiff) {
    debug!(
        "Merging {} LAND records from {} into {}",
        plugin.land.len(),
        plugin.plugin.name,
        merged.plugin.name
    );

    for (coords, land) in plugin.sorted() {
        if let Some(existing) = merged.land.get_mut(coords) {
            let updated = merge_landscape_diff(&plugin.plugin, existing, land);
            *existing = updated;
        } else if land.is_modified() {
            let mut merged_land = land.clone();
            merged_land
                .plugins
                .push((plugin.plugin.clone(), land.modified_data()));
            merged.land.insert(*coords, merged_land);
        }
    }
}

/// Creates a [Landmass] from `parsed_plugins` and updates [`KnownTextures`].
fn create_tes3_landmass<'a>(
    plugin_name: &str,
    parsed_plugins: impl Iterator<Item = &'a Arc<ParsedPlugin>>,
    known_textures: &mut KnownTextures,
) -> Landmass {
    let plugin = Arc::new(ParsedPlugin::empty(plugin_name));
    let master_landmasses =
        parsed_plugins.filter_map(|esm| try_create_landmass(esm, known_textures));
    merge_tes3_landmasses(&plugin, master_landmasses)
}

/// Creates a [`LandmassDiff`] representing a set of empty [`LandscapeDiff`] for the `reference` [Landmass].
/// Prior to returning, the [`LandmassDiff`] will be updated by [`repair_landmass_seams`].
fn create_merged_lands_from_reference(reference: &Landmass) -> LandmassDiff {
    let mut landmass_diff = LandmassDiff::new(reference.plugin.clone());

    for (coords, land) in &reference.land {
        let allowed_data = landscape_flags(land).into();
        let plugin = reference.plugins.get(coords).expect("safe");
        let landscape_diff = LandscapeDiff::from_reference(plugin.clone(), land, allowed_data);
        assert!(!landscape_diff.is_modified());
        landmass_diff.land.insert(*coords, landscape_diff);
    }

    for land in landmass_diff.land.values_mut() {
        assert_eq!(land.plugins.len(), 1);
        let modified_data = land.modified_data();
        let plugin_data = land.plugins.get_mut(0).expect("safe");
        plugin_data.1 = modified_data;
    }

    landmass_diff
}

#[cfg(test)]
mod tests {
    use super::{
        create_merged_lands_from_reference, create_reference_and_modded_landmasses,
        merge_landmass_into, merge_load_order_texture_indices, prompted_path,
        run_merge_on_worker_thread, should_prompt_for_openmw_cfg,
    };
    use crate::io::app_config::{CONFIG_FILE_NAME, MergedLandsConfig};
    use crate::io::meta_schema::{MergeSettings, PluginMeta};
    use crate::io::parsed_plugins::{ParsedPlugin, ParsedPlugins};
    use crate::land::grid_access::Index2D;
    use crate::land::height_map::calculate_vertex_heights_tes3;
    use crate::land::terrain_map::Vec3;
    use crate::land::textures::IndexVTEX;
    use crate::merge::relative_terrain_map::{IsModified, RelativeTerrainMap};
    use crate::repair::cleaning::clean_landmass_diff;
    use clap::Parser;
    use std::fmt::Write;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tes3::esp::{
        Header, Landscape, LandscapeFlags, LandscapeTexture, ObjectFlags, Plugin, TES3Object,
        TextureIndices, VertexColors, VertexNormals,
    };

    fn unique_temp_dir(name: &str) -> PathBuf {
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

    fn write_plugin_file(
        path: &Path,
        plugin_name: &str,
        lands: Vec<Landscape>,
        textures: Vec<LandscapeTexture>,
        masters: Vec<(String, u64)>,
    ) {
        let expected_land_count = lands.len();
        let mut plugin = Plugin::new();
        plugin.objects.push(TES3Object::Header(Header {
            author: format!("test:{plugin_name}").into(),
            description: "integration fixture".to_string().into(),
            masters: Some(masters),
            ..Default::default()
        }));

        for texture in textures {
            plugin.objects.push(TES3Object::LandscapeTexture(texture));
        }

        for land in lands {
            plugin.objects.push(TES3Object::Landscape(land));
        }

        plugin.save_path(path).expect("save fixture plugin");

        let mut loaded = Plugin::new();
        loaded.load_path(path).expect("reload fixture plugin");
        let loaded_land_count = loaded
            .objects
            .iter()
            .filter(|object| matches!(object, TES3Object::Landscape(_)))
            .count();
        assert_eq!(loaded_land_count, expected_land_count);

        for land in loaded.objects.iter().filter_map(|object| match object {
            TES3Object::Landscape(land) => Some(land),
            _ => None,
        }) {
            assert!(
                land.vertex_heights.is_some(),
                "fixture LAND lost vertex heights after serialization"
            );
        }
    }

    fn fixture_land(coords: (i32, i32), height: i32, texture_index: Option<u16>) -> Landscape {
        let mut height_map = vec![[height; 65]; 65].into_boxed_slice();
        height_map[1][1] = height + 8;
        let height_map: Box<[[i32; 65]; 65]> = height_map
            .try_into()
            .expect("valid 65x65 fixture height map");

        let mut land = Landscape {
            flags: ObjectFlags::default(),
            grid: coords,
            landscape_flags: LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS
                | LandscapeFlags::UNKNOWN,
            vertex_heights: Some(calculate_vertex_heights_tes3(&height_map)),
            vertex_normals: Some(VertexNormals::default()),
            ..Landscape::default()
        };

        if let Some(index) = texture_index {
            land.landscape_flags |= LandscapeFlags::USES_TEXTURES;
            land.texture_indices = Some(TextureIndices {
                data: Box::new([[index; 16]; 16]),
            });
        }

        land
    }

    fn fixture_land_with_vertex_color(
        coords: (i32, i32),
        height: i32,
        color: Vec3<u8>,
    ) -> Landscape {
        let mut land = fixture_land(coords, height, None);
        land.landscape_flags |= LandscapeFlags::USES_VERTEX_COLORS;
        land.vertex_colors = Some(VertexColors {
            data: Box::new([[<[u8; 3]>::from(color); 65]; 65]),
        });
        land
    }

    fn parsed_plugin_with_land(name: &str, land: Landscape, meta: PluginMeta) -> Arc<ParsedPlugin> {
        let mut records = Plugin::new();
        records.objects.push(TES3Object::Header(Header {
            author: format!("test:{name}").into(),
            description: "unit fixture".to_string().into(),
            masters: Some(Vec::new()),
            ..Default::default()
        }));
        records.objects.push(TES3Object::Landscape(land));

        Arc::new(ParsedPlugin {
            name: name.to_string(),
            records,
            meta,
        })
    }

    fn merge_test_plugins(
        plugins: Vec<Arc<ParsedPlugin>>,
    ) -> (crate::LandmassDiff, Arc<crate::Landmass>) {
        let parsed_plugins = ParsedPlugins {
            masters: Vec::new(),
            plugins,
        };
        let mut known_textures = crate::land::textures::KnownTextures::new();
        let (reference_landmass, modded_landmasses, raw_load_order_landmass) =
            create_reference_and_modded_landmasses(&parsed_plugins, &mut known_textures);
        let mut merged_lands = create_merged_lands_from_reference(&reference_landmass);

        for modded_landmass in &modded_landmasses {
            merge_landmass_into(&mut merged_lands, modded_landmass);
        }

        (merged_lands, raw_load_order_landmass)
    }

    fn run_with_large_stack(test: impl FnOnce() + Send + 'static) {
        thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(test)
            .expect("spawn test thread")
            .join()
            .expect("test thread should not panic");
    }

    fn fixture_ltex(id: &str, index: u32, file_name: &str) -> LandscapeTexture {
        LandscapeTexture {
            id: id.to_string(),
            index: Some(index),
            file_name: Some(file_name.to_string()),
            ..LandscapeTexture::default()
        }
    }

    fn run_vanilla_merge(
        test_name: &str,
        plugin_names: &[&str],
        output_file_name: &str,
    ) -> (PathBuf, PathBuf) {
        run_vanilla_merge_with_config(test_name, plugin_names, output_file_name, None)
    }

    fn run_vanilla_merge_with_config(
        test_name: &str,
        plugin_names: &[&str],
        output_file_name: &str,
        app_config: Option<&str>,
    ) -> (PathBuf, PathBuf) {
        let root = unique_temp_dir(test_name);
        let data_files = root.join("Data Files");
        let output_dir = root.join("Output");
        let merged_lands_dir = root.join("MergedLands");
        let config_dir = root.join("Config");

        fs::create_dir_all(&data_files).expect("create Data Files dir");
        fs::create_dir_all(&output_dir).expect("create output dir");
        fs::create_dir_all(merged_lands_dir.join("Conflicts")).expect("create conflicts dir");
        fs::create_dir_all(&config_dir).expect("create config dir");
        if let Some(app_config) = app_config {
            fs::write(config_dir.join(CONFIG_FILE_NAME), app_config).expect("write app config");
        }

        let mut args = vec![
            "merged_lands".to_string(),
            "--vanilla".to_string(),
            "--data-files-dir".to_string(),
            data_files.to_string_lossy().to_string(),
            "--merged-lands-dir".to_string(),
            merged_lands_dir.to_string_lossy().to_string(),
            "--config-dir".to_string(),
            config_dir.to_string_lossy().to_string(),
            "--output-file-dir".to_string(),
            output_dir.to_string_lossy().to_string(),
            "--output-file".to_string(),
            output_file_name.to_string(),
            "--sort-order".to_string(),
            "none".to_string(),
        ];

        for plugin in plugin_names {
            args.push((*plugin).to_string());
        }

        let cli = crate::cli::Cli::try_parse_from(args).expect("parse cli args");
        run_merge_on_worker_thread(cli).expect("merge_all should succeed");

        (root, output_dir)
    }

    fn run_openmw_merge(
        test_name: &str,
        plugin_names: &[&str],
        output_file_name: &str,
    ) -> (PathBuf, PathBuf) {
        let root = unique_temp_dir(test_name);
        let data_files = root.join("Data Files");
        let output_dir = root.join("Output");
        let merged_lands_dir = root.join("MergedLands");
        let config_dir = root.join("Config");
        let data_local = root.join("DataLocal");
        let openmw_cfg = root.join("openmw.cfg");

        fs::create_dir_all(&data_files).expect("create Data Files dir");
        fs::create_dir_all(&output_dir).expect("create output dir");
        fs::create_dir_all(&config_dir).expect("create config dir");
        fs::create_dir_all(&data_local).expect("create data-local dir");
        fs::create_dir_all(merged_lands_dir.join("Conflicts")).expect("create conflicts dir");

        let mut cfg = format!(
            "data=\"{}\"\ndata-local=\"{}\"\n",
            data_files.to_string_lossy(),
            data_local.to_string_lossy(),
        );
        for plugin in plugin_names {
            writeln!(&mut cfg, "content=\"{plugin}\"").expect("write openmw.cfg content");
        }
        fs::write(&openmw_cfg, cfg).expect("write openmw.cfg");

        let args = vec![
            "merged_lands".to_string(),
            "--openmw-cfg".to_string(),
            openmw_cfg.to_string_lossy().to_string(),
            "--merged-lands-dir".to_string(),
            merged_lands_dir.to_string_lossy().to_string(),
            "--config-dir".to_string(),
            config_dir.to_string_lossy().to_string(),
            "--output-file-dir".to_string(),
            output_dir.to_string_lossy().to_string(),
            "--output-file".to_string(),
            output_file_name.to_string(),
            "--sort-order".to_string(),
            "none".to_string(),
        ];

        let cli = crate::cli::Cli::try_parse_from(args).expect("parse cli args");
        run_merge_on_worker_thread(cli).expect("merge_all should succeed");

        (root, output_dir)
    }

    fn load_output_plugin(path: &Path) -> Plugin {
        let mut plugin = Plugin::new();
        plugin.load_path(path).expect("load merged output");
        plugin
    }

    fn count_objects(plugin: &Plugin) -> (usize, usize, usize) {
        let mut ltex = 0;
        let mut cell = 0;
        let mut land = 0;

        for object in &plugin.objects {
            match object {
                TES3Object::LandscapeTexture(_) => ltex += 1,
                TES3Object::Cell(_) => cell += 1,
                TES3Object::Landscape(_) => land += 1,
                _ => {}
            }
        }

        (ltex, cell, land)
    }

    fn idx(v: u16) -> IndexVTEX {
        IndexVTEX::new(v)
    }

    #[test]
    fn prompted_path_strips_quotes_and_whitespace() {
        assert_eq!(
            prompted_path("  \"/tmp/openmw/openmw.cfg\"  "),
            Some(PathBuf::from("/tmp/openmw/openmw.cfg"))
        );
        assert_eq!(prompted_path("   "), None);
    }

    #[test]
    fn prompted_path_accepts_windows_paths_with_spaces() {
        let path = r"C:\Users\Pandorable\Documents\My Games\OpenMW\openmw.cfg";

        assert_eq!(prompted_path(path), Some(PathBuf::from(path)));
        assert_eq!(
            prompted_path(&format!("\"{path}\"")),
            Some(PathBuf::from(path))
        );
    }

    #[test]
    fn prompted_path_accepts_shell_escaped_spaces() {
        assert_eq!(
            prompted_path(r"/home/pandorable/My\ Games/OpenMW/openmw.cfg"),
            Some(PathBuf::from("/home/pandorable/My Games/OpenMW/openmw.cfg"))
        );
    }

    #[test]
    fn first_run_prompt_only_applies_to_unspecified_openmw_cfg() {
        let cli = crate::cli::Cli::try_parse_from(["merged_lands"]).expect("parse cli args");
        let config = MergedLandsConfig::default();

        assert!(should_prompt_for_openmw_cfg(&cli, &config, true));
        assert!(!should_prompt_for_openmw_cfg(&cli, &config, false));

        let cli_with_path =
            crate::cli::Cli::try_parse_from(["merged_lands", "--openmw-cfg", "/tmp/openmw.cfg"])
                .expect("parse cli args");
        assert!(!should_prompt_for_openmw_cfg(&cli_with_path, &config, true));

        let vanilla =
            crate::cli::Cli::try_parse_from(["merged_lands", "--vanilla"]).expect("parse cli args");
        assert!(!should_prompt_for_openmw_cfg(&vanilla, &config, true));

        let mut saved_config = MergedLandsConfig::default();
        saved_config.set_openmw_cfg(Path::new("/tmp/openmw.cfg"));
        assert!(!should_prompt_for_openmw_cfg(&cli, &saved_config, true));
    }

    #[test]
    fn load_order_texture_merge_applies_only_changed_cells_from_new() {
        let base = [[idx(0); 16]; 16];

        let mut old = RelativeTerrainMap::<IndexVTEX, 16>::empty(base);
        old.set_value(Index2D::new(0, 0), idx(10));

        let mut new = RelativeTerrainMap::<IndexVTEX, 16>::empty(base);
        new.set_value(Index2D::new(1, 1), idx(40));

        let merged = merge_load_order_texture_indices(Some(&old), Some(&new)).expect("merged map");

        assert_eq!(merged.get_value(Index2D::new(0, 0)).as_u16(), 10);
        assert_eq!(merged.get_value(Index2D::new(1, 1)).as_u16(), 40);
        assert_eq!(merged.get_value(Index2D::new(0, 1)).as_u16(), 0);
        assert_eq!(merged.get_value(Index2D::new(1, 0)).as_u16(), 0);
    }

    #[test]
    fn load_order_texture_merge_returns_old_when_new_has_no_effective_changes() {
        let base = [[idx(0); 16]; 16];

        let mut old = RelativeTerrainMap::<IndexVTEX, 16>::empty(base);
        old.set_value(Index2D::new(0, 0), idx(10));

        let new = RelativeTerrainMap::<IndexVTEX, 16>::empty(base);

        let merged = merge_load_order_texture_indices(Some(&old), Some(&new)).expect("merged map");
        assert!(merged.is_modified());
        assert_eq!(merged.get_value(Index2D::new(0, 0)).as_u16(), 10);
        assert_eq!(merged.get_value(Index2D::new(1, 1)).as_u16(), 0);
    }

    #[test]
    fn auto_height_merge_preserves_late_load_order_winner() {
        run_with_large_stack(|| {
            let coords = (0, 0);
            let plugin_a = parsed_plugin_with_land(
                "A.esp",
                fixture_land(coords, 100, None),
                PluginMeta::default(),
            );
            let plugin_b = parsed_plugin_with_land(
                "B.esp",
                fixture_land(coords, 200, None),
                PluginMeta::default(),
            );

            let (merged_lands, _) = merge_test_plugins(vec![plugin_a, plugin_b]);
            let land = merged_lands
                .land
                .get(&crate::Vec2::new(coords.0, coords.1))
                .expect("merged LAND should exist");
            let height_map = land
                .height_map
                .as_ref()
                .expect("height map should be merged");

            assert_eq!(height_map.get_value(Index2D::new(1, 1)), 208);
        });
    }

    #[test]
    fn excluded_height_does_not_advance_rolling_reference() {
        run_with_large_stack(|| {
            let coords = (0, 0);
            let excluded_height = PluginMeta {
                height_map: MergeSettings {
                    included: false,
                    ..MergeSettings::default()
                },
                ..PluginMeta::default()
            };
            let plugin_a =
                parsed_plugin_with_land("A.esp", fixture_land(coords, 100, None), excluded_height);
            let plugin_b = parsed_plugin_with_land(
                "B.esp",
                fixture_land(coords, 100, None),
                PluginMeta::default(),
            );

            let parsed_plugins = ParsedPlugins {
                masters: Vec::new(),
                plugins: vec![plugin_a, plugin_b],
            };
            let mut known_textures = crate::land::textures::KnownTextures::new();
            let (_, modded_landmasses, _) =
                create_reference_and_modded_landmasses(&parsed_plugins, &mut known_textures);
            let land = modded_landmasses[1]
                .land
                .get(&crate::Vec2::new(coords.0, coords.1))
                .expect("second plugin LAND diff should exist");

            assert!(land.height_map.is_modified());
        });
    }

    #[test]
    fn cleanup_keeps_output_needed_to_override_excluded_loaded_data() {
        run_with_large_stack(|| {
            let coords = (0, 0);
            let plugin_a = parsed_plugin_with_land(
                "A.esp",
                fixture_land(coords, 10, None),
                PluginMeta::default(),
            );

            let excluded_height = PluginMeta {
                height_map: MergeSettings {
                    included: false,
                    ..MergeSettings::default()
                },
                ..PluginMeta::default()
            };
            let plugin_b = parsed_plugin_with_land(
                "B.esp",
                fixture_land_with_vertex_color(coords, 20, Vec3::new(1, 2, 3)),
                excluded_height,
            );

            let (mut merged_lands, raw_load_order_landmass) =
                merge_test_plugins(vec![plugin_a, plugin_b]);
            let coords = crate::Vec2::new(coords.0, coords.1);
            assert!(merged_lands.land.contains_key(&coords));

            clean_landmass_diff(&mut merged_lands, &raw_load_order_landmass);

            assert!(
                merged_lands.land.contains_key(&coords),
                "output LAND is needed to override the excluded height from the loaded source stack"
            );
        });
    }

    #[test]
    fn cleanup_keeps_reference_output_needed_to_override_excluded_later_data() {
        run_with_large_stack(|| {
            let coords = (0, 0);
            let master = parsed_plugin_with_land(
                "Master.esm",
                fixture_land(coords, 10, None),
                PluginMeta::default(),
            );
            let excluded_height = PluginMeta {
                height_map: MergeSettings {
                    included: false,
                    ..MergeSettings::default()
                },
                ..PluginMeta::default()
            };
            let plugin = parsed_plugin_with_land(
                "Patch.esp",
                fixture_land(coords, 20, None),
                excluded_height,
            );

            let parsed_plugins = ParsedPlugins {
                masters: vec![master],
                plugins: vec![plugin],
            };
            let mut known_textures = crate::land::textures::KnownTextures::new();
            let (reference_landmass, modded_landmasses, raw_load_order_landmass) =
                create_reference_and_modded_landmasses(&parsed_plugins, &mut known_textures);
            let mut merged_lands = create_merged_lands_from_reference(&reference_landmass);

            for modded_landmass in &modded_landmasses {
                merge_landmass_into(&mut merged_lands, modded_landmass);
            }

            let coords = crate::Vec2::new(coords.0, coords.1);
            assert!(merged_lands.land.contains_key(&coords));

            clean_landmass_diff(&mut merged_lands, &raw_load_order_landmass);

            assert!(
                merged_lands.land.contains_key(&coords),
                "reference-equivalent output LAND is still needed when excluded later data would otherwise win"
            );
        });
    }

    #[test]
    fn cleanup_removes_new_cell_output_when_only_excluded_height_differs() {
        run_with_large_stack(|| {
            let coords = (0, 0);
            let excluded_height = PluginMeta {
                height_map: MergeSettings {
                    included: false,
                    ..MergeSettings::default()
                },
                ..PluginMeta::default()
            };
            let plugin = parsed_plugin_with_land(
                "Patch.esp",
                fixture_land_with_vertex_color(coords, 20, Vec3::new(1, 2, 3)),
                excluded_height,
            );

            let (mut merged_lands, raw_load_order_landmass) = merge_test_plugins(vec![plugin]);
            let coords = crate::Vec2::new(coords.0, coords.1);
            assert!(merged_lands.land.contains_key(&coords));

            clean_landmass_diff(&mut merged_lands, &raw_load_order_landmass);

            assert!(
                !merged_lands.land.contains_key(&coords),
                "a new cell with no replacement height should not be kept just to materialize default height"
            );
        });
    }

    #[test]
    fn all_excluded_new_land_does_not_create_default_output_land() {
        run_with_large_stack(|| {
            let coords = (0, 0);
            let excluded_land = PluginMeta {
                height_map: MergeSettings {
                    included: false,
                    ..MergeSettings::default()
                },
                vertex_colors: MergeSettings {
                    included: false,
                    ..MergeSettings::default()
                },
                texture_indices: MergeSettings {
                    included: false,
                    ..MergeSettings::default()
                },
                world_map_data: MergeSettings {
                    included: false,
                    ..MergeSettings::default()
                },
                ..PluginMeta::default()
            };
            let plugin = parsed_plugin_with_land(
                "Patch.esp",
                fixture_land_with_vertex_color(coords, 20, Vec3::new(1, 2, 3)),
                excluded_land,
            );

            let (mut merged_lands, raw_load_order_landmass) = merge_test_plugins(vec![plugin]);
            let coords = crate::Vec2::new(coords.0, coords.1);
            assert!(!merged_lands.land.contains_key(&coords));

            clean_landmass_diff(&mut merged_lands, &raw_load_order_landmass);

            assert!(
                !merged_lands.land.contains_key(&coords),
                "an all-excluded new LAND cell must not become a default generated LAND cell"
            );
        });
    }

    #[test]
    fn e2e_single_plugin_writes_app_config_and_header() {
        let root = unique_temp_dir("e2e_single_plugin");
        let data_files = root.join("Data Files");
        fs::create_dir_all(&data_files).expect("create Data Files");

        let plugin_name = "One.esp";
        write_plugin_file(
            &data_files.join(plugin_name),
            plugin_name,
            vec![fixture_land((0, 0), 24, None)],
            vec![],
            vec![],
        );

        let (root, output_dir) =
            run_vanilla_merge("e2e_single_plugin_run", &[plugin_name], "MergedTest.esp");
        let merged_path = output_dir.join("MergedTest.esp");
        let merged = load_output_plugin(&merged_path);
        assert!(matches!(
            merged.objects.first(),
            Some(TES3Object::Header(_))
        ));

        let meta_path = output_dir.join("MergedTest.mergedlands.toml");
        assert!(
            !meta_path.exists(),
            "generated output should not create a plugin sidecar meta file"
        );

        let config_path = root.join("Config").join(CONFIG_FILE_NAME);
        let config_text = fs::read_to_string(config_path).expect("read app config");
        let parsed: toml::Value = toml::from_str(&config_text).expect("parse app config");
        let generated_outputs = parsed["generated_output_files"]
            .as_array()
            .expect("generated output list");
        assert!(
            generated_outputs
                .iter()
                .any(|name| name.as_str() == Some("MergedTest.esp"))
        );
    }

    #[test]
    fn e2e_app_config_ignore_plugins_skips_plugin_before_merge() {
        let root = unique_temp_dir("e2e_ignore_plugin");
        let data_files = root.join("Data Files");
        let output_dir = root.join("Output");
        let merged_lands_dir = root.join("MergedLands");
        let config_dir = root.join("Config");
        fs::create_dir_all(&data_files).expect("create Data Files");
        fs::create_dir_all(&output_dir).expect("create output dir");
        fs::create_dir_all(merged_lands_dir.join("Conflicts")).expect("create conflicts dir");
        fs::create_dir_all(&config_dir).expect("create config dir");

        let ignored_plugin = "Ignored.esp";
        write_plugin_file(
            &data_files.join(ignored_plugin),
            ignored_plugin,
            vec![fixture_land((0, 0), 64, None)],
            vec![],
            vec![],
        );
        fs::write(
            config_dir.join(CONFIG_FILE_NAME),
            "ignore_plugins = [\"Ignored.esp\"]\n",
        )
        .expect("write app config");

        let cli = crate::cli::Cli::try_parse_from([
            "merged_lands",
            "--vanilla",
            "--data-files-dir",
            data_files.to_str().expect("data path utf8"),
            "--merged-lands-dir",
            merged_lands_dir.to_str().expect("merged path utf8"),
            "--config-dir",
            config_dir.to_str().expect("config path utf8"),
            "--output-file-dir",
            output_dir.to_str().expect("output path utf8"),
            "--output-file",
            "IgnoredOut.esp",
            "--sort-order",
            "none",
            ignored_plugin,
        ])
        .expect("parse cli args");
        run_merge_on_worker_thread(cli).expect("merge should succeed");

        let merged = load_output_plugin(&output_dir.join("IgnoredOut.esp"));
        let (_ltex_count, _cell_count, land_count) = count_objects(&merged);
        assert_eq!(land_count, 0);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn e2e_app_config_ignore_plugins_from_path_skips_plugin_before_merge() {
        let root = unique_temp_dir("e2e_ignore_plugin_path");
        let data_files = root.join("Data Files");
        let output_dir = root.join("Output");
        let merged_lands_dir = root.join("MergedLands");
        let config_dir = root.join("Config");
        fs::create_dir_all(&data_files).expect("create Data Files");
        fs::create_dir_all(&output_dir).expect("create output dir");
        fs::create_dir_all(merged_lands_dir.join("Conflicts")).expect("create conflicts dir");
        fs::create_dir_all(&config_dir).expect("create config dir");

        let ignored_plugin = "PathIgnored.esp";
        write_plugin_file(
            &data_files.join(ignored_plugin),
            ignored_plugin,
            vec![fixture_land((0, 0), 64, None)],
            vec![],
            vec![],
        );
        fs::write(
            config_dir.join(CONFIG_FILE_NAME),
            format!(
                "ignore_plugins_from_path = [\"{}\"]\n",
                data_files.to_string_lossy()
            ),
        )
        .expect("write app config");

        let cli = crate::cli::Cli::try_parse_from([
            "merged_lands",
            "--vanilla",
            "--data-files-dir",
            data_files.to_str().expect("data path utf8"),
            "--merged-lands-dir",
            merged_lands_dir.to_str().expect("merged path utf8"),
            "--config-dir",
            config_dir.to_str().expect("config path utf8"),
            "--output-file-dir",
            output_dir.to_str().expect("output path utf8"),
            "--output-file",
            "PathIgnoredOut.esp",
            "--sort-order",
            "none",
            ignored_plugin,
        ])
        .expect("parse cli args");
        run_merge_on_worker_thread(cli).expect("merge should succeed");

        let merged = load_output_plugin(&output_dir.join("PathIgnoredOut.esp"));
        let (_ltex_count, _cell_count, land_count) = count_objects(&merged);
        assert_eq!(land_count, 0);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn e2e_output_is_cell_free_when_cleaned() {
        let root = unique_temp_dir("e2e_cell_toggle");
        let data_files = root.join("Data Files");
        fs::create_dir_all(&data_files).expect("create Data Files");

        let plugin_a = "CellsA.esp";
        let plugin_b = "CellsB.esp";
        write_plugin_file(
            &data_files.join(plugin_a),
            plugin_a,
            vec![fixture_land((1, 1), 32, None)],
            vec![],
            vec![],
        );

        write_plugin_file(
            &data_files.join(plugin_b),
            plugin_b,
            vec![fixture_land((1, 1), 96, None)],
            vec![],
            vec![],
        );

        let (_root_with_cells, output_with_cells) =
            run_openmw_merge("e2e_cells_on", &[plugin_a, plugin_b], "WithCells.esp");
        let with_cells = load_output_plugin(&output_with_cells.join("WithCells.esp"));
        let (_, with_cell_count, with_land_count) = count_objects(&with_cells);
        assert_eq!(with_cell_count, 0);
        assert_eq!(with_land_count, 0);

        let (_root_no_cells, output_no_cells) =
            run_openmw_merge("e2e_cells_off", &[plugin_a, plugin_b], "NoCells.esp");
        let no_cells = load_output_plugin(&output_no_cells.join("NoCells.esp"));
        let (_, no_cell_count, no_land_count) = count_objects(&no_cells);
        assert_eq!(no_cell_count, 0);
        assert_eq!(no_land_count, 0);
    }

    #[test]
    fn e2e_ltex_pruning_removes_all_textures_when_no_land_remains() {
        let root = unique_temp_dir("e2e_ltex_prune");
        let data_files = root.join("Data Files");
        fs::create_dir_all(&data_files).expect("create Data Files");

        let plugin_name = "Textures.esp";
        write_plugin_file(
            &data_files.join(plugin_name),
            plugin_name,
            vec![fixture_land((2, 2), 48, Some(1))],
            vec![
                fixture_ltex("used", 1, "used.dds"),
                fixture_ltex("unused", 2, "unused.dds"),
            ],
            vec![],
        );

        write_plugin_file(
            &data_files.join("TexturesPatch.esp"),
            "TexturesPatch.esp",
            vec![fixture_land((2, 2), 96, Some(1))],
            vec![],
            vec![],
        );

        let (_root_run, output_dir) = run_openmw_merge(
            "e2e_ltex_prune_run",
            &[plugin_name, "TexturesPatch.esp"],
            "LtexOut.esp",
        );
        let merged = load_output_plugin(&output_dir.join("LtexOut.esp"));
        let (ltex_count, _cell_count, land_count) = count_objects(&merged);

        assert_eq!(land_count, 0);
        assert_eq!(ltex_count, 0);
        let names: Vec<_> = merged
            .objects
            .iter()
            .filter_map(|object| match object {
                TES3Object::LandscapeTexture(texture) => Some(texture.id.as_str()),
                _ => None,
            })
            .collect();
        assert!(names.is_empty());
    }

    #[test]
    fn e2e_overlapping_plugins_can_clean_to_empty_output() {
        let root = unique_temp_dir("e2e_overlapping_cleaned");
        let data_files = root.join("Data Files");
        fs::create_dir_all(&data_files).expect("create Data Files");

        write_plugin_file(
            &data_files.join("A.esp"),
            "A.esp",
            vec![fixture_land((10, 10), 16, None)],
            vec![],
            vec![],
        );

        write_plugin_file(
            &data_files.join("B.esp"),
            "B.esp",
            vec![fixture_land((10, 10), 64, None)],
            vec![],
            vec![],
        );

        let (_root_run, output_dir) = run_openmw_merge(
            "e2e_overlapping_cleaned_run",
            &["A.esp", "B.esp"],
            "Both.esp",
        );
        let merged = load_output_plugin(&output_dir.join("Both.esp"));
        let (_ltex_count, cell_count, land_count) = count_objects(&merged);
        assert_eq!(cell_count, 0);
        assert_eq!(land_count, 0);

        let mut coords: Vec<_> = merged
            .objects
            .iter()
            .filter_map(|object| match object {
                TES3Object::Landscape(land) => Some(land.grid),
                _ => None,
            })
            .collect();
        coords.sort_unstable();
        assert!(coords.is_empty());

        let header = merged
            .objects
            .iter()
            .find_map(|object| match object {
                TES3Object::Header(header) => Some(header),
                _ => None,
            })
            .expect("output should include header");
        let master_names: Vec<_> = header
            .masters
            .as_ref()
            .map(|masters| {
                masters
                    .iter()
                    .map(|entry| entry.0.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert!(master_names.is_empty());
    }
}
