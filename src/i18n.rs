//! Translations and language selection.
//!
//! Strings are compiled in (no gettext), so the binary stays self-contained.
//! To add a language: add a `Lang` variant, a `Strings` constant, and list it in
//! `Lang::ALL`. The unit tests check that every placeholder used in English is
//! also used by every other language.

/// A supported UI language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

/// What the user chose: follow the system locale or force a language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LangPref {
    #[default]
    System,
    Fixed(Lang),
}

impl Lang {
    pub const ALL: &'static [Lang] = &[Lang::En, Lang::Fr];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.iter().copied().find(|l| l.code() == code)
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::En => &EN,
            Lang::Fr => &FR,
        }
    }
}

impl LangPref {
    pub fn code(self) -> &'static str {
        match self {
            LangPref::System => "system",
            LangPref::Fixed(l) => l.code(),
        }
    }

    /// Unknown codes fall back to `System`.
    pub fn from_code(code: &str) -> LangPref {
        Lang::from_code(code).map_or(LangPref::System, LangPref::Fixed)
    }

    pub fn resolve(self, system: Lang) -> Lang {
        match self {
            LangPref::System => system,
            LangPref::Fixed(l) => l,
        }
    }
}

/// Language from a locale name such as `fr_FR.UTF-8@euro`, `fr`, `en-US`.
fn lang_of_locale(locale: &str) -> Option<Lang> {
    let base = locale
        .split(['_', '-', '.', '@'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    Lang::from_code(&base)
}

/// System language, following gettext's rules:
/// the locale is `LC_ALL`, else `LC_MESSAGES`, else `LANG` (first non-empty);
/// if it is `C`/`POSIX` (or unset) the UI is English and `LANGUAGE` is ignored;
/// otherwise `LANGUAGE` (a `:`-separated priority list) is tried first.
/// Anything unsupported falls back to English.
pub fn detect(env: impl Fn(&str) -> Option<String>) -> Lang {
    let get = |k: &str| env(k).filter(|v| !v.is_empty());
    let locale = get("LC_ALL")
        .or_else(|| get("LC_MESSAGES"))
        .or_else(|| get("LANG"));
    let Some(locale) = locale else {
        return Lang::En;
    };
    let base = locale.split('.').next().unwrap_or("");
    if base == "C" || base == "POSIX" {
        return Lang::En;
    }
    if let Some(list) = get("LANGUAGE")
        && let Some(l) = list.split(':').find_map(lang_of_locale)
    {
        return l;
    }
    lang_of_locale(&locale).unwrap_or(Lang::En)
}

pub fn detect_from_env() -> Lang {
    detect(|k| std::env::var(k).ok())
}

/// Replace `{name}` placeholders.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut s = template.to_string();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// Every user-visible string. Templates use `{name}` placeholders (see `fill`).
#[derive(Debug)]
pub struct Strings {
    /// The language's own name, shown in the Language menu.
    pub lang_name: &'static str,
    // status line / tooltip
    pub status_disconnected: &'static str,
    pub status_connected: &'static str,
    pub status_connected_to: &'static str,  // {name}
    pub status_connecting_to: &'static str, // {name}
    pub status_disconnecting: &'static str,
    // menu
    pub menu_disconnect: &'static str,
    pub menu_countries: &'static str,
    pub menu_streaming: &'static str,
    pub menu_torrent: &'static str,
    pub menu_favorites: &'static str,
    pub menu_no_favorites: &'static str,
    pub menu_edit_favorites: &'static str,
    pub dialog_title: &'static str,
    pub dialog_text: &'static str,
    pub dialog_col_country: &'static str,
    pub dialog_col_server: &'static str,
    pub n_dialog_missing: &'static str,
    pub n_dialog_missing_body: &'static str, // {error}
    pub server_standard: &'static str,
    pub server_torrent: &'static str,
    pub label_country_server: &'static str, // {country} {server}
    pub menu_loading: &'static str,
    pub menu_refresh: &'static str,
    pub menu_language: &'static str,
    pub menu_language_system: &'static str, // {lang}
    pub menu_quit: &'static str,
    // notifications
    pub n_connected: &'static str,      // {name}
    pub n_connect_failed: &'static str, // {name}
    pub n_previous_dropped: &'static str,
    pub n_disconnected: &'static str,
    pub n_disconnect_failed: &'static str,
    pub n_lists_updated: &'static str,
    pub n_lists_counts: &'static str, // {countries} {services} {torrent}
    pub n_lists_failed: &'static str,
    pub n_cli_missing: &'static str,
    pub n_cli_missing_body: &'static str, // {path}
    // error details
    pub err_country_list: &'static str,   // {error}
    pub err_streaming_list: &'static str, // {error}
    pub err_torrent_list: &'static str,   // {error}
    pub err_spawn: &'static str,          // {program} {reason}
    pub err_timeout: &'static str,        // {secs}
    pub err_no_output: &'static str,
    // console
    pub already_running: &'static str,
}

impl Strings {
    /// All templates with their field name, for completeness tests.
    pub fn entries(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("lang_name", self.lang_name),
            ("status_disconnected", self.status_disconnected),
            ("status_connected", self.status_connected),
            ("status_connected_to", self.status_connected_to),
            ("status_connecting_to", self.status_connecting_to),
            ("status_disconnecting", self.status_disconnecting),
            ("menu_disconnect", self.menu_disconnect),
            ("menu_countries", self.menu_countries),
            ("menu_streaming", self.menu_streaming),
            ("menu_torrent", self.menu_torrent),
            ("menu_favorites", self.menu_favorites),
            ("menu_no_favorites", self.menu_no_favorites),
            ("menu_edit_favorites", self.menu_edit_favorites),
            ("dialog_title", self.dialog_title),
            ("dialog_text", self.dialog_text),
            ("dialog_col_country", self.dialog_col_country),
            ("dialog_col_server", self.dialog_col_server),
            ("n_dialog_missing", self.n_dialog_missing),
            ("n_dialog_missing_body", self.n_dialog_missing_body),
            ("server_standard", self.server_standard),
            ("server_torrent", self.server_torrent),
            ("label_country_server", self.label_country_server),
            ("menu_loading", self.menu_loading),
            ("menu_refresh", self.menu_refresh),
            ("menu_language", self.menu_language),
            ("menu_language_system", self.menu_language_system),
            ("menu_quit", self.menu_quit),
            ("n_connected", self.n_connected),
            ("n_connect_failed", self.n_connect_failed),
            ("n_previous_dropped", self.n_previous_dropped),
            ("n_disconnected", self.n_disconnected),
            ("n_disconnect_failed", self.n_disconnect_failed),
            ("n_lists_updated", self.n_lists_updated),
            ("n_lists_counts", self.n_lists_counts),
            ("n_lists_failed", self.n_lists_failed),
            ("n_cli_missing", self.n_cli_missing),
            ("n_cli_missing_body", self.n_cli_missing_body),
            ("err_country_list", self.err_country_list),
            ("err_streaming_list", self.err_streaming_list),
            ("err_torrent_list", self.err_torrent_list),
            ("err_spawn", self.err_spawn),
            ("err_timeout", self.err_timeout),
            ("err_no_output", self.err_no_output),
            ("already_running", self.already_running),
        ]
    }
}

pub static EN: Strings = Strings {
    lang_name: "English",
    status_disconnected: "Disconnected",
    status_connected: "Connected",
    status_connected_to: "Connected: {name}",
    status_connecting_to: "Connecting to {name}…",
    status_disconnecting: "Disconnecting…",
    menu_disconnect: "Disconnect",
    menu_countries: "Countries",
    menu_streaming: "Streaming",
    menu_torrent: "Torrent",
    menu_favorites: "Favorites",
    menu_no_favorites: "No favorites yet",
    menu_edit_favorites: "Edit favorites…",
    dialog_title: "CyberGhost favorites",
    dialog_text: "Tick the servers to show in Favorites:",
    dialog_col_country: "Country",
    dialog_col_server: "Server",
    n_dialog_missing: "Cannot open the favorites window",
    n_dialog_missing_body: "zenity is required ({error}). Install it, e.g. sudo apt install zenity",
    server_standard: "Standard",
    server_torrent: "Torrent",
    label_country_server: "{country} ({server})",
    menu_loading: "Loading…",
    menu_refresh: "Refresh lists",
    menu_language: "Language",
    menu_language_system: "System default ({lang})",
    menu_quit: "Quit",
    n_connected: "VPN connected: {name}",
    n_connect_failed: "Could not connect to {name}",
    n_previous_dropped: "(the previous connection was closed)",
    n_disconnected: "VPN disconnected",
    n_disconnect_failed: "Could not disconnect",
    n_lists_updated: "Lists updated",
    n_lists_counts: "{countries} countries, {services} streaming services, {torrent} torrent countries",
    n_lists_failed: "Could not refresh the lists",
    n_cli_missing: "CyberGhost not found",
    n_cli_missing_body: "{path} is not installed",
    err_country_list: "country list: {error}",
    err_streaming_list: "streaming list: {error}",
    err_torrent_list: "torrent list: {error}",
    err_spawn: "Cannot run {program}: {reason}",
    err_timeout: "Timed out ({secs} s)",
    err_no_output: "no output",
    already_running: "cyberghost-indicator is already running",
};

pub static FR: Strings = Strings {
    lang_name: "Français",
    status_disconnected: "Déconnecté",
    status_connected: "Connecté",
    status_connected_to: "Connecté : {name}",
    status_connecting_to: "Connexion à {name}…",
    status_disconnecting: "Déconnexion…",
    menu_disconnect: "Se déconnecter",
    menu_countries: "Pays",
    menu_streaming: "Streaming",
    menu_torrent: "Torrent",
    menu_favorites: "Favoris",
    menu_no_favorites: "Aucun favori",
    menu_edit_favorites: "Modifier les favoris…",
    dialog_title: "Favoris CyberGhost",
    dialog_text: "Cochez les serveurs à afficher dans Favoris :",
    dialog_col_country: "Pays",
    dialog_col_server: "Serveur",
    n_dialog_missing: "Impossible d'ouvrir la fenêtre des favoris",
    n_dialog_missing_body: "zenity est nécessaire ({error}). Installez-le, par ex. sudo apt install zenity",
    server_standard: "Standard",
    server_torrent: "Torrent",
    label_country_server: "{country} ({server})",
    menu_loading: "Chargement…",
    menu_refresh: "Actualiser les listes",
    menu_language: "Langue",
    menu_language_system: "Langue du système ({lang})",
    menu_quit: "Quitter",
    n_connected: "VPN connecté : {name}",
    n_connect_failed: "Échec de la connexion à {name}",
    n_previous_dropped: "(l'ancienne connexion a été coupée)",
    n_disconnected: "VPN déconnecté",
    n_disconnect_failed: "Échec de la déconnexion",
    n_lists_updated: "Listes actualisées",
    n_lists_counts: "{countries} pays, {services} services de streaming, {torrent} pays torrent",
    n_lists_failed: "Échec de l'actualisation des listes",
    n_cli_missing: "CyberGhost introuvable",
    n_cli_missing_body: "{path} n'est pas installé",
    err_country_list: "liste des pays : {error}",
    err_streaming_list: "liste streaming : {error}",
    err_torrent_list: "liste torrent : {error}",
    err_spawn: "Impossible d'exécuter {program} : {reason}",
    err_timeout: "Délai dépassé ({secs} s)",
    err_no_output: "aucune sortie",
    already_running: "cyberghost-indicator est déjà lancé",
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashMap};

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn placeholders(s: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            let Some(j) = rest[i..].find('}') else { break };
            out.insert(rest[i + 1..i + j].to_string());
            rest = &rest[i + j + 1..];
        }
        out
    }

    #[test]
    fn no_locale_is_english() {
        assert_eq!(detect(env(&[])), Lang::En);
    }

    #[test]
    fn lang_variable() {
        assert_eq!(detect(env(&[("LANG", "fr_FR.UTF-8")])), Lang::Fr);
        assert_eq!(detect(env(&[("LANG", "en_GB.UTF-8")])), Lang::En);
    }

    #[test]
    fn locale_variants_are_parsed() {
        for l in [
            "fr",
            "fr_CA",
            "fr_BE.UTF-8",
            "fr_FR@euro",
            "fr-CH",
            "FR_fr.utf8",
        ] {
            assert_eq!(detect(env(&[("LANG", l)])), Lang::Fr, "{l}");
        }
    }

    #[test]
    fn unsupported_locale_falls_back_to_english() {
        assert_eq!(detect(env(&[("LANG", "de_DE.UTF-8")])), Lang::En);
        assert_eq!(detect(env(&[("LANG", "garbage")])), Lang::En);
    }

    #[test]
    fn lc_all_beats_lc_messages_beats_lang() {
        let e = env(&[
            ("LC_ALL", "fr_FR.UTF-8"),
            ("LC_MESSAGES", "en_US.UTF-8"),
            ("LANG", "en_US.UTF-8"),
        ]);
        assert_eq!(detect(e), Lang::Fr);
        let e = env(&[("LC_MESSAGES", "fr_FR.UTF-8"), ("LANG", "en_US.UTF-8")]);
        assert_eq!(detect(e), Lang::Fr);
        let e = env(&[("LC_MESSAGES", "en_US.UTF-8"), ("LANG", "fr_FR.UTF-8")]);
        assert_eq!(detect(e), Lang::En);
    }

    #[test]
    fn empty_variables_are_ignored() {
        let e = env(&[("LC_ALL", ""), ("LC_MESSAGES", ""), ("LANG", "fr_FR.UTF-8")]);
        assert_eq!(detect(e), Lang::Fr);
    }

    #[test]
    fn language_list_has_priority() {
        let e = env(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "fr:en")]);
        assert_eq!(detect(e), Lang::Fr);
        // first *supported* entry wins
        let e = env(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "de:it:fr_FR:en")]);
        assert_eq!(detect(e), Lang::Fr);
        // nothing supported in LANGUAGE: use the locale
        let e = env(&[("LANG", "fr_FR.UTF-8"), ("LANGUAGE", "de:it")]);
        assert_eq!(detect(e), Lang::Fr);
    }

    #[test]
    fn c_locale_ignores_language() {
        let e = env(&[("LC_ALL", "C"), ("LANGUAGE", "fr"), ("LANG", "fr_FR.UTF-8")]);
        assert_eq!(detect(e), Lang::En);
        let e = env(&[("LANG", "C.UTF-8"), ("LANGUAGE", "fr")]);
        assert_eq!(detect(e), Lang::En);
        let e = env(&[("LANG", "POSIX"), ("LANGUAGE", "fr")]);
        assert_eq!(detect(e), Lang::En);
    }

    #[test]
    fn language_without_locale_is_ignored() {
        assert_eq!(detect(env(&[("LANGUAGE", "fr")])), Lang::En);
    }

    #[test]
    fn pref_codes_roundtrip() {
        assert_eq!(LangPref::from_code("system"), LangPref::System);
        for &l in Lang::ALL {
            assert_eq!(LangPref::from_code(l.code()), LangPref::Fixed(l));
            assert_eq!(LangPref::Fixed(l).code(), l.code());
        }
        assert_eq!(LangPref::from_code("klingon"), LangPref::System);
        assert_eq!(LangPref::from_code(""), LangPref::System);
    }

    #[test]
    fn pref_resolution() {
        assert_eq!(LangPref::System.resolve(Lang::Fr), Lang::Fr);
        assert_eq!(LangPref::Fixed(Lang::En).resolve(Lang::Fr), Lang::En);
    }

    #[test]
    fn fill_replaces_all_placeholders() {
        assert_eq!(
            fill("{a} and {b}, {a}", &[("a", "x"), ("b", "y")]),
            "x and y, x"
        );
        assert_eq!(fill("no placeholder", &[("a", "x")]), "no placeholder");
        assert_eq!(fill("{missing}", &[]), "{missing}");
    }

    #[test]
    fn every_language_is_complete_and_consistent() {
        let en = EN.entries();
        for &lang in Lang::ALL {
            let other = lang.strings().entries();
            assert_eq!(other.len(), en.len());
            for ((key, e), (key2, o)) in en.iter().zip(other.iter()) {
                assert_eq!(key, key2);
                assert!(!o.trim().is_empty(), "{lang:?}.{key} is empty");
                assert_eq!(
                    placeholders(e),
                    placeholders(o),
                    "{lang:?}.{key} placeholders differ from English"
                );
            }
        }
    }

    #[test]
    fn language_names_are_distinct() {
        let names: BTreeSet<_> = Lang::ALL.iter().map(|l| l.strings().lang_name).collect();
        assert_eq!(names.len(), Lang::ALL.len());
    }
}
