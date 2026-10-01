//! Platform locations and environment overrides for `OpenMW` config discovery.

use anyhow::{Context, Result, anyhow};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(super) struct OpenMWPaths {
    pub user_config: PathBuf,
    pub user_data: PathBuf,
    pub local: PathBuf,
    pub global: Option<PathBuf>,
    global_config: Option<PathBuf>,
    home: Option<PathBuf>,
}

impl OpenMWPaths {
    #[cfg(test)]
    pub(super) fn fixture(root: &Path) -> Self {
        Self {
            user_config: root.join("userconfig"),
            user_data: root.join("userdata"),
            local: root.join("engine"),
            global: Some(root.join("global")),
            global_config: Some(root.join("globalconfig")),
            home: Some(root.to_path_buf()),
        }
    }

    pub fn from_env() -> Result<Self> {
        let get_env = |name: &str| env::var_os(name);
        let home = env::home_dir();
        let executable = env::current_exe().context("Unable to locate the executable")?;
        let local = engine_directory(executable.parent().unwrap_or(Path::new(".")));
        Self::for_platform(
            env::consts::OS,
            &get_env,
            home,
            documents_dir(),
            local,
            Path::new("/.flatpak-info").is_file(),
        )
    }

    fn for_platform(
        os: &str,
        get_env: &impl Fn(&str) -> Option<OsString>,
        home: Option<PathBuf>,
        documents: Option<PathBuf>,
        local: PathBuf,
        flatpak_info: bool,
    ) -> Result<Self> {
        let value = |name| get_env(name).filter(|value| !value.is_empty());
        let home_path = || home.clone().context("Unable to locate the home directory");
        let (user_config, user_data, global) = match os {
            "android" => (
                PathBuf::from("/storage/emulated/0/Alpha3/config"),
                PathBuf::from("/storage/emulated/0/Alpha3"),
                None,
            ),
            "windows" => {
                let documents = match documents {
                    Some(path) => path,
                    None => home_path()?.join("Documents"),
                };
                let user = documents.join("My Games/OpenMW");
                (user.clone(), user, None)
            }
            "macos" => (
                home_path()?.join("Library/Preferences/openmw"),
                home_path()?.join("Library/Application Support/openmw"),
                Some(PathBuf::from("/Library/Application Support/openmw")),
            ),
            _ => {
                let flatpak = os == "linux"
                    && (get_env("OPENMW_CONFIG_USING_FLATPAK").is_some()
                        || get_env("FLATPAK_ID").is_some()
                        || flatpak_info);
                let (config, data) = if flatpak {
                    let app = value("OPENMW_FLATPAK_ID")
                        .or_else(|| value("FLATPAK_ID"))
                        .unwrap_or_else(|| "org.openmw.OpenMW".into());
                    let base = home_path()?.join(".var/app").join(app);
                    (base.join("config"), base.join("data"))
                } else {
                    (
                        match value("XDG_CONFIG_HOME") {
                            Some(path) => PathBuf::from(path),
                            None => home_path()?.join(".config"),
                        },
                        match value("XDG_DATA_HOME") {
                            Some(path) => PathBuf::from(path),
                            None => home_path()?.join(".local/share"),
                        },
                    )
                };
                (
                    config.join("openmw"),
                    data.join("openmw"),
                    Some(PathBuf::from("/usr/share/games/openmw")),
                )
            }
        };
        Ok(Self {
            user_config,
            user_data,
            local,
            global,
            global_config: matches!(os, "linux" | "freebsd" | "openbsd")
                .then(|| PathBuf::from("/etc/openmw")),
            home,
        })
    }

    pub fn expand_home(&self, path: &Path) -> PathBuf {
        let mut components = path.components();
        if components
            .next()
            .is_some_and(|part| part.as_os_str() == "~")
            && let Some(home) = &self.home
        {
            return home.join(components.as_path());
        }
        path.to_path_buf()
    }

    pub fn discover(&self, get_env: &impl Fn(&str) -> Option<OsString>) -> PathBuf {
        if let Some(path) = get_env("OPENMW_CONFIG").filter(|path| !path.is_empty()) {
            return self.expand_home(Path::new(&path));
        }
        if let Some(dirs) = get_env("OPENMW_CONFIG_DIR") {
            for dir in env::split_paths(&dirs).filter(|dir| !dir.as_os_str().is_empty()) {
                let path = self.expand_home(&dir).join("openmw.cfg");
                if path.is_file() {
                    return path;
                }
            }
        }
        let local = self.local.join("openmw.cfg");
        if local.is_file() {
            return local;
        }
        if let Some(global) = &self.global_config {
            let global = global.join("openmw.cfg");
            if global.is_file() {
                return global;
            }
        }
        // A standalone user config remains usable when the engine isn't installed.
        self.user_config.join("openmw.cfg")
    }
}

/// The standard user config directory, independent of a selected profile.
pub fn default_config_dir() -> Result<PathBuf> {
    Ok(OpenMWPaths::from_env()?.user_config)
}

pub(super) fn config_file_path(path: &Path) -> Result<PathBuf> {
    let path = if path.is_dir() {
        path.join("openmw.cfg")
    } else {
        path.to_path_buf()
    };
    let metadata =
        fs::metadata(&path).with_context(|| anyhow!("Unable to locate {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file(),
        "Config is not a file: {}",
        path.display()
    );
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()
            .context("Unable to locate the working directory")?
            .join(path))
    }
}

fn engine_directory(tool_dir: &Path) -> PathBuf {
    let executable = if cfg!(windows) {
        "openmw.exe"
    } else {
        "openmw"
    };
    let search_path = env::var_os("PATH").unwrap_or_default();
    let mut candidates = vec![tool_dir.join(executable)];
    candidates.extend(env::split_paths(&search_path).map(|dir| dir.join(executable)));
    if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from(
            "/Applications/OpenMW.app/Contents/MacOS/openmw",
        ));
    }
    if cfg!(windows)
        && let Some(program_files) = env::var_os("ProgramFiles")
    {
        candidates.push(PathBuf::from(program_files).join("OpenMW/openmw.exe"));
    }
    for candidate in candidates {
        if candidate.is_file()
            && let Ok(path) = fs::canonicalize(&candidate)
            && let Some(dir) = path.parent()
        {
            if cfg!(target_os = "macos") && dir.file_name() == Some(OsStr::new("MacOS")) {
                return dir.parent().expect("MacOS has a parent").join("Resources");
            }
            return dir.to_path_buf();
        }
    }
    tool_dir.to_path_buf()
}

#[cfg(not(windows))]
fn documents_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn platform_defaults_cover_linux_macos_windows_and_android() {
        let home = PathBuf::from("/home/test");
        for (os, config, data) in [
            (
                "linux",
                "/home/test/.config/openmw",
                "/home/test/.local/share/openmw",
            ),
            (
                "macos",
                "/home/test/Library/Preferences/openmw",
                "/home/test/Library/Application Support/openmw",
            ),
            (
                "windows",
                "/redirected/Documents/My Games/OpenMW",
                "/redirected/Documents/My Games/OpenMW",
            ),
            (
                "android",
                "/storage/emulated/0/Alpha3/config",
                "/storage/emulated/0/Alpha3",
            ),
        ] {
            let paths = OpenMWPaths::for_platform(
                os,
                &|_| None,
                Some(home.clone()),
                Some(PathBuf::from("/redirected/Documents")),
                PathBuf::from("/engine"),
                false,
            )
            .expect("paths");
            assert_eq!(paths.user_config, Path::new(config));
            assert_eq!(paths.user_data, Path::new(data));
        }
    }

    #[test]
    fn xdg_and_flatpak_overrides_preserve_config_and_output_locations() {
        let mut environment = HashMap::<&str, OsString>::from([
            ("XDG_CONFIG_HOME", "/xdg/config".into()),
            ("XDG_DATA_HOME", "/xdg/data".into()),
        ]);
        let paths_for = |vars: &HashMap<&str, OsString>| {
            OpenMWPaths::for_platform(
                "linux",
                &|key| vars.get(key).cloned(),
                Some(PathBuf::from("/home/test")),
                None,
                PathBuf::from("/engine"),
                false,
            )
            .expect("paths")
        };
        let xdg = paths_for(&environment);
        assert_eq!(xdg.user_config, Path::new("/xdg/config/openmw"));
        assert_eq!(xdg.user_data, Path::new("/xdg/data/openmw"));
        environment.insert("OPENMW_CONFIG_USING_FLATPAK", "".into());
        let flatpak = paths_for(&environment);
        assert_eq!(
            flatpak.user_config,
            Path::new("/home/test/.var/app/org.openmw.OpenMW/config/openmw")
        );
        assert_eq!(
            flatpak.user_data,
            Path::new("/home/test/.var/app/org.openmw.OpenMW/data/openmw")
        );
        environment.insert("OPENMW_FLATPAK_ID", "custom.OpenMW".into());
        assert_eq!(
            paths_for(&environment).user_config,
            Path::new("/home/test/.var/app/custom.OpenMW/config/openmw")
        );
    }

    #[test]
    fn config_discovery_uses_environment_precedence_and_first_existing_directory() {
        let root = env::temp_dir().join(format!(
            "merged_lands_discovery_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let first = root.join("first");
        let second = root.join("second");
        for dir in [&first, &second] {
            fs::create_dir_all(dir).expect("directory");
            fs::write(dir.join("openmw.cfg"), "").expect("config");
        }
        let paths = OpenMWPaths::fixture(&root);
        let mut environment = HashMap::<&str, OsString>::from([(
            "OPENMW_CONFIG_DIR",
            env::join_paths([root.join("missing"), first.clone(), second]).expect("path list"),
        )]);
        assert_eq!(
            paths.discover(&|key| environment.get(key).cloned()),
            first.join("openmw.cfg")
        );
        environment.insert("OPENMW_CONFIG", "~/explicit/openmw.cfg".into());
        // An invalid explicit override must surface an error when loaded, not fall through.
        assert_eq!(
            paths.discover(&|key| environment.get(key).cloned()),
            root.join("explicit/openmw.cfg")
        );
        environment.insert("OPENMW_CONFIG", first.clone().into_os_string());
        assert_eq!(
            config_file_path(&paths.discover(&|key| environment.get(key).cloned()))
                .expect("resolve directory"),
            fs::canonicalize(first.join("openmw.cfg")).expect("canonical")
        );
        environment.clear();
        assert_eq!(
            paths.discover(&|_| None),
            paths.user_config.join("openmw.cfg")
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn config_discovery_prefers_engine_local_then_global_then_standalone_user_config() {
        let root = env::temp_dir().join(format!(
            "merged_lands_engine_discovery_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let paths = OpenMWPaths::fixture(&root);
        let local = paths.local.join("openmw.cfg");
        let global = paths
            .global_config
            .as_ref()
            .expect("global config")
            .join("openmw.cfg");
        let user = paths.user_config.join("openmw.cfg");
        for file in [&local, &global, &user] {
            fs::create_dir_all(file.parent().expect("parent")).expect("directory");
            fs::write(file, "").expect("config");
        }
        assert_eq!(paths.discover(&|_| None), local);
        fs::remove_file(&local).expect("remove local fixture");
        assert_eq!(paths.discover(&|_| None), global);
        fs::remove_file(&global).expect("remove global fixture");
        assert_eq!(paths.discover(&|_| None), user);
        fs::remove_dir_all(root).expect("cleanup");
    }
}

#[cfg(windows)]
fn documents_dir() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};

    let mut pointer = std::ptr::null_mut();
    // SAFETY: The API initializes pointer to a COM-allocated, NUL-terminated UTF-16 string.
    // Copy it into an owned path before releasing the allocation with CoTaskMemFree.
    unsafe {
        let result =
            SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut pointer);
        let path = if result >= 0 && !pointer.is_null() {
            let mut length = 0;
            while *pointer.add(length) != 0 {
                length += 1;
            }
            Some(PathBuf::from(OsString::from_wide(
                std::slice::from_raw_parts(pointer, length),
            )))
        } else {
            None
        };
        CoTaskMemFree(pointer.cast());
        path
    }
}
