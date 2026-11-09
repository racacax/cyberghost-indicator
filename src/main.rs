use cyberghost_indicator::{app, i18n};

fn usage() -> String {
    format!(
        "{name} {version}
{desc}

Usage: {name} [--help | --version]

Settings ($XDG_CONFIG_HOME/cyberghost-indicator/config.json, all optional):
  \"language\":        \"system\" (default) | {langs}
  \"connect_timeout\": seconds (default 120)
  \"status_interval\": seconds between status checks (default 60)

Environment:
  CYBERGHOST_BIN              path to cyberghostvpn
  CYBERGHOST_INDICATOR_ICONS  icon directory
  CYBERGHOST_INDICATOR_ZENITY dialog program for the favorites window (default: zenity)
  LANGUAGE, LC_ALL, LC_MESSAGES, LANG   system language",
        name = env!("CARGO_PKG_NAME"),
        version = env!("CARGO_PKG_VERSION"),
        desc = env!("CARGO_PKG_DESCRIPTION"),
        langs = i18n::Lang::ALL
            .iter()
            .map(|l| format!("{:?}", l.code()))
            .collect::<Vec<_>>()
            .join(" | "),
    )
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        None => std::process::exit(app::run()),
        Some("--version" | "-V") => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
        }
        Some("--help" | "-h") => println!("{}", usage()),
        Some(other) => {
            eprintln!("unknown argument: {other}\n\n{}", usage());
            std::process::exit(2);
        }
    }
}
