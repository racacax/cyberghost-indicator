//! Tray state and the StatusNotifierItem implementation (icon, tooltip, menu).

use crate::cli::Lists;
use crate::favorites::{Favorite, Favorites};
use crate::i18n::{Lang, LangPref, Strings, fill};
use crate::icons::{Icons, png_16px};
use crate::store::{Config, Connection, Paths};
use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::mpsc::Sender;

/// Marker after a favorite's label.
pub const STAR: &str = "★";
/// Messages to the main loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Connect {
        label: String,
        cc: String,
        args: Vec<String>,
    },
    Disconnect,
    Refresh,
    /// open the favorites window
    EditFavorites,
    /// the favorites window was closed
    EditDone,
    Quit,
    /// an action finished; `up` is the new connection state
    Done {
        up: bool,
    },
    /// VPN interface presence changed (netlink event, or the polling fallback)
    LinkChanged {
        up: bool,
    },
    /// netlink dropped events: resynchronise
    LinkResync,
    /// result of a status check
    Status {
        up: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Busy {
    Connecting(String),
    Disconnecting,
}

pub struct Tray {
    pub tx: Sender<Cmd>,
    pub paths: Arc<Paths>,
    pub icons: Arc<Icons>,
    pub config: Config,
    pub system_lang: Lang,
    pub lists: Arc<Lists>,
    pub favorites: Favorites,
    /// 16 px menu icon PNG per country code, computed once
    pub flags: HashMap<String, Arc<Vec<u8>>>,
    pub busy: Option<Busy>,
    pub frame: u32,
    pub refreshing: bool,
    pub connected: bool,
    pub state: Option<Connection>,
}

impl Tray {
    pub fn lang(&self) -> Lang {
        self.config.language.resolve(self.system_lang)
    }

    pub fn s(&self) -> &'static Strings {
        self.lang().strings()
    }

    pub fn load_flags(&mut self) {
        let lists = Arc::clone(&self.lists);
        let codes = lists
            .traffic
            .iter()
            .chain(&lists.streaming)
            .chain(&lists.torrent)
            .map(|(_, cc)| cc);
        let fav_codes: Vec<String> = self.favorites.iter().map(|f| f.cc.clone()).collect();
        for cc in codes.chain(fav_codes.iter()) {
            if !self.flags.contains_key(cc)
                && let Some(b) = self.icons.flag_file(cc).as_deref().and_then(png_16px)
            {
                self.flags.insert(cc.clone(), Arc::new(b));
            }
        }
    }

    fn flag_data(&self, cc: &str) -> Vec<u8> {
        self.flags
            .get(cc)
            .map(|b| b.as_ref().clone())
            .unwrap_or_default()
    }

    pub fn status_label(&self) -> String {
        let s = self.s();
        match &self.busy {
            Some(Busy::Connecting(name)) => fill(s.status_connecting_to, &[("name", name)]),
            Some(Busy::Disconnecting) => s.status_disconnecting.to_string(),
            None if self.connected => match &self.state {
                Some(c) => fill(s.status_connected_to, &[("name", &c.label)]),
                None => s.status_connected.to_string(),
            },
            None => s.status_disconnected.to_string(),
        }
    }

    pub fn current_icon(&self) -> String {
        if self.busy.is_some() {
            self.icons.path(if self.frame.is_multiple_of(2) {
                "busy-1"
            } else {
                "busy-2"
            })
        } else if self.connected {
            match &self.state {
                Some(c) => self.icons.flag(&c.cc),
                None => self.icons.path("ghost-on"),
            }
        } else {
            self.icons.path("ghost-off")
        }
    }

    pub fn set_language(&mut self, pref: LangPref) {
        if self.config.language != pref {
            self.config.language = pref;
            if let Err(e) = self.config.save(&self.paths) {
                eprintln!("config: {e}");
            }
        }
    }

    /// Replace the favorites (from the favorites window) and save them.
    pub fn set_favorites(&mut self, f: Favorites) {
        self.favorites = f;
        self.load_flags(); // flags of countries that are not in the lists
        if let Err(e) = self.favorites.save(&self.paths.favorites_file()) {
            eprintln!("favorites: {e}");
        }
    }

    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }

    /// Click on a server: connect to it.
    fn activate_server(&mut self, fav: &Favorite, label: &str) {
        self.send(Cmd::Connect {
            label: label.into(),
            cc: fav.cc.clone(),
            args: fav.args(),
        });
    }

    /// An entry of the Countries / Streaming / Torrent lists.
    fn server_item(
        &self,
        text: &str,
        fav: Favorite,
        connection_label: String,
        icon: Vec<u8>,
    ) -> MenuItem<Tray> {
        let marker = self.favorites.contains(&fav).then_some(STAR);
        item(
            with_marker(&escape(text), marker),
            true,
            icon,
            move |t: &mut Tray| t.activate_server(&fav, &connection_label),
        )
    }

    /// An entry of the Favorites menu.
    fn favorite_item(
        &self,
        text: &str,
        fav: Favorite,
        connection_label: String,
        icon: Vec<u8>,
    ) -> MenuItem<Tray> {
        item(escape(text), true, icon, move |t: &mut Tray| {
            t.activate_server(&fav, &connection_label)
        })
    }

    fn favorites_menu(&self, names: &HashMap<String, String>) -> Vec<MenuItem<Tray>> {
        let s = self.s();
        if self.favorites.is_empty() {
            return vec![disabled(s.menu_no_favorites)];
        }
        self.favorites
            .groups(names)
            .into_iter()
            .map(|g| {
                if let [only] = g.items.as_slice() {
                    // one favorite for this country: no sub-list
                    let text = format!("{}  ({})", g.country, only.server_label(s));
                    let label = only.connection_label(&g.country, s);
                    self.favorite_item(&text, only.clone(), label, self.flag_data(&g.cc))
                } else {
                    let sub = g
                        .items
                        .iter()
                        .map(|f| {
                            let label = f.connection_label(&g.country, s);
                            self.favorite_item(&f.server_label(s), f.clone(), label, vec![])
                        })
                        .collect();
                    SubMenu {
                        label: escape(&g.country),
                        icon_data: self.flag_data(&g.cc),
                        submenu: sub,
                        ..Default::default()
                    }
                    .into()
                }
            })
            .collect()
    }
}

fn item<F: Fn(&mut Tray) + Send + 'static>(
    label: String,
    enabled: bool,
    icon: Vec<u8>,
    f: F,
) -> MenuItem<Tray> {
    StandardItem {
        label,
        enabled,
        icon_data: icon,
        activate: Box::new(f),
        ..Default::default()
    }
    .into()
}

fn disabled(label: &str) -> MenuItem<Tray> {
    item(label.into(), false, vec![], |_| {})
}

/// dbusmenu treats `_` as a mnemonic marker: escape it in data-driven labels.
fn escape(label: &str) -> String {
    label.replace('_', "__")
}

fn with_marker(label: &str, marker: Option<&str>) -> String {
    match marker {
        Some(m) => format!("{label}   {m}"),
        None => label.to_string(),
    }
}

fn placeholder_if_empty(mut v: Vec<MenuItem<Tray>>, s: &Strings) -> Vec<MenuItem<Tray>> {
    if v.is_empty() {
        v.push(disabled(s.menu_loading));
    }
    v
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        crate::store::APP.into()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::Communications
    }
    fn title(&self) -> String {
        format!("CyberGhost VPN — {}", self.status_label())
    }
    fn icon_name(&self) -> String {
        self.current_icon()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = self.s();
        let lists_enabled = self.busy.is_none();
        let names = self.lists.country_names();
        let name_of = |cc: &str| names.get(cc).cloned().unwrap_or_else(|| cc.to_string());

        // Countries / Torrent: one entry per country
        let country_list = |list: &[(String, String)], torrent: bool| -> Vec<MenuItem<Self>> {
            list.iter()
                .map(|(n, cc)| {
                    let fav = if torrent {
                        Favorite::torrent(cc)
                    } else {
                        Favorite::standard(cc)
                    };
                    let label = fav.connection_label(n, s);
                    self.server_item(&format!("{n}  ({cc})"), fav, label, self.flag_data(cc))
                })
                .collect()
        };
        let traffic = placeholder_if_empty(country_list(&self.lists.traffic, false), s);
        let torrent = placeholder_if_empty(country_list(&self.lists.torrent, true), s);

        // Streaming: grouped by country (sorted by country name)
        let mut groups: BTreeMap<String, (String, Vec<&str>)> = BTreeMap::new();
        for (svc, cc) in &self.lists.streaming {
            groups
                .entry(name_of(cc).to_lowercase())
                .or_insert_with(|| (cc.clone(), vec![]))
                .1
                .push(svc);
        }
        let streaming: Vec<MenuItem<Self>> = groups
            .into_values()
            .map(|(cc, mut svcs)| {
                svcs.sort_by_key(|s| s.to_lowercase());
                let sub = svcs
                    .into_iter()
                    .map(|svc| {
                        self.server_item(
                            svc,
                            Favorite::streaming(svc, &cc),
                            svc.to_string(),
                            vec![],
                        )
                    })
                    .collect();
                SubMenu {
                    label: escape(&format!("{}  ({cc})", name_of(&cc))),
                    icon_data: self.flag_data(&cc),
                    submenu: sub,
                    ..Default::default()
                }
                .into()
            })
            .collect();
        let streaming = placeholder_if_empty(streaming, s);

        // Language: system default + each language in its own name
        let mut options = vec![RadioItem {
            label: fill(
                s.menu_language_system,
                &[("lang", self.system_lang.strings().lang_name)],
            ),
            ..Default::default()
        }];
        options.extend(Lang::ALL.iter().map(|l| RadioItem {
            label: l.strings().lang_name.into(),
            ..Default::default()
        }));
        let selected = match self.config.language {
            LangPref::System => 0,
            LangPref::Fixed(l) => 1 + Lang::ALL.iter().position(|&x| x == l).unwrap_or(0),
        };
        let language = SubMenu {
            label: s.menu_language.into(),
            submenu: vec![
                RadioGroup {
                    selected,
                    select: Box::new(|t: &mut Tray, i| {
                        let pref = if i == 0 {
                            LangPref::System
                        } else {
                            LangPref::Fixed(Lang::ALL[i - 1])
                        };
                        t.set_language(pref);
                    }),
                    options,
                }
                .into(),
            ],
            ..Default::default()
        };

        let submenu = |label: &str, submenu: Vec<MenuItem<Self>>| -> MenuItem<Self> {
            SubMenu {
                label: label.into(),
                enabled: lists_enabled,
                submenu,
                ..Default::default()
            }
            .into()
        };
        let mut menu = vec![
            disabled(&escape(&self.status_label())),
            item(
                s.menu_disconnect.into(),
                self.connected && self.busy.is_none(),
                vec![],
                |t: &mut Tray| t.send(Cmd::Disconnect),
            ),
            MenuItem::Separator,
            submenu(s.menu_favorites, self.favorites_menu(&names)),
            submenu(s.menu_countries, traffic),
            submenu(s.menu_streaming, streaming),
            submenu(s.menu_torrent, torrent),
            MenuItem::Separator,
            // a window: the menu closes on every click, so it can't be used to tick several
            item(
                s.menu_edit_favorites.into(),
                true,
                vec![],
                |t: &mut Tray| t.send(Cmd::EditFavorites),
            ),
        ];
        menu.extend([
            item(
                s.menu_refresh.into(),
                !self.refreshing,
                vec![],
                |t: &mut Tray| t.send(Cmd::Refresh),
            ),
            language.into(),
            item(s.menu_quit.into(), true, vec![], |t: &mut Tray| {
                t.send(Cmd::Quit)
            }),
        ]);
        menu
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cli::{streaming_args, traffic_args};
    use ksni::Tray as _;
    use std::path::PathBuf;
    use std::sync::mpsc;

    /// Simplified view of a menu entry.
    #[derive(Debug)]
    pub struct Entry {
        pub label: String,
        pub enabled: bool,
        pub has_icon: bool,
        pub children: Vec<Entry>,
    }

    fn view(items: Vec<MenuItem<Tray>>) -> Vec<Entry> {
        items
            .into_iter()
            .map(|i| match i {
                MenuItem::Standard(s) => Entry {
                    label: s.label,
                    enabled: s.enabled,
                    has_icon: !s.icon_data.is_empty(),
                    children: vec![],
                },
                MenuItem::SubMenu(s) => Entry {
                    label: s.label,
                    enabled: s.enabled,
                    has_icon: !s.icon_data.is_empty(),
                    children: view(s.submenu),
                },
                MenuItem::RadioGroup(g) => Entry {
                    label: format!("radio:{}", g.selected),
                    enabled: true,
                    has_icon: false,
                    children: g
                        .options
                        .into_iter()
                        .map(|o| Entry {
                            label: o.label,
                            enabled: o.enabled,
                            has_icon: false,
                            children: vec![],
                        })
                        .collect(),
                },
                MenuItem::Separator => Entry {
                    label: "---".into(),
                    enabled: false,
                    has_icon: false,
                    children: vec![],
                },
                MenuItem::Checkmark(c) => Entry {
                    label: c.label,
                    enabled: c.enabled,
                    has_icon: false,
                    children: vec![],
                },
            })
            .collect()
    }

    fn find<'a>(entries: &'a [Entry], label: &str) -> &'a Entry {
        entries
            .iter()
            .find(|e| e.label == label)
            .unwrap_or_else(|| {
                panic!(
                    "no {label:?} in {:?}",
                    entries.iter().map(|e| &e.label).collect::<Vec<_>>()
                )
            })
    }

    pub struct Fixture {
        pub tray: Tray,
        pub rx: mpsc::Receiver<Cmd>,
        pub _dir: tempfile::TempDir,
    }

    pub fn fixture(lang: Lang) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        let mut tray = Tray {
            tx,
            paths: Arc::new(Paths {
                cache: dir.path().join("cache"),
                config: dir.path().join("config"),
            }),
            icons: Arc::new(Icons::resolve(Some(assets), None, None)),
            config: Config::default(),
            system_lang: lang,
            lists: Arc::new(Lists {
                traffic: vec![
                    ("Andorra".into(), "AD".into()),
                    ("France".into(), "FR".into()),
                    ("Switzerland".into(), "CH".into()),
                    ("United States".into(), "US".into()),
                ],
                streaming: vec![
                    ("Netflix US".into(), "US".into()),
                    ("Disney Plus FR".into(), "FR".into()),
                    ("Apple TV US".into(), "US".into()),
                    ("Zee5 IN".into(), "IN".into()),
                ],
                torrent: vec![
                    ("France".into(), "FR".into()),
                    ("Netherlands".into(), "NL".into()),
                ],
            }),
            favorites: Favorites::default(),
            flags: HashMap::new(),
            busy: None,
            frame: 0,
            refreshing: false,
            connected: false,
            state: None,
        };
        tray.load_flags();
        Fixture {
            tray,
            rx,
            _dir: dir,
        }
    }

    fn activate(items: Vec<MenuItem<Tray>>, path: &[&str], tray: &mut Tray) {
        let (head, rest) = path.split_first().unwrap();
        for i in items {
            match i {
                MenuItem::Standard(s) if s.label == *head && rest.is_empty() => {
                    return (s.activate)(tray);
                }
                MenuItem::SubMenu(s) if s.label == *head => return activate(s.submenu, rest, tray),
                MenuItem::Checkmark(c) if c.label == *head && rest.is_empty() => {
                    return (c.activate)(tray);
                }
                MenuItem::RadioGroup(g) if rest.is_empty() => {
                    if let Some(idx) = g.options.iter().position(|o| o.label == *head) {
                        return (g.select)(tray, idx);
                    }
                }
                _ => {}
            }
        }
        panic!("menu path {path:?} not found");
    }

    #[test]
    fn top_level_menu_in_english() {
        let f = fixture(Lang::En);
        let m = view(f.tray.menu());
        let labels: Vec<_> = m.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Disconnected",
                "Disconnect",
                "---",
                "Favorites",
                "Countries",
                "Streaming",
                "Torrent",
                "---",
                "Edit favorites…",
                "Refresh lists",
                "Language",
                "Quit"
            ]
        );
        assert!(!m[0].enabled, "status line is informational");
        assert!(!find(&m, "Disconnect").enabled, "nothing to disconnect");
    }

    #[test]
    fn top_level_menu_in_french() {
        let f = fixture(Lang::Fr);
        let m = view(f.tray.menu());
        for l in [
            "Déconnecté",
            "Se déconnecter",
            "Favoris",
            "Pays",
            "Streaming",
            "Torrent",
            "Modifier les favoris…",
            "Actualiser les listes",
            "Langue",
            "Quitter",
        ] {
            find(&m, l);
        }
    }

    #[test]
    fn countries_are_listed_with_code_and_flag() {
        let f = fixture(Lang::En);
        let m = view(f.tray.menu());
        let c = &find(&m, "Countries").children;
        let labels: Vec<_> = c.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Andorra  (AD)",
                "France  (FR)",
                "Switzerland  (CH)",
                "United States  (US)"
            ]
        );
        assert!(c.iter().all(|e| e.enabled && e.has_icon));
    }

    #[test]
    fn streaming_is_grouped_by_country_and_sorted() {
        let f = fixture(Lang::En);
        let m = view(f.tray.menu());
        let g = &find(&m, "Streaming").children;
        let labels: Vec<_> = g.iter().map(|e| e.label.as_str()).collect();
        // IN is not in the country list: the code is used as the name
        assert_eq!(labels, ["France  (FR)", "IN  (IN)", "United States  (US)"]);
        let us: Vec<_> = find(g, "United States  (US)")
            .children
            .iter()
            .map(|e| e.label.as_str())
            .collect();
        assert_eq!(us, ["Apple TV US", "Netflix US"]);
        assert!(
            g.iter().all(|e| e.has_icon),
            "groups have flags (IN included)"
        );
    }

    #[test]
    fn empty_lists_show_loading_placeholder() {
        let mut f = fixture(Lang::En);
        f.tray.lists = Arc::new(Lists {
            traffic: vec![],
            streaming: vec![],
            torrent: vec![],
        });
        let m = view(f.tray.menu());
        for sub in ["Countries", "Streaming"] {
            let c = &find(&m, sub).children;
            assert_eq!(c.len(), 1);
            assert_eq!(c[0].label, "Loading…");
            assert!(!c[0].enabled);
        }
    }

    #[test]
    fn clicking_a_country_sends_connect() {
        let mut f = fixture(Lang::En);
        let menu = f.tray.menu();
        activate(menu, &["Countries", "Switzerland  (CH)"], &mut f.tray);
        assert_eq!(
            f.rx.try_recv().unwrap(),
            Cmd::Connect {
                label: "Switzerland".into(),
                cc: "CH".into(),
                args: traffic_args("CH")
            }
        );
    }

    #[test]
    fn clicking_a_streaming_service_sends_connect() {
        let mut f = fixture(Lang::En);
        let menu = f.tray.menu();
        activate(
            menu,
            &["Streaming", "United States  (US)", "Netflix US"],
            &mut f.tray,
        );
        assert_eq!(
            f.rx.try_recv().unwrap(),
            Cmd::Connect {
                label: "Netflix US".into(),
                cc: "US".into(),
                args: streaming_args("Netflix US", "US")
            }
        );
    }

    #[test]
    fn other_items_send_their_command() {
        let mut f = fixture(Lang::En);
        f.tray.connected = true;
        for (label, cmd) in [
            ("Disconnect", Cmd::Disconnect),
            ("Refresh lists", Cmd::Refresh),
            ("Quit", Cmd::Quit),
        ] {
            let menu = f.tray.menu();
            activate(menu, &[label], &mut f.tray);
            assert_eq!(f.rx.try_recv().unwrap(), cmd);
        }
    }

    #[test]
    fn busy_disables_actions_and_shows_progress() {
        let mut f = fixture(Lang::En);
        f.tray.connected = true;
        f.tray.busy = Some(Busy::Connecting("France".into()));
        let m = view(f.tray.menu());
        assert_eq!(m[0].label, "Connecting to France…");
        assert!(!find(&m, "Disconnect").enabled);
        assert!(!find(&m, "Countries").enabled);
        assert!(!find(&m, "Streaming").enabled);
        assert!(find(&m, "Quit").enabled);
        f.tray.busy = Some(Busy::Disconnecting);
        assert_eq!(f.tray.status_label(), "Disconnecting…");
    }

    #[test]
    fn refreshing_disables_refresh() {
        let mut f = fixture(Lang::En);
        f.tray.refreshing = true;
        assert!(!find(&view(f.tray.menu()), "Refresh lists").enabled);
    }

    #[test]
    fn status_labels() {
        let mut f = fixture(Lang::En);
        assert_eq!(f.tray.status_label(), "Disconnected");
        f.tray.connected = true;
        assert_eq!(f.tray.status_label(), "Connected");
        f.tray.state = Some(Connection {
            cc: "CH".into(),
            label: "Switzerland".into(),
        });
        assert_eq!(f.tray.status_label(), "Connected: Switzerland");
        assert_eq!(f.tray.title(), "CyberGhost VPN — Connected: Switzerland");
        f.tray.config.language = LangPref::Fixed(Lang::Fr);
        assert_eq!(f.tray.status_label(), "Connecté : Switzerland");
    }

    #[test]
    fn icon_reflects_state() {
        let mut f = fixture(Lang::En);
        assert!(f.tray.current_icon().ends_with("/ghost-off.png"));
        f.tray.connected = true;
        assert!(
            f.tray.current_icon().ends_with("/ghost-on.png"),
            "connected, country unknown"
        );
        f.tray.state = Some(Connection {
            cc: "CH".into(),
            label: "Switzerland".into(),
        });
        assert!(f.tray.current_icon().ends_with("/flag-CH.png"));
        f.tray.state = Some(Connection {
            cc: "QQ".into(),
            label: "?".into(),
        });
        assert!(f.tray.current_icon().ends_with("/flag-unknown.png"));
        f.tray.busy = Some(Busy::Disconnecting);
        assert!(f.tray.current_icon().ends_with("/busy-1.png"));
        f.tray.frame = 1;
        assert!(
            f.tray.current_icon().ends_with("/busy-2.png"),
            "animation frame"
        );
        f.tray.frame = 2;
        assert!(f.tray.current_icon().ends_with("/busy-1.png"));
    }

    #[test]
    fn language_menu_lists_system_default_and_languages() {
        let f = fixture(Lang::Fr);
        let m = view(f.tray.menu());
        let radio = &find(&m, "Langue").children[0];
        assert_eq!(radio.label, "radio:0");
        let opts: Vec<_> = radio.children.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            opts,
            ["Langue du système (Français)", "English", "Français"]
        );
    }

    #[test]
    fn choosing_a_language_switches_and_persists() {
        let mut f = fixture(Lang::En);
        let menu = f.tray.menu();
        activate(menu, &["Language", "Français"], &mut f.tray);
        assert_eq!(f.tray.lang(), Lang::Fr);
        assert_eq!(f.tray.config.language, LangPref::Fixed(Lang::Fr));
        assert_eq!(
            Config::load(&f.tray.paths).language,
            LangPref::Fixed(Lang::Fr)
        );
        let m = view(f.tray.menu());
        assert_eq!(find(&m, "Langue").children[0].label, "radio:2");
        // back to the system language
        let menu = f.tray.menu();
        activate(
            menu,
            &["Langue", "Langue du système (English)"],
            &mut f.tray,
        );
        assert_eq!(f.tray.lang(), Lang::En);
        assert_eq!(Config::load(&f.tray.paths).language, LangPref::System);
    }

    #[test]
    fn underscores_in_data_labels_are_escaped() {
        let mut f = fixture(Lang::En);
        f.tray.lists = Arc::new(Lists {
            traffic: vec![("Some_Land".into(), "SL".into())],
            streaming: vec![],
            torrent: vec![],
        });
        let m = view(f.tray.menu());
        assert_eq!(find(&m, "Countries").children[0].label, "Some__Land  (SL)");
    }

    // ---------------------------------------------------------------- torrent & favorites
    fn labels(e: &Entry) -> Vec<&str> {
        e.children.iter().map(|c| c.label.as_str()).collect()
    }

    fn fav_fixture() -> Fixture {
        let mut f = fixture(Lang::En);
        f.tray.favorites.add(Favorite::standard("FR"));
        f.tray
            .favorites
            .add(Favorite::streaming("Disney Plus FR", "FR"));
        f.tray.favorites.add(Favorite::standard("CH"));
        f
    }

    #[test]
    fn torrent_list_connects_with_torrent_server_type() {
        let mut f = fixture(Lang::En);
        let m = view(f.tray.menu());
        assert_eq!(
            labels(find(&m, "Torrent")),
            ["France  (FR)", "Netherlands  (NL)"]
        );
        assert!(find(&m, "Torrent").children.iter().all(|e| e.has_icon));
        let menu = f.tray.menu();
        activate(menu, &["Torrent", "Netherlands  (NL)"], &mut f.tray);
        assert_eq!(
            f.rx.try_recv().unwrap(),
            Cmd::Connect {
                label: "Netherlands (Torrent)".into(),
                cc: "NL".into(),
                args: crate::cli::torrent_args("NL")
            }
        );
    }

    #[test]
    fn no_favorites_placeholder() {
        let f = fixture(Lang::En);
        let m = view(f.tray.menu());
        let fav = find(&m, "Favorites");
        assert_eq!(labels(fav), ["No favorites yet"]);
        assert!(!fav.children[0].enabled);
    }

    #[test]
    fn favorites_are_grouped_by_country() {
        let f = fav_fixture();
        let m = view(f.tray.menu());
        let fav = find(&m, "Favorites");
        // France has two favorites: a sub-list; Switzerland one: a single entry
        assert_eq!(labels(fav), ["France", "Switzerland  (Standard)"]);
        assert_eq!(
            labels(find(&fav.children, "France")),
            ["Standard", "Disney Plus FR"]
        );
        assert!(fav.children.iter().all(|e| e.has_icon), "country flags");
    }

    #[test]
    fn favorites_are_starred_in_the_lists() {
        let f = fav_fixture();
        let m = view(f.tray.menu());
        assert_eq!(
            labels(find(&m, "Countries")),
            [
                "Andorra  (AD)",
                "France  (FR)   ★",
                "Switzerland  (CH)   ★",
                "United States  (US)"
            ]
        );
        let fr = find(&find(&m, "Streaming").children, "France  (FR)");
        assert_eq!(labels(fr), ["Disney Plus FR   ★"]);
        assert_eq!(
            labels(find(&m, "Torrent")),
            ["France  (FR)", "Netherlands  (NL)"],
            "torrent FR is not a favorite"
        );
    }

    #[test]
    fn clicking_a_favorite_connects() {
        let mut f = fav_fixture();
        let menu = f.tray.menu();
        activate(
            menu,
            &["Favorites", "France", "Disney Plus FR"],
            &mut f.tray,
        );
        assert_eq!(
            f.rx.try_recv().unwrap(),
            Cmd::Connect {
                label: "Disney Plus FR".into(),
                cc: "FR".into(),
                args: crate::cli::streaming_args("Disney Plus FR", "FR")
            }
        );
        let menu = f.tray.menu();
        activate(menu, &["Favorites", "Switzerland  (Standard)"], &mut f.tray);
        assert_eq!(
            f.rx.try_recv().unwrap(),
            Cmd::Connect {
                label: "Switzerland".into(),
                cc: "CH".into(),
                args: crate::cli::traffic_args("CH")
            }
        );
    }

    #[test]
    fn clicking_a_starred_entry_connects() {
        let mut f = fav_fixture();
        let menu = f.tray.menu();
        activate(menu, &["Countries", "France  (FR)   ★"], &mut f.tray);
        assert!(matches!(f.rx.try_recv().unwrap(), Cmd::Connect { .. }));
        assert_eq!(f.tray.favorites.len(), 3, "unchanged");
    }

    #[test]
    fn favorite_of_a_country_missing_from_lists_uses_code_and_flag() {
        let mut f = fixture(Lang::En);
        let mut fav = Favorites::default();
        fav.add(Favorite::torrent("SE"));
        f.tray.set_favorites(fav);
        let m = view(f.tray.menu());
        let fav = find(&m, "Favorites");
        assert_eq!(labels(fav), ["SE  (Torrent)"]);
        assert!(
            fav.children[0].has_icon,
            "flag loaded when the favorite was added"
        );
    }

    #[test]
    fn torrent_favorite_connection_label_is_translated() {
        let mut f = fixture(Lang::Fr);
        f.tray.favorites.add(Favorite::torrent("FR"));
        let menu = f.tray.menu();
        activate(menu, &["Favoris", "France  (Torrent)"], &mut f.tray);
        match f.rx.try_recv().unwrap() {
            Cmd::Connect { label, .. } => assert_eq!(label, "France (Torrent)"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn edit_favorites_opens_the_window_even_while_busy() {
        let mut f = fav_fixture();
        f.tray.busy = Some(Busy::Connecting("France".into()));
        let m = view(f.tray.menu());
        assert!(find(&m, "Edit favorites…").enabled);
        assert!(!find(&m, "Favorites").enabled, "connecting is still locked");
        let menu = f.tray.menu();
        activate(menu, &["Edit favorites…"], &mut f.tray);
        assert_eq!(f.rx.try_recv().unwrap(), Cmd::EditFavorites);
    }

    #[test]
    fn no_edit_markers_left_in_menus() {
        let f = fav_fixture();
        let m = view(f.tray.menu());
        fn walk(e: &[Entry], out: &mut Vec<String>) {
            for x in e {
                out.push(x.label.clone());
                walk(&x.children, out);
            }
        }
        let mut all = vec![];
        walk(&m, &mut all);
        assert!(
            all.iter().all(|l| !l.contains('☆') && !l.contains('🗑')),
            "{all:?}"
        );
    }

    #[test]
    fn set_favorites_updates_menu_and_file() {
        let mut f = fixture(Lang::En);
        let mut fav = Favorites::default();
        fav.add(Favorite::standard("AD"));
        fav.add(Favorite::torrent("NL"));
        f.tray.set_favorites(fav.clone());
        assert_eq!(Favorites::load(&f.tray.paths.favorites_file()), fav);
        let m = view(f.tray.menu());
        assert_eq!(
            labels(find(&m, "Favorites")),
            ["Andorra  (Standard)", "Netherlands  (Torrent)"]
        );
        assert_eq!(labels(find(&m, "Countries"))[0], "Andorra  (AD)   ★");
        f.tray.set_favorites(Favorites::default());
        assert_eq!(
            labels(find(&view(f.tray.menu()), "Favorites")),
            ["No favorites yet"]
        );
    }
}
