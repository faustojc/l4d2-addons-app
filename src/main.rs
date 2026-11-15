#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bbcode;
mod config;
mod kv;
mod library;
mod steam;
mod theme;
mod ui;
mod vpk;

/// `--check`: headless smoke test against the real install and Steam (debug builds print to console).
fn check() {
    let t = std::time::Instant::now();
    let game = config::detect_game_dir().expect("game dir");
    let addons = game.join("left4dead2").join("addons");
    let lib = library::scan(&addons);
    let bad: Vec<_> = lib
        .addons
        .iter()
        .filter(|a| a.error.is_some())
        .map(|a| (a.key.clone(), a.error.clone()))
        .collect();
    let active = lib
        .conflicts
        .iter()
        .filter(|c| lib.conflict_active(c))
        .count();
    println!(
        "scan: {} add-ons ({} enabled) in {:?}; {} unreadable {:?}",
        lib.addons.len(),
        lib.addons.iter().filter(|a| a.enabled).count(),
        t.elapsed(),
        bad.len(),
        bad.iter().take(3).collect::<Vec<_>>()
    );
    let active_stock = lib
        .conflicts
        .iter()
        .filter(|c| lib.conflict_active(c) && c.stock > 0)
        .count();
    let nf: usize = lib.addons.iter().map(|a| a.files.len()).sum();
    let nb: usize = lib
        .addons
        .iter()
        .flat_map(|a| a.files.iter())
        .map(|(p, _)| p.len())
        .sum();
    println!("files: {nf} entries, {} MB of path text", nb / 1_000_000);
    println!(
        "conflicts: {} pairs, {} active, {} active touching stock files; {} stock paths overridden",
        lib.conflicts.iter().filter(|c| !c.files.is_empty()).count(),
        active,
        active_stock,
        lib.stock.len()
    );
    for c in lib
        .conflicts
        .iter()
        .filter(|c| lib.conflict_active(c))
        .take(5)
    {
        println!(
            "  {} <-> {}: {} differ ({} stock), {} identical, e.g. {}",
            lib.addons[c.a].info.title,
            lib.addons[c.b].info.title,
            c.files.len(),
            c.stock,
            c.identical,
            c.files[0]
        );
    }
    let tmp = std::env::temp_dir().join("addonlist_check.txt");
    std::fs::copy(game.join("left4dead2").join("addonlist.txt"), &tmp).ok();
    library::write_addonlist(&tmp, &lib).expect("write addonlist");
    let back = library::read_addonlist(&tmp);
    let same = lib.addons.iter().all(|a| {
        back.iter()
            .any(|(k, on)| k.eq_ignore_ascii_case(&a.key) && *on == a.enabled)
    });
    println!(
        "addonlist roundtrip (temp copy): {} entries, states preserved: {same}",
        back.len()
    );
    let agent = steam::agent();
    let ids: Vec<u64> = lib.addons.iter().filter_map(|a| a.id).collect();
    let t = std::time::Instant::now();
    let d = steam::fetch_details(&agent, &ids).expect("details");
    println!(
        "details: {} items in {:?}; with file_url: {}",
        d.len(),
        t.elapsed(),
        d.iter().filter(|x| !x.file_url.is_empty()).count()
    );
    let mut upd = 0;
    for x in &d {
        if let Some(a) = lib.by_id(x.id).next() {
            if x.ok && x.time_updated > a.mtime + 120 {
                upd += 1;
            }
        }
    }
    println!("updates by mtime heuristic: {upd}");
    let q = steam::BrowseQuery {
        text: "tank".into(),
        ..Default::default()
    };
    let found = steam::browse(&agent, &q).unwrap_or_default();
    println!("browse 'tank': {} results", found.len());
    // Download the smallest result into a temp folder (never the real addons dir) and verify it.
    if let Some(small) = steam::fetch_details(&agent, &found)
        .unwrap_or_default()
        .into_iter()
        .filter(|d| !d.file_url.is_empty() && d.file_size > 0)
        .min_by_key(|d| d.file_size)
    {
        let dest = std::env::temp_dir().join(format!("addonmgr_check_{}.vpk", small.id));
        let prog = steam::Progress::default();
        let r = steam::download(&agent, &small.file_url, &dest, &prog);
        let parsed = vpk::Vpk::open(&dest).map(|v| v.entries.len());
        println!(
            "download {} ({} B): {r:?}, got {} B, vpk entries {parsed:?}",
            small.id,
            small.file_size,
            prog.done.load(std::sync::atomic::Ordering::Relaxed)
        );
        let _ = std::fs::remove_file(&dest);
    }
    let q = steam::BrowseQuery {
        child_of: Some(2821408044),
        sort: steam::Sort::MostSubscribed,
        ..Default::default()
    };
    let used = steam::browse(&agent, &q).unwrap_or_default();
    println!("used-by vslu: {} first {:?}", used.len(), used.first());
    if let Some(&u) = used.first() {
        println!(
            "page {u}: {:?}",
            steam::fetch_page(&agent, u).map(|p| (p.required, p.creators))
        );
    }
}

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--check") {
        check();
        return Ok(());
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("L4D2 Addons Manager")
        // matches the Linux .desktop file name / StartupWMClass so docks group the window with its launcher
        .with_app_id("l4d2-addons-manager")
        .with_inner_size([1360.0, 820.0])
        .with_min_inner_size([900.0, 520.0]);
    if let Ok(img) = image::load_from_memory(include_bytes!("../assets/icon-256.png")) {
        let img = img.to_rgba8();
        viewport = viewport.with_icon(egui::IconData {
            width: img.width(),
            height: img.height(),
            rgba: img.into_raw(),
        });
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "L4D2 Addons Manager",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
