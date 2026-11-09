//! Startup: lists cache, initial connection state, single instance, CLI args.
mod support;
use serde_json::json;
use std::time::Duration;
use support::*;

#[test]
fn fetches_lists_when_cache_is_empty() {
    let mut t = TestEnv::new();
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    assert!(wait_until(TIMEOUT, || t
        .menu()
        .path(&["Countries"])
        .children
        .len()
        == 5));
    let m = t.menu();
    assert_eq!(
        m.path(&["Countries"]).labels(),
        [
            "Andorra  (AD)",
            "France  (FR)",
            "Germany  (DE)",
            "Switzerland  (CH)",
            "United States  (US)"
        ],
        "sorted by name"
    );
    assert!(
        m.path(&["Countries"]).children.iter().all(|c| c.has_icon),
        "flags in menu"
    );
    assert_eq!(
        m.path(&["Streaming"]).labels(),
        [
            "France  (FR)",
            "IN  (IN)",
            "United States  (US)",
            "ZZ  (ZZ)"
        ],
        "grouped by country, unknown countries by code"
    );
    assert_eq!(
        m.path(&["Streaming", "United States  (US)"]).labels(),
        ["Apple TV US", "Netflix US"]
    );
    assert_eq!(
        m.path(&["Torrent"]).labels(),
        ["France  (FR)", "Netherlands  (NL)", "Switzerland  (CH)"],
        "torrent list sorted by name"
    );
    assert_eq!(m.path(&["Favorites"]).labels(), ["No favorites yet"]);
    // cached for next time
    let cache = t.read_json(&t.cache_file("lists.json"));
    assert_eq!(cache["traffic"].as_array().unwrap().len(), 5);
    assert_eq!(cache["streaming"].as_array().unwrap().len(), 5);
    assert_eq!(cache["torrent"].as_array().unwrap().len(), 3);
    // automatic fetch: no "lists updated" notification
    assert!(t.notifications().is_empty(), "{:?}", t.notifications());
}

#[test]
fn uses_cached_lists_without_calling_the_cli() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    assert_eq!(t.menu().path(&["Countries"]).children.len(), 5);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        t.calls_with("--country-code").is_empty(),
        "{:?}",
        t.cli_calls()
    );
}

#[test]
fn detects_an_existing_connection_without_known_country() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.set_mock("connected", "DE");
    t.start(Options::default());
    t.wait_icon("ghost-on.png");
    assert_eq!(t.status(), "Connected");
    assert!(t.menu().path(&["Disconnect"]).enabled);
}

#[test]
fn restores_the_flag_of_the_last_connection() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.write_cache("state.json", &json!({"cc": "CH", "label": "Switzerland"}));
    t.set_mock("connected", "CH");
    t.start(Options::default());
    t.wait_icon("flag-CH.png");
    assert_eq!(t.status(), "Connected: Switzerland");
    assert_eq!(t.title(), "CyberGhost VPN — Connected: Switzerland");
}

#[test]
fn clears_a_stale_last_connection() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.write_cache("state.json", &json!({"cc": "CH", "label": "Switzerland"}));
    t.start(Options::default()); // mock: not connected
    t.wait_icon("ghost-off.png");
    assert!(wait_until(TIMEOUT, || t
        .read_json(&t.cache_file("state.json"))
        == json!({})));
    assert!(!t.menu().path(&["Disconnect"]).enabled);
}

#[test]
fn survives_corrupted_cache_and_config() {
    let mut t = TestEnv::new();
    for f in ["lists.json", "state.json"] {
        let p = t.cache_file(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "{ this is not json").unwrap();
    }
    std::fs::create_dir_all(t.config_file().parent().unwrap()).unwrap();
    std::fs::write(t.config_file(), "[1, 2").unwrap();
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    assert!(
        wait_until(TIMEOUT, || t.menu().path(&["Countries"]).children.len()
            == 5),
        "lists re-fetched"
    );
}

#[test]
fn only_one_instance_runs() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(Options::default());
    let out = t.command(&Options::default()).output().unwrap();
    assert!(
        out.status.success(),
        "second instance exit {:?}",
        out.status
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "cyberghost-indicator is already running"
    );
    assert!(
        t.wait_exit(Duration::from_millis(300)).is_none(),
        "first instance still running"
    );
}

#[test]
fn already_running_message_is_translated() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(Options::default());
    let opts = Options {
        lang: vec![("LANG", "fr_FR.UTF-8")],
        ..Default::default()
    };
    let out = t.command(&opts).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "cyberghost-indicator est déjà lancé"
    );
}

#[test]
fn quit_menu_item_exits_cleanly() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Quit"]);
    let status = t.wait_exit(TIMEOUT).expect("did not exit");
    assert!(status.success(), "{status:?}");
    // the lock is released: a new instance can start
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
}

#[test]
fn version_and_help() {
    let out = std::process::Command::new(BIN)
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("cyberghost-indicator {}", env!("CARGO_PKG_VERSION"))
    );
    let out = std::process::Command::new(BIN).arg("-h").output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for s in [
        "Usage:",
        "config.json",
        "\"language\"",
        "CYBERGHOST_BIN",
        "\"fr\"",
    ] {
        assert!(help.contains(s), "help lacks {s:?}:\n{help}");
    }
}

#[test]
fn unknown_argument_is_an_error() {
    let out = std::process::Command::new(BIN)
        .arg("--frobnicate")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown argument: --frobnicate"));
}

#[test]
fn missing_icons_fall_back_to_theme_icon() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(Options {
        icons: false,
        ..Default::default()
    });
    assert!(
        wait_until(TIMEOUT, || t.icon() == "network-vpn"),
        "icon {}",
        t.icon()
    );
    assert!(
        t.menu()
            .path(&["Countries"])
            .children
            .iter()
            .all(|c| !c.has_icon)
    );
}

#[test]
fn icons_are_absolute_paths() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    assert!(t.icon_full().starts_with('/'), "{}", t.icon_full());
}

#[test]
fn missing_cli_is_reported() {
    let mut t = TestEnv::new();
    t.start(Options {
        cli_path: Some("/nonexistent/cyberghostvpn".into()),
        ..Default::default()
    });
    let n = t.wait_notification("CyberGhost not found");
    assert!(n.urgent);
    assert_eq!(n.body, "/nonexistent/cyberghostvpn is not installed");
    // no cache + no CLI: the list fetch fails and says why
    let n = t.wait_notification("Could not refresh the lists");
    assert!(
        n.body
            .starts_with("country list: Cannot run /nonexistent/cyberghostvpn:"),
        "{}",
        n.body
    );
    assert_eq!(t.menu().path(&["Countries"]).labels(), ["Loading…"]);
    assert!(!t.menu().path(&["Countries", "Loading…"]).enabled);
}

// ------------------------------------------------------------------ idle efficiency
/// Context switches per thread (a thread that exits between two samples
/// must not make the total go down).
fn wakeups(pid: u32) -> std::collections::HashMap<String, u64> {
    std::fs::read_dir(format!("/proc/{pid}/task"))
        .unwrap()
        .flatten()
        .map(|t| {
            let n = std::fs::read_to_string(t.path().join("status"))
                .unwrap_or_default()
                .lines()
                .filter(|l| l.contains("ctxt_switches"))
                .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
                .sum::<u64>();
            (t.file_name().to_string_lossy().into_owned(), n)
        })
        .collect()
}

fn wakeups_between(
    a: &std::collections::HashMap<String, u64>,
    b: &std::collections::HashMap<String, u64>,
) -> u64 {
    b.iter()
        .map(|(tid, n)| n - a.get(tid).copied().unwrap_or(0))
        .sum()
}

fn cpu_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let f: Vec<&str> = stat
        .rsplit(')')
        .next()
        .unwrap()
        .split_whitespace()
        .collect();
    f[11].parse::<u64>().unwrap() + f[12].parse::<u64>().unwrap() // utime + stime
}

fn settled(connected: bool) -> TestEnv {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    if connected {
        t.set_mock("connected", "CH");
        t.write_cache("state.json", &json!({"cc": "CH", "label": "Switzerland"}));
    }
    t.start(Options::default());
    t.wait_icon(if connected {
        "flag-CH.png"
    } else {
        "ghost-off.png"
    });
    std::thread::sleep(Duration::from_millis(500)); // startup work done
    t
}

fn assert_idle(t: &TestEnv) {
    let pid = t.pid();
    let calls = t.cli_calls().len();
    let (w0, c0) = (wakeups(pid), cpu_ticks(pid));
    std::thread::sleep(Duration::from_secs(4));
    let (w1, c1) = (wakeups(pid), cpu_ticks(pid));
    let woke = wakeups_between(&w0, &w1);
    assert_eq!(
        t.cli_calls().len(),
        calls,
        "no command while idle: {:?}",
        t.cli_calls()
    );
    assert!(woke <= 2, "{} wakeups in 4 s while idle", woke);
    assert_eq!(c1 - c0, 0, "CPU used while idle");
    // main loop, tray (tokio), netlink watcher, plus tokio's blocking-pool
    // thread from startup, which exits by itself after 10 s idle
    let threads = std::fs::read_dir(format!("/proc/{pid}/task"))
        .unwrap()
        .count();
    assert!(threads <= 4, "{threads} threads");
}

#[test]
fn idle_when_disconnected() {
    let t = settled(false);
    assert_eq!(
        t.calls_with("--status").len(),
        1,
        "one check at startup only"
    );
    assert_idle(&t);
}

#[test]
fn idle_when_connected() {
    let t = settled(true);
    assert_idle(&t);
}

#[test]
fn idle_after_a_connection_cycle() {
    let t = settled(false);
    t.click(&["Countries", "France  (FR)"]);
    t.wait_icon("flag-FR.png");
    t.click(&["Disconnect"]);
    t.wait_icon("ghost-off.png");
    std::thread::sleep(Duration::from_millis(500));
    assert_idle(&t); // the hourglass timer is gone
}

#[test]
fn periodic_status_check_can_be_enabled() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(Options {
        config: Some(json!({"status_interval": 1})),
        ..Default::default()
    });
    assert!(wait_until(TIMEOUT, || t.calls_with("--status").len() >= 3));
}
