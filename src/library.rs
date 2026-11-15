//! Local add-on library: scanning VPKs, addonlist.txt, file conflicts.

use crate::{kv, vpk};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Default, Clone, Debug)]
pub struct AddonInfo {
    pub title: String,
    pub author: String,
    pub version: String,
    pub description: String,
    pub url: String,
    pub content: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Addon {
    /// Key as stored in addonlist.txt, e.g. `123.vpk` or `workshop\123.vpk`.
    pub key: String,
    pub path: PathBuf,
    pub id: Option<u64>,
    pub in_workshop_dir: bool,
    pub size: u64,
    pub mtime: u64,
    pub info: AddonInfo,
    pub has_image: bool,
    pub files: Vec<(Box<str>, u32)>,
    pub enabled: bool,
    pub error: Option<String>,
    /// Indices into `Library::conflicts`.
    pub conflicts: Vec<usize>,
}

impl Addon {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct Conflict {
    pub a: usize,
    pub b: usize,
    /// Paths both add-ons ship with different contents.
    pub files: Vec<Box<str>>,
    /// Paths both ship with byte-identical contents (harmless).
    pub identical: usize,
    /// How many of `files` replace a stock game file (skins, sounds, scripts: the classic clash).
    pub stock: usize,
}

#[derive(Default)]
pub struct Library {
    pub addons: Vec<Addon>,
    pub conflicts: Vec<Conflict>,
    /// Stock game paths that at least one add-on overrides.
    pub stock: HashSet<Box<str>>,
}

impl Library {
    pub fn index_of_key(&self, key: &str) -> Option<usize> {
        self.addons
            .iter()
            .position(|a| a.key.eq_ignore_ascii_case(key))
    }
    pub fn by_id(&self, id: u64) -> impl Iterator<Item = &Addon> {
        self.addons.iter().filter(move |a| a.id == Some(id))
    }
    pub fn conflict_active(&self, c: &Conflict) -> bool {
        self.addons[c.a].enabled && self.addons[c.b].enabled && !c.files.is_empty()
    }
    pub fn active_conflicts(&self, i: usize) -> usize {
        self.addons[i]
            .conflicts
            .iter()
            .filter(|&&c| self.conflict_active(&self.conflicts[c]))
            .count()
    }
}

/// Files every add-on is expected to ship, or that the game merges instead of overriding.
fn ignored(path: &str) -> bool {
    matches!(
        path,
        "addoninfo.txt" | "addonimage.jpg" | "addonimage.vtf" | "addonimage.png"
    ) || (path.starts_with("scripts/vscripts/")
        && path.ends_with("_addon.nut")
        && path.matches('/').count() == 2)
}

fn content_category(key: &str) -> Option<&'static str> {
    Some(match key.to_ascii_lowercase().as_str() {
        "addoncontent_campaign" | "addoncontent_map" => "Campaigns",
        "addoncontent_skin" => "Skins",
        "addoncontent_weapon" => "Weapons",
        "addoncontent_bossinfected" | "addoncontent_commoninfected" => "Infected",
        "addoncontent_survivor" => "Survivors",
        "addoncontent_sound" | "addoncontent_music" => "Sounds",
        "addoncontent_script" => "Scripts",
        "addoncontent_prop" => "Items",
        _ => return None,
    })
}

fn load_addon(path: &Path, in_workshop_dir: bool) -> Addon {
    let file = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let key = if in_workshop_dir {
        format!("workshop\\{file}")
    } else {
        file.clone()
    };
    let id = file
        .strip_suffix(".vpk")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 1000);
    let meta = std::fs::metadata(path).ok();
    let mut a = Addon {
        key,
        path: path.to_path_buf(),
        id,
        in_workshop_dir,
        size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
        mtime: meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0),
        info: AddonInfo::default(),
        has_image: false,
        files: Vec::new(),
        enabled: true,
        error: None,
        conflicts: Vec::new(),
    };
    match vpk::Vpk::open(path) {
        Ok(v) => {
            a.has_image = v.find("addonimage.jpg").is_some();
            if let Some(bytes) = v.read_path("addoninfo.txt") {
                let kv = kv::parse_root(&bytes);
                let s = |k: &str| kv.str(k).unwrap_or("").trim().to_string();
                a.info.title = s("addontitle");
                a.info.author = s("addonauthor");
                a.info.version = s("addonversion");
                a.info.description = s("addonDescription");
                a.info.url = s("addonURL0");
                for (k, v) in kv.pairs() {
                    if let (Some(cat), kv::Value::Str(val)) = (content_category(k), v) {
                        if val.trim() != "0" && !a.info.content.contains(&cat) {
                            a.info.content.push(cat);
                        }
                    }
                }
            }
            a.files = v
                .entries
                .into_iter()
                .map(|e| (e.path.into_boxed_str(), e.crc))
                .collect();
        }
        Err(e) => a.error = Some(e.to_string()),
    }
    if a.info.title.is_empty() {
        a.info.title = file.trim_end_matches(".vpk").to_string();
    }
    a
}

/// Reads an add-on's preview image (`addonimage.jpg`) from its VPK.
pub fn read_image(path: &Path) -> Option<Vec<u8>> {
    vpk::Vpk::open(path).ok()?.read_path("addonimage.jpg")
}

pub fn read_addonlist(path: &Path) -> Vec<(String, bool)> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    kv::parse_root(&bytes)
        .pairs()
        .iter()
        .filter_map(|(k, v)| match v {
            kv::Value::Str(s) => Some((k.clone(), s.trim() != "0")),
            _ => None,
        })
        .collect()
}

/// Merges the library's enabled states into addonlist.txt, keeping unknown entries and order.
pub fn write_addonlist(path: &Path, lib: &Library) -> io::Result<()> {
    let mut entries = read_addonlist(path);
    for a in &lib.addons {
        match entries
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(&a.key))
        {
            Some(e) => e.1 = a.enabled,
            None => entries.push((a.key.clone(), a.enabled)),
        }
    }
    let mut out = String::from("\"AddonList\"\n{\n");
    for (k, v) in &entries {
        out.push_str(&format!("\t\"{k}\"\t\t\"{}\"\n", if *v { 1 } else { 0 }));
    }
    out.push_str("}\n");
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)
}

pub fn scan(addons_dir: &Path) -> Library {
    let mut paths = Vec::new();
    for (dir, ws) in [
        (addons_dir.to_path_buf(), false),
        (addons_dir.join("workshop"), true),
    ] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                let is_vpk = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vpk"));
                // Skip numbered parts of multi-file packs (foo_000.vpk); foo_dir.vpk is the entry point.
                let part = p.file_stem().map(|s| s.to_string_lossy()).is_some_and(|s| {
                    s.len() > 4
                        && s.as_bytes()[s.len() - 4] == b'_'
                        && s[s.len() - 3..].bytes().all(|b| b.is_ascii_digit())
                });
                if is_vpk && !part && p.is_file() {
                    paths.push((p, ws));
                }
            }
        }
    }
    // Parse VPK trees in parallel; they're independent and IO-bound.
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let chunk = paths.len().div_ceil(threads).max(1);
    let mut addons: Vec<Addon> = std::thread::scope(|s| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    c.iter()
                        .map(|(p, ws)| load_addon(p, *ws))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });

    let list = read_addonlist(
        &addons_dir
            .parent()
            .unwrap_or(addons_dir)
            .join("addonlist.txt"),
    );
    for a in &mut addons {
        if let Some((_, on)) = list.iter().find(|(k, _)| k.eq_ignore_ascii_case(&a.key)) {
            a.enabled = *on;
        }
    }
    addons.sort_by_cached_key(|a| a.info.title.to_lowercase());

    let stock = addons_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|g| stock_overrides(g, &addons))
        .unwrap_or_default();
    let mut lib = Library {
        addons,
        conflicts: Vec::new(),
        stock,
    };
    compute_conflicts(&mut lib);
    lib
}

/// Reads the game's own VPK trees and keeps only paths some add-on also ships.
fn stock_overrides(game: &Path, addons: &[Addon]) -> HashSet<Box<str>> {
    let shipped: HashSet<&str> = addons
        .iter()
        .flat_map(|a| a.files.iter().map(|(p, _)| p.as_ref()))
        .collect();
    let mut out = HashSet::new();
    for dir in [
        "left4dead2",
        "left4dead2_dlc1",
        "left4dead2_dlc2",
        "left4dead2_dlc3",
        "update",
    ] {
        if let Ok(v) = vpk::Vpk::open(&game.join(dir).join("pak01_dir.vpk")) {
            for e in v.entries {
                if shipped.contains(e.path.as_str()) {
                    out.insert(e.path.into_boxed_str());
                }
            }
        }
    }
    out
}

pub fn compute_conflicts(lib: &mut Library) {
    let mut by_path: HashMap<&str, Vec<(usize, u32)>> = HashMap::new();
    for (i, a) in lib.addons.iter().enumerate() {
        for (p, crc) in &a.files {
            if !ignored(p) {
                by_path.entry(p).or_default().push((i, *crc));
            }
        }
    }
    let mut pairs: HashMap<(usize, usize), (Vec<Box<str>>, usize)> = HashMap::new();
    for (path, owners) in by_path.iter().filter(|(_, o)| o.len() > 1) {
        for x in 0..owners.len() {
            for y in x + 1..owners.len() {
                let (a, b) = (owners[x].0.min(owners[y].0), owners[x].0.max(owners[y].0));
                if a == b {
                    continue;
                }
                let e = pairs.entry((a, b)).or_default();
                if owners[x].1 == owners[y].1 {
                    e.1 += 1;
                } else {
                    e.0.push((*path).into());
                }
            }
        }
    }
    let mut conflicts: Vec<Conflict> = pairs
        .into_iter()
        .map(|((a, b), (mut files, identical))| {
            // stock overrides first, then alphabetical
            files.sort_by(|x, y| {
                lib.stock
                    .contains(y)
                    .cmp(&lib.stock.contains(x))
                    .then(x.cmp(y))
            });
            let stock = files.iter().filter(|f| lib.stock.contains(*f)).count();
            Conflict {
                a,
                b,
                files,
                identical,
                stock,
            }
        })
        .collect();
    conflicts.sort_by(|x, y| {
        (y.stock, y.files.len())
            .cmp(&(x.stock, x.files.len()))
            .then((x.a, x.b).cmp(&(y.a, y.b)))
    });
    for a in &mut lib.addons {
        a.conflicts.clear();
    }
    for (ci, c) in conflicts.iter().enumerate() {
        lib.addons[c.a].conflicts.push(ci);
        lib.addons[c.b].conflicts.push(ci);
    }
    lib.conflicts = conflicts;
}

/// Short category of a conflicting path, for grouping in the UI.
pub fn path_kind(p: &str) -> &'static str {
    let top = p.split('/').next().unwrap_or("");
    match top {
        "models" => "Models",
        "materials" => "Materials",
        "sound" => "Sounds",
        "scripts" => {
            if p.starts_with("scripts/vscripts") {
                "VScripts"
            } else {
                "Scripts"
            }
        }
        "particles" => "Particles",
        "maps" => "Maps",
        "missions" => "Missions",
        "resource" => "UI / Resource",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ignores_merged_files() {
        assert!(ignored("addoninfo.txt"));
        assert!(ignored("scripts/vscripts/director_base_addon.nut"));
        assert!(!ignored("scripts/vscripts/sub/director_base_addon.nut"));
        assert!(!ignored("models/v_rifle.mdl"));
    }
}
