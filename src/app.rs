//! Application state, background workers and non-UI logic.

use crate::bbcode;
use crate::config::{self, Cache, Config, InstallRecord};
use crate::library::{self, Library};
use crate::steam::{self, BrowseQuery, Details, PageInfo, Progress};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DETAILS_TTL: u64 = 30 * 60;
pub const PAGE_TTL: u64 = 3 * 24 * 3600;

pub enum Msg {
    Scanned(Library, Duration),
    Details(Vec<Details>),
    DetailsFailed(Vec<u64>, String),
    Page(u64, PageInfo),
    /// (items left, seconds paused by rate limit)
    PageQueue(usize, u64),
    Children(Vec<(u64, Vec<u64>)>),
    ChildrenFailed(Vec<u64>, String),
    Browse(BrowseQuery, Result<Vec<u64>, String>),
    InstallCollection(u64, Result<Vec<u64>, String>),
    DownloadDone(u64, Result<PathBuf, String>),
    GameRunning(bool),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Library,
    Workshop,
    Conflicts,
    Downloads,
    Settings,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Focus {
    Local(String),
    Workshop(u64),
}

#[derive(Clone, PartialEq, Debug)]
pub enum DlState {
    /// Waiting for Workshop details (file URL).
    Waiting,
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

pub struct Download {
    pub id: u64,
    pub prog: Arc<Progress>,
    pub state: DlState,
    pub started: Option<Instant>,
    pub finished: Option<Instant>,
    pub is_update: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Upd {
    Unknown,
    UpToDate,
    Available,
    Modified,
    Removed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    All,
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LibSort {
    Name,
    Size,
    Updated,
    Installed,
    Conflicts,
    Popularity,
}

pub struct LibFilter {
    pub status: Status,
    pub conflicts: bool,
    pub updates: bool,
    pub missing_deps: bool,
    pub modified: bool,
    pub removed: bool,
    pub local_only: bool,
    pub duplicates: bool,
    pub noted: bool,
    pub categories: HashSet<String>,
    pub sort: LibSort,
    pub desc: bool,
}

pub enum Confirm {
    Delete(Vec<String>),
    DeleteProfile(String),
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub struct WorkshopView {
    pub query: BrowseQuery,
    pub text: String,
    pub results: Vec<u64>,
    pub loading: bool,
    pub error: Option<String>,
    pub url_input: String,
}

pub struct App {
    pub cfg: Config,
    pub cache: Cache,
    pub game_dir: Option<PathBuf>,
    pub lib: Library,
    pub scanning: bool,
    rescan_again: bool,
    pub last_scan: Option<Duration>,
    pub tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub ctx: egui::Context,
    agent: ureq::Agent,
    pending_details: HashSet<u64>,
    page_queue: Arc<Mutex<VecDeque<u64>>>,
    pub page_queue_len: usize,
    pub page_paused: u64,
    pending_children: HashSet<u64>,
    pub tab: Tab,
    pub search: String,
    pub filter: LibFilter,
    pub selected: HashSet<String>,
    pub anchor: Option<String>,
    pub focus: Option<Focus>,
    pub ws: WorkshopView,
    pub downloads: Vec<Download>,
    pending_enable: HashSet<u64>,
    pub game_running: bool,
    pub pending_write: bool,
    backed_up: bool,
    pub toasts: Vec<Toast>,
    pub confirm: Option<Confirm>,
    pub parsed: RefCell<HashMap<u64, Arc<Vec<bbcode::Block>>>>,
    pub local_images: RefCell<HashMap<String, Option<egui::load::Bytes>>>,
    pub profile_name: String,
    pub conflicts_active_only: bool,
    pub conflicts_stock_only: bool,
    pub conflict_search: String,
    pub focus_search: bool,
    pub applied_theme: Option<(bool, f32)>,
    cjk_loaded: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let cfg: Config = config::load("config.json");
        crate::ui::setup_fonts(&cc.egui_ctx);
        let cache: Cache = config::load("cache.json");
        let (tx, rx) = std::sync::mpsc::channel();
        let game_dir = cfg
            .game_dir
            .clone()
            .filter(|p| config::is_game_dir(p))
            .or_else(config::detect_game_dir);
        let mut app = App {
            cfg,
            cache,
            game_dir,
            lib: Library::default(),
            scanning: false,
            rescan_again: false,
            last_scan: None,
            tx,
            rx,
            ctx: cc.egui_ctx.clone(),
            agent: steam::agent(),
            pending_details: HashSet::new(),
            page_queue: Arc::default(),
            page_queue_len: 0,
            page_paused: 0,
            pending_children: HashSet::new(),
            tab: Tab::Library,
            search: String::new(),
            filter: LibFilter {
                status: Status::All,
                conflicts: false,
                updates: false,
                missing_deps: false,
                modified: false,
                removed: false,
                local_only: false,
                duplicates: false,
                noted: false,
                categories: HashSet::new(),
                sort: LibSort::Name,
                desc: false,
            },
            selected: HashSet::new(),
            anchor: None,
            focus: None,
            ws: WorkshopView {
                query: BrowseQuery::default(),
                text: String::new(),
                results: Vec::new(),
                loading: false,
                error: None,
                url_input: String::new(),
            },
            downloads: Vec::new(),
            pending_enable: HashSet::new(),
            game_running: false,
            pending_write: false,
            backed_up: false,
            toasts: Vec::new(),
            confirm: None,
            parsed: RefCell::default(),
            local_images: RefCell::default(),
            profile_name: String::new(),
            conflicts_active_only: true,
            conflicts_stock_only: true,
            conflict_search: String::new(),
            focus_search: false,
            applied_theme: None,
            cjk_loaded: false,
        };
        app.start_page_worker();
        app.start_game_watcher();
        app.rescan();
        app
    }

    // ------------------------------------------------------------ paths

    pub fn addons_dir(&self) -> Option<PathBuf> {
        self.game_dir
            .as_ref()
            .map(|g| g.join("left4dead2").join("addons"))
    }
    fn addonlist_path(&self) -> Option<PathBuf> {
        self.game_dir
            .as_ref()
            .map(|g| g.join("left4dead2").join("addonlist.txt"))
    }
    pub fn set_game_dir(&mut self, p: PathBuf) {
        if config::is_game_dir(&p) {
            self.cfg.game_dir = Some(p.clone());
            self.game_dir = Some(p);
            self.save_config();
            self.rescan();
        } else {
            self.toast_err(format!(
                "{} doesn't look like a Left 4 Dead 2 folder (no left4dead2\\gameinfo.txt)",
                p.display()
            ));
        }
    }

    // ------------------------------------------------------------ toasts

    pub fn toast(&mut self, t: impl Into<String>) {
        self.toasts.push(Toast {
            text: t.into(),
            error: false,
            at: Instant::now(),
        });
    }
    pub fn toast_err(&mut self, t: impl Into<String>) {
        self.toasts.push(Toast {
            text: t.into(),
            error: true,
            at: Instant::now(),
        });
    }

    // ------------------------------------------------------------ workers

    fn spawn(&self, f: impl FnOnce(&ureq::Agent) -> Msg + Send + 'static) {
        let (tx, ctx, agent) = (self.tx.clone(), self.ctx.clone(), self.agent.clone());
        std::thread::spawn(move || {
            let _ = tx.send(f(&agent));
            ctx.request_repaint();
        });
    }

    /// One throttled thread scrapes item pages (dependencies) so Steam doesn't rate-limit us.
    fn start_page_worker(&self) {
        let (q, tx, ctx, agent) = (
            self.page_queue.clone(),
            self.tx.clone(),
            self.ctx.clone(),
            self.agent.clone(),
        );
        // steamcommunity.com answers 429 after ~20 quick requests; pace and back off exponentially.
        std::thread::spawn(move || {
            let mut backoff = 60;
            loop {
                let next = q.lock().ok().and_then(|mut q| q.pop_front());
                let Some(id) = next else {
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                };
                let mut pause = 0;
                match steam::fetch_page(&agent, id) {
                    Ok(p) => {
                        backoff = 60;
                        let _ = tx.send(Msg::Page(id, p));
                    }
                    Err(steam::Error::RateLimited) => {
                        if let Ok(mut q) = q.lock() {
                            q.push_front(id);
                        }
                        pause = backoff;
                        backoff = (backoff * 2).min(900);
                    }
                    Err(_) => {}
                }
                let left = q.lock().map(|q| q.len()).unwrap_or(0);
                let _ = tx.send(Msg::PageQueue(left, pause));
                ctx.request_repaint();
                std::thread::sleep(Duration::from_secs(pause.max(3)));
                if pause > 0 {
                    let _ = tx.send(Msg::PageQueue(left, 0));
                    ctx.request_repaint();
                }
            }
        });
    }

    fn start_game_watcher(&self) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let mut last = None;
            loop {
                let running = game_running();
                if last != Some(running) {
                    last = Some(running);
                    let _ = tx.send(Msg::GameRunning(running));
                    ctx.request_repaint();
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        });
    }

    pub fn rescan(&mut self) {
        let Some(dir) = self.addons_dir() else { return };
        if self.scanning {
            self.rescan_again = true;
            return;
        }
        self.scanning = true;
        self.spawn(move |_| {
            let t = Instant::now();
            let lib = library::scan(&dir);
            Msg::Scanned(lib, t.elapsed())
        });
    }

    /// Requests details for ids that are missing or older than `ttl` seconds.
    pub fn request_details(&mut self, ids: impl IntoIterator<Item = u64>, ttl: u64) {
        let now = steam::now();
        let want: Vec<u64> = ids
            .into_iter()
            .filter(|id| !self.pending_details.contains(id))
            .filter(|id| {
                self.cache
                    .details
                    .get(id)
                    .is_none_or(|d| now.saturating_sub(d.fetched) > ttl)
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if want.is_empty() {
            return;
        }
        self.pending_details.extend(&want);
        for chunk in want.chunks(100) {
            let chunk = chunk.to_vec();
            self.spawn(move |a| match steam::fetch_details(a, &chunk) {
                Ok(d) => Msg::Details(d),
                Err(e) => Msg::DetailsFailed(chunk, e.to_string()),
            });
        }
    }

    pub fn request_page(&mut self, id: u64, priority: bool) {
        let fresh = self
            .cache
            .pages
            .get(&id)
            .is_some_and(|p| steam::now().saturating_sub(p.fetched) < PAGE_TTL);
        if fresh {
            return;
        }
        if !self.cfg.steam_api_key.trim().is_empty() {
            return self.request_children(vec![id]);
        }
        if let Ok(mut q) = self.page_queue.lock() {
            if let Some(pos) = q.iter().position(|&x| x == id) {
                if !priority {
                    return;
                }
                q.remove(pos);
            }
            if priority {
                q.push_front(id);
            } else {
                q.push_back(id);
            }
            self.page_queue_len = q.len();
        }
    }

    /// Batch dependency lookup through the Steam Web API (only with a key).
    pub fn request_children(&mut self, ids: Vec<u64>) {
        let key = self.cfg.steam_api_key.trim().to_string();
        let now = steam::now();
        let want: Vec<u64> = ids
            .into_iter()
            .filter(|id| !self.pending_children.contains(id))
            .filter(|id| {
                self.cache
                    .pages
                    .get(id)
                    .is_none_or(|p| now.saturating_sub(p.fetched) > PAGE_TTL)
            })
            .collect();
        if want.is_empty() || key.is_empty() {
            return;
        }
        self.pending_children.extend(&want);
        self.spawn(move |a| match steam::fetch_children(a, &key, &want) {
            Ok(v) => Msg::Children(v),
            Err(e) => Msg::ChildrenFailed(want, e.to_string()),
        });
    }

    pub fn browse(&mut self) {
        self.ws.loading = true;
        self.ws.error = None;
        let q = self.ws.query.clone();
        self.spawn(move |a| {
            let r = steam::browse(a, &q).map_err(|e| e.to_string());
            Msg::Browse(q, r)
        });
    }

    pub fn check_updates(&mut self) {
        let ids: Vec<u64> = self.lib.addons.iter().filter_map(|a| a.id).collect();
        self.request_details(ids, 0);
        self.toast("Checking Workshop for updates…");
    }

    // ------------------------------------------------------------ message pump

    pub fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Scanned(mut lib, took) => {
                    self.scanning = false;
                    self.last_scan = Some(took);
                    // Apply "enable after install" for freshly downloaded items.
                    if !self.pending_enable.is_empty() {
                        for a in &mut lib.addons {
                            if a.id.is_some_and(|id| self.pending_enable.remove(&id)) {
                                a.enabled = true;
                            }
                        }
                        self.lib = lib;
                        self.enable_deps_of_all_pending();
                        self.write_addonlist();
                    } else {
                        self.lib = lib;
                    }
                    self.selected.retain(|k| self.lib.index_of_key(k).is_some());
                    if !self.cjk_loaded && self.cfg.cjk_fonts {
                        let cjk = self.lib.addons.iter().any(|a| {
                            crate::ui::has_cjk(&a.info.title)
                                || a.id.and_then(|id| self.cache.details.get(&id)).is_some_and(
                                    |d| {
                                        crate::ui::has_cjk(&d.title)
                                            || crate::ui::has_cjk(&d.description)
                                    },
                                )
                        });
                        if cjk {
                            self.cjk_loaded = true;
                            crate::ui::load_cjk_fonts(&self.ctx);
                        }
                    }
                    let ids: Vec<u64> = self.lib.addons.iter().filter_map(|a| a.id).collect();
                    let ttl = if self.cfg.check_updates_on_start {
                        DETAILS_TTL
                    } else {
                        u64::MAX
                    };
                    self.request_details(ids.iter().copied(), ttl);
                    if !self.cfg.steam_api_key.trim().is_empty() {
                        self.request_children(ids);
                    } else if self.cfg.scan_dependencies {
                        for id in ids {
                            self.request_page(id, false);
                        }
                    }
                    if self.rescan_again {
                        self.rescan_again = false;
                        self.rescan();
                    }
                }
                Msg::Details(list) => {
                    if !self.cjk_loaded
                        && self.cfg.cjk_fonts
                        && list.iter().any(|d| {
                            crate::ui::has_cjk(&d.title) || crate::ui::has_cjk(&d.description)
                        })
                    {
                        self.cjk_loaded = true;
                        crate::ui::load_cjk_fonts(&self.ctx);
                    }
                    for d in list {
                        self.pending_details.remove(&d.id);
                        self.parsed.borrow_mut().remove(&d.id);
                        self.cache.details.insert(d.id, d);
                    }
                }
                Msg::DetailsFailed(ids, e) => {
                    for id in &ids {
                        self.pending_details.remove(id);
                    }
                    for d in self
                        .downloads
                        .iter_mut()
                        .filter(|d| d.state == DlState::Waiting && ids.contains(&d.id))
                    {
                        d.state = DlState::Failed(format!("couldn't get item details: {e}"));
                    }
                    self.toast_err(format!("Workshop details: {e}"));
                }
                Msg::Page(id, p) => self.on_page(id, p),
                Msg::PageQueue(n, paused) => {
                    self.page_queue_len = n;
                    self.page_paused = paused;
                }
                Msg::Children(list) => {
                    let mut kids_all = Vec::new();
                    for (id, kids) in list {
                        self.pending_children.remove(&id);
                        kids_all.extend(kids.iter().copied());
                        let creators = self
                            .cache
                            .pages
                            .get(&id)
                            .map(|p| p.creators.clone())
                            .unwrap_or_default();
                        let page = PageInfo {
                            required: kids.into_iter().map(|k| (k, String::new())).collect(),
                            creators,
                            fetched: steam::now(),
                        };
                        self.on_page(id, page);
                    }
                    // titles for the requirement list
                    self.request_details(kids_all, u64::MAX);
                }
                Msg::ChildrenFailed(ids, e) => {
                    for id in ids {
                        self.pending_children.remove(&id);
                    }
                    self.toast_err(format!("Dependency lookup: {e}"));
                }
                Msg::Browse(q, r) => {
                    if q == self.ws.query {
                        self.ws.loading = false;
                        match r {
                            Ok(ids) => {
                                self.request_details(ids.iter().copied(), DETAILS_TTL);
                                self.ws.results = ids;
                            }
                            Err(e) => self.ws.error = Some(e),
                        }
                    }
                }
                Msg::InstallCollection(id, r) => match r {
                    Ok(children) if !children.is_empty() => {
                        self.toast(format!("Installing collection: {} items", children.len()));
                        for c in children {
                            self.install(c, false);
                        }
                    }
                    Ok(_) => self.install(id, false),
                    Err(e) => self.toast_err(e),
                },
                Msg::DownloadDone(id, r) => {
                    let title = self.title_of(id);
                    let is_update = self
                        .downloads
                        .iter()
                        .find(|d| d.id == id)
                        .is_some_and(|d| d.is_update);
                    if let Some(d) = self
                        .downloads
                        .iter_mut()
                        .find(|d| d.id == id && d.state == DlState::Running)
                    {
                        d.finished = Some(Instant::now());
                        d.state = match &r {
                            Ok(_) => DlState::Done,
                            Err(e) if e == "cancelled" => DlState::Cancelled,
                            Err(e) => DlState::Failed(e.clone()),
                        };
                    }
                    match r {
                        Ok(path) => {
                            let tu = self
                                .cache
                                .details
                                .get(&id)
                                .map(|d| d.time_updated)
                                .unwrap_or_else(steam::now);
                            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            self.cfg.installed.insert(
                                id,
                                InstallRecord {
                                    time_updated: tu,
                                    size,
                                },
                            );
                            self.save_config();
                            if self.cfg.enable_after_install && !is_update {
                                self.pending_enable.insert(id);
                            }
                            self.toast(format!(
                                "{} {title}",
                                if is_update { "Updated" } else { "Installed" }
                            ));
                            self.rescan();
                        }
                        Err(e) if e == "cancelled" => {}
                        Err(e) => self.toast_err(format!("{title}: {e}")),
                    }
                }
                Msg::GameRunning(r) => {
                    let was = self.game_running;
                    self.game_running = r;
                    if was && !r {
                        if self.pending_write {
                            self.write_addonlist();
                            self.toast("Game closed — add-on changes saved");
                        } else {
                            self.reload_enabled();
                        }
                    }
                }
            }
        }
        self.pump_downloads();
        self.toasts
            .retain(|t| t.at.elapsed() < Duration::from_secs(if t.error { 8 } else { 4 }));
    }

    fn on_page(&mut self, id: u64, p: PageInfo) {
        let installing = self
            .downloads
            .iter()
            .any(|d| d.id == id && !matches!(d.state, DlState::Failed(_) | DlState::Cancelled));
        let req: Vec<u64> = p.required.iter().map(|r| r.0).collect();
        self.cache.pages.insert(id, p);
        if installing && self.cfg.install_dependencies {
            for r in req {
                if self.lib.by_id(r).next().is_none() {
                    self.install(r, false);
                }
            }
        }
    }

    // ------------------------------------------------------------ add-on state

    pub fn title_of(&self, id: u64) -> String {
        self.cache
            .details
            .get(&id)
            .filter(|d| !d.title.is_empty())
            .map(|d| d.title.clone())
            .or_else(|| self.lib.by_id(id).next().map(|a| a.info.title.clone()))
            .unwrap_or_else(|| format!("Workshop item {id}"))
    }

    pub fn display_title(&self, a: &library::Addon) -> String {
        a.id.and_then(|id| self.cache.details.get(&id))
            .filter(|d| d.ok && !d.title.is_empty())
            .map(|d| d.title.clone())
            .unwrap_or_else(|| a.info.title.clone())
    }

    pub fn update_state(&self, a: &library::Addon) -> Upd {
        let Some(id) = a.id else { return Upd::Unknown };
        let Some(d) = self.cache.details.get(&id) else {
            return Upd::Unknown;
        };
        if !d.ok || d.banned {
            return Upd::Removed;
        }
        match self.cfg.installed.get(&id) {
            Some(r) if d.time_updated > r.time_updated => Upd::Available,
            Some(r) if r.size != a.size => Upd::Modified,
            Some(_) => Upd::UpToDate,
            // Not installed by us: compare against the file's modification time.
            None if d.time_updated > a.mtime + 120 => Upd::Available,
            None if d.file_size != 0 && d.file_size != a.size => Upd::Modified,
            None => Upd::UpToDate,
        }
    }

    pub fn requires(&self, id: u64) -> Option<&[(u64, String)]> {
        self.cache.pages.get(&id).map(|p| p.required.as_slice())
    }

    pub fn missing_deps(&self, id: u64) -> Vec<u64> {
        self.requires(id)
            .into_iter()
            .flatten()
            .map(|r| r.0)
            .filter(|r| self.lib.by_id(*r).next().is_none())
            .collect()
    }

    /// Installed add-ons that list `id` as a required item.
    pub fn required_by(&self, id: u64) -> Vec<usize> {
        (0..self.lib.addons.len())
            .filter(|&i| {
                self.lib.addons[i]
                    .id
                    .and_then(|me| self.requires(me))
                    .is_some_and(|r| r.iter().any(|x| x.0 == id))
            })
            .collect()
    }

    pub fn is_duplicate(&self, a: &library::Addon) -> bool {
        a.id.is_some_and(|id| self.lib.by_id(id).count() > 1)
    }

    pub fn categories(&self, a: &library::Addon) -> Vec<String> {
        let mut out: Vec<String> = a.info.content.iter().map(|s| s.to_string()).collect();
        if let Some(d) = a.id.and_then(|id| self.cache.details.get(&id)) {
            for t in &d.tags {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
                    out.push(t.clone());
                }
            }
        }
        out
    }

    pub fn set_enabled(&mut self, keys: &[String], on: bool) {
        let mut changed = 0;
        let mut ids = Vec::new();
        for k in keys {
            if let Some(i) = self.lib.index_of_key(k) {
                if self.lib.addons[i].enabled != on {
                    self.lib.addons[i].enabled = on;
                    changed += 1;
                }
                ids.extend(self.lib.addons[i].id);
            }
        }
        if on && self.cfg.auto_enable_deps {
            let n = self.enable_deps(&ids);
            if n > 0 {
                self.toast(format!("Also enabled {n} required add-on(s)"));
            }
            let missing: usize = ids.iter().map(|&id| self.missing_deps(id).len()).sum();
            if missing > 0 {
                self.toast_err(format!("{missing} required item(s) aren't installed — see the preview's Requires section"));
            }
        }
        if !on {
            let needed: HashSet<usize> = ids
                .iter()
                .flat_map(|&id| self.required_by(id))
                .filter(|&i| self.lib.addons[i].enabled)
                .collect();
            if !needed.is_empty() {
                self.toast_err(format!(
                    "{} enabled add-on(s) require what you just disabled",
                    needed.len()
                ));
            }
        }
        if changed > 0 {
            self.write_addonlist();
        }
    }

    /// Enables installed dependencies (transitively). Returns how many were switched on.
    fn enable_deps(&mut self, roots: &[u64]) -> usize {
        let mut stack: Vec<u64> = roots.to_vec();
        let mut seen: HashSet<u64> = HashSet::new();
        let mut n = 0;
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let req: Vec<u64> = self
                .requires(id)
                .into_iter()
                .flatten()
                .map(|r| r.0)
                .collect();
            for r in req {
                for a in self.lib.addons.iter_mut().filter(|a| a.id == Some(r)) {
                    if !a.enabled {
                        a.enabled = true;
                        n += 1;
                    }
                }
                stack.push(r);
            }
        }
        n
    }

    fn enable_deps_of_all_pending(&mut self) {
        if self.cfg.auto_enable_deps {
            let ids: Vec<u64> = self
                .lib
                .addons
                .iter()
                .filter(|a| a.enabled)
                .filter_map(|a| a.id)
                .collect();
            self.enable_deps(&ids);
        }
    }

    pub fn write_addonlist(&mut self) {
        if self.game_running {
            // The game rewrites addonlist.txt on exit; write ours after it closes.
            self.pending_write = true;
            return;
        }
        let Some(path) = self.addonlist_path() else {
            return;
        };
        if !self.backed_up && path.exists() {
            let _ = std::fs::copy(&path, path.with_extension("txt.addonmgr.bak"));
            self.backed_up = true;
        }
        match library::write_addonlist(&path, &self.lib) {
            Ok(()) => self.pending_write = false,
            Err(e) => self.toast_err(format!("Couldn't save addonlist.txt: {e}")),
        }
    }

    fn reload_enabled(&mut self) {
        let Some(path) = self.addonlist_path() else {
            return;
        };
        let list = library::read_addonlist(&path);
        for a in &mut self.lib.addons {
            if let Some((_, on)) = list.iter().find(|(k, _)| k.eq_ignore_ascii_case(&a.key)) {
                a.enabled = *on;
            }
        }
    }

    pub fn delete(&mut self, keys: &[String]) {
        let mut n = 0;
        for k in keys {
            if let Some(i) = self.lib.index_of_key(k) {
                let a = &self.lib.addons[i];
                match trash::delete(&a.path) {
                    Ok(()) => {
                        n += 1;
                        if let Some(id) = a.id {
                            self.cfg.installed.remove(&id);
                        }
                    }
                    Err(e) => {
                        let msg = format!("Couldn't delete {}: {e}", a.file_name());
                        self.toast_err(msg);
                    }
                }
            }
        }
        if n > 0 {
            self.toast(format!("Moved {n} add-on(s) to the Recycle Bin"));
            self.save_config();
            self.selected.clear();
            self.rescan();
        }
    }

    /// Moves a Steam-managed `addons/workshop/<id>.vpk` into `addons/`.
    pub fn move_out_of_workshop(&mut self, key: &str) {
        let (Some(i), Some(dir)) = (self.lib.index_of_key(key), self.addons_dir()) else {
            return;
        };
        let a = &self.lib.addons[i];
        let dest = dir.join(a.file_name());
        if dest.exists() {
            return self.toast_err(format!("{} already exists in addons/", a.file_name()));
        }
        let enabled = a.enabled;
        match std::fs::rename(&a.path, &dest) {
            Ok(()) => {
                let a = &mut self.lib.addons[i];
                a.key = a.file_name();
                a.path = dest;
                a.in_workshop_dir = false;
                a.enabled = enabled;
                self.write_addonlist();
                self.toast("Moved to addons/ (unsubscribe in Steam so it isn't downloaded twice)");
                self.rescan();
            }
            Err(e) => self.toast_err(format!("Move failed: {e} (close the game first)")),
        }
    }

    // ------------------------------------------------------------ profiles

    pub fn save_profile(&mut self, name: &str) {
        let keys = self
            .lib
            .addons
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.key.clone())
            .collect();
        self.cfg.profiles.insert(name.to_string(), keys);
        self.save_config();
        self.toast(format!("Saved profile \"{name}\""));
    }

    pub fn apply_profile(&mut self, name: &str) {
        let Some(keys) = self.cfg.profiles.get(name).cloned() else {
            return;
        };
        let set: HashSet<String> = keys.iter().map(|k| k.to_lowercase()).collect();
        for a in &mut self.lib.addons {
            a.enabled = set.contains(&a.key.to_lowercase());
        }
        let missing = keys
            .iter()
            .filter(|k| self.lib.index_of_key(k).is_none())
            .count();
        self.write_addonlist();
        if missing > 0 {
            self.toast_err(format!(
                "Profile \"{name}\" applied; {missing} of its add-ons are no longer installed"
            ));
        } else {
            self.toast(format!("Profile \"{name}\" applied"));
        }
    }

    pub fn export_profile(&mut self, name: &str) {
        let Some(keys) = self.cfg.profiles.get(name) else {
            return;
        };
        let ids: Vec<String> = keys.clone();
        if let Some(p) = rfd::FileDialog::new()
            .set_file_name(format!("{name}.l4d2profile.json"))
            .save_file()
        {
            let v = serde_json::json!({ "name": name, "addons": ids });
            if let Err(e) = std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap_or_default()) {
                self.toast_err(e.to_string());
            }
        }
    }

    pub fn import_profile(&mut self) {
        let Some(p) = rfd::FileDialog::new()
            .add_filter("Profile", &["json"])
            .pick_file()
        else {
            return;
        };
        let parsed = std::fs::read(&p)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        let Some(v) = parsed else {
            return self.toast_err("Not a profile file");
        };
        let name = v["name"].as_str().unwrap_or("Imported").to_string();
        let keys: Vec<String> = v["addons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| k.as_str().map(String::from))
            .collect();
        // Offer to install Workshop items from the profile that aren't present.
        let missing: Vec<u64> = keys
            .iter()
            .filter(|k| self.lib.index_of_key(k).is_none())
            .filter_map(|k| {
                k.rsplit(['\\', '/'])
                    .next()?
                    .strip_suffix(".vpk")?
                    .parse()
                    .ok()
            })
            .collect();
        self.cfg.profiles.insert(name.clone(), keys);
        self.save_config();
        self.toast(format!("Imported profile \"{name}\""));
        if !missing.is_empty() {
            self.toast(format!(
                "Installing {} add-on(s) from the profile that you don't have",
                missing.len()
            ));
            for id in missing {
                self.install(id, false);
            }
        }
    }

    // ------------------------------------------------------------ downloads

    pub fn install_from_input(&mut self, s: &str) {
        let Some(id) = steam::parse_id(s) else {
            return self.toast_err("Paste a Workshop link or item id");
        };
        self.toast("Looking up item…");
        self.spawn(move |a| {
            Msg::InstallCollection(
                id,
                steam::collection_children(a, id).map_err(|e| e.to_string()),
            )
        });
    }

    pub fn install_collection(&mut self, id: u64) {
        self.spawn(move |a| {
            Msg::InstallCollection(
                id,
                steam::collection_children(a, id).map_err(|e| e.to_string()),
            )
        });
    }

    pub fn install(&mut self, id: u64, is_update: bool) {
        if self.downloads.iter().any(|d| {
            d.id == id
                && matches!(
                    d.state,
                    DlState::Waiting | DlState::Queued | DlState::Running
                )
        }) {
            return;
        }
        self.downloads.retain(|d| d.id != id);
        self.downloads.push(Download {
            id,
            prog: Arc::default(),
            state: DlState::Waiting,
            started: None,
            finished: None,
            is_update,
        });
        // Fresh details so we get the current file URL.
        self.request_details([id], if is_update { 0 } else { 300 });
        if self.cfg.install_dependencies && !is_update {
            if let Some(req) = self.requires(id).map(|r| r.to_vec()) {
                if steam::now().saturating_sub(self.cache.pages[&id].fetched) < PAGE_TTL {
                    for (r, _) in req {
                        if self.lib.by_id(r).next().is_none() {
                            self.install(r, false);
                        }
                    }
                    return;
                }
            }
            self.request_page(id, true);
        }
    }

    pub fn update_all(&mut self) {
        let ids: Vec<u64> = self
            .lib
            .addons
            .iter()
            .filter(|a| self.update_state(a) == Upd::Available)
            .filter_map(|a| a.id)
            .collect();
        let n = ids.len();
        for id in ids {
            self.install(id, true);
        }
        self.toast(format!("Queued {n} update(s)"));
    }

    fn pump_downloads(&mut self) {
        let now = steam::now();
        for d in &mut self.downloads {
            if d.state == DlState::Waiting {
                if let Some(det) = self
                    .cache
                    .details
                    .get(&d.id)
                    .filter(|x| now.saturating_sub(x.fetched) < 600 || !d.is_update)
                {
                    d.state = if !det.ok {
                        DlState::Failed(
                            "item is private, removed or not a Left 4 Dead 2 item".into(),
                        )
                    } else if det.file_url.is_empty() {
                        DlState::Failed("no direct download for this item (it may be a collection — use Install collection)".into())
                    } else {
                        DlState::Queued
                    };
                }
            }
        }
        let Some(addons) = self.addons_dir() else {
            return;
        };
        let mut running = self
            .downloads
            .iter()
            .filter(|d| d.state == DlState::Running)
            .count();
        let max = self.cfg.max_downloads.clamp(1, 8);
        for i in 0..self.downloads.len() {
            if running >= max {
                break;
            }
            if self.downloads[i].state != DlState::Queued {
                continue;
            }
            let id = self.downloads[i].id;
            let Some(url) = self.cache.details.get(&id).map(|d| d.file_url.clone()) else {
                continue;
            };
            // Updates replace the existing file wherever it lives; new installs go to the chosen folder.
            let dest = self
                .lib
                .by_id(id)
                .next()
                .map(|a| a.path.clone())
                .unwrap_or_else(|| {
                    if self.cfg.install_to_workshop_dir {
                        addons.join("workshop")
                    } else {
                        addons.clone()
                    }
                    .join(format!("{id}.vpk"))
                });
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let d = &mut self.downloads[i];
            d.state = DlState::Running;
            d.started = Some(Instant::now());
            let prog = d.prog.clone();
            running += 1;
            self.spawn(move |a| {
                let r = steam::download(a, &url, &dest, &prog).map(|_| dest);
                Msg::DownloadDone(id, r)
            });
        }
        if running > 0 {
            self.ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    pub fn cancel_download(&mut self, id: u64) {
        if let Some(d) = self.downloads.iter_mut().find(|d| d.id == id) {
            match d.state {
                DlState::Running => d.prog.cancel.store(true, Ordering::Relaxed),
                DlState::Waiting | DlState::Queued => d.state = DlState::Cancelled,
                _ => {}
            }
        }
    }

    pub fn save_config(&self) {
        config::save("config.json", &self.cfg);
    }
    pub fn save_all(&self) {
        self.save_config();
        config::save("cache.json", &self.cache);
    }
}

pub fn game_running() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq left4dead2.exe", "/NH", "/FO", "CSV"])
            .creation_flags(0x0800_0000)
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .to_ascii_lowercase()
                    .contains("left4dead2.exe")
            })
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        // native build (hl2_linux / hl2_osx -game left4dead2) or the Windows build under Proton
        std::process::Command::new("pgrep")
            .args(["-f", r"hl2_(linux|osx).*left4dead2|left4dead2\.exe"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

pub fn open(target: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer")
            .arg(target)
            .creation_flags(0x0800_0000)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(target).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
}

pub fn reveal(path: &std::path::Path) {
    #[cfg(windows)]
    {
        // explorer parses its own command line: `/select,"C:\path with spaces\x.vpk"` must reach it verbatim.
        // `.arg()` would quote the whole token (`"/select,C:\..."`), which explorer rejects and opens Documents instead.
        use std::os::windows::process::CommandExt;
        let p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let s = p.display().to_string();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s).replace('/', "\\");
        let _ = std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{s}\""))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    // ponytail: xdg has no portable "select file", so this opens the containing folder.
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = path.parent().unwrap_or(path);
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump();
        crate::ui::draw(self, ui);
    }
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_all();
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_all();
    }
}
