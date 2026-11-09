//! Favorite servers: model, grouping for the menu, persistence.
//!
//! A favorite is a country plus a server type: the standard server, the
//! torrent-optimized server, or a streaming service. They are shown grouped by
//! country; a country with a single favorite is shown as one entry.

use crate::cli::{streaming_args, torrent_args, traffic_args};
use crate::i18n::{Strings, fill};
use crate::store::{read_json, write_json};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Server {
    /// `--traffic`
    Standard,
    /// `--torrent`
    Torrent,
    /// `--streaming <service>`
    Streaming(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Favorite {
    pub cc: String,
    pub server: Server,
}

impl Favorite {
    pub fn standard(cc: &str) -> Favorite {
        Favorite {
            cc: cc.into(),
            server: Server::Standard,
        }
    }
    pub fn torrent(cc: &str) -> Favorite {
        Favorite {
            cc: cc.into(),
            server: Server::Torrent,
        }
    }
    pub fn streaming(service: &str, cc: &str) -> Favorite {
        Favorite {
            cc: cc.into(),
            server: Server::Streaming(service.into()),
        }
    }

    /// CLI arguments to connect to this server.
    pub fn args(&self) -> Vec<String> {
        match &self.server {
            Server::Standard => traffic_args(&self.cc),
            Server::Torrent => torrent_args(&self.cc),
            Server::Streaming(svc) => streaming_args(svc, &self.cc),
        }
    }

    /// Server name inside its country group: "Standard", "Torrent", or the service.
    pub fn server_label(&self, s: &Strings) -> String {
        match &self.server {
            Server::Standard => s.server_standard.into(),
            Server::Torrent => s.server_torrent.into(),
            Server::Streaming(svc) => svc.clone(),
        }
    }

    /// Name used in the status line and notifications once connected:
    /// "France", "France (Torrent)", "TF1 FR".
    pub fn connection_label(&self, country: &str, s: &Strings) -> String {
        match &self.server {
            Server::Standard => country.into(),
            Server::Torrent => fill(
                s.label_country_server,
                &[("country", country), ("server", s.server_torrent)],
            ),
            Server::Streaming(svc) => svc.clone(),
        }
    }

    fn sort_key(&self) -> (u8, String) {
        match &self.server {
            Server::Standard => (0, String::new()),
            Server::Torrent => (1, String::new()),
            Server::Streaming(svc) => (2, svc.to_lowercase()),
        }
    }

    pub fn to_json(&self) -> Value {
        match &self.server {
            Server::Standard => json!({"kind": "standard", "cc": self.cc}),
            Server::Torrent => json!({"kind": "torrent", "cc": self.cc}),
            Server::Streaming(svc) => json!({"kind": "streaming", "cc": self.cc, "service": svc}),
        }
    }

    pub fn from_json(v: &Value) -> Option<Favorite> {
        let cc = v["cc"].as_str()?;
        if cc.len() != 2 || !cc.chars().all(|c| c.is_ascii_uppercase()) {
            return None;
        }
        let server = match v["kind"].as_str()? {
            "standard" => Server::Standard,
            "torrent" => Server::Torrent,
            "streaming" => Server::Streaming(
                v["service"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())?
                    .into(),
            ),
            _ => return None,
        };
        Some(Favorite {
            cc: cc.into(),
            server,
        })
    }
}

/// Favorites in the order they were added (duplicates impossible).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Favorites(Vec<Favorite>);

/// One country of the Favorites menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub cc: String,
    pub country: String,
    /// sorted: Standard, Torrent, then streaming services alphabetically
    pub items: Vec<Favorite>,
}

impl Favorites {
    pub fn contains(&self, f: &Favorite) -> bool {
        self.0.contains(f)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Favorite> {
        self.0.iter()
    }

    pub fn add(&mut self, f: Favorite) -> bool {
        if self.contains(&f) {
            return false;
        }
        self.0.push(f);
        true
    }

    pub fn remove(&mut self, f: &Favorite) -> bool {
        let before = self.0.len();
        self.0.retain(|x| x != f);
        self.0.len() != before
    }

    /// Add if missing, remove if present. Returns true if it is now a favorite.
    pub fn toggle(&mut self, f: Favorite) -> bool {
        if self.remove(&f) {
            false
        } else {
            self.0.push(f);
            true
        }
    }

    /// Grouped by country, countries sorted by name. `names` maps codes to
    /// country names; unknown codes are shown as the code itself.
    pub fn groups(&self, names: &HashMap<String, String>) -> Vec<Group> {
        let mut by_cc: BTreeMap<String, Vec<Favorite>> = BTreeMap::new();
        for f in &self.0 {
            by_cc.entry(f.cc.clone()).or_default().push(f.clone());
        }
        let mut groups: Vec<Group> = by_cc
            .into_iter()
            .map(|(cc, mut items)| {
                items.sort_by_key(Favorite::sort_key);
                let country = names.get(&cc).cloned().unwrap_or_else(|| cc.clone());
                Group { cc, country, items }
            })
            .collect();
        groups.sort_by_key(|g| (g.country.to_lowercase(), g.cc.clone()));
        groups
    }

    pub fn to_json(&self) -> Value {
        json!({"favorites": self.0.iter().map(Favorite::to_json).collect::<Vec<_>>()})
    }

    /// Invalid entries and duplicates are dropped.
    pub fn from_json(v: &Value) -> Favorites {
        let mut f = Favorites::default();
        for item in v["favorites"].as_array().into_iter().flatten() {
            if let Some(x) = Favorite::from_json(item) {
                f.add(x);
            }
        }
        f
    }

    pub fn load(path: &Path) -> Favorites {
        read_json(path).map_or_else(Favorites::default, |v| Favorites::from_json(&v))
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_json(path, &self.to_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{EN, FR};

    fn names() -> HashMap<String, String> {
        [
            ("FR", "France"),
            ("CH", "Switzerland"),
            ("AD", "Andorra"),
            ("US", "United States"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    #[test]
    fn add_remove_toggle() {
        let mut f = Favorites::default();
        assert!(f.add(Favorite::standard("FR")));
        assert!(!f.add(Favorite::standard("FR")), "no duplicates");
        assert!(f.add(Favorite::torrent("FR")), "same country, other server");
        assert!(f.add(Favorite::streaming("TF1 FR", "FR")));
        assert_eq!(f.len(), 3);
        assert!(f.remove(&Favorite::torrent("FR")));
        assert!(!f.remove(&Favorite::torrent("FR")));
        assert!(!f.toggle(Favorite::standard("FR")), "toggle removes");
        assert!(f.toggle(Favorite::standard("FR")), "toggle adds back");
        assert!(f.contains(&Favorite::standard("FR")));
        assert!(!f.contains(&Favorite::standard("CH")));
    }

    #[test]
    fn groups_by_country_sorted_by_name_then_server() {
        let mut f = Favorites::default();
        f.add(Favorite::streaming("TF1 FR", "FR"));
        f.add(Favorite::standard("CH"));
        f.add(Favorite::streaming("Arte FR", "FR"));
        f.add(Favorite::torrent("FR"));
        f.add(Favorite::standard("FR"));
        f.add(Favorite::standard("AD"));
        let g = f.groups(&names());
        let countries: Vec<_> = g.iter().map(|g| g.country.as_str()).collect();
        assert_eq!(countries, ["Andorra", "France", "Switzerland"]);
        let fr: Vec<_> = g[1].items.iter().map(|x| x.server_label(&EN)).collect();
        assert_eq!(fr, ["Standard", "Torrent", "Arte FR", "TF1 FR"]);
        assert_eq!(g[0].items.len(), 1);
    }

    #[test]
    fn unknown_country_uses_its_code() {
        let mut f = Favorites::default();
        f.add(Favorite::torrent("NL"));
        let g = f.groups(&names());
        assert_eq!(g[0].country, "NL");
    }

    #[test]
    fn connection_arguments() {
        assert_eq!(
            Favorite::standard("FR").args(),
            ["--traffic", "--country-code", "FR", "--connect"]
        );
        assert_eq!(
            Favorite::torrent("FR").args(),
            ["--torrent", "--country-code", "FR", "--connect"]
        );
        assert_eq!(
            Favorite::streaming("TF1 FR", "FR").args(),
            ["--streaming", "TF1 FR", "--country-code", "FR", "--connect"]
        );
    }

    #[test]
    fn labels() {
        assert_eq!(
            Favorite::standard("FR").connection_label("France", &EN),
            "France"
        );
        assert_eq!(
            Favorite::torrent("FR").connection_label("France", &EN),
            "France (Torrent)"
        );
        assert_eq!(
            Favorite::streaming("TF1 FR", "FR").connection_label("France", &FR),
            "TF1 FR"
        );
        assert_eq!(Favorite::standard("FR").server_label(&FR), "Standard");
        assert_eq!(Favorite::torrent("FR").server_label(&FR), "Torrent");
    }

    #[test]
    fn json_roundtrip_keeps_order() {
        let mut f = Favorites::default();
        f.add(Favorite::streaming("TF1 FR", "FR"));
        f.add(Favorite::standard("CH"));
        f.add(Favorite::torrent("FR"));
        assert_eq!(Favorites::from_json(&f.to_json()), f);
    }

    #[test]
    fn invalid_entries_are_dropped() {
        let v = json!({"favorites": [
            {"kind": "standard", "cc": "FR"},
            {"kind": "standard", "cc": "FR"},
            {"kind": "standard", "cc": "fr"},
            {"kind": "standard", "cc": "FRA"},
            {"kind": "streaming", "cc": "US"},
            {"kind": "streaming", "cc": "US", "service": "  "},
            {"kind": "warp", "cc": "US"},
            {"cc": "US"},
            "garbage",
            {"kind": "torrent", "cc": "NL"}
        ]});
        let f = Favorites::from_json(&v);
        assert_eq!(
            f.iter().cloned().collect::<Vec<_>>(),
            [Favorite::standard("FR"), Favorite::torrent("NL")]
        );
        assert!(Favorites::from_json(&json!(null)).is_empty());
        assert!(Favorites::from_json(&json!({"favorites": 3})).is_empty());
    }

    #[test]
    fn load_and_save() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/favorites.json");
        assert!(Favorites::load(&p).is_empty(), "missing file");
        let mut f = Favorites::default();
        f.add(Favorite::streaming("Netflix US", "US"));
        f.save(&p).unwrap();
        assert_eq!(Favorites::load(&p), f);
        std::fs::write(&p, "{oops").unwrap();
        assert!(Favorites::load(&p).is_empty(), "corrupted file");
    }
}
