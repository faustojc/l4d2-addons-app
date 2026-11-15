//! Steam-client look: palette, fonts, egui visuals and a few hand-painted widgets.

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Response, Sense, Shadow, Stroke, Ui,
    Vec2, Widget,
};
use std::path::PathBuf;
use std::sync::Arc;

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

// Steam store / client palette.
pub const BG_DEEP: Color32 = rgb(0x0e, 0x14, 0x1b); // title nav, footer
pub const BG_NAV: Color32 = rgb(0x17, 0x1a, 0x21);
pub const BG: Color32 = rgb(0x1b, 0x28, 0x38); // store body
pub const BG_LOW: Color32 = rgb(0x13, 0x1c, 0x27);
pub const PANEL: Color32 = rgb(0x16, 0x20, 0x2d); // side columns
pub const SURFACE: Color32 = rgb(0x20, 0x2e, 0x3e);
pub const SURFACE_HI: Color32 = rgb(0x2a, 0x47, 0x5e); // rgba(103,193,245,.2) on BG
pub const MENU: Color32 = rgb(0x3d, 0x44, 0x50); // context menus / tooltips
pub const INPUT: Color32 = rgb(0x0f, 0x17, 0x21);
pub const ACCENT: Color32 = rgb(0x66, 0xc0, 0xf4);
pub const ACCENT_STRONG: Color32 = rgb(0x1a, 0x9f, 0xff);
pub const TEXT: Color32 = rgb(0xc7, 0xd5, 0xe0);
pub const TEXT_STRONG: Color32 = Color32::WHITE;
pub const MUTED: Color32 = rgb(0x8f, 0x98, 0xa0);
pub const DIM: Color32 = rgb(0x56, 0x70, 0x7f);
pub const GREEN: Color32 = rgb(0xa4, 0xd0, 0x07);
pub const GREEN_TXT: Color32 = rgb(0xd2, 0xef, 0xa9);
pub const RED: Color32 = rgb(0xe3, 0x5b, 0x4f);
pub const ORANGE: Color32 = rgb(0xe5, 0xa0, 0x3a);
pub const PURPLE: Color32 = rgb(0xb0, 0x93, 0xf0);
pub const GRAY: Color32 = MUTED;
pub const BLUE: Color32 = ACCENT;

pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display".into()))
}
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

// ---------------------------------------------------------------- fonts

fn fonts_dir() -> PathBuf {
    std::env::var("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
        .join("Fonts")
}

fn families() -> [FontFamily; 4] {
    [
        FontFamily::Proportional,
        FontFamily::Monospace,
        FontFamily::Name("bold".into()),
        FontFamily::Name("display".into()),
    ]
}

/// Segoe UI body, Segoe UI Bold for emphasis, Bahnschrift (DIN) for Steam-style uppercase headings.
/// egui's bundled fonts stay as fallbacks for emoji/icons.
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let defaults_prop = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut load = |name: &str, file: &str| -> bool {
        match std::fs::read(fonts_dir().join(file)) {
            Ok(b) => {
                fonts
                    .font_data
                    .insert(name.into(), Arc::new(egui::FontData::from_owned(b)));
                true
            }
            Err(_) => false,
        }
    };
    let body = load("segoeui", "segoeui.ttf");
    let strong = load("segoeuib", "segoeuib.ttf");
    let disp = load("bahnschrift", "bahnschrift.ttf");
    let sym = load("seguisym", "seguisym.ttf");
    let chain = |first: &[(&str, bool)]| -> Vec<String> {
        let mut v: Vec<String> = first
            .iter()
            .filter(|(_, ok)| *ok)
            .map(|(n, _)| n.to_string())
            .collect();
        if sym {
            v.push("seguisym".into());
        }
        v.extend(defaults_prop.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, chain(&[("segoeui", body)]));
    fonts.families.insert(
        FontFamily::Name("bold".into()),
        chain(&[("segoeuib", strong), ("segoeui", body)]),
    );
    fonts.families.insert(
        FontFamily::Name("display".into()),
        chain(&[("bahnschrift", disp), ("segoeui", body)]),
    );
    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    for (n, ok) in [("segoeui", body), ("seguisym", sym)] {
        if ok {
            mono.push(n.into());
        }
    }
    ctx.set_fonts(fonts);
}

/// The CJK fonts cost ~30-40 MB, so they're only loaded once CJK text actually shows up.
pub fn load_cjk_fonts(ctx: &egui::Context) {
    for (name, file) in [
        ("msyh", "msyh.ttc"),
        ("malgun", "malgun.ttf"),
        ("yugoth", "YuGothM.ttc"),
    ] {
        if let Ok(bytes) = std::fs::read(fonts_dir().join(file)) {
            let fams = families()
                .into_iter()
                .map(|family| InsertFontFamily {
                    family,
                    priority: FontPriority::Lowest,
                })
                .collect();
            ctx.add_font(FontInsert::new(
                name,
                egui::FontData::from_owned(bytes),
                fams,
            ));
        }
    }
}

// ---------------------------------------------------------------- visuals

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles.insert(Body, FontId::proportional(13.5));
        s.text_styles.insert(Button, FontId::proportional(13.5));
        s.text_styles.insert(Small, FontId::proportional(11.0));
        s.text_styles.insert(Heading, display(20.0));
        s.text_styles.insert(Monospace, FontId::monospace(12.0));
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 24.0;
        // Always-visible scrollbar: ACCENT handle (white on hover) on the INPUT track = 9:1 contrast (WCAG AAA ≥ 7:1).
        s.spacing.scroll = egui::style::ScrollStyle::solid();
        s.spacing.scroll.bar_width = 10.0;
        s.spacing.scroll.bar_inner_margin = 3.0;
        s.spacing.scroll.handle_min_length = 24.0;
        s.spacing.scroll.foreground_color = true;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.interaction.selectable_labels = false;

        let v = &mut s.visuals;
        v.dark_mode = true;
        v.override_text_color = None;
        v.weak_text_color = Some(MUTED);
        v.hyperlink_color = ACCENT;
        v.panel_fill = BG;
        v.window_fill = MENU;
        v.window_stroke = Stroke::new(1.0, Color32::from_black_alpha(80));
        v.window_corner_radius = CornerRadius::same(4);
        v.menu_corner_radius = CornerRadius::same(3);
        v.window_shadow = Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(150),
        };
        v.popup_shadow = Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(140),
        };
        v.faint_bg_color = SURFACE;
        v.extreme_bg_color = INPUT;
        v.text_edit_bg_color = Some(INPUT);
        v.code_bg_color = INPUT;
        v.warn_fg_color = ORANGE;
        v.error_fg_color = RED;
        v.striped = false;
        v.slider_trailing_fill = true;
        v.indent_has_left_vline = false;
        v.collapsing_header_frame = false;
        v.selection.bg_fill = rgb(0x2f, 0x6d, 0xa0);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.text_cursor.stroke = Stroke::new(2.0, ACCENT);

        let r = CornerRadius::same(2);
        let w = &mut v.widgets;
        // Labels and frames.
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, rgb(0x24, 0x33, 0x45));
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;
        // Buttons: Steam's "btnv6_blue_hoverfade" — translucent blue with light-blue text.
        w.inactive.weak_bg_fill = SURFACE_HI;
        w.inactive.bg_fill = INPUT; // checkbox/radio/slider wells
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.5, ACCENT);
        w.inactive.corner_radius = r;
        w.hovered.weak_bg_fill = rgb(0x4f, 0x94, 0xbc);
        w.hovered.bg_fill = rgb(0x1c, 0x2c, 0x3c);
        w.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
        w.hovered.fg_stroke = Stroke::new(1.5, TEXT_STRONG);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;
        w.active.weak_bg_fill = ACCENT;
        w.active.bg_fill = rgb(0x22, 0x36, 0x4a);
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(1.5, TEXT_STRONG);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;
        w.open = w.hovered;
    });
}

// ---------------------------------------------------------------- painting helpers

/// Vertical gradient fill.
pub fn vgradient(p: &egui::Painter, r: Rect, top: Color32, bottom: Color32) {
    gradient(p, r, [top, top, bottom, bottom]);
}

/// Horizontal gradient fill.
pub fn hgradient(p: &egui::Painter, r: Rect, left: Color32, right: Color32) {
    gradient(p, r, [left, right, right, left]);
}

/// Corner colors: top-left, top-right, bottom-right, bottom-left.
pub fn gradient(p: &egui::Painter, r: Rect, c: [Color32; 4]) {
    let mut m = egui::Mesh::default();
    for (pos, col) in [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ]
    .into_iter()
    .zip(c)
    {
        m.colored_vertex(pos, col);
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(m);
}

/// Uppercase, letter-spaced caption (Steam's "ABOUT THIS GAME" style).
pub fn caps(text: &str, size: f32, color: Color32) -> egui::RichText {
    egui::RichText::new(text.to_uppercase())
        .font(display(size))
        .extra_letter_spacing(size * 0.09)
        .color(color)
}

/// Section caption with a fading accent rule under it.
pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    let r = ui.add(egui::Label::new(caps(title, 12.5, TEXT)).selectable(false));
    rule(ui, r.rect.left_bottom().y + 3.0);
    ui.add_space(4.0);
}

/// Thin gradient line across the available width at `y`.
pub fn rule(ui: &Ui, y: f32) {
    let x0 = ui.min_rect().left().min(ui.max_rect().left());
    let w = ui.available_width().max(ui.min_rect().width());
    hgradient(
        ui.painter(),
        Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 1.0)),
        ACCENT.gamma_multiply(0.55),
        Color32::TRANSPARENT,
    );
}

/// Small rounded pill with tinted background.
pub fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), bold(10.5), fg);
    let size = galley.size() + Vec2::new(10.0, 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, 2.0, bg);
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
    resp
}

pub fn badge(ui: &mut Ui, text: &str, color: Color32, tip: &str) {
    let r = pill(ui, text, color, color.gamma_multiply(0.17));
    if !tip.is_empty() {
        r.on_hover_text(tip);
    }
}

/// Steam store tag chip: blue tint, solid accent when selected.
pub fn chip(ui: &mut Ui, text: &str, on: bool) -> Response {
    let fg0 = if on { TEXT_STRONG } else { ACCENT };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(12.0), fg0);
    let size = galley.size() + Vec2::new(14.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let bg = if on {
        ACCENT_STRONG.gamma_multiply(0.85)
    } else {
        SURFACE_HI.lerp_to_gamma(rgb(0x4f, 0x94, 0xbc), t)
    };
    let fg = if on {
        TEXT_STRONG
    } else {
        ACCENT.lerp_to_gamma(TEXT_STRONG, t)
    };
    ui.painter().rect_filled(rect, 2.0, bg);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Animated toggle switch (Steam settings style).
pub fn toggle(ui: &mut Ui, on: bool) -> Response {
    let size = Vec2::new(32.0, 17.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id, on, 0.14);
        let hov = resp.hovered();
        let track = rgb(0x3a, 0x45, 0x55).lerp_to_gamma(ACCENT_STRONG, t);
        let track = if hov {
            track.lerp_to_gamma(Color32::WHITE, 0.12)
        } else {
            track
        };
        ui.painter().rect_filled(rect, 8.5, track);
        let x = egui::lerp(rect.left() + 8.5..=rect.right() - 8.5, t);
        ui.painter().circle_filled(
            Pos2::new(x, rect.center().y),
            6.0,
            rgb(0xdc, 0xde, 0xdf).lerp_to_gamma(Color32::WHITE, t),
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// "Add to cart" / Play green.
    Green,
    /// Store blue gradient (primary).
    Blue,
    /// Translucent blue (secondary).
    Dark,
    /// Destructive.
    Danger,
}

/// Gradient button in Steam's style. Use with `ui.add` / `ui.add_enabled`.
pub struct Button<'a> {
    text: &'a str,
    kind: Kind,
    big: bool,
    min_w: f32,
}

impl<'a> Button<'a> {
    pub fn new(text: &'a str, kind: Kind) -> Self {
        Self {
            text,
            kind,
            big: false,
            min_w: 0.0,
        }
    }
    pub fn big(mut self) -> Self {
        self.big = true;
        self
    }
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_w = w;
        self
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let font = if self.big {
            display(16.0)
        } else {
            FontId::proportional(13.0)
        };
        let text = if self.big {
            self.text.to_uppercase()
        } else {
            self.text.to_string()
        };
        let mut job = egui::text::LayoutJob::simple_singleline(text, font, Color32::WHITE);
        if self.big {
            job.sections
                .iter_mut()
                .for_each(|s| s.format.extra_letter_spacing = 1.2);
        }
        let galley = ui.painter().layout_job(job);
        let pad = if self.big {
            Vec2::new(22.0, 9.0)
        } else {
            Vec2::new(12.0, 4.0)
        };
        let size = Vec2::new(
            (galley.size().x + pad.x * 2.0).max(self.min_w),
            (galley.size().y + pad.y * 2.0).max(24.0),
        );
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        if ui.is_rect_visible(rect) {
            let enabled = ui.is_enabled();
            let t = if enabled {
                ui.ctx()
                    .animate_bool_with_time(resp.id, resp.hovered(), 0.12)
            } else {
                0.0
            };
            let pressed = resp.is_pointer_button_down_on();
            let (a, b, fg_rest, fg_hot) = match self.kind {
                Kind::Green => (
                    (rgb(0x75, 0xb0, 0x22), rgb(0x58, 0x8a, 0x1b)),
                    (rgb(0x8e, 0xd6, 0x29), rgb(0x6a, 0xa6, 0x21)),
                    GREEN_TXT,
                    TEXT_STRONG,
                ),
                Kind::Blue => (
                    (rgb(0x47, 0xbf, 0xff), rgb(0x1a, 0x44, 0xc2)),
                    (rgb(0x66, 0xcc, 0xff), rgb(0x2d, 0x73, 0xff)),
                    TEXT_STRONG,
                    TEXT_STRONG,
                ),
                Kind::Dark => (
                    (SURFACE_HI, SURFACE_HI),
                    (rgb(0x66, 0xc0, 0xf4), rgb(0x41, 0x7a, 0x9b)),
                    ACCENT,
                    TEXT_STRONG,
                ),
                Kind::Danger => (
                    (rgb(0x4a, 0x24, 0x28), rgb(0x3a, 0x1d, 0x22)),
                    (rgb(0xc8, 0x45, 0x3a), rgb(0x9c, 0x2f, 0x28)),
                    rgb(0xf0, 0x9a, 0x90),
                    TEXT_STRONG,
                ),
            };
            let l = a.0.lerp_to_gamma(b.0, t);
            let rr = a.1.lerp_to_gamma(b.1, t);
            let (l, rr) = if pressed {
                (l.gamma_multiply(0.85), rr.gamma_multiply(0.85))
            } else {
                (l, rr)
            };
            let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            // Rounded gradient: mask the corners by painting the gradient inset, then rounded caps.
            p.rect_filled(rect, 2.0, l.lerp_to_gamma(rr, 0.5));
            hgradient(&p, rect.shrink2(Vec2::new(2.0, 0.0)), l, rr);
            if self.big {
                // subtle top sheen
                vgradient(
                    &p,
                    Rect::from_min_size(rect.min, Vec2::new(rect.width(), rect.height() * 0.5)),
                    Color32::from_white_alpha(18),
                    Color32::TRANSPARENT,
                );
            }
            let mut fg = fg_rest.lerp_to_gamma(fg_hot, t);
            if !enabled {
                fg = fg.gamma_multiply(0.45);
            }
            p.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, fg);
            if !enabled {
                p.rect_filled(rect, 2.0, Color32::from_black_alpha(90));
            }
        }
        if ui.is_enabled() {
            resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            resp
        }
    }
}

/// Steam-like search field: dark well, magnifier, accent border on focus.
pub fn search_box(ui: &mut Ui, text: &mut String, hint: &str, width: f32, id: &str) -> Response {
    let id = egui::Id::unique(id);
    let focused = ui.memory(|m| m.has_focus(id));
    let frame = egui::Frame::new()
        .fill(INPUT)
        .stroke(Stroke::new(
            1.0,
            if focused {
                ACCENT
            } else {
                rgb(0x2a, 0x3d, 0x52)
            },
        ))
        .corner_radius(3)
        .inner_margin(egui::Margin {
            left: 8,
            right: 6,
            top: 4,
            bottom: 4,
        });
    frame
        .show(ui, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("🔍").color(if focused { ACCENT } else { DIM }));
                let r = ui.add(
                    egui::TextEdit::singleline(text)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .hint_text(egui::RichText::new(hint).color(DIM))
                        .desired_width(ui.available_width() - 22.0),
                );
                if !text.is_empty()
                    && ui
                        .add(
                            egui::Label::new(egui::RichText::new("✖").color(MUTED))
                                .sense(Sense::click()),
                        )
                        .on_hover_text("Clear")
                        .clicked()
                {
                    text.clear();
                }
                r
            })
            .inner
        })
        .inner
}

/// Progress bar: dark track, Steam-blue gradient fill, centered caption.
pub fn progress(ui: &mut Ui, frac: f32, width: f32, caption: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 16.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, INPUT);
    let f = frac.clamp(0.0, 1.0);
    if f > 0.0 {
        let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * f, rect.height()));
        hgradient(p, fill, rgb(0x1a, 0x44, 0xc2), ACCENT_STRONG);
        // moving sheen
        let t = ui.input(|i| i.time) as f32;
        let x = fill.left() + (t * 120.0) % (fill.width() + 60.0) - 30.0;
        let sheen = Rect::from_x_y_ranges(
            x.max(fill.left())..=(x + 30.0).min(fill.right()),
            fill.y_range(),
        );
        if sheen.width() > 0.0 {
            hgradient(
                p,
                sheen,
                Color32::TRANSPARENT,
                Color32::from_white_alpha(40),
            );
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        caption,
        FontId::proportional(11.0),
        TEXT_STRONG,
    );
}

/// Frame for side panels.
pub fn panel_frame(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(10, 8))
}
