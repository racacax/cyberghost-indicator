//! Torrent servers and favorites (menu, ★ markers, favorites window, persistence).
mod support;
use serde_json::json;
use std::time::Duration;
use support::*;

fn started(favorites: Option<serde_json::Value>, opts: Options) -> TestEnv {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    if let Some(f) = favorites {
        t.write_favorites(&f);
    }
    t.start(opts);
    t.wait_icon("ghost-off.png");
    t
}

fn sample_favorites() -> serde_json::Value {
    json!({"favorites": [
        {"kind": "standard", "cc": "FR"},
        {"kind": "streaming", "cc": "FR", "service": "Disney Plus FR"},
        {"kind": "torrent", "cc": "NL"}
    ]})
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Rows the fake zenity should tick ("Country|Server").
fn select(t: &TestEnv, rows: &[&str]) {
    t.set_mock(
        "zenity_select",
        &rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
    );
}

fn open_window(t: &TestEnv) {
    let before = t.dialog_runs();
    t.click(&["Edit favorites…"]);
    assert!(
        wait_until(TIMEOUT, || t.dialog_runs() > before),
        "favorites window not opened"
    );
}

// ------------------------------------------------------------------ torrent
#[test]
fn connect_to_a_torrent_server() {
    let t = started(None, Options::default());
    assert!(
        t.menu()
            .path(&["Torrent"])
            .children
            .iter()
            .all(|c| c.has_icon),
        "flags"
    );
    t.click(&["Torrent", "Netherlands  (NL)"]);
    t.wait_icon("flag-NL.png");
    assert_eq!(t.status(), "Connected: Netherlands (Torrent)");
    assert_eq!(
        t.calls_with("--connect"),
        [args(&["--torrent", "--country-code", "NL", "--connect"])]
    );
    t.wait_notification("VPN connected: Netherlands (Torrent)");
}

#[test]
fn torrent_menu_is_locked_while_busy() {
    let t = started(None, Options::default());
    t.set_mock("connect_mode", "slow");
    t.click(&["Torrent", "France  (FR)"]);
    t.wait_status("Connecting to France (Torrent)…");
    assert!(!t.menu().path(&["Torrent"]).enabled);
    assert!(!t.menu().path(&["Favorites"]).enabled);
    t.wait_icon("flag-FR.png");
}

// ------------------------------------------------------------------ favorites menu
#[test]
fn favorites_are_grouped_by_country() {
    let t = started(Some(sample_favorites()), Options::default());
    let m = t.menu();
    let fav = m.path(&["Favorites"]);
    // France: two favorites, a sub-list; Netherlands: one, a single entry
    assert_eq!(fav.labels(), ["France", "Netherlands  (Torrent)"]);
    assert_eq!(
        fav.path(&["France"]).labels(),
        ["Standard", "Disney Plus FR"]
    );
    assert!(fav.children.iter().all(|c| c.has_icon), "country flags");
}

#[test]
fn favorites_are_starred_in_the_lists() {
    let t = started(Some(sample_favorites()), Options::default());
    let m = t.menu();
    assert_eq!(
        m.path(&["Countries"]).labels(),
        [
            "Andorra  (AD)",
            "France  (FR)   ★",
            "Germany  (DE)",
            "Switzerland  (CH)",
            "United States  (US)"
        ]
    );
    assert_eq!(
        m.path(&["Streaming", "France  (FR)"]).labels(),
        ["Disney Plus FR   ★"]
    );
    assert_eq!(
        m.path(&["Torrent"]).labels(),
        ["France  (FR)", "Netherlands  (NL)   ★", "Switzerland  (CH)"]
    );
}

#[test]
fn connect_from_favorites() {
    let t = started(Some(sample_favorites()), Options::default());
    t.click(&["Favorites", "France", "Disney Plus FR"]);
    t.wait_icon("flag-FR.png");
    assert_eq!(t.status(), "Connected: Disney Plus FR");
    t.click(&["Favorites", "Netherlands  (Torrent)"]);
    t.wait_icon("flag-NL.png");
    assert_eq!(t.status(), "Connected: Netherlands (Torrent)");
    let connects: Vec<String> = t
        .calls_with("--connect")
        .into_iter()
        .map(|c| c.join(" "))
        .collect();
    assert_eq!(
        connects,
        [
            "--streaming Disney Plus FR --country-code FR --connect",
            "--torrent --country-code NL --connect"
        ]
    );
}

#[test]
fn starred_entry_connects() {
    let t = started(Some(sample_favorites()), Options::default());
    t.click(&["Countries", "France  (FR)   ★"]);
    t.wait_icon("flag-FR.png");
    assert_eq!(
        t.read_json(&t.favorites_file()),
        sample_favorites(),
        "favorites untouched"
    );
    assert_eq!(t.dialog_runs(), 0);
}

#[test]
fn favorite_for_a_country_no_longer_listed_is_kept() {
    let t = started(
        Some(json!({"favorites": [{"kind": "torrent", "cc": "SE"}]})),
        Options::default(),
    );
    let fav = t.menu().path(&["Favorites"]).clone();
    assert_eq!(fav.labels(), ["SE  (Torrent)"]);
    assert!(fav.children[0].has_icon, "Swedish flag");
}

#[test]
fn corrupted_favorites_file_is_ignored() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    let p = t.favorites_file();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "{not json").unwrap();
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    assert_eq!(t.menu().path(&["Favorites"]).labels(), ["No favorites yet"]);
}

// ------------------------------------------------------------------ favorites window
#[test]
fn window_lists_every_server_with_favorites_first() {
    let t = started(Some(sample_favorites()), Options::default());
    select(
        &t,
        &[
            "France  (FR)|Standard",
            "France  (FR)|Disney Plus FR",
            "Netherlands  (NL)|Torrent",
        ],
    );
    open_window(&t);
    let rows = t.dialog_rows();
    // rows are passed as arguments (all loaded before the window opens), not streamed on stdin
    assert!(
        !t.mock("zenity_errors.log").exists(),
        "zenity got data on stdin"
    );
    assert_eq!(
        rows.len(),
        5 + 5 + 3,
        "every standard, streaming and torrent server"
    );
    let ticked: Vec<_> = rows
        .iter()
        .take_while(|r| r.0)
        .map(|r| format!("{} | {}", r.1, r.2))
        .collect();
    assert_eq!(
        ticked,
        [
            "France  (FR) | Standard",
            "France  (FR) | Disney Plus FR",
            "Netherlands  (NL) | Torrent"
        ]
    );
    assert_eq!(
        rows.iter().filter(|r| r.0).count(),
        3,
        "only favorites are ticked"
    );
    assert_eq!(
        rows[3],
        (false, "Andorra  (AD)".into(), "Standard".into()),
        "then the rest by country"
    );
    let a = t.dialog_args();
    for x in [
        "--list",
        "--checklist",
        "CyberGhost favorites",
        "Country",
        "Server",
        "--print-column",
    ] {
        assert!(a.iter().any(|v| v == x), "zenity args lack {x:?}: {a:?}");
    }
}

#[test]
fn ticking_in_the_window_updates_favorites() {
    let t = started(None, Options::default());
    select(
        &t,
        &[
            "Switzerland  (CH)|Standard",
            "United States  (US)|Netflix US",
            "France  (FR)|Torrent",
        ],
    );
    open_window(&t);
    // new favorites are added in the window's order (by country)
    let want = json!({"favorites": [
        {"kind": "torrent", "cc": "FR"},
        {"kind": "standard", "cc": "CH"},
        {"kind": "streaming", "cc": "US", "service": "Netflix US"}
    ]});
    assert!(
        wait_until(TIMEOUT, || t.read_json(&t.favorites_file()) == want),
        "{}",
        t.read_json(&t.favorites_file())
    );
    let m = t.menu();
    assert_eq!(
        m.path(&["Favorites"]).labels(),
        [
            "France  (Torrent)",
            "Switzerland  (Standard)",
            "United States  (Netflix US)"
        ]
    );
    assert_eq!(m.path(&["Torrent"]).labels()[0], "France  (FR)   ★");
    assert!(
        t.calls_with("--connect").is_empty(),
        "the window never connects"
    );
}

#[test]
fn unticking_removes_and_keeps_the_order_of_the_others() {
    let t = started(Some(sample_favorites()), Options::default());
    select(&t, &["Netherlands  (NL)|Torrent", "France  (FR)|Standard"]);
    open_window(&t);
    let want =
        json!({"favorites": [{"kind": "standard", "cc": "FR"}, {"kind": "torrent", "cc": "NL"}]});
    assert!(wait_until(TIMEOUT, || t.read_json(&t.favorites_file()) == want));
    // France drops back to a single entry
    assert_eq!(
        t.menu().path(&["Favorites"]).labels(),
        ["France  (Standard)", "Netherlands  (Torrent)"]
    );
}

#[test]
fn ok_with_nothing_ticked_removes_all_favorites() {
    let t = started(Some(sample_favorites()), Options::default());
    select(&t, &[]);
    open_window(&t);
    assert!(wait_until(TIMEOUT, || t.read_json(&t.favorites_file())
        == json!({"favorites": []})));
    assert!(wait_until(TIMEOUT, || t
        .menu()
        .path(&["Favorites"])
        .labels()
        == ["No favorites yet"]));
}

#[test]
fn cancel_changes_nothing() {
    let t = started(Some(sample_favorites()), Options::default());
    t.set_mock("zenity_mode", "cancel");
    select(&t, &[]);
    open_window(&t);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(t.read_json(&t.favorites_file()), sample_favorites());
    assert_eq!(
        t.menu().path(&["Favorites"]).labels(),
        ["France", "Netherlands  (Torrent)"]
    );
}

#[test]
fn only_one_window_at_a_time() {
    let t = started(None, Options::default());
    t.set_mock("zenity_mode", "slow");
    open_window(&t);
    t.click(&["Edit favorites…"]);
    t.click(&["Edit favorites…"]);
    std::thread::sleep(Duration::from_millis(2500)); // the first window closes
    assert_eq!(t.dialog_runs(), 1);
    t.clear_mock("zenity_mode");
    open_window(&t);
    assert_eq!(t.dialog_runs(), 2, "can be opened again once closed");
}

#[test]
fn window_can_be_used_while_connecting() {
    let t = started(None, Options::default());
    t.set_mock("connect_mode", "slow");
    t.click(&["Countries", "Germany  (DE)"]);
    t.wait_status("Connecting to Germany…");
    assert!(t.menu().path(&["Edit favorites…"]).enabled);
    select(&t, &["Germany  (DE)|Standard"]);
    open_window(&t);
    t.wait_icon("flag-DE.png");
    assert!(wait_until(TIMEOUT, || t.read_json(&t.favorites_file())
        == json!({"favorites": [{"kind": "standard", "cc": "DE"}]})));
    assert_eq!(t.calls_with("--connect").len(), 1);
}

#[test]
fn unlisted_favorites_can_be_removed_from_the_window() {
    let t = started(
        Some(json!({"favorites": [{"kind": "torrent", "cc": "SE"}]})),
        Options::default(),
    );
    select(&t, &[]);
    open_window(&t);
    assert_eq!(
        t.dialog_rows()[0],
        (true, "SE  (SE)".into(), "Torrent".into())
    );
    assert!(wait_until(TIMEOUT, || t.read_json(&t.favorites_file())
        == json!({"favorites": []})));
}

#[test]
fn missing_zenity_is_reported() {
    let t = started(
        None,
        Options {
            zenity: Some("/nonexistent/zenity".into()),
            ..Default::default()
        },
    );
    t.click(&["Edit favorites…"]);
    let n = t.wait_notification("Cannot open the favorites window");
    assert!(n.urgent);
    assert!(
        n.body
            .starts_with("zenity is required (/nonexistent/zenity:"),
        "{}",
        n.body
    );
    assert!(n.body.ends_with("sudo apt install zenity"), "{}", n.body);
    // still usable afterwards
    t.click(&["Edit favorites…"]);
    assert!(wait_until(TIMEOUT, || t.notifications().len() == 2));
}

#[test]
fn favorites_persist_across_restarts() {
    let mut t = started(None, Options::default());
    select(&t, &["Switzerland  (CH)|Standard"]);
    open_window(&t);
    assert!(wait_until(TIMEOUT, || t.favorites_file().exists()));
    t.stop();
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    assert_eq!(
        t.menu().path(&["Favorites"]).labels(),
        ["Switzerland  (Standard)"]
    );
}

#[test]
fn favorites_in_french() {
    let t = started(
        Some(
            json!({"favorites": [{"kind": "torrent", "cc": "FR"}, {"kind": "standard", "cc": "CH"}]}),
        ),
        Options {
            lang: vec![("LANG", "fr_FR.UTF-8")],
            ..Default::default()
        },
    );
    let m = t.menu();
    assert_eq!(
        m.path(&["Favoris"]).labels(),
        ["France  (Torrent)", "Switzerland  (Standard)"]
    );
    select(&t, &["France  (FR)|Torrent", "Switzerland  (CH)|Standard"]);
    let before = t.dialog_runs();
    t.click(&["Modifier les favoris…"]);
    assert!(wait_until(TIMEOUT, || t.dialog_runs() > before));
    let a = t.dialog_args();
    for x in ["Favoris CyberGhost", "Pays", "Serveur"] {
        assert!(a.iter().any(|v| v == x), "{x:?} not in {a:?}");
    }
    assert!(a.iter().any(|v| v.starts_with("Cochez les serveurs")));
    t.click(&["Favoris", "France  (Torrent)"]);
    t.wait_status("Connecté : France (Torrent)");
}
