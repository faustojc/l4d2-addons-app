//! All drawing. Views read `&App` and push `Act`s; `apply` mutates afterwards (keeps borrowck happy).

use crate::app::*;
use crate::bbcode;
use crate::library::{self, Addon};
use crate::steam::{self, Details, Sort};
use crate::theme::{
    self as th, ACCENT, BLUE, DIM, GRAY, GREEN, Kind, MUTED, ORANGE, PURPLE, RED, TEXT,
    TEXT_STRONG, badge, caps,
};
pub use crate::theme::{load_cjk_fonts, setup_fonts};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, FontId, Key, Layout, RichText, Sense, Vec2};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Allocated height of one library row; rows are separated by ROW_GAP.
const ROW_H: f32 = 52.0;
const ROW_GAP: f32 = 2.0;

pub enum Act {
    SetEnabled(Vec<String>, bool),
    Focus(Focus),
    Click {
        key: String,
        ctrl: bool,
        shift: bool,
    },
    Install(u64),
    Update(u64),
    InstallCollection(u64),
    AskDelete(Vec<String>),
    MoveOut(String),
    UsedBy(u64),
    SetNote(String, String),
    CancelDl(u64),
    ClearFinished,
    Category(String),
    WsTag(String),
    Tab(Tab),
}

thread_local! {
    static MATCHER: SkimMatcherV2 = SkimMatcherV2::default().ignore_case();
}

// ---------------------------------------------------------------- helpers

pub fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF))
}

pub fn size(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{b} B")
    }
}

/// Unix seconds -> YYYY-MM-DD (UTC), no chrono needed.
pub fn date(ts: u64) -> String {
    if ts == 0 {
        return "—".into();
    }
    let z = (ts / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn ago(ts: u64) -> String {
    let s = steam::now().saturating_sub(ts);
    match s {
        0..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        86_400..=2_591_999 => format!("{} days ago", s / 86400),
        2_592_000..=31_535_999 => format!("{} months ago", s / 2_592_000),
        _ => format!("{:.1} years ago", s as f64 / 31_536_000.0),
    }
}

fn count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn highlighted(text: &str, hl: &[usize], size: f32, strong: bool) -> LayoutJob {
    let base = if strong { TEXT_STRONG } else { MUTED };
    let font = if strong {
        th::bold(size)
    } else {
        FontId::proportional(size)
    };
    let mut job = LayoutJob::default();
    let set: HashSet<usize> = hl.iter().copied().collect();
    let mut buf = String::new();
    let mut cur = false;
    let flush = |job: &mut LayoutJob, buf: &mut String, on: bool| {
        if !buf.is_empty() {
            let fmt = TextFormat {
                font_id: font.clone(),
                color: if on { ACCENT } else { base },
                background: if on {
                    ACCENT.gamma_multiply(0.14)
                } else {
                    Color32::TRANSPARENT
                },
                ..Default::default()
            };
            job.append(buf, 0.0, fmt);
            buf.clear();
        }
    };
    for (i, c) in text.chars().enumerate() {
        let on = set.contains(&i);
        if on != cur {
            flush(&mut job, &mut buf, cur);
            cur = on;
        }
        buf.push(c);
    }
    flush(&mut job, &mut buf, cur);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

fn thumb(
    app: &App,
    a: Option<&Addon>,
    d: Option<&Details>,
    px: u32,
) -> Option<egui::ImageSource<'static>> {
    if let Some(url) = d.and_then(|d| d.thumb(px)) {
        return Some(egui::ImageSource::Uri(url.into()));
    }
    let a = a.filter(|a| a.has_image)?;
    let mut cache = app.local_images.borrow_mut();
    let bytes = cache
        .entry(a.key.clone())
        .or_insert_with(|| library::read_image(&a.path).map(egui::load::Bytes::from))
        .clone()?;
    Some(egui::ImageSource::Bytes {
        uri: format!("bytes://local/{}.jpg", a.key).into(),
        bytes,
    })
}

fn image_box(ui: &mut egui::Ui, src: Option<egui::ImageSource<'static>>, sz: Vec2) {
    match src {
        Some(s) => {
            ui.add(
                egui::Image::new(s)
                    .fit_to_exact_size(sz)
                    .corner_radius(2.0)
                    .show_loading_spinner(false),
            );
        }
        None => {
            let (r, _) = ui.allocate_exact_size(sz, Sense::hover());
            th::vgradient(ui.painter(), r, th::SURFACE_HI, th::SURFACE);
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "☣",
                FontId::proportional(sz.y * 0.5),
                DIM,
            );
        }
    }
}

// ---------------------------------------------------------------- library view model

pub struct Row {
    pub i: usize,
    score: i64,
    hl: Vec<usize>,
}

fn matches(app: &App, a: &Addon, title: &str, terms: &[String]) -> Option<(i64, Vec<usize>)> {
    if terms.is_empty() {
        return Some((0, Vec::new()));
    }
    let d = a.id.and_then(|id| app.cache.details.get(&id));
    let cats = app.categories(a).join(" ");
    let author = d
        .and_then(|_| app.cache.pages.get(&a.id?))
        .map(|p| p.creators.join(" "))
        .unwrap_or_default();
    let note = app
        .cfg
        .notes
        .get(&a.key)
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    let mut total = 0;
    let mut hl = Vec::new();
    MATCHER.with(|m| {
        for t in terms {
            let mut best: i64 = -1;
            if let Some((s, idx)) = m.fuzzy_indices(title, t) {
                best = s * 2;
                hl.extend(idx);
            }
            for hay in [
                a.info.author.as_str(),
                author.as_str(),
                cats.as_str(),
                a.info.title.as_str(),
            ] {
                if let Some(s) = m.fuzzy_match(hay, t) {
                    best = best.max(s);
                }
            }
            let lt = t.to_lowercase();
            if a.key.to_lowercase().contains(&lt) || note.contains(&lt) {
                best = best.max(60);
            }
            if app.cfg.search_descriptions {
                let desc = d
                    .map(|d| d.description.as_str())
                    .unwrap_or(&a.info.description);
                if desc.to_lowercase().contains(&lt) {
                    best = best.max(20);
                }
            }
            if best < 0 {
                return None;
            }
            total += best;
        }
        Some(())
    })?;
    Some((total, hl))
}

pub fn library_view(app: &App) -> Vec<Row> {
    let terms: Vec<String> = app.search.split_whitespace().map(String::from).collect();
    let f = &app.filter;
    let mut rows: Vec<Row> = Vec::new();
    for (i, a) in app.lib.addons.iter().enumerate() {
        match f.status {
            Status::Enabled if !a.enabled => continue,
            Status::Disabled if a.enabled => continue,
            _ => {}
        }
        let upd = app.update_state(a);
        if (f.conflicts && app.lib.active_conflicts(i) == 0)
            || (f.updates && upd != Upd::Available)
            || (f.modified && upd != Upd::Modified)
            || (f.removed && upd != Upd::Removed)
            || (f.local_only && a.id.is_some())
            || (f.duplicates && !app.is_duplicate(a))
            || (f.noted && !app.cfg.notes.contains_key(&a.key))
            || (f.missing_deps && a.id.is_none_or(|id| app.missing_deps(id).is_empty()))
        {
            continue;
        }
        if !f.categories.is_empty() {
            let cats = app.categories(a);
            if !cats.iter().any(|c| f.categories.contains(c)) {
                continue;
            }
        }
        let title = app.display_title(a);
        if let Some((score, hl)) = matches(app, a, &title, &terms) {
            rows.push(Row { i, score, hl });
        }
    }
    let lib = &app.lib;
    let key = |r: &Row| -> i64 {
        let a = &lib.addons[r.i];
        let d = a.id.and_then(|id| app.cache.details.get(&id));
        match f.sort {
            LibSort::Name => 0,
            LibSort::Size => a.size as i64,
            LibSort::Updated => d.map(|d| d.time_updated as i64).unwrap_or(0),
            LibSort::Installed => a.mtime as i64,
            LibSort::Conflicts => lib.active_conflicts(r.i) as i64,
            LibSort::Popularity => d.map(|d| d.subscriptions as i64).unwrap_or(0),
        }
    };
    if !terms.is_empty() && f.sort == LibSort::Name {
        rows.sort_by(|a, b| b.score.cmp(&a.score));
    } else if f.sort == LibSort::Name {
        // library is already sorted by title; desc just reverses
        if f.desc {
            rows.reverse();
        }
    } else {
        rows.sort_by_key(|r| std::cmp::Reverse(key(r)));
        if f.desc {
            rows.reverse();
        }
    }
    rows
}

// ---------------------------------------------------------------- frame

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    if app.applied_theme != Some((true, app.cfg.ui_scale)) {
        th::apply(&ctx);
        ctx.set_zoom_factor(app.cfg.ui_scale.clamp(0.6, 2.0));
        app.applied_theme = Some((true, app.cfg.ui_scale));
    }
    let mut acts: Vec<Act> = Vec::new();

    // global shortcuts
    let typing = ctx.text_edit_focused();
    ctx.input(|i| {
        if i.modifiers.command && i.key_pressed(Key::F) {
            app.focus_search = true;
            if !matches!(app.tab, Tab::Library | Tab::Workshop) {
                app.tab = Tab::Library;
            }
        }
        if i.key_pressed(Key::F5) {
            app.rescan();
        }
        if !typing && app.tab == Tab::Library {
            if i.key_pressed(Key::Escape) {
                app.selected.clear();
            }
            if i.key_pressed(Key::Delete) && !app.selected.is_empty() {
                acts.push(Act::AskDelete(app.selected.iter().cloned().collect()));
            }
            if i.key_pressed(Key::Space) && !app.selected.is_empty() {
                let keys: Vec<String> = app.selected.iter().cloned().collect();
                let any_off = keys.iter().any(|k| {
                    app.lib
                        .index_of_key(k)
                        .is_some_and(|i| !app.lib.addons[i].enabled)
                });
                acts.push(Act::SetEnabled(keys, any_off));
            }
        }
    });

    top_bar(app, ui, &mut acts);
    status_bar(app, ui);
    if app.pending_write {
        egui::Panel::top("banner").frame(egui::Frame::new().fill(Color32::from_rgb(0x4a, 0x36, 0x14)).inner_margin(egui::Margin::symmetric(12, 6))).show_separator_line(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠").color(ORANGE).size(15.0));
                ui.label(RichText::new("Left 4 Dead 2 is running. Your add-on changes are saved automatically when the game closes — keep this window open until then.").color(Color32::from_rgb(0xf3, 0xd9, 0xa8)));
            });
        });
    }

    if app.game_dir.is_none() {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(th::BG))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.label(RichText::new("☣").size(64.0).color(DIM));
                    ui.label(caps("Couldn't find Left 4 Dead 2", 22.0, TEXT_STRONG));
                    ui.label(
                        RichText::new("Pick the Left 4 Dead 2 install folder (the one that contains the left4dead2 folder).").color(MUTED),
                    );
                    ui.add_space(10.0);
                    if ui
                        .add(th::Button::new("Browse…", Kind::Blue).big())
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            app.set_game_dir(p);
                        }
                    }
                });
            });
    } else {
        match app.tab {
            Tab::Library => library_tab(app, ui, &mut acts),
            Tab::Workshop => workshop_tab(app, ui, &mut acts),
            Tab::Conflicts => conflicts_tab(app, ui, &mut acts),
            Tab::Downloads => downloads_tab(app, ui, &mut acts),
            Tab::Settings => settings_tab(app, ui),
        }
    }

    lightbox_ui(&ctx);
    confirm_modal(app, &ctx);
    toasts(app, &ctx);
    apply(app, acts);
}

fn apply(app: &mut App, acts: Vec<Act>) {
    for a in acts {
        match a {
            Act::SetEnabled(keys, on) => app.set_enabled(&keys, on),
            Act::Focus(f) => {
                if let Focus::Workshop(id) = f {
                    app.request_details([id], DETAILS_TTL);
                }
                app.focus = Some(f);
            }
            Act::Click { key, ctrl, shift } => {
                if shift {
                    if let Some(anchor) = app.anchor.clone() {
                        let view = library_view(app);
                        let pos = |k: &str| view.iter().position(|r| app.lib.addons[r.i].key == k);
                        if let (Some(x), Some(y)) = (pos(&anchor), pos(&key)) {
                            let keys: Vec<String> = view[x.min(y)..=x.max(y)]
                                .iter()
                                .map(|r| app.lib.addons[r.i].key.clone())
                                .collect();
                            if !ctrl {
                                app.selected.clear();
                            }
                            app.selected.extend(keys);
                        }
                    }
                } else if ctrl {
                    if !app.selected.remove(&key) {
                        app.selected.insert(key.clone());
                    }
                    app.anchor = Some(key.clone());
                } else {
                    app.selected.clear();
                    app.selected.insert(key.clone());
                    app.anchor = Some(key.clone());
                }
                app.focus = Some(Focus::Local(key));
            }
            Act::Install(id) => app.install(id, false),
            Act::Update(id) => app.install(id, true),
            Act::InstallCollection(id) => app.install_collection(id),
            Act::AskDelete(keys) => app.confirm = Some(Confirm::Delete(keys)),
            Act::MoveOut(k) => app.move_out_of_workshop(&k),
            Act::UsedBy(id) => {
                app.ws.query = steam::BrowseQuery {
                    child_of: Some(id),
                    sort: Sort::MostSubscribed,
                    ..Default::default()
                };
                app.ws.text.clear();
                app.tab = Tab::Workshop;
                app.browse();
            }
            Act::SetNote(k, n) => {
                if n.trim().is_empty() {
                    app.cfg.notes.remove(&k);
                } else {
                    app.cfg.notes.insert(k, n);
                }
                app.save_config();
            }
            Act::CancelDl(id) => app.cancel_download(id),
            Act::ClearFinished => app.downloads.retain(|d| {
                matches!(
                    d.state,
                    DlState::Waiting | DlState::Queued | DlState::Running
                )
            }),
            Act::Category(c) => {
                app.tab = Tab::Library;
                if !app.filter.categories.remove(&c) {
                    app.filter.categories.insert(c);
                }
            }
            Act::WsTag(t) => {
                app.tab = Tab::Workshop;
                app.ws.query.tags = vec![t];
                app.ws.query.page = 1;
                app.ws.query.child_of = None;
                app.browse();
            }
            Act::Tab(t) => app.tab = t,
        }
    }
}

fn top_bar(app: &mut App, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    egui::Panel::top("top")
        .exact_size(58.0)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(th::BG_NAV)
                .inner_margin(egui::Margin::symmetric(16, 0)),
        )
        .show(ui, |ui| {
            let full = ui.max_rect().expand2(egui::vec2(16.0, 0.0));
            th::vgradient(ui.painter(), full, th::BG_NAV, th::BG_DEEP);
            th::hgradient(
                ui.painter(),
                egui::Rect::from_min_size(
                    full.left_bottom() - egui::vec2(0.0, 1.0),
                    egui::vec2(full.width(), 1.0),
                ),
                ACCENT.gamma_multiply(0.35),
                Color32::TRANSPARENT,
            );
            ui.horizontal_centered(|ui| {
                // wordmark
                ui.label(RichText::new("☣").size(26.0).color(ACCENT));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = -3.0;
                    ui.add_space(9.0);
                    ui.label(
                        RichText::new("L4D2")
                            .font(th::display(19.0))
                            .extra_letter_spacing(2.0)
                            .color(TEXT_STRONG),
                    );
                    ui.label(caps("Addons manager", 9.5, DIM));
                });
                ui.add_space(22.0);

                // Matches the Conflicts tab's default filter (enabled pairs that replace stock game files).
                let active_conf = app
                    .lib
                    .conflicts
                    .iter()
                    .filter(|c| app.lib.conflict_active(c) && c.stock > 0)
                    .count();
                let dl_active = app
                    .downloads
                    .iter()
                    .filter(|d| {
                        matches!(
                            d.state,
                            DlState::Waiting | DlState::Queued | DlState::Running
                        )
                    })
                    .count();
                let tabs = [
                    (
                        Tab::Library,
                        "Library",
                        Some((app.lib.addons.len(), th::SURFACE_HI, ACCENT)),
                    ),
                    (Tab::Workshop, "Workshop", None),
                    (
                        Tab::Conflicts,
                        "Conflicts",
                        (active_conf > 0).then_some((active_conf, RED.gamma_multiply(0.25), RED)),
                    ),
                    (
                        Tab::Downloads,
                        "Downloads",
                        (dl_active > 0).then_some((dl_active, th::ACCENT_STRONG, TEXT_STRONG)),
                    ),
                    (Tab::Settings, "Settings", None),
                ];
                let mut active_rect = None;
                for (t, label, count) in tabs {
                    let on = app.tab == t;
                    let mut job = LayoutJob::simple_singleline(
                        label.to_uppercase(),
                        th::display(16.5),
                        Color32::WHITE,
                    );
                    job.sections[0].format.extra_letter_spacing = 1.4;
                    let galley = ui.painter().layout_job(job);
                    let cnt = count.map(|(n, bg, fg)| {
                        (
                            ui.painter()
                                .layout_no_wrap(n.to_string(), th::bold(10.0), fg),
                            bg,
                        )
                    });
                    let extra = cnt.as_ref().map(|(g, _)| g.size().x + 14.0).unwrap_or(0.0);
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(galley.size().x + extra + 20.0, 58.0),
                        Sense::click(),
                    );
                    let hov = ui
                        .ctx()
                        .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                    let color = if on {
                        th::ACCENT_STRONG
                    } else {
                        Color32::from_rgb(0xdc, 0xde, 0xdf).lerp_to_gamma(TEXT_STRONG, hov)
                    };
                    let tpos =
                        egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0);
                    let gw = galley.size().x;
                    ui.painter()
                        .galley_with_override_text_color(tpos, galley, color);
                    if let Some((g, bg)) = cnt {
                        let pr = egui::Rect::from_min_size(
                            egui::pos2(tpos.x + gw + 6.0, rect.center().y - 8.0),
                            egui::vec2(g.size().x + 8.0, 15.0),
                        );
                        ui.painter().rect_filled(pr, 7.5, bg);
                        ui.painter()
                            .galley(pr.center() - g.size() / 2.0, g, TEXT_STRONG);
                    }
                    if on {
                        active_rect = Some(rect);
                    }
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        acts.push(Act::Tab(t));
                        if t == Tab::Workshop && app.ws.results.is_empty() && !app.ws.loading {
                            app.browse();
                        }
                    }
                }
                // sliding underline under the active tab
                if let Some(r) = active_rect {
                    let ctx = ui.ctx();
                    let x = ctx.animate_value_with_time(
                        egui::Id::unique("nav_x"),
                        r.left() + 10.0,
                        0.18,
                    );
                    let w = ctx.animate_value_with_time(
                        egui::Id::unique("nav_w"),
                        r.width() - 20.0,
                        0.18,
                    );
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(x, r.bottom() - 3.0),
                        egui::vec2(w, 3.0),
                    );
                    ui.painter().rect_filled(bar, 1.5, th::ACCENT_STRONG);
                    ui.painter().rect_filled(
                        bar.expand2(egui::vec2(4.0, 2.0)),
                        3.0,
                        th::ACCENT_STRONG.gamma_multiply(0.15),
                    );
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if app.game_running {
                        let r =
                            ui.label(caps("In-game", 13.0, Color32::from_rgb(0x90, 0xba, 0x3c)));
                        let t = ui.input(|i| i.time) as f32;
                        let a = 0.55 + 0.45 * (t * 2.5).sin().abs();
                        ui.painter().circle_filled(
                            r.rect.left_center() - egui::vec2(8.0, 0.0),
                            4.0,
                            Color32::from_rgb(0x90, 0xba, 0x3c).gamma_multiply(a),
                        );
                        ui.add_space(10.0);
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(50));
                    } else if ui
                        .add(th::Button::new("▶  Play", Kind::Green).big())
                        .on_hover_text("Launch Left 4 Dead 2 through Steam")
                        .clicked()
                    {
                        open("steam://rungameid/550");
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_enabled(!app.scanning, th::Button::new("⟳  Rescan", Kind::Dark))
                        .on_hover_text("F5")
                        .clicked()
                    {
                        app.rescan();
                    }
                    if app.scanning {
                        ui.spinner();
                    }
                });
            });
        });
}

fn status_bar(app: &App, ui: &mut egui::Ui) {
    egui::Panel::bottom("status").exact_size(28.0).show_separator_line(false).frame(egui::Frame::new().fill(th::BG_DEEP).inner_margin(egui::Margin::symmetric(14, 0))).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let enabled: Vec<&Addon> = app.lib.addons.iter().filter(|a| a.enabled).collect();
            let updates = app.lib.addons.iter().filter(|a| app.update_state(a) == Upd::Available).count();
            let stat = |ui: &mut egui::Ui, k: &str, v: String, c: Color32| {
                ui.label(caps(k, 10.0, DIM));
                ui.label(RichText::new(v).size(12.0).color(c));
                ui.add_space(10.0);
            };
            stat(ui, "Add-ons", app.lib.addons.len().to_string(), TEXT);
            stat(ui, "Enabled", format!("{}  ·  {}", enabled.len(), size(enabled.iter().map(|a| a.size).sum())), TEXT);
            stat(ui, "On disk", size(app.lib.addons.iter().map(|a| a.size).sum()), TEXT);
            stat(ui, "Updates", updates.to_string(), if updates > 0 { ACCENT } else { TEXT });
            if app.page_queue_len > 0 {
                if app.page_paused > 0 {
                    ui.label(RichText::new(format!("⏸ Steam rate limit — requirement reading paused {} s ({} left)", app.page_paused, app.page_queue_len)).size(12.0).color(ORANGE))
                        .on_hover_text("steamcommunity.com limits how fast pages can be read. Add a free Steam Web API key in Settings to read all requirements in a few API calls instead.");
                } else {
                    ui.spinner();
                    ui.label(RichText::new(format!("Reading requirements from Workshop… {} left", app.page_queue_len)).size(12.0).color(MUTED));
                }
            }
            if let Some(t) = app.last_scan {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("scanned in {} ms", t.as_millis())).size(11.0).color(DIM));
                });
            }
        });
    });
}

fn toasts(app: &App, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    // Steam friend-notification style: dark gradient card with a colored edge, sliding in from the right.
    egui::Area::new(egui::Id::unique("toasts"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -40.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for (n, t) in app.toasts.iter().rev().take(5).enumerate() {
                let id = egui::Id::unique(("toast", n, &t.text));
                let now = ui.input(|i| i.time);
                let born = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
                let f = ((now - born) / 0.25).clamp(0.0, 1.0) as f32;
                let ease = 1.0 - (1.0 - f).powi(3);
                ui.scope(|ui| {
                    ui.multiply_opacity(ease);
                    let r = egui::Frame::new()
                        .fill(Color32::from_rgb(0x1c, 0x25, 0x33))
                        .corner_radius(3)
                        .shadow(egui::Shadow {
                            offset: [0, 6],
                            blur: 18,
                            spread: 0,
                            color: Color32::from_black_alpha(160),
                        })
                        .inner_margin(egui::Margin {
                            left: 16,
                            right: 14,
                            top: 10,
                            bottom: 10,
                        })
                        .show(ui, |ui| {
                            ui.set_width(340.0);
                            ui.label(caps(
                                if t.error { "Error" } else { "Addons Manager" },
                                10.5,
                                if t.error { RED } else { ACCENT },
                            ));
                            ui.label(RichText::new(&t.text).color(TEXT));
                        });
                    let r = r.response.rect;
                    th::vgradient(
                        ui.painter(),
                        egui::Rect::from_min_size(r.min, egui::vec2(3.0, r.height())),
                        if t.error { RED } else { th::ACCENT_STRONG },
                        if t.error {
                            RED.gamma_multiply(0.5)
                        } else {
                            Color32::from_rgb(0x1a, 0x44, 0xc2)
                        },
                    );
                });
                if f < 1.0 {
                    ui.ctx().request_repaint();
                }
            }
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}

fn confirm_modal(app: &mut App, ctx: &egui::Context) {
    let Some(c) = &app.confirm else { return };
    let mut close = false;
    let mut go = false;
    egui::Modal::new(egui::Id::unique("confirm")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        match c {
            Confirm::Delete(keys) => {
                ui.label(caps(
                    &format!("Delete {} add-on(s)?", keys.len()),
                    18.0,
                    TEXT_STRONG,
                ));
                ui.label(
                    RichText::new(
                        "The files are moved to the Recycle Bin, so you can restore them.",
                    )
                    .color(MUTED),
                );
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for k in keys {
                            let name = app
                                .lib
                                .index_of_key(k)
                                .map(|i| app.display_title(&app.lib.addons[i]))
                                .unwrap_or_else(|| k.clone());
                            ui.label(format!("• {name}"));
                        }
                    });
            }
            Confirm::DeleteProfile(n) => {
                ui.label(caps(&format!("Delete profile \"{n}\"?"), 18.0, TEXT_STRONG));
            }
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add(th::Button::new("Delete", Kind::Danger).min_width(90.0))
                .clicked()
            {
                go = true;
            }
            if ui
                .add(th::Button::new("Cancel", Kind::Dark).min_width(90.0))
                .clicked()
                || ui.input(|i| i.key_pressed(Key::Escape))
            {
                close = true;
            }
        });
    });
    if go {
        match app.confirm.take() {
            Some(Confirm::Delete(keys)) => app.delete(&keys),
            Some(Confirm::DeleteProfile(n)) => {
                app.cfg.profiles.remove(&n);
                app.save_config();
            }
            None => {}
        }
    } else if close {
        app.confirm = None;
    }
}

// ---------------------------------------------------------------- library tab

fn library_tab(app: &mut App, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    egui::Panel::left("filters")
        .resizable(true)
        .default_size(224.0)
        .min_size(180.0)
        .show_separator_line(false)
        .frame(th::panel_frame(th::PANEL))
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| filters_panel(app, ui));
        });
    preview_panel(app, ui, acts);

    let view = library_view(app);
    egui::CentralPanel::default()
        .frame(central_frame())
        .show(ui, |ui| {
            paint_backdrop(ui);
            // toolbar (wraps instead of disappearing under the preview when the list gets narrow)
            ui.horizontal_wrapped(|ui| {
                let r = th::search_box(
                    ui,
                    &mut app.search,
                    "Search title, author, tag, id, notes…   Ctrl+F",
                    (ui.available_width() - 250.0).clamp(160.0, 560.0),
                    "lib_search",
                );
                if app.focus_search {
                    r.request_focus();
                    app.focus_search = false;
                }
                ui.horizontal(|ui| {
                    ui.label(caps("Sort", 10.5, DIM));
                    egui::ComboBox::from_id_salt("sort")
                        .selected_text(sort_label(app.filter.sort))
                        .show_ui(ui, |ui| {
                            for s in [
                                LibSort::Name,
                                LibSort::Size,
                                LibSort::Updated,
                                LibSort::Installed,
                                LibSort::Conflicts,
                                LibSort::Popularity,
                            ] {
                                ui.selectable_value(&mut app.filter.sort, s, sort_label(s));
                            }
                        });
                    if ui
                        .add(th::Button::new(
                            if app.filter.desc { "⬇" } else { "⬆" },
                            Kind::Dark,
                        ))
                        .on_hover_text(if app.filter.desc {
                            "Descending — click to reverse"
                        } else {
                            "Ascending — click to reverse"
                        })
                        .clicked()
                    {
                        app.filter.desc = !app.filter.desc;
                    }
                });
            });
            ui.add_space(2.0);
            let total = app.lib.addons.len();
            let narrowed = view.len() < total;
            let nfilters = active_filters(&app.filter);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(view.len().to_string())
                        .font(th::bold(13.0))
                        .color(TEXT_STRONG),
                );
                ui.label(RichText::new(format!("of {total} add-ons")).color(MUTED));
                if nfilters > 0 || !app.search.is_empty() {
                    let what = match (nfilters, app.search.is_empty()) {
                        (0, _) => "Clear search".to_string(),
                        (n, true) => format!("Clear {n} filter{}", if n == 1 { "" } else { "s" }),
                        (n, false) => {
                            format!("Clear search + {n} filter{}", if n == 1 { "" } else { "s" })
                        }
                    };
                    if th::chip(ui, &format!("✖ {what}"), false)
                        .on_hover_text("Show every add-on again")
                        .clicked()
                    {
                        clear_filters(&mut app.filter);
                        app.search.clear();
                    }
                }
                ui.add_space(6.0);
                let visible_keys = || {
                    view.iter()
                        .map(|r| app.lib.addons[r.i].key.clone())
                        .collect::<Vec<_>>()
                };
                let (en_label, dis_label) = if narrowed {
                    (
                        format!("Enable all {}", view.len()),
                        format!("Disable all {}", view.len()),
                    )
                } else {
                    ("Enable all".to_string(), "Disable all".to_string())
                };
                let scope = if narrowed {
                    "every add-on in the filtered list below"
                } else {
                    "every installed add-on"
                };
                if ui
                    .add_enabled(!view.is_empty(), th::Button::new(&en_label, Kind::Dark))
                    .on_hover_text(format!("Enable {scope}"))
                    .clicked()
                {
                    acts.push(Act::SetEnabled(visible_keys(), true));
                }
                if ui
                    .add_enabled(!view.is_empty(), th::Button::new(&dis_label, Kind::Dark))
                    .on_hover_text(format!("Disable {scope}"))
                    .clicked()
                {
                    acts.push(Act::SetEnabled(visible_keys(), false));
                }
            });
            if !app.selected.is_empty() {
                // selection bar: its own strip so it never pushes the list actions out of view
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x1d, 0x3a, 0x55))
                    .corner_radius(3)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            th::pill(
                                ui,
                                &format!("{} SELECTED", app.selected.len()),
                                TEXT_STRONG,
                                th::ACCENT_STRONG.gamma_multiply(0.8),
                            );
                            let sel: Vec<String> = app.selected.iter().cloned().collect();
                            if ui
                                .add(th::Button::new("Enable", Kind::Dark))
                                .on_hover_text("Space toggles")
                                .clicked()
                            {
                                acts.push(Act::SetEnabled(sel.clone(), true));
                            }
                            if ui
                                .add(th::Button::new("Disable", Kind::Dark))
                                .on_hover_text("Space toggles")
                                .clicked()
                            {
                                acts.push(Act::SetEnabled(sel.clone(), false));
                            }
                            let upd: Vec<u64> = sel
                                .iter()
                                .filter_map(|k| app.lib.index_of_key(k))
                                .filter(|&i| app.update_state(&app.lib.addons[i]) == Upd::Available)
                                .filter_map(|i| app.lib.addons[i].id)
                                .collect();
                            if !upd.is_empty()
                                && ui
                                    .add(th::Button::new(
                                        &format!("Update {}", upd.len()),
                                        Kind::Green,
                                    ))
                                    .clicked()
                            {
                                for id in upd {
                                    acts.push(Act::Update(id));
                                }
                            }
                            if ui
                                .add(th::Button::new("Delete…", Kind::Danger))
                                .on_hover_text("Moves the files to the Recycle Bin (Del)")
                                .clicked()
                            {
                                acts.push(Act::AskDelete(sel));
                            }
                            if th::chip(ui, "✖ Deselect", false)
                                .on_hover_text("Esc")
                                .clicked()
                            {
                                app.selected.clear();
                            }
                        });
                    });
            }
            ui.add_space(4.0);
            th::rule(ui, ui.cursor().top());
            ui.add_space(4.0);

            // keyboard: ctrl+A, up/down
            let typing = ui.ctx().text_edit_focused();
            if !typing {
                let (all, up, down) = ui.input(|i| {
                    (
                        i.modifiers.command && i.key_pressed(Key::A),
                        i.key_pressed(Key::ArrowUp),
                        i.key_pressed(Key::ArrowDown),
                    )
                });
                if all {
                    app.selected = view
                        .iter()
                        .map(|r| app.lib.addons[r.i].key.clone())
                        .collect();
                }
                if up || down {
                    let cur = match &app.focus {
                        Some(Focus::Local(k)) => {
                            view.iter().position(|r| &app.lib.addons[r.i].key == k)
                        }
                        _ => None,
                    };
                    let next = match cur {
                        Some(c) if up => c.saturating_sub(1),
                        Some(c) => (c + 1).min(view.len().saturating_sub(1)),
                        None => 0,
                    };
                    if let Some(r) = view.get(next) {
                        acts.push(Act::Click {
                            key: app.lib.addons[r.i].key.clone(),
                            ctrl: false,
                            shift: false,
                        });
                    }
                }
            }

            if app.lib.addons.is_empty() || view.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.label(RichText::new("☣").size(48.0).color(DIM));
                    ui.label(caps(
                        if app.scanning {
                            "Scanning add-ons…"
                        } else if app.lib.addons.is_empty() {
                            "No add-ons found"
                        } else {
                            "Nothing matches"
                        },
                        16.0,
                        MUTED,
                    ));
                    if !app.lib.addons.is_empty() && !app.scanning {
                        ui.label(
                            RichText::new(
                                "Try a shorter search, or clear the filters on the left.",
                            )
                            .color(DIM),
                        );
                        ui.add_space(8.0);
                        if ui
                            .add(th::Button::new("Clear search and filters", Kind::Blue))
                            .clicked()
                        {
                            clear_filters(&mut app.filter);
                            app.search.clear();
                        }
                    }
                });
                return;
            }
            let app_ref: &App = app;
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            // show_rows adds item_spacing itself, so pass the exact allocated row height (keeps rows from jumping while scrolling).
            egui::ScrollArea::vertical()
                .id_salt("lib_rows")
                .auto_shrink([false; 2])
                .show_rows(ui, ROW_H, view.len(), |ui, range| {
                    for r in &view[range] {
                        // Stable per-add-on id scope: widgets (toggle, hover/focus animations) belong to the add-on,
                        // not to the on-screen slot, so they don't replay animations from the row that scrolled away.
                        ui.push_id(&app_ref.lib.addons[r.i].key, |ui| {
                            addon_row(app_ref, ui, r, acts)
                        });
                    }
                });
        });
}

fn sort_label(s: LibSort) -> &'static str {
    match s {
        LibSort::Name => "Name / relevance",
        LibSort::Size => "Size",
        LibSort::Updated => "Workshop update",
        LibSort::Installed => "File date",
        LibSort::Conflicts => "Conflicts",
        LibSort::Popularity => "Popularity",
    }
}

fn active_filters(f: &LibFilter) -> usize {
    [
        f.status != Status::All,
        f.conflicts,
        f.updates,
        f.missing_deps,
        f.modified,
        f.removed,
        f.local_only,
        f.duplicates,
        f.noted,
    ]
    .iter()
    .filter(|b| **b)
    .count()
        + f.categories.len()
}

fn clear_filters(f: &mut LibFilter) {
    f.status = Status::All;
    f.conflicts = false;
    f.updates = false;
    f.missing_deps = false;
    f.modified = false;
    f.removed = false;
    f.local_only = false;
    f.duplicates = false;
    f.noted = false;
    f.categories.clear();
}

fn central_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(th::BG)
        .inner_margin(egui::Margin::symmetric(14, 10))
}

/// Store-page atmosphere: a cool blue glow at the top fading into the body color.
fn paint_backdrop(ui: &egui::Ui) {
    let r = ui.max_rect().expand2(egui::vec2(14.0, 10.0));
    let glow = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), 220.0f32.min(r.height())));
    th::vgradient(
        ui.painter(),
        glow,
        Color32::from_rgb(0x23, 0x3c, 0x55),
        th::BG,
    );
    th::vgradient(
        ui.painter(),
        egui::Rect::from_min_max(egui::pos2(r.left(), r.bottom() - 160.0), r.max),
        th::BG,
        th::BG_LOW,
    );
}

/// Steam-sidebar style filter line: label + count, accent bar and text when on.
fn filter_row(ui: &mut egui::Ui, on: &mut bool, icon: &str, label: &str, n: usize, color: Color32) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 24.0), Sense::click());
    let hov = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let sel = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("on"), *on, 0.15);
    let p = ui.painter();
    if sel > 0.0 {
        th::hgradient(
            p,
            rect,
            color.gamma_multiply(0.22 * sel),
            Color32::TRANSPARENT,
        );
        p.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            0.0,
            color.gamma_multiply(sel),
        );
    } else if hov > 0.0 {
        p.rect_filled(rect, 2.0, Color32::from_white_alpha((10.0 * hov) as u8));
    }
    let dim = n == 0 && !*on;
    let ic = if dim { DIM } else { color };
    p.text(
        egui::pos2(rect.left() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icon,
        FontId::proportional(12.5),
        ic,
    );
    let tc = if *on {
        TEXT_STRONG
    } else if dim {
        DIM
    } else {
        TEXT
    };
    p.text(
        egui::pos2(rect.left() + 30.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        tc,
    );
    p.text(
        egui::pos2(rect.right() - 6.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        n.to_string(),
        th::bold(11.0),
        if dim { DIM } else { MUTED },
    );
    if resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        *on = !*on;
    }
}

fn filters_panel(app: &mut App, ui: &mut egui::Ui) {
    let lib = &app.lib;
    let en = lib.addons.iter().filter(|a| a.enabled).count();
    ui.add_space(2.0);
    side_caption(ui, "Show");
    let total = lib.addons.len();
    let f = &mut app.filter;
    // segmented control
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let w = (ui.available_width() / 3.0).floor();
        for (s, l, n) in [
            (Status::All, "All", total),
            (Status::Enabled, "On", en),
            (Status::Disabled, "Off", total - en),
        ] {
            let on = f.status == s;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 40.0), Sense::click());
            let hov = ui
                .ctx()
                .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
            let bg = if on {
                th::SURFACE_HI
            } else {
                th::INPUT.lerp_to_gamma(th::SURFACE, hov)
            };
            ui.painter()
                .rect_filled(rect.shrink2(egui::vec2(1.0, 0.0)), 2.0, bg);
            if on {
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        rect.left_bottom() - egui::vec2(-1.0, 2.0),
                        egui::vec2(w - 2.0, 2.0),
                    ),
                    0.0,
                    th::ACCENT_STRONG,
                );
            }
            ui.painter().text(
                rect.center() - egui::vec2(0.0, 7.0),
                egui::Align2::CENTER_CENTER,
                n.to_string(),
                th::bold(14.0),
                if on { TEXT_STRONG } else { TEXT },
            );
            ui.painter().text(
                rect.center() + egui::vec2(0.0, 9.0),
                egui::Align2::CENTER_CENTER,
                l.to_uppercase(),
                th::display(10.0),
                if on { ACCENT } else { DIM },
            );
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                f.status = s;
            }
        }
    });
    side_caption(ui, "Only");
    let app_ro: &App = app;
    let conf = (0..app_ro.lib.addons.len())
        .filter(|&i| app_ro.lib.active_conflicts(i) > 0)
        .count();
    let upd = app_ro
        .lib
        .addons
        .iter()
        .filter(|a| app_ro.update_state(a) == Upd::Available)
        .count();
    let modi = app_ro
        .lib
        .addons
        .iter()
        .filter(|a| app_ro.update_state(a) == Upd::Modified)
        .count();
    let rem = app_ro
        .lib
        .addons
        .iter()
        .filter(|a| app_ro.update_state(a) == Upd::Removed)
        .count();
    let miss = app_ro
        .lib
        .addons
        .iter()
        .filter(|a| a.id.is_some_and(|id| !app_ro.missing_deps(id).is_empty()))
        .count();
    let local = app_ro.lib.addons.iter().filter(|a| a.id.is_none()).count();
    let dup = app_ro
        .lib
        .addons
        .iter()
        .filter(|a| app_ro.is_duplicate(a))
        .count();
    let noted = app_ro.cfg.notes.len();
    let mut cats: BTreeMap<String, usize> = BTreeMap::new();
    for a in &app_ro.lib.addons {
        for c in app_ro.categories(a) {
            *cats.entry(c).or_default() += 1;
        }
    }
    let f = &mut app.filter;
    ui.spacing_mut().item_spacing.y = 1.0;
    filter_row(ui, &mut f.conflicts, "⚠", "Conflicting", conf, RED);
    filter_row(ui, &mut f.updates, "⬆", "Update available", upd, ACCENT);
    filter_row(
        ui,
        &mut f.missing_deps,
        "🔗",
        "Missing requirements",
        miss,
        ORANGE,
    );
    filter_row(
        ui,
        &mut f.modified,
        "✎",
        "Differs from Workshop",
        modi,
        MUTED,
    );
    filter_row(ui, &mut f.removed, "✖", "Gone from Workshop", rem, ORANGE);
    filter_row(
        ui,
        &mut f.local_only,
        "◇",
        "Not from Workshop",
        local,
        PURPLE,
    );
    filter_row(ui, &mut f.duplicates, "2×", "Installed twice", dup, ORANGE);
    filter_row(ui, &mut f.noted, "📝", "Has notes", noted, TEXT);
    ui.spacing_mut().item_spacing.y = 5.0;
    ui.horizontal(|ui| {
        side_caption(ui, "Categories");
        if !f.categories.is_empty() {
            ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                if ui
                    .add(
                        egui::Label::new(RichText::new("clear").size(11.0).color(ACCENT))
                            .sense(Sense::click()),
                    )
                    .clicked()
                {
                    f.categories.clear();
                }
            });
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for (c, n) in &cats {
            let on = f.categories.contains(c);
            if th::chip(ui, &format!("{c}  {n}"), on).clicked() {
                if on {
                    f.categories.remove(c);
                } else {
                    f.categories.insert(c.clone());
                }
            }
        }
    });
    profiles_ui(app, ui);
}

fn side_caption(ui: &mut egui::Ui, t: &str) {
    ui.add_space(10.0);
    ui.label(caps(t, 11.0, DIM));
    ui.add_space(2.0);
}

fn profiles_ui(app: &mut App, ui: &mut egui::Ui) {
    side_caption(ui, "Profiles");
    let names: Vec<String> = app.cfg.profiles.keys().cloned().collect();
    if names.is_empty() {
        ui.label(
            RichText::new("Save sets of enabled add-ons and switch between them.")
                .size(11.5)
                .color(DIM),
        );
    }
    for n in names {
        ui.horizontal(|ui| {
            let count = app.cfg.profiles[&n].len();
            if ui
                .add(
                    th::Button::new(&format!("▶  {n}"), Kind::Dark)
                        .min_width(ui.available_width() - 34.0),
                )
                .on_hover_text(format!("Apply: enable exactly these {count} add-ons"))
                .clicked()
            {
                app.apply_profile(&n);
            }
            ui.menu_button("⋯", |ui| {
                if ui.button("Overwrite with current").clicked() {
                    app.save_profile(&n);
                    ui.close();
                }
                if ui.button("Export…").clicked() {
                    app.export_profile(&n);
                    ui.close();
                }
                if ui.button(RichText::new("Delete").color(RED)).clicked() {
                    app.confirm = Some(Confirm::DeleteProfile(n.clone()));
                    ui.close();
                }
            });
        });
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.profile_name)
                .hint_text("New profile name")
                .desired_width((ui.available_width() - 56.0).max(80.0)),
        );
        if ui
            .add_enabled(
                !app.profile_name.trim().is_empty(),
                th::Button::new("Save", Kind::Blue),
            )
            .clicked()
        {
            let n = app.profile_name.trim().to_string();
            app.save_profile(&n);
            app.profile_name.clear();
        }
    });
    if ui
        .add(
            egui::Label::new(RichText::new("Import profile…").size(12.0).color(ACCENT))
                .sense(Sense::click()),
        )
        .clicked()
    {
        app.import_profile();
    }
}

fn addon_row(app: &App, ui: &mut egui::Ui, r: &Row, acts: &mut Vec<Act>) {
    let a = &app.lib.addons[r.i];
    let d = a.id.and_then(|id| app.cache.details.get(&id));
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let selected = app.selected.contains(&a.key);
    let focused = app.focus.as_ref() == Some(&Focus::Local(a.key.clone()));
    // Pointer anywhere over the row (also over its toggle / menu button) counts as hovering it.
    let over = ui.rect_contains_pointer(rect);
    let hov = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), over, 0.1);
    let foc = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("foc"), focused, 0.16);
    {
        let p = ui.painter();
        let base = Color32::from_rgb(0x1f, 0x2d, 0x3d)
            .lerp_to_gamma(Color32::from_rgb(0x2a, 0x3d, 0x52), hov);
        p.rect_filled(
            rect,
            2.0,
            if selected && !focused {
                Color32::from_rgb(0x24, 0x40, 0x5c)
            } else {
                base
            },
        );
        if foc > 0.0 {
            th::hgradient(
                p,
                rect,
                Color32::from_rgb(0x2f, 0x6d, 0xa0).gamma_multiply(foc),
                Color32::from_rgb(0x23, 0x3c, 0x55).gamma_multiply(foc),
            );
            p.rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
                0.0,
                th::ACCENT_STRONG.gamma_multiply(foc),
            );
        } else if selected {
            p.rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
                0.0,
                ACCENT.gamma_multiply(0.6),
            );
        }
    }
    let conflicts = app.lib.active_conflicts(r.i);
    let upd = app.update_state(a);
    let missing = a.id.map(|id| app.missing_deps(id).len()).unwrap_or(0);
    // Narrow list: icon-only badges so they stay visible next to the title.
    let compact = rect.width() < 560.0;
    let mut badges: Vec<(String, Color32, String)> = Vec::new();
    let mut add = |full: &str, short: &str, c: Color32, tip: &str| {
        badges.push((
            if compact { short } else { full }.to_string(),
            c,
            tip.to_string(),
        ))
    };
    if conflicts > 0 {
        add(
            &format!("⚠ {conflicts}"),
            &format!("⚠{conflicts}"),
            RED,
            &format!("Conflicts with {conflicts} other enabled add-on(s)"),
        );
    }
    match upd {
        Upd::Available => add("UPDATE", "⬆", ACCENT, "A newer version is on the Workshop"),
        Upd::Modified => add(
            "✎ PATCHED",
            "✎",
            GRAY,
            "Local file differs from the Workshop version (patched locally?)",
        ),
        Upd::Removed => add(
            "GONE",
            "✖",
            ORANGE,
            "Removed, private or banned on the Workshop",
        ),
        _ => {}
    }
    if missing > 0 {
        add(
            &format!("🔗 {missing} MISSING"),
            &format!("🔗{missing}"),
            ORANGE,
            &format!("{missing} required Workshop item(s) aren't installed"),
        );
    }
    if a.in_workshop_dir {
        add(
            "WORKSHOP",
            "W",
            PURPLE,
            "In addons/workshop (managed by Steam subscriptions)",
        );
    }
    if app.is_duplicate(a) {
        add(
            "2×",
            "2×",
            ORANGE,
            "Installed both in addons/ and addons/workshop",
        );
    }
    if let Some(n) = app.cfg.notes.get(&a.key) {
        add("📝", "📝", TEXT, n);
    }
    if let Some(e) = &a.error {
        add("CORRUPT", "!", RED, e);
    }

    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(10.0, 4.0)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            if th::toggle(ui, a.enabled)
                .on_hover_text(if a.enabled {
                    "Enabled — click to disable"
                } else {
                    "Disabled — click to enable"
                })
                .clicked()
            {
                acts.push(Act::SetEnabled(vec![a.key.clone()], !a.enabled));
            }
            ui.add_space(4.0);
            ui.scope(|ui| {
                if !a.enabled {
                    ui.multiply_opacity(0.45);
                }
                image_box(ui, thumb(app, Some(a), d, 96), egui::vec2(44.0, 44.0));
            });
            ui.add_space(4.0);
            // Right side first (menu + badges), then the title takes whatever width is left.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let menu_w = 26.0;
                if over || focused || selected {
                    ui.scope(|ui| {
                        ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0);
                        ui.menu_button(RichText::new("⋯").size(16.0), |ui| {
                            local_actions_menu(app, ui, a, acts)
                        })
                        .response
                        .on_hover_text("More actions (or right-click the row)");
                    });
                } else {
                    ui.add_space(menu_w);
                }
                for (t, c, tip) in &badges {
                    badge(ui, t, *c, tip);
                }
                ui.add_space(4.0);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.set_width(ui.available_width().max(40.0));
                        ui.add_space(5.0);
                        let title = app.display_title(a);
                        ui.add(
                            egui::Label::new(highlighted(&title, &r.hl, 14.0, a.enabled))
                                .selectable(false)
                                .truncate(),
                        );
                        let mut sub = vec![size(a.size)];
                        if !a.info.author.is_empty() {
                            sub.push(a.info.author.clone());
                        }
                        if let Some(d) = d.filter(|d| d.time_updated > 0) {
                            sub.push(format!("updated {}", ago(d.time_updated)));
                        }
                        let cats = app.categories(a);
                        if !cats.is_empty() {
                            sub.push(cats.into_iter().take(3).collect::<Vec<_>>().join(", "));
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(sub.join("   ·   "))
                                    .size(11.5)
                                    .color(if a.enabled { MUTED } else { DIM }),
                            )
                            .selectable(false)
                            .truncate(),
                        );
                    });
                });
            });
        },
    );

    if resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        acts.push(Act::Click {
            key: a.key.clone(),
            ctrl: m.command,
            shift: m.shift,
        });
    }
    if resp.double_clicked() {
        acts.push(Act::SetEnabled(vec![a.key.clone()], !a.enabled));
    }
    resp.context_menu(|ui| {
        local_actions_menu(app, ui, a, acts);
    });
}

fn local_actions_menu(app: &App, ui: &mut egui::Ui, a: &Addon, acts: &mut Vec<Act>) {
    let keys: Vec<String> = if app.selected.contains(&a.key) && app.selected.len() > 1 {
        app.selected.iter().cloned().collect()
    } else {
        vec![a.key.clone()]
    };
    let n = keys.len();
    if ui
        .button(format!(
            "Enable{}",
            if n > 1 {
                format!(" {n}")
            } else {
                String::new()
            }
        ))
        .clicked()
    {
        acts.push(Act::SetEnabled(keys.clone(), true));
        ui.close();
    }
    if ui
        .button(format!(
            "Disable{}",
            if n > 1 {
                format!(" {n}")
            } else {
                String::new()
            }
        ))
        .clicked()
    {
        acts.push(Act::SetEnabled(keys.clone(), false));
        ui.close();
    }
    ui.separator();
    if let Some(id) = a.id {
        if app.update_state(a) == Upd::Available && ui.button("⬆ Update").clicked() {
            acts.push(Act::Update(id));
            ui.close();
        }
        if ui.button("Open on Workshop").clicked() {
            ui.ctx()
                .open_url(egui::OpenUrl::new_tab(steam::item_url(id)));
            ui.close();
        }
        if ui.button("Copy Workshop link").clicked() {
            ui.ctx().copy_text(steam::item_url(id));
            ui.close();
        }
        if ui.button("Find add-ons that require this").clicked() {
            acts.push(Act::UsedBy(id));
            ui.close();
        }
    }
    if ui.button("Show in Explorer").clicked() {
        reveal(&a.path);
        ui.close();
    }
    if a.in_workshop_dir && ui.button("Move to addons/").clicked() {
        acts.push(Act::MoveOut(a.key.clone()));
        ui.close();
    }
    ui.separator();
    if ui
        .button(
            RichText::new(format!(
                "Delete{}…",
                if n > 1 {
                    format!(" {n}")
                } else {
                    String::new()
                }
            ))
            .color(RED),
        )
        .clicked()
    {
        acts.push(Act::AskDelete(keys));
        ui.close();
    }
}

// ---------------------------------------------------------------- preview

fn preview_panel(app: &mut App, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    // Make sure the focused item has fresh dependency info.
    let fid = match &app.focus {
        Some(Focus::Local(k)) => app.lib.index_of_key(k).and_then(|i| app.lib.addons[i].id),
        Some(Focus::Workshop(id)) => Some(*id),
        None => None,
    };
    if let Some(id) = fid {
        app.request_page(id, true);
    }
    let mut note_change: Option<(String, String)> = None;
    egui::Panel::right("preview").resizable(true).default_size(440.0).min_size(320.0).max_size((ui.available_width() - 380.0).max(320.0)).show_separator_line(false).frame(egui::Frame::new().fill(th::PANEL)).show(ui, |ui| {
        let app: &App = app;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let (local, id) = match &app.focus {
                Some(Focus::Local(k)) => match app.lib.index_of_key(k) {
                    Some(i) => (Some(i), app.lib.addons[i].id),
                    None => (None, None),
                },
                Some(Focus::Workshop(id)) => (app.lib.addons.iter().position(|a| a.id == Some(*id)), Some(*id)),
                None => (None, None),
            };
            if local.is_none() && id.is_none() {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(24, 0)).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(90.0);
                        ui.label(RichText::new("☣").size(56.0).color(th::SURFACE_HI));
                        ui.label(caps("No add-on selected", 15.0, MUTED));
                        ui.add_space(4.0);
                        ui.label(RichText::new("Select one to see its Workshop page, requirements and conflicts.").color(DIM));
                        ui.add_space(18.0);
                    });
                    for (k, v) in [
                        ("Double-click", "toggle on / off"),
                        ("Ctrl / Shift-click", "multi-select"),
                        ("Space", "toggle selection"),
                        ("Del", "delete selection"),
                        ("Ctrl+F", "search"),
                        ("Right-click", "more actions"),
                    ] {
                        ui.horizontal(|ui| {
                            th::pill(ui, k, ACCENT, th::SURFACE_HI);
                            ui.label(RichText::new(v).color(MUTED));
                        });
                    }
                });
                return;
            }
            note_change = preview(app, ui, local, id, acts);
        });
    });
    if let Some((k, n)) = note_change {
        acts.push(Act::SetNote(k, n));
    }
}

fn preview(
    app: &App,
    ui: &mut egui::Ui,
    local: Option<usize>,
    id: Option<u64>,
    acts: &mut Vec<Act>,
) -> Option<(String, String)> {
    let a = local.map(|i| &app.lib.addons[i]);
    let d = id.and_then(|id| app.cache.details.get(&id));
    let page = id.and_then(|id| app.cache.pages.get(&id));
    let w = ui.available_width();

    // hero image, fading into the panel
    let hero = if let Some(url) = d.filter(|d| !d.preview_url.is_empty()).map(|d| {
        format!(
            "{}?imw={}&ima=fit",
            d.preview_url,
            (w as u32).clamp(200, 900)
        )
    }) {
        Some(egui::ImageSource::Uri(url.into()))
    } else {
        thumb(app, a, None, 0)
    };
    let hero_h = (w * 0.56).min(320.0);
    // gallery: the main preview image, then the screenshots the author uploaded
    let shots: &[String] = page.and_then(|p| p.previews.as_deref()).unwrap_or(&[]);
    let main_img = d.map(|d| d.preview_url.as_str()).filter(|u| !u.is_empty());
    let gallery: Vec<String> = main_img.into_iter().map(str::to_string).chain(shots.iter().cloned()).collect();
    let (hero_rect, hero_resp) = ui.allocate_exact_size(egui::vec2(w, hero_h), Sense::click());
    if main_img.is_some() && hero_resp.on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
        open_lightbox(ui.ctx(), &gallery, 0);
    }
    th::vgradient(
        ui.painter(),
        hero_rect,
        Color32::from_rgb(0x23, 0x3c, 0x55),
        th::PANEL,
    );
    if let Some(src) = hero {
        let img = egui::Image::new(src)
            .max_width(w)
            .max_height(hero_h)
            .show_loading_spinner(true);
        let size = img
            .load_and_calc_size(ui, egui::vec2(w, hero_h))
            .unwrap_or(egui::vec2(w, hero_h));
        img.paint_at(ui, egui::Rect::from_center_size(hero_rect.center(), size));
    }
    let fade = egui::Rect::from_min_max(
        egui::pos2(hero_rect.left(), hero_rect.bottom() - hero_h * 0.35),
        hero_rect.max,
    );
    th::vgradient(ui.painter(), fade, Color32::TRANSPARENT, th::PANEL);

    let mut note_change = None;
    egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 0, bottom: 16 }).show(ui, |ui| {
        ui.add_space(-26.0);
        let title = match (a, d) {
            (Some(a), _) => app.display_title(a),
            (None, Some(d)) if !d.title.is_empty() => d.title.clone(),
            _ => id.map(|id| format!("Workshop item {id}")).unwrap_or_default(),
        };
        ui.add(egui::Label::new(RichText::new(title).font(th::display(24.0)).color(TEXT_STRONG)).wrap());
        let creators = page.map(|p| p.creators.join(", ")).filter(|s| !s.is_empty()).or_else(|| a.map(|a| a.info.author.clone()).filter(|s| !s.is_empty()));
        if let Some(c) = creators {
            ui.horizontal(|ui| {
                ui.label(caps("By", 10.5, DIM));
                ui.label(RichText::new(c).color(ACCENT));
            });
        }
        if id.is_some() && d.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Loading Workshop details…").color(MUTED));
            });
        }
        if !shots.is_empty() {
            ui.add_space(4.0);
            gallery_strip(ui, &gallery, gallery.len() - shots.len());
        }

        // status
        if let Some(a) = a {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                badge(ui, if a.enabled { "ENABLED" } else { "DISABLED" }, if a.enabled { GREEN } else { GRAY }, "");
                match app.update_state(a) {
                    Upd::Available => badge(ui, "UPDATE AVAILABLE", BLUE, ""),
                    Upd::Modified => badge(ui, "DIFFERS FROM WORKSHOP", GRAY, "The local file's size doesn't match the Workshop file. It may be patched locally — updating would undo that."),
                    Upd::Removed => badge(ui, "GONE FROM WORKSHOP", ORANGE, ""),
                    Upd::UpToDate => badge(ui, "UP TO DATE", GREEN, ""),
                    Upd::Unknown => {}
                }
                if a.in_workshop_dir {
                    badge(ui, "ADDONS/WORKSHOP", PURPLE, "Steam manages this file through your subscription");
                }
                if let Some(e) = &a.error {
                    badge(ui, "UNREADABLE VPK", RED, e);
                }
            });
        }

        // primary actions: a Steam "purchase area" strip
        ui.add_space(8.0);
        egui::Frame::new().fill(Color32::from_rgb(0x10, 0x18, 0x22)).corner_radius(3).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if let Some(a) = a {
                    if a.enabled {
                        if ui.add(th::Button::new("Disable", Kind::Dark).big()).clicked() {
                            acts.push(Act::SetEnabled(vec![a.key.clone()], false));
                        }
                    } else if ui.add(th::Button::new("Enable", Kind::Green).big()).clicked() {
                        acts.push(Act::SetEnabled(vec![a.key.clone()], true));
                    }
                    if app.update_state(a) == Upd::Available && ui.add(th::Button::new("⬆ Update", Kind::Blue).big()).clicked() {
                        acts.push(Act::Update(a.id.unwrap_or(0)));
                    }
                    if app.update_state(a) == Upd::Modified && ui.add(th::Button::new("Re-download", Kind::Dark)).on_hover_text("Replace the local file with the Workshop version").clicked() {
                        acts.push(Act::Update(a.id.unwrap_or(0)));
                    }
                } else if let Some(id) = id {
                    let dl = app.downloads.iter().find(|x| x.id == id);
                    match dl.map(|x| &x.state) {
                        Some(DlState::Waiting | DlState::Queued | DlState::Running) => {
                            ui.spinner();
                            ui.label(caps("Downloading…", 13.0, ACCENT));
                        }
                        _ => {
                            let missing = app.missing_deps(id).len();
                            let label = if missing > 0 && app.cfg.install_dependencies { format!("Install  +{missing} required") } else { "Install".into() };
                            if ui.add(th::Button::new(&label, Kind::Green).big()).clicked() {
                                acts.push(Act::Install(id));
                            }
                            if d.is_some_and(|d| d.file_url.is_empty() && d.ok) && ui.add(th::Button::new("Install as collection", Kind::Blue).big()).clicked() {
                                acts.push(Act::InstallCollection(id));
                            }
                        }
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(a) = a {
                        if ui.add(th::Button::new("🗑", Kind::Danger)).on_hover_text("Delete (Recycle Bin)").clicked() {
                            acts.push(Act::AskDelete(vec![a.key.clone()]));
                        }
                        if ui.add(th::Button::new("📂", Kind::Dark)).on_hover_text("Show in Explorer").clicked() {
                            reveal(&a.path);
                        }
                    }
                });
            });
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            if let Some(id) = id {
                if ui.add(th::Button::new("Workshop page ↗", Kind::Dark)).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(steam::item_url(id)));
                }
                if ui.add(th::Button::new("Used by…", Kind::Dark)).on_hover_text("Browse Workshop items that list this as a required item").clicked() {
                    acts.push(Act::UsedBy(id));
                }
            }
            if let Some(a) = a {
                if a.in_workshop_dir && ui.add(th::Button::new("Move to addons/", Kind::Dark)).clicked() {
                    acts.push(Act::MoveOut(a.key.clone()));
                }
            }
        });

        // stats: Steam's dim-label / value column
        ui.add_space(6.0);
        egui::Grid::new("stats").num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, k: &str, v: String| {
                ui.label(caps(k, 10.5, DIM));
                ui.label(RichText::new(v).color(TEXT));
                ui.end_row();
            };
            if let Some(a) = a {
                row(ui, "File", a.key.clone());
                row(ui, "Size", size(a.size));
                if !a.info.version.is_empty() {
                    row(ui, "Version", a.info.version.clone());
                }
            } else if let Some(d) = d {
                row(ui, "Size", size(d.file_size));
            }
            if let Some(d) = d.filter(|d| d.ok) {
                row(ui, "Updated", format!("{}  ({})", date(d.time_updated), ago(d.time_updated)));
                row(ui, "Released", date(d.time_created));
                ui.label(caps("Popularity", 10.5, DIM));
                ui.horizontal(|ui| {
                    for (n, l) in [(d.subscriptions, "subs"), (d.favorited, "favs"), (d.views, "views")] {
                        ui.label(RichText::new(count(n)).font(th::bold(13.0)).color(ACCENT));
                        ui.label(RichText::new(l).size(11.5).color(MUTED));
                        ui.add_space(4.0);
                    }
                });
                ui.end_row();
            }
        });

        // tags
        let tags: Vec<String> = match a {
            Some(a) => app.categories(a),
            None => d.map(|d| d.tags.clone()).unwrap_or_default(),
        };
        if !tags.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for t in tags {
                    let on = a.is_some() && app.filter.categories.contains(&t);
                    if th::chip(ui, &t, on).on_hover_text(if a.is_some() { "Filter library by this" } else { "Browse Workshop by this tag" }).clicked() {
                        acts.push(if a.is_some() { Act::Category(t) } else { Act::WsTag(t) });
                    }
                }
            });
        }

        // notes
        if let Some(a) = a {
            ui.add_space(6.0);
            let mut note = app.cfg.notes.get(&a.key).cloned().unwrap_or_default();
            let r = ui.add(egui::TextEdit::multiline(&mut note).hint_text("📝 Personal note (searchable)").desired_rows(1).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(8, 6)));
            if r.changed() {
                note_change = Some((a.key.clone(), note));
            }
        }
        preview_sections(app, ui, a, local, id, d, page, acts);
    });
    note_change
}

/// Collapsible Steam-style section: uppercase caption, accent rule.
fn fold(
    ui: &mut egui::Ui,
    salt: &str,
    title: &str,
    color: Color32,
    open: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(6.0);
    let r = egui::CollapsingHeader::new(caps(title, 12.5, color))
        .id_salt(salt)
        .default_open(open)
        .show(ui, |ui| {
            ui.add_space(2.0);
            body(ui);
        });
    let hr = r.header_response.rect;
    th::hgradient(
        ui.painter(),
        egui::Rect::from_min_size(
            egui::pos2(hr.left(), hr.bottom()),
            egui::vec2(ui.available_width(), 1.0),
        ),
        color.gamma_multiply(0.45),
        Color32::TRANSPARENT,
    );
}

#[allow(clippy::too_many_arguments)]
fn preview_sections(
    app: &App,
    ui: &mut egui::Ui,
    a: Option<&Addon>,
    local: Option<usize>,
    id: Option<u64>,
    d: Option<&Details>,
    page: Option<&steam::PageInfo>,
    acts: &mut Vec<Act>,
) {
    // requirements
    if let Some(id) = id {
        let req = page.map(|p| p.required.clone());
        let n = req
            .as_ref()
            .map(|r| r.len().to_string())
            .unwrap_or("…".into());
        let missing = req.as_ref().is_some_and(|r| {
            r.iter()
                .any(|(rid, _)| app.lib.by_id(*rid).next().is_none())
        });
        fold(
            ui,
            "req",
            &format!("Requires  {n}"),
            if missing { ORANGE } else { TEXT },
            true,
            |ui| match req {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Reading the Workshop page…").color(MUTED));
                    });
                }
                Some(r) if r.is_empty() => {
                    ui.label(RichText::new("No required items listed on the Workshop.").color(DIM));
                }
                Some(r) => {
                    for (rid, rtitle) in r {
                        let inst: Vec<&Addon> = app.lib.by_id(rid).collect();
                        let name = if rtitle.is_empty() {
                            app.title_of(rid)
                        } else {
                            rtitle
                        };
                        let (c, tip) = match inst.first() {
                            Some(x) if x.enabled => (GREEN, "Installed and enabled"),
                            Some(_) => (ORANGE, "Installed but disabled"),
                            None => (RED, "Not installed"),
                        };
                        dep_row(ui, c, tip, |ui| {
                            if ui.link(RichText::new(&name).color(TEXT)).clicked() {
                                acts.push(match inst.first() {
                                    Some(x) => Act::Click {
                                        key: x.key.clone(),
                                        ctrl: false,
                                        shift: false,
                                    },
                                    None => Act::Focus(Focus::Workshop(rid)),
                                });
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                match inst.first() {
                                    Some(x) if !x.enabled => {
                                        if ui.add(th::Button::new("Enable", Kind::Green)).clicked()
                                        {
                                            acts.push(Act::SetEnabled(vec![x.key.clone()], true));
                                        }
                                    }
                                    None => {
                                        if ui.add(th::Button::new("Install", Kind::Green)).clicked()
                                        {
                                            acts.push(Act::Install(rid));
                                        }
                                    }
                                    _ => {
                                        ui.label(caps("OK", 10.0, GREEN));
                                    }
                                }
                            });
                        });
                    }
                }
            },
        );
        let by = app.required_by(id);
        if !by.is_empty() {
            fold(
                ui,
                "reqby",
                &format!("Required by installed  {}", by.len()),
                TEXT,
                true,
                |ui| {
                    for i in by {
                        let x = &app.lib.addons[i];
                        dep_row(
                            ui,
                            if x.enabled { GREEN } else { DIM },
                            if x.enabled { "Enabled" } else { "Disabled" },
                            |ui| {
                                if ui
                                    .link(RichText::new(app.display_title(x)).color(TEXT))
                                    .clicked()
                                {
                                    acts.push(Act::Click {
                                        key: x.key.clone(),
                                        ctrl: false,
                                        shift: false,
                                    });
                                }
                            },
                        );
                    }
                },
            );
        }
    }

    // conflicts
    if let Some(i) = local {
        let a = &app.lib.addons[i];
        if !a.conflicts.is_empty() {
            let active = app.lib.active_conflicts(i);
            fold(
                ui,
                "conf",
                &format!("Conflicts  {active} active · {} total", a.conflicts.len()),
                if active > 0 { RED } else { MUTED },
                active > 0,
                |ui| {
                    for &ci in &a.conflicts {
                        let c = &app.lib.conflicts[ci];
                        let other = &app.lib.addons[if c.a == i { c.b } else { c.a }];
                        conflict_block(app, ui, c, other, acts);
                    }
                },
            );
        }
    }

    // description
    fold(ui, "desc", "About this add-on", TEXT, true, |ui| {
        if let Some(d) = d.filter(|d| d.ok && !d.description.is_empty()) {
            let blocks = {
                let mut cache = app.parsed.borrow_mut();
                cache
                    .entry(d.id)
                    .or_insert_with(|| Arc::new(bbcode::parse(&d.description)))
                    .clone()
            };
            bbcode::show(ui, &blocks);
        } else if let Some(a) = a.filter(|a| !a.info.description.is_empty()) {
            ui.label(caps("From addoninfo.txt — no Workshop page", 10.0, DIM));
            ui.label(a.info.description.replace("\\n", "\n"));
        } else {
            ui.label(RichText::new("No description.").color(DIM));
        }
    });

    // files
    if let Some(a) = a {
        fold(
            ui,
            "files",
            &format!("Files  {}", a.files.len()),
            TEXT,
            false,
            |ui| {
                file_tree_ui(ui, a);
            },
        );
    }
}

const LIGHTBOX: &str = "lightbox";

fn open_lightbox(ctx: &egui::Context, urls: &[String], at: usize) {
    let state: (Arc<Vec<String>>, usize) = (Arc::new(urls.to_vec()), at);
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(LIGHTBOX), state));
}

/// Steam-style row of screenshot thumbnails; `first` = index of the first screenshot in `urls`.
fn gallery_strip(ui: &mut egui::Ui, urls: &[String], first: usize) {
    const TW: f32 = 112.0;
    const TH: f32 = 63.0;
    egui::ScrollArea::horizontal()
        .id_salt("gallery")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (k, u) in urls.iter().enumerate().skip(first) {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(TW, TH), Sense::click());
                    ui.painter().rect_filled(rect, 2.0, th::BG_DEEP);
                    let img = egui::Image::new(bbcode::sized_image_url(u, TW * 2.0)).show_loading_spinner(true);
                    let size = img.load_and_calc_size(ui, rect.size()).unwrap_or(rect.size());
                    img.paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
                    let hover = resp.hovered();
                    ui.painter().rect_stroke(
                        rect,
                        2.0,
                        egui::Stroke::new(if hover { 2.0 } else { 1.0 }, if hover { ACCENT } else { th::SURFACE_HI }),
                        egui::StrokeKind::Inside,
                    );
                    if resp.on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
                        open_lightbox(ui.ctx(), urls, k);
                    }
                }
            });
        });
}

/// Enlarged gallery image centered over the window. Esc / click outside closes; arrows page.
fn lightbox_ui(ctx: &egui::Context) {
    let id = egui::Id::unique(LIGHTBOX);
    let Some((urls, mut i)) = ctx.data_mut(|d| d.get_temp::<(Arc<Vec<String>>, usize)>(id)) else {
        return;
    };
    let n = urls.len();
    if n == 0 {
        return ctx.data_mut(|d| d.remove::<(Arc<Vec<String>>, usize)>(id));
    }
    i = i.min(n - 1);
    let scr = ctx.content_rect();
    let max = egui::vec2(scr.width() * 0.86, scr.height() * 0.78);
    let (left, right) = ctx.input(|inp| (inp.key_pressed(Key::ArrowLeft), inp.key_pressed(Key::ArrowRight)));
    let mut step = right as isize - left as isize;
    let r = egui::Modal::new(id.with("modal"))
        .frame(egui::Frame::new())
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let img = egui::Image::new(bbcode::sized_image_url(&urls[i], max.x.min(2048.0)))
                    .max_size(max)
                    .corner_radius(3.0)
                    .show_loading_spinner(true);
                // reserve the full box while loading so the arrows don't jump around
                let size = img.load_and_calc_size(ui, max).unwrap_or(max);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                img.paint_at(ui, rect);
                if n > 1 {
                    // arrows sit on the image's edges, counter underneath
                    for (dir, x) in [(-1isize, rect.left() + 30.0), (1, rect.right() - 30.0)] {
                        let r = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(40.0, 60.0));
                        let resp = ui.interact(r, id.with(dir), Sense::click());
                        let hot = resp.hovered();
                        let p = ui.painter();
                        p.rect_filled(r, 4.0, Color32::from_black_alpha(if hot { 210 } else { 130 }));
                        let (c, dx) = (r.center(), 5.0 * dir as f32);
                        let s = egui::Stroke::new(2.5, if hot { ACCENT } else { TEXT_STRONG });
                        p.line_segment([c + egui::vec2(-dx, -9.0), c + egui::vec2(dx, 0.0)], s);
                        p.line_segment([c + egui::vec2(dx, 0.0), c + egui::vec2(-dx, 9.0)], s);
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            step = dir;
                        }
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("{} / {n}", i + 1)).color(TEXT));
                }
            });
        });
    if r.should_close() {
        ctx.data_mut(|d| d.remove::<(Arc<Vec<String>>, usize)>(id));
    } else if step != 0 {
        let j = (i as isize + step).rem_euclid(n as isize) as usize;
        ctx.data_mut(|d| d.insert_temp(id, (urls, j)));
    }
}

/// Collapsible folder tree of an add-on's files, threaded like a comment section: each open
/// folder hangs a line down to its last entry. Click a folder, or any point on its line, to
/// collapse it. At most 500px tall.
fn file_tree_ui(ui: &mut egui::Ui, a: &Addon) {
    const ROW: f32 = 20.0;
    const INDENT: f32 = 16.0;
    const KNOB: f32 = 5.0; // radius of a folder's +/- knob
    let tree_id = egui::Id::unique(("ftree", a.key.as_str(), a.files.len()));
    let open_id = egui::Id::unique(("ftree_open", a.key.as_str()));
    let hover_id = egui::Id::unique(("ftree_hover", a.key.as_str()));
    let tree: Arc<Vec<library::TreeRow>> = match ui.data_mut(|d| d.get_temp(tree_id)) {
        Some(t) => t,
        None => {
            let t = Arc::new(library::file_tree(&a.files));
            ui.data_mut(|d| d.insert_temp(tree_id, t.clone()));
            t
        }
    };
    let mut open: HashSet<usize> = ui.data_mut(|d| d.get_temp(open_id)).unwrap_or_default();
    // folder whose thread was hovered last frame: its whole line lights up
    let lit: Option<usize> = ui.data_mut(|d| d.get_temp::<Option<usize>>(hover_id)).flatten();
    let rows = library::visible_rows(&tree, &open);
    let (mut toggle, mut hovered) = (None, None);
    egui::ScrollArea::vertical()
        .max_height(500.0)
        // inside the preview's own scroll area, available height is only what's left on screen
        .min_scrolled_height(500.0)
        .auto_shrink([false, true])
        .id_salt("files")
        .show_rows(ui, ROW, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for &i in &rows[range] {
                let r = &tree[i];
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), Sense::click());
                let gx = |depth: usize| rect.left() + depth as f32 * INDENT + KNOB + 1.0;
                let (y, d) = (rect.center().y, r.depth);
                let line = |f: usize| {
                    egui::Stroke::new(if lit == Some(f) { 2.0 } else { 1.0 }, if lit == Some(f) { ACCENT } else { DIM })
                };

                // ancestors from the parent upwards, with whether the child on our path is its last entry
                let mut anc = Vec::with_capacity(d);
                let mut cur = i;
                while let Some(p) = tree[cur].parent {
                    anc.push((p, tree[cur].last));
                    cur = p;
                }

                // what the pointer targets: a thread line (left of the item) or the row's own folder
                let target = resp.hover_pos().and_then(|p| {
                    let col = ((p.x - rect.left()) / INDENT).floor();
                    if col >= 0.0 && (col as usize) < d {
                        Some(anc[d - 1 - col as usize].0)
                    } else {
                        r.dir.map(|_| i)
                    }
                });
                hovered = hovered.or(target);
                if target.is_some() && resp.clicked() {
                    toggle = target;
                }

                let p = ui.painter();
                if resp.hovered() && target == Some(i) {
                    let bg = egui::Rect::from_min_max(egui::pos2(gx(d) - KNOB - 3.0, rect.top()), rect.max);
                    p.rect_filled(bg, 3.0, th::SURFACE);
                }
                // thread lines of ancestors
                for (j, &(f, child_last)) in anc.iter().enumerate() {
                    let x = gx(d - 1 - j);
                    let s = line(f);
                    if j == 0 {
                        // elbow into this row: down, round the corner, across to the item
                        let bend = 6.0;
                        let end = if r.last { y - bend } else { rect.bottom() };
                        p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, end)], s);
                        p.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
                            [egui::pos2(x, y - bend), egui::pos2(x, y), egui::pos2(x + bend, y)],
                            false,
                            Color32::TRANSPARENT,
                            s,
                        ));
                        p.line_segment([egui::pos2(x + bend, y), egui::pos2(gx(d) - KNOB - 2.0, y)], s);
                    } else if !child_last {
                        p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], s);
                    }
                }

                let font = FontId::monospace(11.5);
                let c = egui::pos2(gx(d), y);
                let name_x = gx(d) + KNOB + 6.0;
                match r.dir {
                    Some((count, _)) => {
                        let is_open = open.contains(&i);
                        let s = line(i);
                        if is_open {
                            // start of this folder's own thread
                            p.line_segment([egui::pos2(c.x, y + KNOB), egui::pos2(c.x, rect.bottom())], s);
                        }
                        let knob = if lit == Some(i) { ACCENT } else { MUTED };
                        p.circle(c, KNOB, th::PANEL, egui::Stroke::new(1.0, knob));
                        p.line_segment([c - egui::vec2(2.5, 0.0), c + egui::vec2(2.5, 0.0)], egui::Stroke::new(1.2, knob));
                        if !is_open {
                            p.line_segment([c - egui::vec2(0.0, 2.5), c + egui::vec2(0.0, 2.5)], egui::Stroke::new(1.2, knob));
                        }
                        let t = p.text(egui::pos2(name_x, y), egui::Align2::LEFT_CENTER, r.name.as_ref(), font, TEXT);
                        p.text(
                            egui::pos2(t.right() + 8.0, y),
                            egui::Align2::LEFT_CENTER,
                            if count == 1 { "1 file".to_string() } else { format!("{count} files") },
                            FontId::proportional(11.0),
                            DIM,
                        );
                    }
                    None => {
                        p.circle_filled(c, 2.0, DIM);
                        let (stem, ext) = match r.name.rfind('.') {
                            Some(dot) if dot > 0 => r.name.split_at(dot),
                            _ => (r.name.as_ref(), ""),
                        };
                        let t = p.text(egui::pos2(name_x, y), egui::Align2::LEFT_CENTER, stem, font.clone(), MUTED);
                        p.text(egui::pos2(t.right(), y), egui::Align2::LEFT_CENTER, ext, font, DIM);
                    }
                }
                if target.is_some() {
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
            }
        });
    if hovered != lit {
        ui.data_mut(|d| d.insert_temp(hover_id, hovered));
        ui.ctx().request_repaint();
    }
    if let Some(i) = toggle {
        if !open.remove(&i) {
            open.insert(i);
        }
        ui.data_mut(|d| d.insert_temp(open_id, open));
    }
}

/// One requirement / dependent line: status dot, then content.
fn dep_row(ui: &mut egui::Ui, dot: Color32, tip: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_rgb(0x1b, 0x27, 0x36))
        .corner_radius(2)
        .inner_margin(egui::Margin {
            left: 10,
            right: 6,
            top: 3,
            bottom: 3,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(8.0, 18.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 3.5, dot);
                resp.on_hover_text(tip);
                body(ui);
            });
        });
}

fn conflict_block(
    app: &App,
    ui: &mut egui::Ui,
    c: &library::Conflict,
    other: &Addon,
    acts: &mut Vec<Act>,
) {
    let active = app.lib.conflict_active(c);
    dep_row(
        ui,
        if active { RED } else { DIM },
        if active {
            "Both enabled — they fight over these files"
        } else {
            "One side is disabled"
        },
        |ui| {
            if ui
                .link(RichText::new(app.display_title(other)).color(TEXT))
                .clicked()
            {
                acts.push(Act::Click {
                    key: other.key.clone(),
                    ctrl: false,
                    shift: false,
                });
            }
            ui.label(
                RichText::new(format!(
                    "{} differ{}{}",
                    c.files.len(),
                    if c.stock > 0 {
                        format!(" · {} game files", c.stock)
                    } else {
                        String::new()
                    },
                    if c.identical > 0 {
                        format!(" · {} identical", c.identical)
                    } else {
                        String::new()
                    }
                ))
                .size(11.5)
                .color(MUTED),
            );
            if other.enabled {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(th::Button::new("Disable it", Kind::Dark)).clicked() {
                        acts.push(Act::SetEnabled(vec![other.key.clone()], false));
                    }
                });
            }
        },
    );
    if !c.files.is_empty() {
        ui.indent(("cf", c.a, c.b), |ui| {
            let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for f in &c.files {
                groups.entry(library::path_kind(f)).or_default().push(f);
            }
            egui::CollapsingHeader::new(
                groups
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.len()))
                    .collect::<Vec<_>>()
                    .join(" • "),
            )
            .id_salt(("cfh", c.a, c.b))
            .show(ui, |ui| {
                for (k, files) in groups {
                    ui.label(caps(k, 10.5, TEXT));
                    for f in files.iter().take(200) {
                        file_line(ui, f, &app.lib.stock);
                    }
                    if files.len() > 200 {
                        ui.weak(format!("…and {} more", files.len() - 200));
                    }
                }
            });
        });
    }
}

// ---------------------------------------------------------------- workshop tab

fn workshop_tab(app: &mut App, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    preview_panel(app, ui, acts);
    egui::CentralPanel::default()
        .frame(central_frame())
        .show(ui, |ui| {
            paint_backdrop(ui);
            let mut go = false;
            ui.horizontal(|ui| {
                let r = th::search_box(
                    ui,
                    &mut app.ws.text,
                    "Search the Workshop…",
                    (ui.available_width()
                        - if app.ws.query.sort == Sort::Trending {
                            340.0
                        } else {
                            240.0
                        })
                    .max(140.0),
                    "ws_search",
                );
                if app.focus_search {
                    r.request_focus();
                    app.focus_search = false;
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    go = true;
                }
                if ui.add(th::Button::new("Search", Kind::Blue)).clicked() {
                    go = true;
                }
                let before = (
                    app.ws.query.sort,
                    app.ws.query.days,
                    app.ws.query.collections,
                );
                egui::ComboBox::from_id_salt("ws_sort")
                    .selected_text(app.ws.query.sort.label())
                    .show_ui(ui, |ui| {
                        for s in Sort::ALL {
                            ui.selectable_value(&mut app.ws.query.sort, s, s.label());
                        }
                    });
                if app.ws.query.sort == Sort::Trending {
                    egui::ComboBox::from_id_salt("ws_days")
                        .width(90.0)
                        .selected_text(match app.ws.query.days {
                            1 => "Today",
                            7 => "This week",
                            30 => "This month",
                            90 => "3 months",
                            365 => "This year",
                            _ => "All time",
                        })
                        .show_ui(ui, |ui| {
                            for (d, l) in [
                                (1, "Today"),
                                (7, "This week"),
                                (30, "This month"),
                                (90, "3 months"),
                                (365, "This year"),
                                (-1, "All time"),
                            ] {
                                ui.selectable_value(&mut app.ws.query.days, d, l);
                            }
                        });
                }
                if before
                    != (
                        app.ws.query.sort,
                        app.ws.query.days,
                        app.ws.query.collections,
                    )
                {
                    go = true;
                }
            });
            ui.horizontal(|ui| {
                let tag_label = if app.ws.query.tags.is_empty() {
                    "Tags ▾".to_string()
                } else {
                    format!("Tags: {} ▾", app.ws.query.tags.join(", "))
                };
                ui.menu_button(tag_label, |ui| {
                    ui.set_max_height(400.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if ui.button("Clear tags").clicked() {
                            app.ws.query.tags.clear();
                            go = true;
                        }
                        for t in steam::TAGS {
                            let mut on = app.ws.query.tags.iter().any(|x| x == t);
                            if ui.checkbox(&mut on, *t).changed() {
                                if on {
                                    app.ws.query.tags.push(t.to_string());
                                } else {
                                    app.ws.query.tags.retain(|x| x != t);
                                }
                                go = true;
                            }
                        }
                    });
                });
                if ui
                    .checkbox(&mut app.ws.query.collections, "Collections")
                    .changed()
                {
                    go = true;
                }
                if let Some(c) = app.ws.query.child_of {
                    let t = app.title_of(c);
                    if th::chip(ui, &format!("Requires: {t}  ✖"), true)
                        .on_hover_text(
                            "Showing items that list this as a required item. Click to clear.",
                        )
                        .clicked()
                    {
                        app.ws.query.child_of = None;
                        go = true;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(th::Button::new("Install", Kind::Green))
                        .on_hover_text("Install a Workshop link, id or collection")
                        .clicked()
                    {
                        let s = std::mem::take(&mut app.ws.url_input);
                        app.install_from_input(&s);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut app.ws.url_input)
                            .hint_text("Paste Workshop link / id / collection")
                            .desired_width((ui.available_width() - 30.0).clamp(110.0, 260.0))
                            .margin(egui::Margin::symmetric(8, 4)),
                    );
                });
            });
            if go {
                app.ws.query.text = app.ws.text.clone();
                app.ws.query.page = 1;
                app.browse();
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let heading = if app.ws.query.child_of.is_some() {
                    "Items that require this".to_string()
                } else if !app.ws.query.text.is_empty() {
                    format!("Results for “{}”", app.ws.query.text)
                } else {
                    app.ws.query.sort.label().to_string()
                };
                ui.label(caps(&heading, 15.0, TEXT_STRONG));
                if app.ws.loading {
                    ui.spinner();
                }
                if let Some(e) = &app.ws.error {
                    ui.label(RichText::new(e).color(RED));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let page = app.ws.query.page;
                    if ui
                        .add_enabled(
                            app.ws.results.len() >= steam::PER_PAGE && !app.ws.loading,
                            th::Button::new("▶", Kind::Dark),
                        )
                        .on_hover_text("Next page")
                        .clicked()
                    {
                        app.ws.query.page += 1;
                        app.browse();
                    }
                    ui.label(
                        RichText::new(format!("PAGE {page}"))
                            .font(th::display(12.0))
                            .color(MUTED),
                    );
                    if ui
                        .add_enabled(
                            page > 1 && !app.ws.loading,
                            th::Button::new("◀", Kind::Dark),
                        )
                        .on_hover_text("Previous page")
                        .clicked()
                    {
                        app.ws.query.page -= 1;
                        app.browse();
                    }
                });
            });
            th::rule(ui, ui.cursor().top() + 2.0);
            ui.add_space(8.0);
            let app_ref: &App = app;
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if app_ref.ws.results.is_empty() && !app_ref.ws.loading {
                        ui.vertical_centered(|ui| {
                            ui.add_space(60.0);
                            ui.label(caps("No results", 16.0, MUTED));
                        });
                    }
                    // fit as many equal columns as the width allows
                    let gap = 12.0;
                    let avail = ui.available_width();
                    let cols = ((avail + gap) / (170.0 + gap)).floor().max(1.0);
                    let card_w = ((avail - gap * (cols - 1.0)) / cols).floor();
                    // Explicit grid: each card gets a fixed cell, so nothing inside a card can push
                    // the row wider than the panel (horizontal_wrapped failed to wrap here).
                    let cols = cols as usize;
                    let row_h = card_w.round() + 84.0;
                    ui.spacing_mut().item_spacing.y = gap;
                    for (r, chunk) in app_ref.ws.results.chunks(cols).enumerate() {
                        let (row, _) = ui.allocate_exact_size(egui::vec2(avail, row_h), Sense::hover());
                        for (c, &id) in chunk.iter().enumerate() {
                            let cell = egui::Rect::from_min_size(
                                egui::pos2(row.left() + c as f32 * (card_w + gap), row.top()),
                                egui::vec2(card_w, row_h),
                            );
                            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(cell).id_salt(id));
                            ws_card(app_ref, &mut cui, id, r * cols + c, card_w, acts);
                        }
                    }
                });
        });
}

fn ws_card(app: &App, ui: &mut egui::Ui, id: u64, n: usize, card_w: f32, acts: &mut Vec<Act>) {
    let d = app.cache.details.get(&id);
    let installed: Option<&Addon> = app.lib.by_id(id).next();
    let focused = app.focus == Some(Focus::Workshop(id))
        || installed.is_some_and(|a| app.focus == Some(Focus::Local(a.key.clone())));
    let img_h = card_w.round();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(card_w, img_h + 84.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    // staggered fade-in the first time a result is shown
    let now = ui.input(|i| i.time);
    let born =
        ui.data_mut(|m| *m.get_temp_mut_or_insert_with(egui::Id::unique(("wsborn", id)), || now));
    let fade = (((now - born) - n as f64 * 0.025) / 0.3).clamp(0.0, 1.0) as f32;
    if fade < 1.0 {
        ui.ctx().request_repaint();
    }
    let hov = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.14);
    // lift on hover, rise in on first appearance
    let card = rect.translate(egui::vec2(0.0, -hov * 3.0 + (1.0 - fade) * 10.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
        ui.multiply_opacity(fade);
        let p = ui.painter();
        if hov > 0.0 {
            p.rect_filled(
                card.translate(egui::vec2(0.0, 6.0)).shrink(4.0),
                4.0,
                Color32::from_black_alpha((90.0 * hov) as u8),
            );
        }
        let top = Color32::from_rgb(0x1e, 0x2c, 0x3c)
            .lerp_to_gamma(Color32::from_rgb(0x2a, 0x47, 0x5e), hov);
        th::vgradient(
            p,
            card,
            top,
            Color32::from_rgb(0x16, 0x20, 0x2d)
                .lerp_to_gamma(Color32::from_rgb(0x1f, 0x34, 0x48), hov),
        );
        if focused {
            p.rect_stroke(
                card,
                2.0,
                egui::Stroke::new(1.5, th::ACCENT_STRONG),
                egui::StrokeKind::Inside,
            );
        }
        let img_rect = egui::Rect::from_min_size(card.min, egui::vec2(card_w, img_h));
        ui.scope_builder(egui::UiBuilder::new().max_rect(img_rect), |ui| {
            image_box(ui, thumb(app, installed, d, 256), img_rect.size())
        });
        // ribbons on the capsule
        let p = ui.painter();
        let ribbon = |text: &str, bg: Color32, fg: Color32, y: f32| {
            let g = p.layout_no_wrap(text.to_string(), th::display(10.5), fg);
            let r = egui::Rect::from_min_size(
                egui::pos2(img_rect.left(), img_rect.top() + y),
                g.size() + egui::vec2(12.0, 5.0),
            );
            p.rect_filled(
                r,
                egui::CornerRadius {
                    nw: 0,
                    ne: 2,
                    se: 2,
                    sw: 0,
                },
                bg,
            );
            p.galley(r.center() - g.size() / 2.0, g, fg);
        };
        if let Some(a) = installed {
            if app.update_state(a) == Upd::Available {
                ribbon("UPDATE AVAILABLE", th::ACCENT_STRONG, TEXT_STRONG, 8.0);
            } else {
                ribbon(
                    "IN LIBRARY",
                    Color32::from_rgb(0x5c, 0x7e, 0x10),
                    th::GREEN_TXT,
                    8.0,
                );
            }
        }
        let body = egui::Rect::from_min_max(
            egui::pos2(card.left() + 8.0, img_rect.bottom() + 6.0),
            card.max - egui::vec2(8.0, 6.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let title = d
                    .map(|d| d.title.clone())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| format!("#{id}"));
                let mut job = LayoutJob::simple(title, th::bold(13.0), TEXT_STRONG, body.width());
                job.wrap.max_rows = 2;
                ui.add(egui::Label::new(job).selectable(false));
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        if let Some(d) = d {
                            ui.label(
                                RichText::new(format!("👥 {}", count(d.subscriptions)))
                                    .size(11.5)
                                    .color(MUTED),
                            );
                            ui.label(RichText::new(size(d.file_size)).size(11.5).color(DIM));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let dl = app.downloads.iter().find(|x| x.id == id).map(|x| &x.state);
                            if matches!(
                                dl,
                                Some(DlState::Waiting | DlState::Queued | DlState::Running)
                            ) {
                                ui.spinner();
                            } else if let Some(a) = installed {
                                if app.update_state(a) == Upd::Available {
                                    if ui.add(th::Button::new("Update", Kind::Blue)).clicked() {
                                        acts.push(Act::Update(id));
                                    }
                                } else {
                                    ui.label(RichText::new("✔").color(GREEN)).on_hover_text(
                                        if a.enabled {
                                            "Installed (enabled)"
                                        } else {
                                            "Installed (disabled)"
                                        },
                                    );
                                }
                            } else if app.ws.query.collections {
                                if ui
                                    .add(th::Button::new("Install all", Kind::Green))
                                    .on_hover_text("Install every item in this collection")
                                    .clicked()
                                {
                                    acts.push(Act::InstallCollection(id));
                                }
                            } else if ui.add(th::Button::new("Install", Kind::Green)).clicked() {
                                acts.push(Act::Install(id));
                            }
                        });
                    });
                });
            },
        );
    });
    if resp.clicked() {
        acts.push(Act::Focus(Focus::Workshop(id)));
    }
    if resp.double_clicked() && installed.is_none() && !app.ws.query.collections {
        acts.push(Act::Install(id));
    }
    resp.context_menu(|ui| {
        if ui.button("Open on Workshop").clicked() {
            ui.ctx()
                .open_url(egui::OpenUrl::new_tab(steam::item_url(id)));
            ui.close();
        }
        if ui.button("Copy link").clicked() {
            ui.ctx().copy_text(steam::item_url(id));
            ui.close();
        }
        if ui.button("Find items that require this").clicked() {
            acts.push(Act::UsedBy(id));
            ui.close();
        }
    });
}

// ---------------------------------------------------------------- conflicts tab

fn conflicts_tab(app: &mut App, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    egui::CentralPanel::default().frame(central_frame()).show(ui, |ui| {
        paint_backdrop(ui);
        ui.horizontal(|ui| {
            th::search_box(ui, &mut app.conflict_search, "Filter by add-on or file…", 300.0, "conf_search");
            ui.add_space(8.0);
            ui.checkbox(&mut app.conflicts_active_only, "Only between enabled add-ons");
            ui.checkbox(&mut app.conflicts_stock_only, "Only ones replacing game files 🎮").on_hover_text("Two skins/sounds/scripts replacing the same stock file — the usual cause of 'my mod doesn't show up'. Conflicts between custom files (often campaigns sharing assets) are hidden.");
        });
        ui.add_space(2.0);
        ui.label(RichText::new("Two add-ons shipping the same file path with different contents fight over it; only one copy is used. Identical copies and merged scripts (*_addon.nut, addoninfo) are ignored. 🎮 = replaces a stock game file.").size(12.0).color(DIM));
        ui.add_space(4.0);
        let app_ref: &App = app;
        let q = app_ref.conflict_search.to_lowercase();
        let list: Vec<&library::Conflict> = app_ref
            .lib
            .conflicts
            .iter()
            .filter(|c| !c.files.is_empty())
            .filter(|c| !app_ref.conflicts_active_only || app_ref.lib.conflict_active(c))
            .filter(|c| !app_ref.conflicts_stock_only || c.stock > 0)
            .filter(|c| {
                q.is_empty()
                    || app_ref.display_title(&app_ref.lib.addons[c.a]).to_lowercase().contains(&q)
                    || app_ref.display_title(&app_ref.lib.addons[c.b]).to_lowercase().contains(&q)
                    || c.files.iter().any(|f| f.contains(&q))
            })
            .collect();
        ui.horizontal(|ui| {
            ui.label(caps(&format!("{} conflicting pair(s)", list.len()), 15.0, TEXT_STRONG));
        });
        th::rule(ui, ui.cursor().top() + 2.0);
        ui.add_space(8.0);
        if list.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(RichText::new("✔").size(44.0).color(GREEN));
                ui.label(caps("No conflicts", 16.0, GREEN));
            });
        }
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for c in list {
                let (a, b) = (&app_ref.lib.addons[c.a], &app_ref.lib.addons[c.b]);
                let active = app_ref.lib.conflict_active(c);
                ui.push_id((c.a, c.b), |ui| {
                let r = egui::Frame::new().fill(Color32::from_rgb(0x1f, 0x2d, 0x3d)).corner_radius(2).inner_margin(egui::Margin { left: 14, right: 10, top: 8, bottom: 6 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        for (x, sep) in [(a, true), (b, false)] {
                            if th::toggle(ui, x.enabled).on_hover_text(if x.enabled { "Enabled — click to disable" } else { "Disabled — click to enable" }).clicked() {
                                acts.push(Act::SetEnabled(vec![x.key.clone()], !x.enabled));
                            }
                            let t = RichText::new(app_ref.display_title(x)).font(th::bold(13.5)).color(if x.enabled { TEXT_STRONG } else { MUTED });
                            if ui.link(t).clicked() {
                                acts.push(Act::Click { key: x.key.clone(), ctrl: false, shift: false });
                                acts.push(Act::Tab(Tab::Library));
                            }
                            if sep {
                                ui.label(RichText::new("  VS  ").font(th::display(12.0)).color(if active { RED } else { DIM }));
                            }
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if c.stock > 0 {
                                badge(ui, &format!("🎮 {} GAME FILES", c.stock), ORANGE, "Replaces stock game files");
                            }
                        });
                    });
                    conflict_files(ui, c, &app_ref.lib.stock);
                });
                let rr = r.response.rect;
                ui.painter().rect_filled(egui::Rect::from_min_size(rr.min, egui::vec2(3.0, rr.height())), 0.0, if active { RED } else { DIM });
                });
            }
        });
    });
}

fn file_line(ui: &mut egui::Ui, f: &str, stock: &HashSet<Box<str>>) {
    let is_stock = stock.contains(f);
    let t = RichText::new(if is_stock {
        format!("🎮 {f}")
    } else {
        f.to_string()
    })
    .monospace()
    .size(11.5);
    let r = ui.add(
        egui::Label::new(if is_stock {
            t.color(ORANGE)
        } else {
            t.color(MUTED)
        })
        .truncate(),
    );
    if is_stock {
        r.on_hover_text("Replaces a stock game file");
    }
}

fn conflict_files(ui: &mut egui::Ui, c: &library::Conflict, stock: &HashSet<Box<str>>) {
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in &c.files {
        groups.entry(library::path_kind(f)).or_default().push(f);
    }
    let summary = groups
        .iter()
        .map(|(k, v)| format!("{k} {}", v.len()))
        .collect::<Vec<_>>()
        .join(" • ");
    let stock_note = if c.stock > 0 {
        format!(" — {} replace game files", c.stock)
    } else {
        String::new()
    };
    egui::CollapsingHeader::new(
        RichText::new(format!(
            "{} differing file(s){stock_note}: {summary}{}",
            c.files.len(),
            if c.identical > 0 {
                format!("  (+{} identical)", c.identical)
            } else {
                String::new()
            }
        ))
        .size(12.0)
        .color(MUTED),
    )
    .id_salt(("conf", c.a, c.b))
    .show(ui, |ui| {
        for (k, files) in groups {
            ui.label(caps(k, 10.5, TEXT));
            for f in files.iter().take(300) {
                file_line(ui, f, stock);
            }
            if files.len() > 300 {
                ui.weak(format!("…and {} more", files.len() - 300));
            }
        }
    });
}

// ---------------------------------------------------------------- downloads tab

fn downloads_tab(app: &mut App, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    egui::CentralPanel::default()
        .frame(central_frame())
        .show(ui, |ui| {
            paint_backdrop(ui);
            let updates = app
                .lib
                .addons
                .iter()
                .filter(|a| app.update_state(a) == Upd::Available)
                .count();
            let running = app
                .downloads
                .iter()
                .filter(|d| matches!(d.state, DlState::Running))
                .count();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(caps(
                        if running > 0 {
                            "Downloading"
                        } else {
                            "Downloads"
                        },
                        22.0,
                        TEXT_STRONG,
                    ));
                    ui.label(
                        RichText::new(format!(
                            "New installs go to {}. Updates replace the file in place.",
                            if app.cfg.install_to_workshop_dir {
                                "addons/workshop/"
                            } else {
                                "addons/"
                            }
                        ))
                        .size(12.0)
                        .color(DIM),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(th::Button::new("Clear finished", Kind::Dark))
                        .clicked()
                    {
                        acts.push(Act::ClearFinished);
                    }
                    if ui
                        .add_enabled(
                            updates > 0,
                            th::Button::new(&format!("⬆  Update all  {updates}"), Kind::Blue).big(),
                        )
                        .clicked()
                    {
                        app.update_all();
                    }
                    if ui
                        .add(th::Button::new("⟳  Check for updates", Kind::Dark))
                        .clicked()
                    {
                        app.check_updates();
                    }
                });
            });
            ui.add_space(6.0);
            th::rule(ui, ui.cursor().top());
            ui.add_space(8.0);
            if app.downloads.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(RichText::new("⬇").size(44.0).color(th::SURFACE_HI));
                    ui.label(caps("Nothing downloading", 16.0, MUTED));
                    ui.label(
                        RichText::new(
                            "Install items from the Workshop tab, or paste a link there.",
                        )
                        .color(DIM),
                    );
                });
            }
            let app_ref: &App = app;
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for d in app_ref.downloads.iter().rev() {
                        let live = matches!(d.state, DlState::Running);
                        egui::Frame::new()
                            .fill(if live {
                                Color32::from_rgb(0x23, 0x3c, 0x55)
                            } else {
                                Color32::from_rgb(0x1f, 0x2d, 0x3d)
                            })
                            .corner_radius(2)
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    image_box(
                                        ui,
                                        thumb(app_ref, None, app_ref.cache.details.get(&d.id), 128),
                                        egui::vec2(96.0, 54.0),
                                    );
                                    ui.add_space(4.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(app_ref.title_of(d.id))
                                                .font(th::bold(14.0))
                                                .color(TEXT_STRONG),
                                        );
                                        let done = d.prog.done.load(Ordering::Relaxed);
                                        let total = d.prog.total.load(Ordering::Relaxed).max(
                                            app_ref
                                                .cache
                                                .details
                                                .get(&d.id)
                                                .map(|x| x.file_size)
                                                .unwrap_or(0),
                                        );
                                        match &d.state {
                                            DlState::Waiting => {
                                                ui.label(caps("Getting file info…", 11.0, MUTED));
                                            }
                                            DlState::Queued => {
                                                ui.label(caps("Queued", 11.0, MUTED));
                                            }
                                            DlState::Running => {
                                                let secs = d
                                                    .started
                                                    .map(|s| s.elapsed().as_secs_f64())
                                                    .unwrap_or(1.0)
                                                    .max(0.1);
                                                let speed = done as f64 / secs;
                                                let frac = if total > 0 {
                                                    done as f32 / total as f32
                                                } else {
                                                    0.0
                                                };
                                                th::progress(
                                                    ui,
                                                    frac,
                                                    (ui.available_width() - 120.0)
                                                        .clamp(160.0, 520.0),
                                                    &format!(
                                                        "{} / {}   ·   {}/s",
                                                        size(done),
                                                        size(total),
                                                        size(speed as u64)
                                                    ),
                                                );
                                            }
                                            DlState::Done => {
                                                ui.label(caps(
                                                    &format!(
                                                        "{}  ·  {}",
                                                        if d.is_update {
                                                            "Updated"
                                                        } else {
                                                            "Installed"
                                                        },
                                                        size(done)
                                                    ),
                                                    11.0,
                                                    GREEN,
                                                ));
                                            }
                                            DlState::Failed(e) => {
                                                ui.label(RichText::new(e).color(RED));
                                            }
                                            DlState::Cancelled => {
                                                ui.label(caps("Cancelled", 11.0, DIM));
                                            }
                                        }
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        match &d.state {
                                            DlState::Waiting
                                            | DlState::Queued
                                            | DlState::Running => {
                                                if ui
                                                    .add(th::Button::new("Cancel", Kind::Dark))
                                                    .clicked()
                                                {
                                                    acts.push(Act::CancelDl(d.id));
                                                }
                                            }
                                            DlState::Failed(_) | DlState::Cancelled => {
                                                if ui
                                                    .add(th::Button::new(
                                                        "Workshop page ↗",
                                                        Kind::Dark,
                                                    ))
                                                    .clicked()
                                                {
                                                    ui.ctx().open_url(egui::OpenUrl::new_tab(
                                                        steam::item_url(d.id),
                                                    ));
                                                }
                                                if ui
                                                    .add(th::Button::new("Retry", Kind::Blue))
                                                    .clicked()
                                                {
                                                    acts.push(if d.is_update {
                                                        Act::Update(d.id)
                                                    } else {
                                                        Act::Install(d.id)
                                                    });
                                                }
                                            }
                                            DlState::Done => {
                                                ui.label(
                                                    RichText::new("✔").size(18.0).color(GREEN),
                                                );
                                            }
                                        }
                                    });
                                });
                            });
                    }
                });
        });
}

// ---------------------------------------------------------------- settings tab

fn settings_tab(app: &mut App, ui: &mut egui::Ui) {
    egui::CentralPanel::default().frame(central_frame()).show(ui, |ui| {
        paint_backdrop(ui);
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.set_max_width(760.0);
            let before = serde_json::to_string(&app.cfg).unwrap_or_default();
            ui.label(caps("Settings", 22.0, TEXT_STRONG));
            settings_card(ui, "Game", |ui| {
                ui.horizontal(|ui| {
                    ui.label(caps("Folder", 10.5, DIM));
                    ui.label(RichText::new(app.game_dir.as_ref().map(|p| p.display().to_string()).unwrap_or("not found".into())).monospace().color(TEXT));
                });
                ui.horizontal(|ui| {
                    if ui.add(th::Button::new("Browse…", Kind::Dark)).clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            app.set_game_dir(p);
                        }
                    }
                    if ui.add(th::Button::new("Auto-detect", Kind::Dark)).clicked() {
                        match crate::config::detect_game_dir() {
                            Some(p) => app.set_game_dir(p),
                            None => app.toast_err("Couldn't find Left 4 Dead 2 in any Steam library"),
                        }
                    }
                    if let Some(d) = app.addons_dir() {
                        if ui.add(th::Button::new("Open addons folder", Kind::Dark)).clicked() {
                            open(&d.display().to_string());
                        }
                    }
                });
            });
            settings_card(ui, "Installing", |ui| {
            ui.radio_value(&mut app.cfg.install_to_workshop_dir, false, "Install into addons/  (you manage it; no Steam subscription needed)");
            ui.radio_value(&mut app.cfg.install_to_workshop_dir, true, "Install into addons/workshop/");
            ui.checkbox(&mut app.cfg.enable_after_install, "Enable add-ons after installing");
            ui.checkbox(&mut app.cfg.install_dependencies, "Also install required items");
            ui.checkbox(&mut app.cfg.auto_enable_deps, "Enabling an add-on also enables its installed requirements");
            ui.add(egui::Slider::new(&mut app.cfg.max_downloads, 1..=8).text("parallel downloads"));
            });
            settings_card(ui, "Workshop", |ui| {
            ui.checkbox(&mut app.cfg.check_updates_on_start, "Check for updates on start (re-fetches details older than 30 min)");
            ui.checkbox(&mut app.cfg.scan_dependencies, "Read requirements of installed add-ons in the background (one Workshop page every 3 s; cached 3 days)");
            ui.checkbox(&mut app.cfg.search_descriptions, "Search also matches descriptions");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(caps("Steam Web API key", 10.5, DIM));
                ui.label(RichText::new("optional").size(11.0).color(DIM));
            });
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut app.cfg.steam_api_key).password(true).desired_width(300.0).margin(egui::Margin::symmetric(8, 4)));
                if r.lost_focus() && !app.cfg.steam_api_key.trim().is_empty() {
                    let ids: Vec<u64> = app.lib.addons.iter().filter_map(|a| a.id).collect();
                    app.request_children(ids);
                }
                ui.hyperlink_to("get one ↗", "https://steamcommunity.com/dev/apikey");
                if !app.cfg.steam_api_key.trim().is_empty() {
                    badge(ui, "✔ KEY SET", GREEN, "");
                }
            });
            ui.label(RichText::new("With a key, requirements for all add-ons come from 3 API calls instead of reading every Workshop page (no rate limits). The key stays in your local settings file.").size(12.0).color(DIM));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add(th::Button::new("Clear Workshop cache", Kind::Dark)).clicked() {
                    app.cache = Default::default();
                    app.parsed.borrow_mut().clear();
                    app.save_all();
                    app.rescan();
                }
                ui.label(RichText::new(format!("{} details, {} pages cached", app.cache.details.len(), app.cache.pages.len())).size(12.0).color(DIM));
            });
            });
            settings_card(ui, "Appearance", |ui| {
            ui.add(egui::Slider::new(&mut app.cfg.ui_scale, 0.75..=1.75).text("UI scale"));
            ui.checkbox(&mut app.cfg.cjk_fonts, "Chinese / Japanese / Korean text support (loaded only when such text appears; ~30 MB RAM)");
            });
            settings_card(ui, "Data", |ui| {
            ui.horizontal(|ui| {
                if ui.add(th::Button::new("Open data folder", Kind::Dark)).clicked() {
                    open(&crate::config::data_dir().display().to_string());
                }
            });
            ui.label(RichText::new("Settings, profiles, notes and cache. addonlist.txt is backed up to addonlist.txt.addonmgr.bak before the first change each session.").size(12.0).color(DIM));
            });
            if serde_json::to_string(&app.cfg).unwrap_or_default() != before {
                app.save_config();
            }
        });
    });
}

fn settings_card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    th::section(ui, title);
    egui::Frame::new()
        .fill(Color32::from_rgb(0x1f, 0x2d, 0x3d))
        .corner_radius(2)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
}
