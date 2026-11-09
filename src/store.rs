//! Files on disk: user config, cache (lists, last connection), paths.

use crate::cli::Lists;
use crate::i18n::LangPref;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const APP: &str = "cyberghost-indicator";

#[derive(Debug, Clone)]
pub struct Paths {
    /// `$XDG_CACHE_HOME/cyberghost-indicator`: lists.json, state.json, lock
    pub cache: PathBuf,
    /// `$XDG_CONFIG_HOME/cyberghost-indicator`: config.json
    pub config: PathBuf,
}

/// `$VAR` when it is an absolute path (XDG spec: relative values are ignored), else `$HOME/<fallback>`.
fn xdg_dir(var: Option<String>, home: &Path, fallback: &str) -> PathBuf {
    var.map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(fallback))
}

impl Paths {
    pub fn resolve(env: impl Fn(&str) -> Option<String>) -> Paths {
        let home = PathBuf::from(env("HOME").unwrap_or_else(|| "/tmp".into()));
        Paths {
            cache: xdg_dir(env("XDG_CACHE_HOME"), &home, ".cache").join(APP),
            config: xdg_dir(env("XDG_CONFIG_HOME"), &home, ".config").join(APP),
        }
    }

    pub fn from_env() -> Paths {
        Paths::resolve(|k| std::env::var(k).ok())
    }

    pub fn lists_file(&self) -> PathBuf {
        self.cache.join("lists.json")
    }
    pub fn state_file(&self) -> PathBuf {
        self.cache.join("state.json")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.cache.join("lock")
    }
    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.json")
    }
    pub fn favorites_file(&self) -> PathBuf {
        self.config.join("favorites.json")
    }
}

/// Write atomically (temp file + rename), creating the directory.
pub fn write_json(path: &Path, v: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_string_pretty(v).unwrap_or_default() + "\n",
    )?;
    fs::rename(tmp, path)
}

/// `None` if missing or not valid JSON.
pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

// ---------------------------------------------------------------- config
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub language: LangPref,
    /// max duration of `cyberghostvpn --connect`
    pub connect_timeout: Duration,
    /// how often `cyberghostvpn --status` is checked; zero = never (default:
    /// interface changes trigger a check, see `netwatch`)
    pub status_interval: Duration,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            language: LangPref::System,
            connect_timeout: Duration::from_secs(120),
            status_interval: Duration::ZERO,
        }
    }
}

impl Config {
    /// Missing file, bad JSON or bad values → defaults for those fields.
    pub fn from_json(v: &Value) -> Config {
        let d = Config::default();
        let secs = |key: &str, min: u64, def: Duration| {
            v[key]
                .as_u64()
                .filter(|&s| (min..=3600).contains(&s))
                .map_or(def, Duration::from_secs)
        };
        Config {
            language: v["language"]
                .as_str()
                .map_or(d.language, LangPref::from_code),
            connect_timeout: secs("connect_timeout", 1, d.connect_timeout),
            status_interval: secs("status_interval", 0, d.status_interval),
        }
    }

    /// Only the settings that differ from the defaults: saving the language must
    /// not freeze the other defaults in the file (a later version may change them).
    pub fn to_json(&self) -> Value {
        let d = Config::default();
        let mut m = serde_json::Map::new();
        if self.language != d.language {
            m.insert("language".into(), json!(self.language.code()));
        }
        if self.connect_timeout != d.connect_timeout {
            m.insert(
                "connect_timeout".into(),
                json!(self.connect_timeout.as_secs()),
            );
        }
        if self.status_interval != d.status_interval {
            m.insert(
                "status_interval".into(),
                json!(self.status_interval.as_secs()),
            );
        }
        Value::Object(m)
    }

    pub fn load(paths: &Paths) -> Config {
        read_json(&paths.config_file()).map_or_else(Config::default, |v| Config::from_json(&v))
    }

    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        write_json(&paths.config_file(), &self.to_json())
    }
}

// ---------------------------------------------------------------- lists cache
fn pairs(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

pub fn lists_from_json(v: &Value) -> Lists {
    Lists {
        traffic: pairs(&v["traffic"]),
        streaming: pairs(&v["streaming"]),
        torrent: pairs(&v["torrent"]),
    }
}

pub fn lists_to_json(l: &Lists) -> Value {
    json!({"traffic": l.traffic, "streaming": l.streaming, "torrent": l.torrent})
}

pub fn load_lists(paths: &Paths) -> Lists {
    lists_from_json(&read_json(&paths.lists_file()).unwrap_or(Value::Null))
}

pub fn save_lists(paths: &Paths, l: &Lists) -> std::io::Result<()> {
    write_json(&paths.lists_file(), &lists_to_json(l))
}

// ---------------------------------------------------------------- last connection
/// What we connected to (the CLI's `--status` doesn't say).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub cc: String,
    pub label: String,
}

pub fn state_from_json(v: &Value) -> Option<Connection> {
    let cc = v["cc"].as_str()?;
    let ok = cc.len() == 2 && cc.chars().all(|c| c.is_ascii_uppercase());
    ok.then(|| Connection {
        cc: cc.to_string(),
        label: v["label"].as_str().unwrap_or(cc).to_string(),
    })
}

pub fn state_to_json(c: &Option<Connection>) -> Value {
    match c {
        Some(c) => json!({"cc": c.cc, "label": c.label}),
        None => json!({}),
    }
}

pub fn load_state(paths: &Paths) -> Option<Connection> {
    state_from_json(&read_json(&paths.state_file())?)
}

pub fn save_state(paths: &Paths, c: &Option<Connection>) -> std::io::Result<()> {
    write_json(&paths.state_file(), &state_to_json(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn xdg_paths() {
        let p = Paths::resolve(env(&[("HOME", "/home/u")]));
        assert_eq!(
            p.cache,
            PathBuf::from("/home/u/.cache/cyberghost-indicator")
        );
        assert_eq!(
            p.config,
            PathBuf::from("/home/u/.config/cyberghost-indicator")
        );
        let p = Paths::resolve(env(&[
            ("HOME", "/home/u"),
            ("XDG_CACHE_HOME", "/c"),
            ("XDG_CONFIG_HOME", "/k"),
        ]));
        assert_eq!(p.cache, PathBuf::from("/c/cyberghost-indicator"));
        assert_eq!(
            p.config_file(),
            PathBuf::from("/k/cyberghost-indicator/config.json")
        );
    }

    #[test]
    fn relative_xdg_values_are_ignored() {
        let p = Paths::resolve(env(&[("HOME", "/home/u"), ("XDG_CACHE_HOME", "rel/cache")]));
        assert_eq!(
            p.cache,
            PathBuf::from("/home/u/.cache/cyberghost-indicator")
        );
    }

    #[test]
    fn config_defaults() {
        let c = Config::from_json(&Value::Null);
        assert_eq!(c, Config::default());
        assert_eq!(c.language, LangPref::System);
        assert_eq!(c.connect_timeout, Duration::from_secs(120));
        assert_eq!(
            c.status_interval,
            Duration::ZERO,
            "periodic --status off by default"
        );
    }

    #[test]
    fn config_parses_values() {
        let c = Config::from_json(
            &json!({"language": "fr", "connect_timeout": 30, "status_interval": 5}),
        );
        assert_eq!(c.language, LangPref::Fixed(Lang::Fr));
        assert_eq!(c.connect_timeout, Duration::from_secs(30));
        assert_eq!(c.status_interval, Duration::from_secs(5));
    }

    #[test]
    fn config_rejects_bad_values_field_by_field() {
        let c = Config::from_json(&json!({
            "language": "xx", "connect_timeout": 0, "status_interval": "fast"
        }));
        assert_eq!(c, Config::default());
        let c = Config::from_json(&json!({"connect_timeout": 99999, "language": "en"}));
        assert_eq!(c.connect_timeout, Config::default().connect_timeout);
        assert_eq!(c.language, LangPref::Fixed(Lang::En));
    }

    #[test]
    fn only_non_default_settings_are_saved() {
        assert_eq!(Config::default().to_json(), json!({}));
        let c = Config {
            language: LangPref::Fixed(Lang::Fr),
            ..Config::default()
        };
        assert_eq!(c.to_json(), json!({"language": "fr"}));
        let c = Config {
            status_interval: Duration::from_secs(30),
            ..Config::default()
        };
        assert_eq!(c.to_json(), json!({"status_interval": 30}));
        assert_eq!(Config::from_json(&c.to_json()), c);
    }

    #[test]
    fn status_interval_zero_means_off() {
        let c = Config::from_json(&json!({"status_interval": 30}));
        assert_eq!(c.status_interval, Duration::from_secs(30));
        let c = Config::from_json(&json!({"status_interval": 0}));
        assert_eq!(c.status_interval, Duration::ZERO);
        let c = Config::from_json(&json!({"connect_timeout": 0}));
        assert_eq!(
            c.connect_timeout,
            Config::default().connect_timeout,
            "a zero timeout is invalid"
        );
    }

    #[test]
    fn config_roundtrip_on_disk() {
        let d = tmp();
        let paths = Paths {
            cache: d.path().join("c"),
            config: d.path().join("k"),
        };
        assert_eq!(Config::load(&paths), Config::default()); // missing file
        let c = Config {
            language: LangPref::Fixed(Lang::Fr),
            connect_timeout: Duration::from_secs(10),
            status_interval: Duration::from_secs(2),
        };
        c.save(&paths).unwrap();
        assert_eq!(Config::load(&paths), c);
        fs::write(paths.config_file(), "{not json").unwrap();
        assert_eq!(Config::load(&paths), Config::default());
    }

    #[test]
    fn lists_roundtrip_and_corruption() {
        let d = tmp();
        let paths = Paths {
            cache: d.path().join("c"),
            config: d.path().join("k"),
        };
        assert!(load_lists(&paths).traffic.is_empty());
        let l = Lists {
            traffic: vec![("France".into(), "FR".into())],
            streaming: vec![("Netflix US".into(), "US".into())],
            torrent: vec![("Netherlands".into(), "NL".into())],
        };
        save_lists(&paths, &l).unwrap();
        assert_eq!(load_lists(&paths), l);
        fs::write(paths.lists_file(), "garbage").unwrap();
        assert!(load_lists(&paths).traffic.is_empty());
        // wrong shapes are skipped, not fatal
        let v = json!({"traffic": [["France", "FR"], ["bad"], [1, 2], "x"], "streaming": 3});
        let l = lists_from_json(&v);
        assert_eq!(l.traffic, vec![("France".to_string(), "FR".to_string())]);
        assert!(l.streaming.is_empty());
        assert!(l.torrent.is_empty(), "older cache without torrent list");
    }

    #[test]
    fn state_roundtrip() {
        let d = tmp();
        let paths = Paths {
            cache: d.path().join("c"),
            config: d.path().join("k"),
        };
        assert_eq!(load_state(&paths), None);
        let c = Some(Connection {
            cc: "CH".into(),
            label: "Switzerland".into(),
        });
        save_state(&paths, &c).unwrap();
        assert_eq!(load_state(&paths), c);
        save_state(&paths, &None).unwrap();
        assert_eq!(load_state(&paths), None);
        assert_eq!(read_json(&paths.state_file()).unwrap(), json!({}));
    }

    #[test]
    fn state_validation() {
        assert_eq!(state_from_json(&json!({"cc": "fr", "label": "x"})), None);
        assert_eq!(state_from_json(&json!({"cc": "FRA"})), None);
        assert_eq!(state_from_json(&json!({"label": "x"})), None);
        assert_eq!(
            state_from_json(&json!({"cc": "FR"})),
            Some(Connection {
                cc: "FR".into(),
                label: "FR".into()
            })
        );
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_file() {
        let d = tmp();
        let f = d.path().join("sub/dir/x.json");
        write_json(&f, &json!({"a": 1})).unwrap();
        assert_eq!(read_json(&f).unwrap(), json!({"a": 1}));
        let names: Vec<_> = fs::read_dir(f.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("x.json")]);
    }
}
