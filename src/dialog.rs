//! "Edit favorites" window (zenity checklist).
//!
//! A tray menu closes on every click (the panel's GTK menu does that, the menu
//! protocol has no "stay open"), so favorites are edited in a real window:
//! every server is listed, current favorites are ticked and listed first.

use crate::cli::Lists;
use crate::favorites::{Favorite, Favorites, Server};
use crate::i18n::Strings;
use std::process::{Command, Stdio};

/// `$CYBERGHOST_INDICATOR_ZENITY`, else `zenity` from PATH.
pub fn program() -> String {
    std::env::var("CYBERGHOST_INDICATOR_ZENITY")
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "zenity".into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub fav: Favorite,
    /// "France  (FR)"
    pub country: String,
    /// "Standard", "Torrent" or the streaming service
    pub server: String,
    pub checked: bool,
}

fn sort_key(country: &str, f: &Favorite) -> (String, String, u8, String) {
    let (rank, svc) = match &f.server {
        Server::Standard => (0, String::new()),
        Server::Torrent => (1, String::new()),
        Server::Streaming(s) => (2, s.to_lowercase()),
    };
    (country.to_lowercase(), f.cc.clone(), rank, svc)
}

/// Every server of the lists, plus favorites that are no longer listed (so they
/// can still be removed). Favorites first, then the rest; each part sorted by
/// country, then Standard / Torrent / services.
pub fn rows(lists: &Lists, favorites: &Favorites, s: &Strings) -> Vec<Row> {
    let names = lists.country_names();
    let name_of = |cc: &str| names.get(cc).cloned().unwrap_or_else(|| cc.to_string());
    let mut all: Vec<Favorite> = Vec::new();
    all.extend(lists.traffic.iter().map(|(_, cc)| Favorite::standard(cc)));
    all.extend(lists.torrent.iter().map(|(_, cc)| Favorite::torrent(cc)));
    all.extend(
        lists
            .streaming
            .iter()
            .map(|(svc, cc)| Favorite::streaming(svc, cc)),
    );
    all.extend(favorites.iter().cloned());
    let mut seen = std::collections::HashSet::new();
    all.retain(|f| seen.insert(f.clone()));

    let mut rows: Vec<Row> = all
        .into_iter()
        .map(|f| Row {
            country: format!("{}  ({})", name_of(&f.cc), f.cc),
            server: f.server_label(s),
            checked: favorites.contains(&f),
            fav: f,
        })
        .collect();
    rows.sort_by_cached_key(|r| (!r.checked, sort_key(&r.country, &r.fav)));
    rows
}

pub fn args(s: &Strings) -> Vec<String> {
    [
        "--list",
        "--checklist",
        "--title",
        s.dialog_title,
        "--text",
        s.dialog_text,
        "--width",
        "640",
        "--height",
        "720",
        "--column",
        "★",
        "--column",
        s.dialog_col_country,
        "--column",
        s.dialog_col_server,
        "--column",
        "#",
        "--hide-column",
        "4",
        "--print-column",
        "4",
        "--separator",
        "\n",
    ]
    .map(String::from)
    .to_vec()
}

/// The list cells, row after row (4 per row), passed as command-line arguments.
///
/// Not on stdin: zenity reads stdin a little at a time while the window is
/// already shown, so the rows appeared one by one. Arguments are all parsed
/// before the window opens (≈430 rows × 4 cells is far below ARG_MAX).
pub fn cells(rows: &[Row]) -> Vec<String> {
    // a cell starting with '-' would be taken for an option: a leading space avoids that
    let clean = |s: &str| {
        let s = s.replace(['\n', '\r'], " ");
        if s.starts_with('-') {
            format!(" {s}")
        } else {
            s
        }
    };
    let mut out = Vec::with_capacity(rows.len() * 4);
    for (i, r) in rows.iter().enumerate() {
        out.push(if r.checked { "TRUE" } else { "FALSE" }.to_string());
        out.push(clean(&r.country));
        out.push(clean(&r.server));
        out.push(i.to_string());
    }
    out
}

/// New favorites from zenity's output (ticked row numbers). Favorites that stay
/// keep their order, new ones are appended in list order. Unknown numbers are ignored.
pub fn apply(output: &str, rows: &[Row], current: &Favorites) -> Favorites {
    let picked: Vec<&Favorite> = output
        .split(['\n', '|'])
        .filter_map(|l| l.trim().parse::<usize>().ok())
        .filter_map(|i| rows.get(i))
        .map(|r| &r.fav)
        .collect();
    let mut f = Favorites::default();
    for x in current.iter().filter(|x| picked.contains(x)) {
        f.add(x.clone());
    }
    for x in picked {
        f.add(x.clone());
    }
    f
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Saved(Favorites),
    Cancelled,
    /// the dialog program could not be started
    Missing(String),
}

/// Show the dialog and wait for the user. Blocking: call from a worker thread.
pub fn edit(program: &str, lists: &Lists, current: &Favorites, s: &Strings) -> Outcome {
    let rows = rows(lists, current, s);
    let child = Command::new(program)
        .args(args(s))
        .args(cells(&rows))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // GTK warnings
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => return Outcome::Missing(format!("{program}: {e}")),
    };
    match child.wait_with_output() {
        // 0 = OK (even with nothing ticked), 1 = Cancel / window closed
        Ok(o) if o.status.success() => {
            Outcome::Saved(apply(&String::from_utf8_lossy(&o.stdout), &rows, current))
        }
        _ => Outcome::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{EN, FR};

    fn lists() -> Lists {
        Lists {
            traffic: vec![
                ("France".into(), "FR".into()),
                ("Andorra".into(), "AD".into()),
            ],
            streaming: vec![
                ("TF1 FR".into(), "FR".into()),
                ("Netflix US".into(), "US".into()),
            ],
            torrent: vec![
                ("France".into(), "FR".into()),
                ("Netherlands".into(), "NL".into()),
            ],
        }
    }

    fn summary(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| {
                format!(
                    "{}{} | {}",
                    if r.checked { "x " } else { "" },
                    r.country,
                    r.server
                )
            })
            .collect()
    }

    #[test]
    fn lists_every_server_sorted_by_country() {
        let r = rows(&lists(), &Favorites::default(), &EN);
        assert_eq!(
            summary(&r),
            [
                "Andorra  (AD) | Standard",
                "France  (FR) | Standard",
                "France  (FR) | Torrent",
                "France  (FR) | TF1 FR",
                "Netherlands  (NL) | Torrent",
                "US  (US) | Netflix US",
            ]
        );
    }

    #[test]
    fn favorites_are_ticked_and_first() {
        let mut fav = Favorites::default();
        fav.add(Favorite::torrent("NL"));
        fav.add(Favorite::streaming("TF1 FR", "FR"));
        let r = rows(&lists(), &fav, &EN);
        assert_eq!(
            &summary(&r)[..3],
            [
                "x France  (FR) | TF1 FR",
                "x Netherlands  (NL) | Torrent",
                "Andorra  (AD) | Standard"
            ]
        );
        assert_eq!(r.iter().filter(|r| r.checked).count(), 2);
    }

    #[test]
    fn unlisted_favorites_are_kept_so_they_can_be_removed() {
        let mut fav = Favorites::default();
        fav.add(Favorite::standard("SE"));
        let r = rows(&lists(), &fav, &EN);
        assert_eq!(summary(&r)[0], "x SE  (SE) | Standard");
        assert_eq!(r.len(), 7);
    }

    #[test]
    fn server_labels_are_translated() {
        let r = rows(&lists(), &Favorites::default(), &FR);
        assert!(r.iter().any(|r| r.server == "Torrent"));
        assert!(r.iter().any(|r| r.server == "Standard"));
    }

    #[test]
    fn four_cells_per_row() {
        let mut fav = Favorites::default();
        fav.add(Favorite::standard("FR"));
        let r = rows(&lists(), &fav, &EN);
        let c = cells(&r);
        assert_eq!(c.len(), r.len() * 4);
        assert_eq!(c[..4], ["TRUE", "France  (FR)", "Standard", "0"]);
        assert_eq!(c[4..8], ["FALSE", "Andorra  (AD)", "Standard", "1"]);
    }

    #[test]
    fn cells_cannot_be_taken_for_options() {
        let l = Lists {
            traffic: vec![],
            streaming: vec![("--text=evil".into(), "US".into())],
            torrent: vec![],
        };
        let c = cells(&rows(&l, &Favorites::default(), &EN));
        assert_eq!(c[2], " --text=evil");
        assert!(c.iter().all(|x| !x.starts_with('-')));
    }

    #[test]
    fn full_lists_fit_on_the_command_line() {
        // ~430 real rows; ARG_MAX on Linux is >= 2 MB, a single argument may be 128 KB
        let svc: Vec<(String, String)> = (0..500)
            .map(|i| (format!("Some Streaming Service {i} XX"), "US".into()))
            .collect();
        let l = Lists {
            traffic: vec![],
            streaming: svc,
            torrent: vec![],
        };
        let c = cells(&rows(&l, &Favorites::default(), &EN));
        let total: usize = c.iter().map(|x| x.len() + 1).sum();
        assert!(total < 256 * 1024, "{total} bytes");
    }

    #[test]
    fn newlines_in_names_cannot_break_rows() {
        let l = Lists {
            traffic: vec![],
            streaming: vec![("Bad\nName".into(), "US".into())],
            torrent: vec![],
        };
        let c = cells(&rows(&l, &Favorites::default(), &EN));
        assert_eq!(c.len(), 4);
        assert_eq!(c[2], "Bad Name");
    }

    #[test]
    fn apply_keeps_order_adds_and_removes() {
        let mut cur = Favorites::default();
        cur.add(Favorite::torrent("NL"));
        cur.add(Favorite::standard("FR"));
        cur.add(Favorite::streaming("TF1 FR", "FR"));
        let r = rows(&lists(), &cur, &EN);
        let idx = |f: &Favorite| r.iter().position(|x| &x.fav == f).unwrap().to_string();
        // untick FR standard, keep NL + TF1, tick Andorra
        let out = [
            idx(&Favorite::standard("AD")),
            idx(&Favorite::streaming("TF1 FR", "FR")),
            idx(&Favorite::torrent("NL")),
        ]
        .join("\n");
        let new = apply(&out, &r, &cur);
        assert_eq!(
            new.iter().cloned().collect::<Vec<_>>(),
            [
                Favorite::torrent("NL"),
                Favorite::streaming("TF1 FR", "FR"),
                Favorite::standard("AD")
            ]
        );
    }

    #[test]
    fn apply_with_nothing_ticked_clears_everything() {
        let mut cur = Favorites::default();
        cur.add(Favorite::standard("FR"));
        let r = rows(&lists(), &cur, &EN);
        assert!(apply("", &r, &cur).is_empty());
        assert!(apply("\n", &r, &cur).is_empty());
    }

    #[test]
    fn apply_ignores_garbage_and_accepts_pipe_separator() {
        let r = rows(&lists(), &Favorites::default(), &EN);
        let new = apply("999\nabc\n-1\n0|2\n0\n", &r, &Favorites::default());
        assert_eq!(
            new.iter().cloned().collect::<Vec<_>>(),
            [r[0].fav.clone(), r[2].fav.clone()]
        );
    }

    #[test]
    fn dialog_arguments() {
        let a = args(&FR);
        let pos = |x: &str| a.iter().position(|v| v == x).unwrap();
        assert_eq!(a[pos("--title") + 1], "Favoris CyberGhost");
        assert_eq!(a[pos("--print-column") + 1], "4");
        assert_eq!(a[pos("--hide-column") + 1], "4");
        assert_eq!(a[pos("--separator") + 1], "\n");
        assert_eq!(a.iter().filter(|v| *v == "--column").count(), 4);
        assert!(a.contains(&"--checklist".to_string()));
    }

    #[test]
    fn missing_program_is_reported() {
        let out = edit("/nonexistent/zenity", &lists(), &Favorites::default(), &EN);
        assert!(
            matches!(out, Outcome::Missing(ref m) if m.starts_with("/nonexistent/zenity")),
            "{out:?}"
        );
    }

    #[test]
    fn ok_and_cancel_are_distinguished() {
        let d = tempfile::tempdir().unwrap();
        let ok = d.path().join("ok");
        std::fs::write(&ok, "#!/bin/sh\ncat >/dev/null\necho 1\n").unwrap();
        let cancel = d.path().join("cancel");
        std::fs::write(&cancel, "#!/bin/sh\ncat >/dev/null\nexit 1\n").unwrap();
        for p in [&ok, &cancel] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let r = rows(&lists(), &Favorites::default(), &EN);
        match edit(ok.to_str().unwrap(), &lists(), &Favorites::default(), &EN) {
            Outcome::Saved(f) => {
                assert_eq!(f.iter().cloned().collect::<Vec<_>>(), [r[1].fav.clone()])
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            edit(
                cancel.to_str().unwrap(),
                &lists(),
                &Favorites::default(),
                &EN
            ),
            Outcome::Cancelled
        );
    }
}
