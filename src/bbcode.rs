//! Steam Workshop BBCode -> blocks -> egui. Lenient: unknown `[tags]` stay literal text
//! (so titles like "[VScripts] Library" survive), mismatched closers are ignored.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Stroke};
use std::sync::Arc;

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    link: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    style: Style,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para(Vec<Span>),
    Heading(u8, Vec<Span>),
    Image(String),
    Quote(Option<String>, Vec<Block>),
    Code(String),
    List(bool, Vec<Vec<Block>>),
    Hr,
    Table(Vec<Vec<Vec<Block>>>),
    Spoiler(Vec<Block>),
    Video(String),
}

const KNOWN: &[&str] = &[
    "b",
    "i",
    "u",
    "s",
    "strike",
    "h1",
    "h2",
    "h3",
    "url",
    "img",
    "quote",
    "code",
    "noparse",
    "list",
    "olist",
    "*",
    "hr",
    "table",
    "tr",
    "th",
    "td",
    "spoiler",
    "previewyoutube",
];

#[derive(Debug)]
enum Tok {
    Text(String),
    Open(String, Option<String>),
    Close(String),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = src;
    while let Some(i) = rest.find('[') {
        text.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[1..end];
        let (closing, body) = match inner.strip_prefix('/') {
            Some(b) => (true, b),
            None => (false, inner),
        };
        let (name, arg) = match body.find('=') {
            Some(e) => (
                &body[..e],
                Some(
                    body[e + 1..]
                        .trim_matches(|c| c == '"' || c == '\'')
                        .to_string(),
                ),
            ),
            None => (body, None),
        };
        let name = name.trim().to_ascii_lowercase();
        if !KNOWN.contains(&name.as_str()) || (closing && arg.is_some()) {
            text.push('[');
            rest = &rest[1..];
            continue;
        }
        if !text.is_empty() {
            out.push(Tok::Text(std::mem::take(&mut text)));
        }
        rest = &rest[end + 1..];
        if closing {
            out.push(Tok::Close(name));
        } else if name == "noparse" {
            let lower = rest.to_ascii_lowercase();
            let stop = lower.find("[/noparse]").unwrap_or(rest.len());
            out.push(Tok::Text(rest[..stop].to_string()));
            rest = &rest[(stop + 10).min(rest.len())..];
        } else {
            out.push(Tok::Open(name, arg));
        }
    }
    text.push_str(rest);
    if !text.is_empty() {
        out.push(Tok::Text(text));
    }
    out
}

#[derive(Debug)]
enum Node {
    Text(String),
    Tag(String, Option<String>, Vec<Node>),
}

fn tree(toks: &[Tok], i: &mut usize, stack: &mut Vec<String>) -> Vec<Node> {
    let mut out = Vec::new();
    while *i < toks.len() {
        match &toks[*i] {
            Tok::Text(t) => {
                out.push(Node::Text(t.clone()));
                *i += 1;
            }
            Tok::Open(n, a) => {
                *i += 1;
                if n == "*" || n == "hr" {
                    out.push(Node::Tag(n.clone(), a.clone(), Vec::new()));
                } else {
                    stack.push(n.clone());
                    let kids = tree(toks, i, stack);
                    stack.pop();
                    out.push(Node::Tag(n.clone(), a.clone(), kids));
                }
            }
            Tok::Close(n) => {
                if stack.last() == Some(n) {
                    *i += 1;
                    return out;
                }
                if stack.contains(n) {
                    return out; // let the matching ancestor consume it
                }
                *i += 1; // stray closer
            }
        }
    }
    out
}

fn raw_text(nodes: &[Node], out: &mut String) {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Tag(_, _, k) => raw_text(k, out),
        }
    }
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    para: Vec<Span>,
}

impl Builder {
    fn push_text(&mut self, t: &str, st: &Style) {
        let t = if self.para.is_empty() {
            t.trim_start_matches(['\r', '\n'])
        } else {
            t
        };
        if t.is_empty() {
            return;
        }
        let t = t.replace("\r\n", "\n").replace('\r', "\n");
        match self.para.last_mut() {
            Some(last) if last.style == *st => last.text.push_str(&t),
            _ => self.para.push(Span {
                text: t,
                style: st.clone(),
            }),
        }
    }
    fn flush(&mut self) {
        while let Some(last) = self.para.last_mut() {
            let trimmed = last.text.trim_end().len();
            last.text.truncate(trimmed);
            if last.text.is_empty() {
                self.para.pop();
            } else {
                break;
            }
        }
        if !self.para.is_empty() {
            self.blocks
                .push(Block::Para(std::mem::take(&mut self.para)));
        }
    }
    fn block(&mut self, b: Block) {
        self.flush();
        self.blocks.push(b);
    }
    fn finish(mut self) -> Vec<Block> {
        self.flush();
        self.blocks
    }
}

fn autolink(text: &str, st: &Style, b: &mut Builder) {
    if st.link.is_some() {
        return b.push_text(text, st);
    }
    let next = |s: &str| {
        [s.find("http://"), s.find("https://")]
            .into_iter()
            .flatten()
            .min()
    };
    let mut rest = text;
    while let Some(i) = next(rest) {
        b.push_text(&rest[..i], st);
        let tail = &rest[i..];
        let mut end = tail
            .find(|c: char| c.is_whitespace() || "<>\"[]".contains(c))
            .unwrap_or(tail.len());
        while end > 0 && ".,)!?;:'".contains(tail.as_bytes()[end - 1] as char) {
            end -= 1;
        }
        let url = &tail[..end];
        b.push_text(
            url,
            &Style {
                link: Some(url.into()),
                underline: true,
                ..st.clone()
            },
        );
        rest = &tail[end..];
    }
    b.push_text(rest, st);
}

fn sub(nodes: &[Node], st: &Style) -> Vec<Block> {
    let mut b = Builder::default();
    emit(nodes, st, &mut b);
    b.finish()
}

fn emit(nodes: &[Node], st: &Style, b: &mut Builder) {
    for n in nodes {
        let Node::Tag(name, arg, kids) = n else {
            if let Node::Text(t) = n {
                autolink(t, st, b);
            }
            continue;
        };
        let with = |f: &dyn Fn(&mut Style)| {
            let mut s = st.clone();
            f(&mut s);
            s
        };
        match name.as_str() {
            "b" => emit(kids, &with(&|s| s.bold = true), b),
            "i" => emit(kids, &with(&|s| s.italic = true), b),
            "u" => emit(kids, &with(&|s| s.underline = true), b),
            "s" | "strike" => emit(kids, &with(&|s| s.strike = true), b),
            "url" => {
                let mut text = String::new();
                raw_text(kids, &mut text);
                let target = arg.clone().unwrap_or_else(|| text.trim().to_string());
                let ls = with(&|s| {
                    s.link = Some(target.as_str().into());
                    s.underline = true;
                });
                if text.trim().is_empty() {
                    b.push_text(&target, &ls);
                } else {
                    emit(kids, &ls, b);
                }
            }
            "h1" | "h2" | "h3" => {
                let mut hb = Builder::default();
                emit(kids, &with(&|s| s.bold = true), &mut hb);
                hb.flush();
                let spans = hb
                    .blocks
                    .into_iter()
                    .flat_map(|bl| {
                        if let Block::Para(p) = bl {
                            p
                        } else {
                            Vec::new()
                        }
                    })
                    .collect();
                b.block(Block::Heading(name.as_bytes()[1] - b'0', spans));
            }
            "img" => {
                let mut url = String::new();
                raw_text(kids, &mut url);
                let url = url.trim().to_string();
                if url.starts_with("http") {
                    b.block(Block::Image(url));
                }
            }
            "quote" => b.block(Block::Quote(arg.clone(), sub(kids, st))),
            "code" => {
                let mut t = String::new();
                raw_text(kids, &mut t);
                b.block(Block::Code(
                    t.trim_matches(['\r', '\n']).replace("\r\n", "\n"),
                ));
            }
            "list" | "olist" => {
                let mut items: Vec<Vec<Block>> = Vec::new();
                let mut cur: Vec<&Node> = Vec::new();
                let mut started = false;
                let push = |cur: &mut Vec<&Node>, items: &mut Vec<Vec<Block>>| {
                    let mut ib = Builder::default();
                    for n in cur.drain(..) {
                        emit(std::slice::from_ref(n), st, &mut ib);
                    }
                    let blocks = ib.finish();
                    if !blocks.is_empty() {
                        items.push(blocks);
                    }
                };
                for k in kids {
                    if matches!(k, Node::Tag(n, _, _) if n == "*") {
                        if started {
                            push(&mut cur, &mut items);
                        }
                        cur.clear();
                        started = true;
                    } else if started {
                        cur.push(k);
                    }
                }
                if started {
                    push(&mut cur, &mut items);
                }
                b.block(Block::List(name == "olist", items));
            }
            "hr" => b.block(Block::Hr),
            "table" => {
                let rows = kids
                    .iter()
                    .filter_map(|r| match r {
                        Node::Tag(n, _, cells) if n == "tr" => Some(
                            cells
                                .iter()
                                .filter_map(|c| match c {
                                    Node::Tag(n, _, k) if n == "td" => Some(sub(k, st)),
                                    Node::Tag(n, _, k) if n == "th" => {
                                        Some(sub(k, &with(&|s| s.bold = true)))
                                    }
                                    _ => None,
                                })
                                .collect::<Vec<_>>(),
                        ),
                        _ => None,
                    })
                    .collect();
                b.block(Block::Table(rows));
            }
            "spoiler" => b.block(Block::Spoiler(sub(kids, st))),
            "previewyoutube" => {
                if let Some(id) = arg.as_deref().and_then(|a| a.split(';').next()) {
                    b.block(Block::Video(id.to_string()));
                }
            }
            _ => emit(kids, st, b),
        }
    }
}

pub fn parse(src: &str) -> Vec<Block> {
    let toks = tokenize(src);
    let nodes = tree(&toks, &mut 0, &mut Vec::new());
    sub(&nodes, &Style::default())
}

// ---------------------------------------------------------------- rendering

fn spans_job(
    ui: &egui::Ui,
    spans: &[Span],
    size: f32,
    wrap: f32,
) -> (LayoutJob, Vec<(usize, usize, Arc<str>)>) {
    let mut job = LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: wrap,
            ..Default::default()
        },
        ..Default::default()
    };
    let vis = ui.visuals();
    let mut links = Vec::new();
    let mut chars = 0;
    for s in spans {
        let color = if s.style.link.is_some() {
            vis.hyperlink_color
        } else if s.style.bold {
            vis.strong_text_color()
        } else {
            vis.text_color()
        };
        let fmt = TextFormat {
            font_id: if s.style.bold {
                crate::theme::bold(size)
            } else {
                FontId::proportional(size)
            },
            color,
            italics: s.style.italic,
            underline: if s.style.underline {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            },
            strikethrough: if s.style.strike {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        };
        let n = s.text.chars().count();
        if let Some(l) = &s.style.link {
            links.push((chars, chars + n, l.clone()));
        }
        chars += n;
        job.append(&s.text, 0.0, fmt);
    }
    (job, links)
}

fn spans_ui(ui: &mut egui::Ui, spans: &[Span], size: f32) {
    let (job, links) = spans_job(ui, spans, size, ui.available_width());
    let label = egui::Label::new(job).sense(if links.is_empty() {
        egui::Sense::hover()
    } else {
        egui::Sense::click()
    });
    let (pos, galley, resp) = label.layout_in_ui(ui);
    ui.painter()
        .galley(pos, galley.clone(), ui.visuals().text_color());
    if links.is_empty() {
        return;
    }
    if let Some(p) = resp.hover_pos() {
        let c = galley.cursor_from_pos(p - pos).index.0;
        let rect_hit = galley.rect.translate(pos.to_vec2()).contains(p);
        if let Some((_, _, url)) = links.iter().find(|(a, b, _)| rect_hit && c >= *a && c < *b) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            let url = url.to_string();
            if resp.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(&url));
            }
            resp.on_hover_text(url);
        }
    }
}

/// Steam's image CDN can downscale; avoid uploading 4K GIFs to the GPU for a side panel.
pub(crate) fn sized_image_url(url: &str, w: f32) -> String {
    if url.contains("steamusercontent.com") && !url.contains('?') {
        format!("{url}?imw={}&ima=fit", (w as u32).clamp(64, 1024))
    } else {
        url.to_string()
    }
}

/// Unwrapped width a cell's content would like; non-text blocks take whatever they get.
fn natural_w(ui: &egui::Ui, blocks: &[Block]) -> f32 {
    let text_w = |s: &[Span], size| {
        ui.painter()
            .layout_job(spans_job(ui, s, size, f32::INFINITY).0)
            .size()
            .x
    };
    blocks
        .iter()
        .map(|b| match b {
            Block::Para(s) => text_w(s, 13.5),
            Block::Heading(_, s) => text_w(s, 17.0),
            Block::List(_, items) => 16.0 + items.iter().map(|i| natural_w(ui, i)).fold(0.0, f32::max),
            _ => f32::INFINITY,
        })
        .fold(0.0, f32::max)
}

/// Column widths like an HTML auto table: columns that fit in an equal share keep their
/// natural width, the rest split what's left evenly (and wrap).
fn fit_columns(nat: &[f32], avail: f32) -> Vec<f32> {
    let mut w = nat.to_vec();
    let mut left = avail;
    let mut open: Vec<usize> = (0..nat.len()).collect();
    while !open.is_empty() {
        let share = left / open.len() as f32;
        let (small, big): (Vec<usize>, Vec<usize>) = open.iter().partition(|&&i| nat[i] <= share);
        if small.is_empty() {
            for &i in &big {
                w[i] = share;
            }
            break;
        }
        for &i in &small {
            left -= nat[i];
        }
        open = big;
    }
    w
}

fn table_ui(ui: &mut egui::Ui, rows: &[Vec<Vec<Block>>]) {
    let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if ncols == 0 {
        return;
    }
    const PAD: f32 = 6.0;
    let avail = ui.available_width();
    let mut nat = vec![0.0f32; ncols];
    for row in rows {
        for (c, cell) in row.iter().enumerate() {
            nat[c] = nat[c].max(natural_w(ui, cell).min(avail) + 2.0 * PAD + 1.0);
        }
    }
    let widths = fit_columns(&nat, avail);
    let stripe = ui.visuals().faint_bg_color;
    for (r, row) in rows.iter().enumerate() {
        egui::Frame::new()
            .fill(if r % 2 == 0 { stripe } else { Color32::TRANSPARENT })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal_top(|ui| {
                    for (cell, &w) in row.iter().zip(&widths) {
                        ui.allocate_ui_with_layout(
                            egui::vec2(w, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(w);
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(PAD as i8, 3))
                                    .show(ui, |ui| show(ui, cell));
                            },
                        );
                    }
                });
            });
        ui.add_space(-ui.spacing().item_spacing.y);
    }
}

pub fn show(ui: &mut egui::Ui, blocks: &[Block]) {
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            ui.add_space(4.0);
        }
        match b {
            Block::Para(s) => spans_ui(ui, s, 13.5),
            Block::Heading(l, s) => {
                ui.add_space(4.0);
                spans_ui(
                    ui,
                    s,
                    match l {
                        1 => 20.0,
                        2 => 17.0,
                        _ => 15.0,
                    },
                );
            }
            Block::Image(url) => {
                let w = ui.available_width();
                ui.add(
                    egui::Image::new(sized_image_url(url, w))
                        .max_width(w)
                        .max_height(420.0)
                        .corner_radius(3.0),
                )
                .on_hover_text(url);
            }
            Block::Quote(who, inner) => {
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        if let Some(w) = who {
                            ui.weak(format!("Originally posted by {w}:"));
                        }
                        show(ui, inner);
                    });
            }
            Block::Code(t) => {
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().extreme_bg_color)
                    .show(ui, |ui| {
                        // Code keeps its lines; long ones scroll sideways instead of wrapping.
                        egui::ScrollArea::horizontal()
                            .id_salt(ui.next_auto_id())
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(egui::RichText::new(t).monospace()).extend());
                            });
                    });
            }
            Block::List(ordered, items) => {
                for (n, item) in items.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        ui.label(if *ordered {
                            format!("{}.", n + 1)
                        } else {
                            "•".to_string()
                        });
                        ui.vertical(|ui| show(ui, item));
                    });
                }
            }
            Block::Hr => {
                ui.separator();
            }
            Block::Table(rows) => table_ui(ui, rows),
            Block::Spoiler(inner) => {
                egui::CollapsingHeader::new(egui::RichText::new("Spoiler").color(Color32::GRAY))
                    .id_salt(ui.next_auto_id())
                    .show(ui, |ui| show(ui, inner));
            }
            Block::Video(id) => {
                ui.hyperlink_to(
                    format!("▶ YouTube video ({id})"),
                    format!("https://www.youtube.com/watch?v={id}"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(b: &Block) -> String {
        match b {
            Block::Para(s) | Block::Heading(_, s) => s.iter().map(|s| s.text.as_str()).collect(),
            _ => String::new(),
        }
    }
    #[test]
    fn keeps_unknown_brackets_and_styles() {
        let b = parse("[VScripts] Library of [b]Utils[/b]\n[h1]Title[/h1]\nafter");
        assert_eq!(text(&b[0]), "[VScripts] Library of Utils");
        assert!(matches!(&b[1], Block::Heading(1, _)));
        assert_eq!(text(&b[2]), "after");
    }
    #[test]
    fn lists_links_images() {
        let b = parse(
            "[list][*]one [url=https://x.y]link[/url][*]two[/list][img]https://i.png[/img] see https://a.b/c.",
        );
        let Block::List(false, items) = &b[0] else {
            panic!("{b:?}")
        };
        assert_eq!(items.len(), 2);
        assert_eq!(b[1], Block::Image("https://i.png".into()));
        let Block::Para(s) = &b[2] else { panic!() };
        assert_eq!(s[1].text, "https://a.b/c");
        assert_eq!(s[2].text, ".");
    }
    #[test]
    fn table_columns_fit() {
        // fits: natural widths kept
        assert_eq!(fit_columns(&[50.0, 80.0], 300.0), vec![50.0, 80.0]);
        // narrow column keeps natural width, wide one gets the rest
        assert_eq!(fit_columns(&[60.0, 900.0], 300.0), vec![60.0, 240.0]);
        // both too wide: even split
        assert_eq!(fit_columns(&[500.0, 900.0], 300.0), vec![150.0, 150.0]);
    }
    #[test]
    fn tolerates_garbage() {
        let b = parse("[b]unclosed [i]x[/b] [/u] [noparse][b]raw[/b][/noparse]");
        let all: String = b.iter().map(text).collect();
        assert_eq!(all, "unclosed x  [b]raw[/b]");
    }
}
