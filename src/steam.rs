//! Steam Workshop access without an API key:
//! - `ISteamRemoteStorage/GetPublishedFileDetails` (title, BBCode description, direct file URL, tags, stats)
//! - `ISteamRemoteStorage/GetCollectionDetails` (collection children)
//! - Workshop browse page (search / sort / tag filter / "items requiring X") scraped for item ids
//! - Item page scraped for "Required items" and creator names

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const APP_ID: u32 = 550;
const API: &str = "https://api.steampowered.com/ISteamRemoteStorage";

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) L4D2AddonManager/0.1")
        .http_status_as_error(false)
        .build()
        .into()
}

#[derive(Debug)]
pub enum Error {
    RateLimited,
    Other(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::RateLimited => write!(f, "Steam is rate-limiting requests; retrying shortly"),
            Error::Other(s) => f.write_str(s),
        }
    }
}
impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::Other(e.to_string())
    }
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Details {
    pub id: u64,
    /// false when the item is private, removed or otherwise unavailable.
    pub ok: bool,
    pub title: String,
    pub description: String,
    pub file_url: String,
    pub filename: String,
    pub preview_url: String,
    pub file_size: u64,
    pub time_created: u64,
    pub time_updated: u64,
    pub tags: Vec<String>,
    pub subscriptions: u64,
    pub favorited: u64,
    pub views: u64,
    pub banned: bool,
    pub creator: String,
    pub fetched: u64,
}

impl Details {
    /// Steam's image CDN can resize on the fly; use small thumbnails for lists.
    pub fn thumb(&self, px: u32) -> Option<String> {
        (!self.preview_url.is_empty()).then(|| {
            format!(
                "{}?imw={px}&imh={px}&ima=fit&impolicy=Letterbox&imcolor=%23000000&letterbox=true",
                self.preview_url
            )
        })
    }
}

pub fn item_url(id: u64) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}")
}

fn num(v: &serde_json::Value) -> u64 {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}
fn s(v: &serde_json::Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// Fetches details for any number of items (batched by 100).
pub fn fetch_details(agent: &ureq::Agent, ids: &[u64]) -> Result<Vec<Details>, Error> {
    let mut out = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(100) {
        let mut form: Vec<(String, String)> = vec![("itemcount".into(), chunk.len().to_string())];
        for (i, id) in chunk.iter().enumerate() {
            form.push((format!("publishedfileids[{i}]"), id.to_string()));
        }
        let mut resp = agent
            .post(format!("{API}/GetPublishedFileDetails/v1/"))
            .send_form(form)?;
        if resp.status() == 429 {
            return Err(Error::RateLimited);
        }
        let v: serde_json::Value = resp.body_mut().read_json()?;
        let now = now();
        for d in v["response"]["publishedfiledetails"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let id = num(&d["publishedfileid"]);
            out.push(Details {
                id,
                ok: d["result"].as_u64() == Some(1)
                    && d["consumer_app_id"].as_u64().unwrap_or(APP_ID as u64) == APP_ID as u64,
                title: s(&d["title"]),
                description: s(&d["description"]),
                file_url: s(&d["file_url"]),
                filename: s(&d["filename"]),
                preview_url: s(&d["preview_url"]),
                file_size: num(&d["file_size"]),
                time_created: num(&d["time_created"]),
                time_updated: num(&d["time_updated"]),
                tags: d["tags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| s(&t["tag"]))
                    .filter(|t| !t.is_empty())
                    .collect(),
                subscriptions: num(&d["lifetime_subscriptions"]).max(num(&d["subscriptions"])),
                favorited: num(&d["favorited"]),
                views: num(&d["views"]),
                banned: num(&d["banned"]) != 0,
                creator: s(&d["creator"]),
                fetched: now,
            });
        }
    }
    Ok(out)
}

/// Returns the children of a collection, or an empty list if `id` isn't a collection.
pub fn collection_children(agent: &ureq::Agent, id: u64) -> Result<Vec<u64>, Error> {
    let form = [
        ("collectioncount", "1".to_string()),
        ("publishedfileids[0]", id.to_string()),
    ];
    let mut resp = agent
        .post(format!("{API}/GetCollectionDetails/v1/"))
        .send_form(form)?;
    if resp.status() == 429 {
        return Err(Error::RateLimited);
    }
    let v: serde_json::Value = resp.body_mut().read_json()?;
    Ok(v["response"]["collectiondetails"][0]["children"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| num(&c["publishedfileid"]))
        .filter(|&n| n != 0)
        .collect())
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Sort {
    Trending,
    MostRecent,
    LastUpdated,
    TopRated,
    MostSubscribed,
}
impl Sort {
    pub const ALL: [Sort; 5] = [
        Sort::Trending,
        Sort::MostRecent,
        Sort::LastUpdated,
        Sort::TopRated,
        Sort::MostSubscribed,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Sort::Trending => "Most popular",
            Sort::MostRecent => "Most recent",
            Sort::LastUpdated => "Last updated",
            Sort::TopRated => "Top rated (all time)",
            Sort::MostSubscribed => "Most subscribed",
        }
    }
    fn param(self) -> &'static str {
        match self {
            Sort::Trending => "trend",
            Sort::MostRecent => "mostrecent",
            Sort::LastUpdated => "lastupdated",
            Sort::TopRated => "toprated",
            Sort::MostSubscribed => "totaluniquesubscribers",
        }
    }
}

/// Official L4D2 Workshop tags.
pub const TAGS: &[&str] = &[
    "Survivors",
    "Common Infected",
    "Special Infected",
    "Weapons",
    "Items",
    "Sounds",
    "Scripts",
    "UI",
    "Models",
    "Textures",
    "Miscellaneous",
    "Campaigns",
    "Single Chapter",
    "Co-op",
    "Versus",
    "Survival",
    "Scavenge",
    "Realism",
    "Mutations",
    "Realism Versus",
    "Grenade Launcher",
    "M60",
    "Melee",
    "Pistol",
    "Rifle",
    "Shotgun",
    "SMG",
    "Sniper",
    "Throwable",
    "Adrenaline",
    "Defibrillator",
    "Medkit",
    "Pills",
    "Other",
    "Boomer",
    "Charger",
    "Hunter",
    "Jockey",
    "Smoker",
    "Spitter",
    "Tank",
    "Witch",
    "Bill",
    "Francis",
    "Louis",
    "Zoey",
    "Coach",
    "Ellis",
    "Nick",
    "Rochelle",
];

#[derive(Clone, Debug, PartialEq)]
pub struct BrowseQuery {
    pub text: String,
    pub sort: Sort,
    /// For `Trending`: look-back window in days (1, 7, 30, 90, 180, 365, or -1 for all time).
    pub days: i32,
    pub tags: Vec<String>,
    pub page: u32,
    /// Only items that list this id as a required item ("used by").
    pub child_of: Option<u64>,
    pub collections: bool,
}

impl Default for BrowseQuery {
    fn default() -> Self {
        BrowseQuery {
            text: String::new(),
            sort: Sort::Trending,
            days: 7,
            tags: Vec::new(),
            page: 1,
            child_of: None,
            collections: false,
        }
    }
}

fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

pub const PER_PAGE: usize = 30;

pub fn browse(agent: &ureq::Agent, q: &BrowseQuery) -> Result<Vec<u64>, Error> {
    let (sort, actual) = if !q.text.trim().is_empty() && q.sort == Sort::Trending {
        ("textsearch", "textsearch")
    } else {
        (q.sort.param(), q.sort.param())
    };
    let mut url = format!(
        "https://steamcommunity.com/workshop/browse/?appid={APP_ID}&browsesort={sort}&actualsort={actual}&section={}&p={}&numperpage={PER_PAGE}",
        if q.collections {
            "collections"
        } else {
            "readytouseitems"
        },
        q.page.max(1)
    );
    if !q.text.trim().is_empty() {
        url.push_str(&format!("&searchtext={}", enc(q.text.trim())));
    }
    if q.sort == Sort::Trending {
        url.push_str(&format!("&days={}", q.days));
    }
    for t in &q.tags {
        url.push_str(&format!("&requiredtags%5B%5D={}", enc(t)));
    }
    if let Some(c) = q.child_of {
        url.push_str(&format!("&childpublishedfileid={c}"));
    }
    let html = get_text(agent, &url)?;
    let mut ids = Vec::new();
    for part in html.split("filedetails/?id=").skip(1) {
        let n: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(id) = n.parse::<u64>() {
            if Some(id) != q.child_of && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    Ok(ids)
}

fn get_text(agent: &ureq::Agent, url: &str) -> Result<String, Error> {
    let mut resp = agent
        .get(url)
        .header("Accept-Language", "en-US,en;q=0.8")
        .call()?;
    match resp.status().as_u16() {
        429 => Err(Error::RateLimited),
        200 => Ok(resp
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_string()?),
        c => Err(Error::Other(format!("HTTP {c} from Steam"))),
    }
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct PageInfo {
    /// Items this one says it requires: (id, title).
    pub required: Vec<(u64, String)>,
    pub creators: Vec<String>,
    pub fetched: u64,
}

pub fn unescape_html(s: &str) -> String {
    let mut out = s
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&#x27;", "'");
    out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    unescape_html(out.split_whitespace().collect::<Vec<_>>().join(" ").trim())
}

pub fn parse_item_page(html: &str) -> PageInfo {
    let mut info = PageInfo {
        fetched: now(),
        ..Default::default()
    };
    if let Some(start) = html.find("id=\"RequiredItems\"") {
        let rest = &html[start..];
        let end = rest
            .find("<!-- created by -->")
            .or_else(|| rest.find("rightSectionTopTitle"))
            .unwrap_or(rest.len().min(20_000));
        for part in rest[..end].split("filedetails/?id=").skip(1) {
            let n: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            let title = part
                .find("class=\"requiredItem\">")
                .map(|i| &part[i + 21..])
                .and_then(|t| t.find("</div>").map(|e| strip_tags(&t[..e])))
                .unwrap_or_default();
            if let Ok(id) = n.parse::<u64>() {
                if !info.required.iter().any(|(r, _)| *r == id) {
                    info.required.push((id, title));
                }
            }
        }
    }
    for part in html.split("class=\"friendBlockContent\">").skip(1) {
        if let Some(e) = part.find("<br") {
            let name = strip_tags(&part[..e]);
            if !name.is_empty() && !info.creators.contains(&name) {
                info.creators.push(name);
            }
        }
    }
    info
}

/// With a (free) Steam Web API key: required items for up to 100 ids per call, no scraping.
pub fn fetch_children(
    agent: &ureq::Agent,
    key: &str,
    ids: &[u64],
) -> Result<Vec<(u64, Vec<u64>)>, Error> {
    let mut out = Vec::new();
    for chunk in ids.chunks(100) {
        let mut url = format!(
            "https://api.steampowered.com/IPublishedFileService/GetDetails/v1/?key={}&includechildren=true",
            enc(key.trim())
        );
        for (i, id) in chunk.iter().enumerate() {
            url.push_str(&format!("&publishedfileids%5B{i}%5D={id}"));
        }
        let mut resp = agent.get(&url).call()?;
        match resp.status().as_u16() {
            200 => {}
            429 => return Err(Error::RateLimited),
            401 | 403 => return Err(Error::Other("Steam Web API key was rejected".into())),
            c => return Err(Error::Other(format!("HTTP {c} from Steam API"))),
        }
        let v: serde_json::Value = resp.body_mut().read_json()?;
        for d in v["response"]["publishedfiledetails"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let kids = d["children"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| num(&c["publishedfileid"]))
                .filter(|&n| n != 0)
                .collect();
            out.push((num(&d["publishedfileid"]), kids));
        }
    }
    Ok(out)
}

pub fn fetch_page(agent: &ureq::Agent, id: u64) -> Result<PageInfo, Error> {
    Ok(parse_item_page(&get_text(agent, &item_url(id))?))
}

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

/// Streams `url` into `dest` (via a `.part` file), verifying it is a VPK before replacing anything.
pub fn download(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    prog: &Progress,
) -> Result<(), String> {
    let resp = agent.get(url).call().map_err(|e| e.to_string())?;
    if resp.status() != 200 {
        return Err(format!("download failed: HTTP {}", resp.status()));
    }
    let body = resp.into_body();
    if let Some(len) = body.content_length() {
        prog.total.store(len, Ordering::Relaxed);
    }
    let part = dest.with_extension("vpk.part");
    let result = (|| -> Result<(), String> {
        let mut file = std::io::BufWriter::with_capacity(
            1 << 20,
            std::fs::File::create(&part)
                .map_err(|e| format!("can't write {}: {e}", part.display()))?,
        );
        let mut reader = body.into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut first = true;
        loop {
            if prog.cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            if first {
                first = false;
                if n < 4 || u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) != 0x55AA_1234 {
                    return Err(
                        "downloaded file is not a VPK (Workshop item may not be a plain add-on)"
                            .into(),
                    );
                }
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            prog.done.fetch_add(n as u64, Ordering::Relaxed);
        }
        file.flush().map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&part, dest).map_err(|e| {
            format!(
                "can't replace {} ({e}). Close Left 4 Dead 2 if it is running and retry.",
                dest.display()
            )
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Extracts a Workshop id from a URL, `steam://` link or plain number.
pub fn parse_id(s: &str) -> Option<u64> {
    let s = s.trim();
    let digits = match s.find("id=") {
        Some(i) => &s[i + 3..],
        None => s.rsplit('/').find(|p| !p.is_empty()).unwrap_or(s),
    };
    let n: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
    n.parse().ok().filter(|&n| n > 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ids() {
        assert_eq!(
            parse_id(
                "https://steamcommunity.com/sharedfiles/filedetails/?id=3097829248&searchtext="
            ),
            Some(3097829248)
        );
        assert_eq!(parse_id("3097829248"), Some(3097829248));
        assert_eq!(
            parse_id("steam://url/CommunityFilePage/2821408044"),
            Some(2821408044)
        );
        assert_eq!(parse_id("hello"), None);
    }
    #[test]
    fn parses_required_items() {
        let html = r#"<div class="requiredItemsContainer" id="RequiredItems">
            <a href="https://steamcommunity.com/workshop/filedetails/?id=2821408044" target="_blank" data-subscribed="0">
            <div class="requiredItem">
                [VScripts] Library of Utils &amp; more   </div></a></div>
            <!-- created by --><div class="friendBlockContent">
                changethis<br><span>Offline</span>"#;
        let p = parse_item_page(html);
        assert_eq!(
            p.required,
            vec![(2821408044, "[VScripts] Library of Utils & more".to_string())]
        );
        assert_eq!(p.creators, vec!["changethis".to_string()]);
    }
}
