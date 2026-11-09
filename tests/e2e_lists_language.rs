//! Refreshing the lists, and language selection.
mod support;
use serde_json::json;
use support::*;

fn opts_lang(pairs: &[(&'static str, &'static str)]) -> Options {
    Options {
        lang: pairs.to_vec(),
        ..Default::default()
    }
}

// ------------------------------------------------------------------ lists
#[test]
fn manual_refresh_notifies_the_counts() {
    let mut t = TestEnv::new();
    t.write_cache(
        "lists.json",
        &json!({"traffic": [["France", "FR"]], "streaming": [["Netflix US", "US"]], "torrent": [["France", "FR"]]}),
    );
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    assert_eq!(t.menu().path(&["Countries"]).children.len(), 1);
    t.click(&["Refresh lists"]);
    let n = t.wait_notification("Lists updated");
    assert_eq!(
        n.body,
        "5 countries, 5 streaming services, 3 torrent countries"
    );
    assert!(!n.urgent);
    assert!(
        wait_until(TIMEOUT, || t.menu().path(&["Countries"]).children.len()
            == 5),
        "menu updated"
    );
    assert_eq!(
        t.read_json(&t.cache_file("lists.json"))["traffic"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn failed_refresh_keeps_the_old_lists() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.set_mock("lists_mode", "garbage");
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Refresh lists"]);
    let n = t.wait_notification("Could not refresh the lists");
    assert!(n.urgent);
    assert_eq!(n.body, "country list: Error: API unreachable");
    assert_eq!(t.menu().path(&["Countries"]).children.len(), 5);
    assert_eq!(
        t.read_json(&t.cache_file("lists.json")),
        cached_lists(),
        "cache untouched"
    );
    assert!(t.menu().path(&["Refresh lists"]).enabled, "can retry");
}

#[test]
fn failed_streaming_list_is_reported_as_such() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.set_mock("lists_mode", "traffic_only");
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Refresh lists"]);
    assert_eq!(
        t.wait_notification("Could not refresh the lists").body,
        "streaming list: Error: streaming API down"
    );
}

#[test]
fn failed_torrent_list_is_reported_as_such() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.set_mock("lists_mode", "torrent_down");
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Refresh lists"]);
    assert_eq!(
        t.wait_notification("Could not refresh the lists").body,
        "torrent list: Error: torrent API down"
    );
}

#[test]
fn cache_from_an_older_version_is_completed_at_startup() {
    // lists.json written before torrent support: no "torrent" key
    let mut t = TestEnv::new();
    let mut old = cached_lists();
    old.as_object_mut().unwrap().remove("torrent");
    t.write_cache("lists.json", &old);
    t.start(Options::default());
    assert!(
        wait_until(TIMEOUT, || t.menu().path(&["Torrent"]).children.len() == 3),
        "torrent list fetched"
    );
    assert_eq!(
        t.read_json(&t.cache_file("lists.json"))["torrent"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(t.notifications().is_empty(), "automatic refresh is silent");
}

#[test]
fn first_fetch_failure_is_reported_and_menu_shows_loading() {
    let mut t = TestEnv::new();
    t.set_mock("lists_mode", "garbage");
    t.start(Options::default());
    let n = t.wait_notification("Could not refresh the lists");
    assert_eq!(n.body, "country list: Error: API unreachable");
    let m = t.menu();
    assert_eq!(m.path(&["Countries"]).labels(), ["Loading…"]);
    assert_eq!(m.path(&["Streaming"]).labels(), ["Loading…"]);
    // fixed: refresh works
    t.clear_mock("lists_mode");
    t.click(&["Refresh lists"]);
    t.wait_notification("Lists updated");
    assert_eq!(t.menu().path(&["Countries"]).children.len(), 5);
}

// ------------------------------------------------------------------ language
const EN_TOP: [&str; 7] = [
    "Disconnect",
    "Countries",
    "Streaming",
    "Refresh lists",
    "Language",
    "Quit",
    "Disconnected",
];
const FR_TOP: [&str; 7] = [
    "Se déconnecter",
    "Pays",
    "Streaming",
    "Actualiser les listes",
    "Langue",
    "Quitter",
    "Déconnecté",
];

fn assert_top(t: &TestEnv, want: &[&str]) {
    let m = t.menu();
    for l in want {
        assert!(
            m.children.iter().any(|c| c.label == *l),
            "missing {l:?} in {:?}",
            m.labels()
        );
    }
}

fn started(opts: Options) -> TestEnv {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(opts);
    t.wait_icon("ghost-off.png");
    assert!(wait_until(TIMEOUT, || t.status() != "…"));
    t
}

#[test]
fn follows_a_french_system_locale() {
    let t = started(opts_lang(&[("LANG", "fr_FR.UTF-8")]));
    t.wait_status("Déconnecté");
    assert_top(&t, &FR_TOP);
    assert_eq!(t.title(), "CyberGhost VPN — Déconnecté");
}

#[test]
fn english_system_locale() {
    let t = started(opts_lang(&[("LANG", "en_GB.UTF-8")]));
    t.wait_status("Disconnected");
    assert_top(&t, &EN_TOP);
}

#[test]
fn unsupported_locale_falls_back_to_english() {
    let t = started(opts_lang(&[("LANG", "de_DE.UTF-8")]));
    t.wait_status("Disconnected");
    assert_top(&t, &EN_TOP);
}

#[test]
fn no_locale_at_all_is_english() {
    let t = started(opts_lang(&[]));
    t.wait_status("Disconnected");
}

#[test]
fn language_variable_has_priority_over_lang() {
    let t = started(opts_lang(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "fr:en")]));
    t.wait_status("Déconnecté");
}

#[test]
fn lc_all_c_forces_english() {
    let t = started(opts_lang(&[
        ("LC_ALL", "C"),
        ("LANGUAGE", "fr"),
        ("LANG", "fr_FR.UTF-8"),
    ]));
    t.wait_status("Disconnected");
}

#[test]
fn lc_messages_beats_lang() {
    let t = started(opts_lang(&[
        ("LC_MESSAGES", "fr_FR.UTF-8"),
        ("LANG", "en_US.UTF-8"),
    ]));
    t.wait_status("Déconnecté");
}

#[test]
fn language_menu_shows_choices_with_current_selection() {
    let t = started(opts_lang(&[("LANG", "fr_FR.UTF-8")]));
    let lang = t.menu().path(&["Langue"]).clone();
    assert_eq!(
        lang.labels(),
        ["Langue du système (Français)", "English", "Français"]
    );
    let states: Vec<_> = lang.children.iter().map(|c| c.toggle_state).collect();
    assert_eq!(
        states,
        [Some(1), Some(0), Some(0)],
        "system default selected"
    );
}

#[test]
fn choosing_a_language_applies_immediately_and_persists() {
    let mut t = started(opts_lang(&[("LANG", "en_US.UTF-8")]));
    t.click(&["Language", "Français"]);
    t.wait_status("Déconnecté");
    assert_top(&t, &FR_TOP);
    assert_eq!(t.read_json(&t.config_file())["language"], "fr");
    let states: Vec<_> = t
        .menu()
        .path(&["Langue"])
        .children
        .iter()
        .map(|c| c.toggle_state)
        .collect();
    assert_eq!(states, [Some(0), Some(0), Some(1)]);

    // kept after a restart, even though the system is English
    t.stop();
    t.start(opts_lang(&[("LANG", "en_US.UTF-8")]));
    t.wait_status("Déconnecté");

    // back to the system language
    t.click(&["Langue", "Langue du système (English)"]);
    t.wait_status("Disconnected");
    assert_eq!(
        t.read_json(&t.config_file()),
        json!({}),
        "the system default is not written to the file"
    );
}

#[test]
fn notifications_use_the_chosen_language() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(Options {
        config: Some(json!({"language": "fr"})),
        ..Default::default()
    }); // LANG=en
    t.wait_status("Déconnecté");
    t.click(&["Pays", "France  (FR)"]);
    t.wait_notification("VPN connecté : France");
    t.wait_status("Connecté : France");
    t.click(&["Se déconnecter"]);
    t.wait_notification("VPN déconnecté");
    t.set_mock("connect_mode", "hang");
    t.stop();
    t.start(Options {
        config: Some(json!({"language": "fr", "connect_timeout": 1})),
        ..Default::default()
    });
    t.wait_status("Déconnecté");
    t.click(&["Pays", "France  (FR)"]);
    assert_eq!(
        t.wait_notification("Échec de la connexion à France").body,
        "Délai dépassé (1 s)"
    );
}

#[test]
fn invalid_language_in_config_uses_the_system_language() {
    let mut t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t.start(Options {
        config: Some(json!({"language": "tlh"})),
        lang: vec![("LANG", "fr_FR.UTF-8")],
        ..Default::default()
    });
    t.wait_status("Déconnecté");
}

#[test]
fn changing_language_keeps_other_settings() {
    let t = started(Options {
        config: Some(json!({"connect_timeout": 7, "status_interval": 9})),
        ..Default::default()
    });
    t.click(&["Language", "Français"]);
    t.wait_status("Déconnecté");
    let cfg = t.read_json(&t.config_file());
    assert_eq!(
        cfg,
        json!({"language": "fr", "connect_timeout": 7, "status_interval": 9})
    );
}
