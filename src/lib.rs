//! CyberGhost VPN tray indicator.
//!
//! Pure D-Bus StatusNotifierItem (ksni/zbus): no GTK, the desktop's tray draws
//! the menu. Icons ship with the app, runtime state lives in
//! `$XDG_CACHE_HOME/cyberghost-indicator`, settings in
//! `$XDG_CONFIG_HOME/cyberghost-indicator/config.json`.

pub mod app;
pub mod cli;
pub mod dialog;
pub mod favorites;
pub mod i18n;
pub mod icons;
pub mod netwatch;
pub mod store;
pub mod tray;
