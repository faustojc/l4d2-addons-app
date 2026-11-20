//! Persistent settings, profiles, install records and Workshop cache (JSON in %APPDATA%).

use crate::kv;
use crate::steam::{Details, PageInfo};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct InstallRecord {
    pub time_updated: u64,
    pub size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub game_dir: Option<PathBuf>,
    pub install_to_workshop_dir: bool,
    pub enable_after_install: bool,
    pub install_dependencies: bool,
    pub auto_enable_deps: bool,
    pub check_updates_on_start: bool,
    pub scan_dependencies: bool,
    pub max_downloads: usize,
    pub dark: bool,
    pub ui_scale: f32,
    pub search_descriptions: bool,
    /// Load Windows CJK fonts so Chinese/Japanese/Korean descriptions render (~30 MB RAM).
    pub cjk_fonts: bool,
    /// Optional Steam Web API key: dependency info via API instead of scraping item pages.
    pub steam_api_key: String,
    /// name -> enabled add-on keys
    pub profiles: BTreeMap<String, Vec<String>>,
    /// Workshop id -> what we installed (used for precise update checks)
    pub installed: HashMap<u64, InstallRecord>,
    /// Lower-cased addonlist key -> enabled, as this app last wrote it. A different value in
    /// addonlist.txt means something else changed it (the in-game menu, or a crash resetting it).
    pub saved_states: HashMap<String, bool>,
    /// add-on key -> personal note
    pub notes: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            game_dir: None,
            install_to_workshop_dir: false,
            enable_after_install: true,
            install_dependencies: true,
            auto_enable_deps: true,
            check_updates_on_start: true,
            scan_dependencies: true,
            max_downloads: 3,
            dark: true,
            ui_scale: 1.0,
            search_descriptions: false,
            cjk_fonts: true,
            steam_api_key: String::new(),
            profiles: BTreeMap::new(),
            installed: HashMap::new(),
            saved_states: HashMap::new(),
            notes: HashMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Cache {
    pub details: HashMap<u64, Details>,
    pub pages: HashMap<u64, PageInfo>,
}

pub fn data_dir() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".config")))
    };
    // Folder name kept from the first release so existing settings carry over.
    let d = base.unwrap_or_else(|| PathBuf::from(".")).join("L4D2AddonManager");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn load<T: Default + for<'de> Deserialize<'de>>(name: &str) -> T {
    std::fs::read(data_dir().join(name))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save<T: Serialize>(name: &str, v: &T) {
    let path = data_dir().join(name);
    let tmp = path.with_extension("tmp");
    if let Ok(b) = serde_json::to_vec(v) {
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

pub fn is_game_dir(p: &Path) -> bool {
    p.join("left4dead2").join("gameinfo.txt").is_file()
}

/// Finds the Left 4 Dead 2 install: next to this exe, then every Steam library.
pub fn detect_game_dir() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(found) = exe.ancestors().find(|p| is_game_dir(p)) {
            return Some(found.to_path_buf());
        }
    }
    let mut steam_roots: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(o) = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
            .creation_flags(0x0800_0000)
            .output()
        {
            let out = String::from_utf8_lossy(&o.stdout);
            if let Some(i) = out.find("REG_SZ") {
                steam_roots.push(PathBuf::from(out[i + 6..].trim()));
            }
        }
    }
    #[cfg(windows)]
    steam_roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    #[cfg(not(windows))]
    {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        for p in [
            ".local/share/Steam",
            ".steam/steam",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam", // Flatpak
            "snap/steam/common/.local/share/Steam",
            "Library/Application Support/Steam", // macOS
        ] {
            steam_roots.push(home.join(p));
        }
    }
    for root in steam_roots {
        let mut libs = vec![root.clone()];
        if let Ok(b) = std::fs::read(root.join("steamapps").join("libraryfolders.vdf")) {
            for (_, lib) in kv::parse_root(&b).pairs() {
                if let Some(p) = lib.str("path") {
                    libs.push(PathBuf::from(p.replace("\\\\", "\\")));
                }
            }
        }
        for l in libs {
            let p = l.join("steamapps").join("common").join("Left 4 Dead 2");
            if is_game_dir(&p) {
                return Some(p);
            }
        }
    }
    None
}
