//! End-to-end test harness.
//!
//! Each `TestEnv` gets: a private D-Bus session bus (`dbus-daemon`), a fake tray
//! host (StatusNotifierWatcher), and fake `cyberghostvpn` / `sudo` / `pkexec` /
//! `notify-send` scripts that log their calls. The real binary is started with
//! a clean environment pointing at all of that, and driven over D-Bus exactly
//! like a panel would (dbusmenu `GetLayout` / `Event`, SNI properties).
#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

pub const BIN: &str = env!("CARGO_BIN_EXE_cyberghost-indicator");
pub const TIMEOUT: Duration = Duration::from_secs(15);

const MOCK_CLI: &str = r#"#!/bin/bash
# Fake cyberghostvpn. Behaviour is driven by files in $MOCK_DIR:
#   connected        exists when "connected" (content: country code)
#   connect_mode     ok | fail | slow | hang | silent   (default ok)
#   stop_mode        ok | fail                          (default ok)
#   lists_mode       ok | garbage | traffic_only | torrent_down   (default ok)
#   status_mode      normal | unknown                   (default normal)
D="$MOCK_DIR"
printf '%s\x1f' "$@" >> "$D/calls.log"; echo >> "$D/calls.log"
mode() { cat "$D/$1" 2>/dev/null || echo "$2"; }
has() { local x="$1"; shift; for a in "$@"; do [ "$a" = "$x" ] && return 0; done; return 1; }
after() { local x="$1"; shift; while [ $# -gt 0 ]; do [ "$1" = "$x" ] && { echo "$2"; return; }; shift; done; }

if has --status "$@"; then
    [ "$(mode status_mode normal)" = unknown ] && { echo "Unexpected output"; exit 0; }
    if [ -f "$D/connected" ]; then echo "VPN connection found."; exit 0; fi
    echo "No VPN connections found."; exit 1
fi
if has --stop "$@"; then
    if [ "$(mode stop_mode ok)" = fail ]; then echo "Error: could not terminate OpenVPN"; exit 0; fi
    rm -f "$D/connected"
    echo "OpenVPN process found, terminating all OpenVPN process ..."
    echo "All OpenVPN processes are terminated!"; exit 0
fi
if has --connect "$@"; then
    cc="$(after --country-code "$@")"
    case "$(mode connect_mode ok)" in
        fail)
            echo 'Traceback (most recent call last):'
            echo '  File "requests/models.py", line 1120, in json'
            echo 'requests.exceptions.JSONDecodeError: Expecting value: line 1 column 1 (char 0)'
            echo 'Connection failed: Expecting value: line 1 column 1 (char 0)'
            exit 0 ;;
        silent) exit 0 ;;
        hang) exec sleep 60 ;;
        slow) sleep 2 ;;
    esac
    echo "$cc" > "$D/connected"
    echo "Select server ... test-s01"; echo "Connecting ... "; echo "VPN connection established."
    exit 0
fi
if has --country-code "$@"; then
    lm="$(mode lists_mode ok)"
    [ "$lm" = garbage ] && { echo "Error: API unreachable"; exit 0; }
    if has --traffic "$@"; then
        cat <<'T'
+-----+----------------------+--------------+
| No. |     Country Name     | Country Code |
+-----+----------------------+--------------+
|  1  |     Switzerland      |      CH      |
|  2  |        France        |      FR      |
|  3  |       Germany        |      DE      |
|  4  |    United States     |      US      |
|  5  |       Andorra        |      AD      |
+-----+----------------------+--------------+
T
    elif has --streaming "$@"; then
        [ "$lm" = traffic_only ] && { echo "Error: streaming API down"; exit 0; }
        cat <<'S'
+-----+-----------------------+--------------+
| No. |        Service        | Country Code |
+-----+-----------------------+--------------+
|  1  |       Netflix US      |      US      |
|  2  |     Disney Plus FR    |      FR      |
|  3  |      Apple TV US      |      US      |
|  4  |        Zee5 IN        |      IN      |
|  5  |      Test Svc ZZ      |      ZZ      |
+-----+-----------------------+--------------+
S
    elif has --torrent "$@"; then
        [ "$lm" = torrent_down ] && { echo "Error: torrent API down"; exit 0; }
        cat <<'R'
+-----+----------------------+--------------+
| No. |     Country Name     | Country Code |
+-----+----------------------+--------------+
|  1  |     Switzerland      |      CH      |
|  2  |     Netherlands      |      NL      |
|  3  |        France        |      FR      |
+-----+----------------------+--------------+
R
    fi
    exit 0
fi
echo "unknown arguments: $*"; exit 1
"#;

const FAKE_SUDO: &str = r#"#!/bin/bash
# Fake sudo: `sudo -n -l <cmd>` succeeds unless $MOCK_DIR/sudo_denied exists;
# `sudo -n <cmd…>` runs the command as the current user.
printf 'sudo\x1f%s\n' "$*" >> "$MOCK_DIR/root.log"
[ "$1" = -n ] && shift
if [ "$1" = -l ]; then [ -f "$MOCK_DIR/sudo_denied" ] && exit 1; exit 0; fi
[ -f "$MOCK_DIR/sudo_denied" ] && { echo "sudo: a password is required" >&2; exit 1; }
exec "$@"
"#;

const FAKE_PKEXEC: &str = r#"#!/bin/bash
printf 'pkexec\x1f%s\n' "$*" >> "$MOCK_DIR/root.log"
exec "$@"
"#;

const FAKE_ZENITY: &str = r#"#!/bin/bash
# Fake zenity checklist. Records its options and the rows it receives (the
# cells after "--separator <sep>", 4 per row), then
# "ticks" the rows listed in $MOCK_DIR/zenity_select ("Country|Server" lines),
# or keeps the pre-ticked rows when that file is absent.
#   zenity_mode: ok | cancel | slow   (default ok)
D="$MOCK_DIR"
opts=(); while [ $# -gt 0 ]; do
    opts+=("$1"); [ "$1" = --separator ] && { opts+=("$2"); shift 2; break; }; shift
done
printf '%s\x1f' "${opts[@]}" > "$D/zenity_args.log"
printf '%s\n' "$@" > "$D/zenity_rows.log"
[ -t 0 ] || [ -z "$(cat)" ] || echo "unexpected stdin" >> "$D/zenity_errors.log"
echo run >> "$D/zenity_runs.log"
mode="$(cat "$D/zenity_mode" 2>/dev/null || echo ok)"
[ "$mode" = slow ] && sleep 2
[ "$mode" = cancel ] && exit 1
while IFS= read -r check && IFS= read -r country && IFS= read -r server && IFS= read -r idx; do
    if [ -f "$D/zenity_select" ]; then
        grep -qxF -- "$country|$server" "$D/zenity_select" && echo "$idx"
    else
        [ "$check" = TRUE ] && echo "$idx"
    fi
done < "$D/zenity_rows.log"
exit 0
"#;

const FAKE_NOTIFY: &str = r#"#!/bin/bash
# one record per call: args separated by \x1f, records by \x1e (bodies may contain newlines)
printf '%s\x1f' "$@" >> "$MOCK_DIR/notify.log"; printf '\x1e' >> "$MOCK_DIR/notify.log"
"#;

// ------------------------------------------------------------------ fake tray host
struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String) {
        self.items.lock().unwrap().push(service);
    }
    fn register_status_notifier_host(&self, _service: String) {}
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

// ------------------------------------------------------------------ menu model
#[derive(Debug, Clone)]
pub struct Node {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub has_icon: bool,
    pub toggle_state: Option<i32>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn find(&self, label: &str) -> Option<&Node> {
        if self.label == label {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(label))
    }

    /// Follow a path of labels from this node's direct children.
    pub fn path(&self, path: &[&str]) -> &Node {
        let mut n = self;
        for p in path {
            n = n
                .children
                .iter()
                .find(|c| c.label == *p)
                .unwrap_or_else(|| panic!("no {p:?} under {:?}: {:?}", n.label, n.labels()));
        }
        n
    }

    pub fn labels(&self) -> Vec<String> {
        self.children
            .iter()
            .filter(|c| c.visible)
            .map(|c| c.label.clone())
            .collect()
    }
}

fn to_node(v: &Value) -> Node {
    match v {
        Value::Value(inner) => to_node(inner),
        Value::Structure(s) => {
            let f = s.fields();
            let id = match &f[0] {
                Value::I32(i) => *i,
                other => panic!("bad id {other:?}"),
            };
            let props: HashMap<String, OwnedValue> = match &f[1] {
                Value::Dict(d) => d.try_clone().unwrap().try_into().unwrap(),
                other => panic!("bad props {other:?}"),
            };
            let children = match &f[2] {
                Value::Array(a) => a.iter().map(to_node).collect(),
                other => panic!("bad children {other:?}"),
            };
            make_node(id, props, children)
        }
        other => panic!("unexpected layout value {other:?}"),
    }
}

fn make_node(id: i32, props: HashMap<String, OwnedValue>, children: Vec<Node>) -> Node {
    let get_str = |k: &str| {
        props
            .get(k)
            .and_then(|v| String::try_from(v.try_clone().unwrap()).ok())
    };
    let get_bool = |k: &str, d: bool| {
        props
            .get(k)
            .and_then(|v| bool::try_from(v.try_clone().unwrap()).ok())
            .unwrap_or(d)
    };
    Node {
        id,
        label: get_str("label").unwrap_or_default(),
        enabled: get_bool("enabled", true),
        visible: get_bool("visible", true),
        has_icon: props.contains_key("icon-data"),
        toggle_state: props
            .get("toggle-state")
            .and_then(|v| i32::try_from(v.try_clone().unwrap()).ok()),
        children,
    }
}

// ------------------------------------------------------------------ notifications
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub icon: String,
    pub urgent: bool,
    pub summary: String,
    pub body: String,
}

fn parse_notification(line: &str) -> Notification {
    let mut args: Vec<&str> = line.split('\x1f').collect();
    if args.last() == Some(&"") {
        args.pop();
    }
    let mut n = Notification {
        icon: String::new(),
        urgent: false,
        summary: String::new(),
        body: String::new(),
    };
    let mut pos = vec![];
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "-a" => i += 1,
            "-i" => {
                n.icon = args[i + 1].to_string();
                i += 1
            }
            "-u" => {
                n.urgent = args[i + 1] == "critical";
                i += 1
            }
            a => pos.push(a),
        }
        i += 1;
    }
    n.summary = pos.first().unwrap_or(&"").to_string();
    n.body = pos.get(1).unwrap_or(&"").to_string();
    n
}

// ------------------------------------------------------------------ environment
pub struct Options {
    pub lang: Vec<(&'static str, &'static str)>,
    pub config: Option<serde_json::Value>,
    pub icons: bool,
    pub cli_path: Option<String>,
    /// dialog program (default: the fake zenity)
    pub zenity: Option<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            lang: vec![("LANG", "en_US.UTF-8")],
            config: None,
            icons: true,
            cli_path: None,
            zenity: None,
        }
    }
}

pub struct TestEnv {
    pub dir: tempfile::TempDir,
    bus: Child,
    pub address: String,
    pub conn: Connection,
    _watcher: Connection,
    pub app: Option<Child>,
    pub opts_env: Vec<(String, String)>,
}

pub fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < timeout {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    f()
}

fn write_exe(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

impl TestEnv {
    pub fn new() -> TestEnv {
        let dir = tempfile::tempdir().unwrap();
        for d in ["bin", "mock", "home", "cache", "config", "work"] {
            fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let bin = dir.path().join("bin");
        write_exe(&bin.join("cyberghostvpn"), MOCK_CLI);
        write_exe(&bin.join("sudo"), FAKE_SUDO);
        write_exe(&bin.join("pkexec"), FAKE_PKEXEC);
        write_exe(&bin.join("notify-send"), FAKE_NOTIFY);
        write_exe(&bin.join("zenity"), FAKE_ZENITY);

        let mut bus = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon is required for end-to-end tests");
        let mut address = String::new();
        BufReader::new(bus.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_string();

        let watcher = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .serve_at(
                "/StatusNotifierWatcher",
                Watcher {
                    items: Arc::default(),
                },
            )
            .unwrap()
            .build()
            .unwrap();
        let conn = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .build()
            .unwrap();
        TestEnv {
            dir,
            bus,
            address,
            conn,
            _watcher: watcher,
            app: None,
            opts_env: vec![],
        }
    }

    pub fn path(&self, p: &str) -> PathBuf {
        self.dir.path().join(p)
    }
    pub fn mock(&self, file: &str) -> PathBuf {
        self.path("mock").join(file)
    }
    pub fn set_mock(&self, file: &str, value: &str) {
        fs::write(self.mock(file), value).unwrap();
    }
    pub fn clear_mock(&self, file: &str) {
        let _ = fs::remove_file(self.mock(file));
    }
    pub fn cache_file(&self, f: &str) -> PathBuf {
        self.path("cache/cyberghost-indicator").join(f)
    }
    pub fn config_file(&self) -> PathBuf {
        self.path("config/cyberghost-indicator/config.json")
    }
    pub fn favorites_file(&self) -> PathBuf {
        self.path("config/cyberghost-indicator/favorites.json")
    }
    pub fn write_favorites(&self, v: &serde_json::Value) {
        let p = self.favorites_file();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, v.to_string()).unwrap();
    }
    pub fn write_cache(&self, f: &str, v: &serde_json::Value) {
        let p = self.cache_file(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, v.to_string()).unwrap();
    }
    pub fn read_json(&self, p: &Path) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(p).unwrap_or_default())
            .unwrap_or(serde_json::Value::Null)
    }

    pub fn command(&self, opts: &Options) -> Command {
        let mut c = Command::new(BIN);
        c.env_clear()
            .current_dir(self.path("work"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.path("bin").display()),
            )
            .env("HOME", self.path("home"))
            .env("USER", "tester")
            .env("XDG_CACHE_HOME", self.path("cache"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
            .env("MOCK_DIR", self.path("mock"))
            .env(
                "CYBERGHOST_BIN",
                opts.cli_path
                    .clone()
                    .unwrap_or_else(|| self.path("bin/cyberghostvpn").display().to_string()),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c.env(
            "CYBERGHOST_INDICATOR_ZENITY",
            opts.zenity
                .clone()
                .unwrap_or_else(|| self.path("bin/zenity").display().to_string()),
        );
        if opts.icons {
            c.env(
                "CYBERGHOST_INDICATOR_ICONS",
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/icons"),
            );
        }
        for (k, v) in &opts.lang {
            c.env(k, v);
        }
        c
    }

    /// Start the app and wait until it is on the bus.
    pub fn start(&mut self, opts: Options) -> &mut TestEnv {
        if let Some(cfg) = &opts.config {
            fs::create_dir_all(self.config_file().parent().unwrap()).unwrap();
            fs::write(self.config_file(), cfg.to_string()).unwrap();
        }
        let child = self.command(&opts).spawn().unwrap();
        self.app = Some(child);
        let ok = wait_until(TIMEOUT, || self.name().is_some_and(|n| self.has_owner(&n)));
        assert!(ok, "indicator did not appear on the bus");
        self
    }

    pub fn pid(&self) -> u32 {
        self.app.as_ref().expect("app not started").id()
    }

    pub fn name(&self) -> Option<String> {
        self.app
            .as_ref()
            .map(|a| format!("org.kde.StatusNotifierItem-{}-1", a.id()))
    }

    fn has_owner(&self, name: &str) -> bool {
        Proxy::new(
            &self.conn,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .and_then(|p| p.call::<_, _, bool>("NameHasOwner", &(name,)))
        .unwrap_or(false)
    }

    fn sni(&self) -> Proxy<'_> {
        Proxy::new(
            &self.conn,
            self.name().unwrap(),
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
        )
        .unwrap()
    }

    fn menu_proxy(&self) -> Proxy<'_> {
        Proxy::new(
            &self.conn,
            self.name().unwrap(),
            "/MenuBar",
            "com.canonical.dbusmenu",
        )
        .unwrap()
    }

    /// Bare file name of the current icon (or the theme icon name).
    pub fn icon(&self) -> String {
        let full: String = self.sni().get_property("IconName").unwrap();
        full.rsplit('/').next().unwrap().to_string()
    }

    pub fn icon_full(&self) -> String {
        self.sni().get_property("IconName").unwrap()
    }

    pub fn title(&self) -> String {
        self.sni().get_property("Title").unwrap()
    }

    pub fn menu(&self) -> Node {
        type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);
        let (_rev, (id, props, children)): (u32, Layout) = self
            .menu_proxy()
            .call("GetLayout", &(0i32, -1i32, Vec::<String>::new()))
            .unwrap();
        make_node(id, props, children.iter().map(|c| to_node(c)).collect())
    }

    /// Status line (first menu entry).
    pub fn status(&self) -> String {
        self.menu().children[0].label.clone()
    }

    pub fn click_id(&self, id: i32) {
        let _: () = self
            .menu_proxy()
            .call("Event", &(id, "clicked", Value::from(0i32), 0u32))
            .unwrap_or(());
    }

    /// Click the entry at `path` (labels from the menu root).
    pub fn click(&self, path: &[&str]) {
        let id = self.menu().path(path).id;
        self.click_id(id);
    }

    /// Wait for an icon (file name, e.g. "flag-CH.png"); returns every icon seen meanwhile.
    pub fn wait_icon(&self, want: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let ok = wait_until(TIMEOUT, || {
            let i = self.icon();
            seen.insert(i.clone());
            i == want
        });
        assert!(ok, "icon {want} never shown; seen {seen:?}");
        seen
    }

    pub fn wait_status(&self, want: &str) {
        let mut last = String::new();
        let ok = wait_until(TIMEOUT, || {
            last = self.status();
            last == want
        });
        assert!(ok, "status {want:?} never shown; last {last:?}");
    }

    /// CLI invocations (each a list of arguments).
    pub fn cli_calls(&self) -> Vec<Vec<String>> {
        fs::read_to_string(self.mock("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| {
                l.split('\x1f')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            })
            .collect()
    }

    pub fn calls_with(&self, arg: &str) -> Vec<Vec<String>> {
        self.cli_calls()
            .into_iter()
            .filter(|c| c.iter().any(|a| a == arg))
            .collect()
    }

    pub fn root_log(&self) -> Vec<String> {
        fs::read_to_string(self.mock("root.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    pub fn notifications(&self) -> Vec<Notification> {
        fs::read_to_string(self.mock("notify.log"))
            .unwrap_or_default()
            .split('\x1e')
            .filter(|l| !l.is_empty())
            .map(parse_notification)
            .collect()
    }

    pub fn wait_notification(&self, summary: &str) -> Notification {
        let mut found = None;
        let ok = wait_until(TIMEOUT, || {
            found = self
                .notifications()
                .into_iter()
                .find(|n| n.summary == summary);
            found.is_some()
        });
        assert!(
            ok,
            "no notification {summary:?}; got {:?}",
            self.notifications()
        );
        found.unwrap()
    }

    /// Rows the favorites window received: (ticked, country, server).
    pub fn dialog_rows(&self) -> Vec<(bool, String, String)> {
        let data = fs::read_to_string(self.mock("zenity_rows.log")).unwrap_or_default();
        let lines: Vec<&str> = data.lines().collect();
        lines
            .chunks(4)
            .map(|c| (c[0] == "TRUE", c[1].to_string(), c[2].to_string()))
            .collect()
    }

    pub fn dialog_args(&self) -> Vec<String> {
        fs::read_to_string(self.mock("zenity_args.log"))
            .unwrap_or_default()
            .split('\x1f')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    }

    pub fn dialog_runs(&self) -> usize {
        fs::read_to_string(self.mock("zenity_runs.log"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let app = self.app.as_mut()?;
        let t = Instant::now();
        while t.elapsed() < timeout {
            if let Ok(Some(s)) = app.try_wait() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    pub fn stop(&mut self) {
        if let Some(mut a) = self.app.take() {
            let _ = a.kill();
            let _ = a.wait();
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        self.stop();
        let _ = self.bus.kill();
        let _ = self.bus.wait();
    }
}

/// Cached lists matching the mock CLI.
pub fn cached_lists() -> serde_json::Value {
    serde_json::json!({
        "traffic": [["Andorra","AD"],["France","FR"],["Germany","DE"],["Switzerland","CH"],["United States","US"]],
        "streaming": [["Netflix US","US"],["Disney Plus FR","FR"],["Apple TV US","US"],["Zee5 IN","IN"],["Test Svc ZZ","ZZ"]],
        "torrent": [["France","FR"],["Netherlands","NL"],["Switzerland","CH"]]
    })
}
