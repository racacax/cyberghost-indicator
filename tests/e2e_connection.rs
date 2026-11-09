//! Connecting, switching, disconnecting, errors, privilege escalation, external changes.
mod support;
use serde_json::json;
use std::time::Duration;
use support::*;

fn env_with_lists() -> TestEnv {
    let t = TestEnv::new();
    t.write_cache("lists.json", &cached_lists());
    t
}

fn connected_to_ch() -> TestEnv {
    let mut t = env_with_lists();
    t.write_cache("state.json", &json!({"cc": "CH", "label": "Switzerland"}));
    t.set_mock("connected", "CH");
    t.start(Options::default());
    t.wait_icon("flag-CH.png");
    t
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn connect_to_a_country() {
    let mut t = env_with_lists();
    t.set_mock("connect_mode", "slow"); // long enough to watch the busy state
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Countries", "Switzerland  (CH)"]);

    t.wait_status("Connecting to Switzerland…");
    let m = t.menu();
    assert!(!m.path(&["Countries"]).enabled, "menu locked while busy");
    assert!(!m.path(&["Streaming"]).enabled);
    assert!(!m.path(&["Disconnect"]).enabled);
    let seen = t.wait_icon("flag-CH.png");
    assert!(
        seen.contains("busy-1.png") && seen.contains("busy-2.png"),
        "hourglass animates: {seen:?}"
    );

    assert_eq!(t.status(), "Connected: Switzerland");
    assert!(t.menu().path(&["Disconnect"]).enabled);
    assert!(t.menu().path(&["Countries"]).enabled);
    let n = t.wait_notification("VPN connected: Switzerland");
    assert!(!n.urgent);
    assert!(n.icon.ends_with("/flag-CH.png"), "{}", n.icon);
    assert_eq!(
        t.read_json(&t.cache_file("state.json")),
        json!({"cc": "CH", "label": "Switzerland"})
    );
    assert_eq!(
        t.calls_with("--connect"),
        [args(&["--traffic", "--country-code", "CH", "--connect"])]
    );
    assert!(
        t.root_log().iter().any(|l| l.starts_with("sudo\x1f-n -l ")),
        "sudo rule checked"
    );
    assert!(
        t.root_log()
            .iter()
            .any(|l| l.contains("--traffic --country-code CH --connect")
                && l.starts_with("sudo\x1f-n ")),
        "connect ran through sudo -n: {:?}",
        t.root_log()
    );
    assert!(t.calls_with("--stop").is_empty(), "nothing to stop first");
}

#[test]
fn connect_to_a_streaming_service() {
    let mut t = env_with_lists();
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Streaming", "United States  (US)", "Netflix US"]);
    t.wait_icon("flag-US.png");
    assert_eq!(t.status(), "Connected: Netflix US");
    // the service name stays a single argument
    assert_eq!(
        t.calls_with("--connect"),
        [args(&[
            "--streaming",
            "Netflix US",
            "--country-code",
            "US",
            "--connect"
        ])]
    );
    t.wait_notification("VPN connected: Netflix US");
}

#[test]
fn country_without_flag_uses_question_flag() {
    let mut t = env_with_lists();
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    assert!(
        t.menu().path(&["Streaming", "ZZ  (ZZ)"]).has_icon,
        "fallback flag in menu"
    );
    t.click(&["Streaming", "ZZ  (ZZ)", "Test Svc ZZ"]);
    t.wait_icon("flag-unknown.png");
}

#[test]
fn failed_connection_reports_the_cli_error() {
    let mut t = env_with_lists();
    t.set_mock("connect_mode", "fail");
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Countries", "France  (FR)"]);
    let n = t.wait_notification("Could not connect to France");
    assert!(n.urgent);
    assert!(n.icon.ends_with("/error.png"));
    assert_eq!(
        n.body,
        "Connection failed: Expecting value: line 1 column 1 (char 0)"
    );
    t.wait_icon("ghost-off.png");
    t.wait_status("Disconnected");
    assert_eq!(t.read_json(&t.cache_file("state.json")), json!({}));
}

#[test]
fn silent_cli_failure_says_no_output() {
    let mut t = env_with_lists();
    t.set_mock("connect_mode", "silent");
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Countries", "France  (FR)"]);
    assert_eq!(
        t.wait_notification("Could not connect to France").body,
        "no output"
    );
}

#[test]
fn connection_timeout() {
    let mut t = env_with_lists();
    t.set_mock("connect_mode", "hang");
    t.start(Options {
        config: Some(json!({"connect_timeout": 1})),
        ..Default::default()
    });
    t.wait_icon("ghost-off.png");
    t.click(&["Countries", "France  (FR)"]);
    let n = t.wait_notification("Could not connect to France");
    assert_eq!(n.body, "Timed out (1 s)");
    t.wait_icon("ghost-off.png");
    assert!(
        t.menu().path(&["Countries"]).enabled,
        "usable again after the timeout"
    );
}

#[test]
fn switching_country_stops_the_current_connection_first() {
    let t = connected_to_ch();
    t.click(&["Countries", "France  (FR)"]);
    t.wait_icon("flag-FR.png");
    let calls: Vec<String> = t
        .cli_calls()
        .into_iter()
        .filter(|c| !c.contains(&"--status".to_string()))
        .map(|c| c.join(" "))
        .collect();
    assert_eq!(calls, ["--stop", "--traffic --country-code FR --connect"]);
    assert_eq!(t.status(), "Connected: France");
}

#[test]
fn failed_switch_says_the_previous_connection_was_closed() {
    let t = connected_to_ch();
    t.set_mock("connect_mode", "fail");
    t.click(&["Countries", "France  (FR)"]);
    let n = t.wait_notification("Could not connect to France");
    assert_eq!(
        n.body,
        "Connection failed: Expecting value: line 1 column 1 (char 0)\n(the previous connection was closed)"
    );
    t.wait_icon("ghost-off.png");
}

#[test]
fn disconnect() {
    let t = connected_to_ch();
    t.click(&["Disconnect"]);
    t.wait_icon("ghost-off.png");
    t.wait_status("Disconnected");
    let n = t.wait_notification("VPN disconnected");
    assert!(!n.urgent);
    assert!(n.icon.ends_with("/ghost-off.png"));
    assert_eq!(t.read_json(&t.cache_file("state.json")), json!({}));
    assert!(!t.menu().path(&["Disconnect"]).enabled);
    assert_eq!(t.calls_with("--stop").len(), 1);
}

#[test]
fn failed_disconnect_keeps_the_connection() {
    let t = connected_to_ch();
    t.set_mock("stop_mode", "fail");
    t.click(&["Disconnect"]);
    let n = t.wait_notification("Could not disconnect");
    assert!(n.urgent);
    assert_eq!(n.body, "Error: could not terminate OpenVPN");
    t.wait_icon("flag-CH.png");
    assert_eq!(t.status(), "Connected: Switzerland");
    assert_eq!(t.read_json(&t.cache_file("state.json"))["cc"], "CH");
}

#[test]
fn clicks_while_busy_are_ignored() {
    let mut t = env_with_lists();
    t.set_mock("connect_mode", "slow");
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    let m = t.menu();
    let ch = m.path(&["Countries", "Switzerland  (CH)"]).id;
    let fr = m.path(&["Countries", "France  (FR)"]).id;
    let refresh_or_disconnect = m.path(&["Disconnect"]).id;
    t.click_id(ch);
    t.wait_status("Connecting to Switzerland…");
    t.click_id(fr); // stale ids, as a panel would send them
    t.click_id(refresh_or_disconnect);
    t.wait_icon("flag-CH.png");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(t.calls_with("--connect").len(), 1, "{:?}", t.cli_calls());
    assert!(t.calls_with("--stop").is_empty());
}

#[test]
fn disconnect_is_ignored_when_not_connected() {
    let mut t = env_with_lists();
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Disconnect"]); // disabled, but a panel could still send the event
    std::thread::sleep(Duration::from_millis(500));
    assert!(t.calls_with("--stop").is_empty());
    assert!(t.notifications().is_empty());
}

#[test]
fn falls_back_to_pkexec_without_sudo_rule() {
    let mut t = env_with_lists();
    t.set_mock("sudo_denied", "");
    t.start(Options::default());
    t.wait_icon("ghost-off.png");
    t.click(&["Countries", "Germany  (DE)"]);
    t.wait_icon("flag-DE.png");
    let log = t.root_log();
    let pk = log
        .iter()
        .find(|l| l.starts_with("pkexec"))
        .unwrap_or_else(|| panic!("no pkexec: {log:?}"));
    let home = t.path("home");
    assert!(
        pk.contains(&format!("env SUDO_USER=tester HOME={}", home.display())),
        "{pk}"
    );
    assert!(
        pk.ends_with("--traffic --country-code DE --connect"),
        "{pk}"
    );
}

#[test]
fn notices_a_disconnection_made_outside_the_app() {
    let mut t = env_with_lists();
    t.write_cache("state.json", &json!({"cc": "CH", "label": "Switzerland"}));
    t.set_mock("connected", "CH");
    t.start(Options {
        config: Some(json!({"status_interval": 1})),
        ..Default::default()
    });
    t.wait_icon("flag-CH.png");
    t.clear_mock("connected"); // e.g. `cyberghostvpn --stop` in a terminal
    t.wait_icon("ghost-off.png");
    assert!(wait_until(TIMEOUT, || t
        .read_json(&t.cache_file("state.json"))
        == json!({})));
    assert!(
        t.notifications().is_empty(),
        "no notification for external changes"
    );
}

#[test]
fn notices_a_connection_made_outside_the_app() {
    let mut t = env_with_lists();
    t.start(Options {
        config: Some(json!({"status_interval": 1})),
        ..Default::default()
    });
    t.wait_icon("ghost-off.png");
    t.set_mock("connected", "FR");
    t.wait_icon("ghost-on.png");
    assert_eq!(t.status(), "Connected");
}

#[test]
fn unrecognised_status_output_does_not_crash() {
    let mut t = env_with_lists();
    t.set_mock("status_mode", "unknown");
    t.start(Options {
        config: Some(json!({"status_interval": 1})),
        ..Default::default()
    });
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        t.wait_exit(Duration::from_millis(100)).is_none(),
        "still running"
    );
    let icon = t.icon(); // depends on the host's interfaces: just a valid state
    assert!(icon == "ghost-off.png" || icon == "ghost-on.png", "{icon}");
}
