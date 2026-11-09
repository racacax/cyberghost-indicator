//! Main loop: owns the tray handle, dispatches actions to worker threads.

use crate::cli::{Cli, vpn_iface_up};
use crate::dialog;
use crate::favorites::Favorites;
use crate::i18n::{self, Strings, fill};
use crate::icons::Icons;
use crate::netwatch;
use crate::store::{self, Config, Connection, Paths};
use crate::tray::{Busy, Cmd, Tray};
use ksni::blocking::{Handle, TrayMethods};
use std::collections::HashMap;
use std::fs::File;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

const BUSY_TICK: Duration = Duration::from_millis(500); // hourglass animation
const POLL_TICK: Duration = Duration::from_secs(3); // interface check, only without netlink
const RECHECK_DELAY: Duration = Duration::from_secs(2); // confirmation after an interface change

pub fn notify(summary: &str, body: &str, icon: &str, urgent: bool) {
    let mut c = Command::new("notify-send");
    c.args(["-a", "CyberGhost", "-i", icon]);
    if urgent {
        c.args(["-u", "critical"]);
    }
    if let Ok(mut child) = c
        .arg(summary)
        .arg(body)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || child.wait());
    }
}

/// Shared, read-only context for worker threads.
struct Ctx {
    cli: Cli,
    paths: Arc<Paths>,
    icons: Arc<Icons>,
}

impl Ctx {
    fn save_state(&self, c: &Option<Connection>) {
        if let Err(e) = store::save_state(&self.paths, c) {
            eprintln!("state: {e}");
        }
    }
}

fn strings(h: &Handle<Tray>) -> &'static Strings {
    h.update(|t| t.s()).unwrap_or(&i18n::EN)
}

fn do_connect(
    ctx: Arc<Ctx>,
    h: Handle<Tray>,
    tx: Sender<Cmd>,
    label: String,
    cc: String,
    args: Vec<String>,
    was: bool,
) {
    if was {
        let _ = ctx.cli.stop();
    }
    let res = ctx.cli.connect(&args);
    let up = ctx.cli.status();
    let s = strings(&h);
    let state = match &res {
        Ok(()) if up => Some(Connection {
            cc: cc.clone(),
            label: label.clone(),
        }),
        _ => None,
    };
    ctx.save_state(&state);
    // unlock the main loop before the tray shows the result, so a click
    // made as soon as the icon changes is not dropped as "busy"
    let _ = tx.send(Cmd::Done { up });
    h.update(|t| {
        t.busy = None;
        t.connected = up;
        t.state = state.clone();
    });
    // notify after the tray shows the new state
    if state.is_some() {
        notify(
            &fill(s.n_connected, &[("name", &label)]),
            "",
            &ctx.icons.flag(&cc),
            false,
        );
    } else {
        let mut msg = match res {
            Err(f) => f.describe(s),
            Ok(()) => String::new(), // confirmed by the CLI but status says down
        };
        if was && !up {
            msg.push('\n');
            msg.push_str(s.n_previous_dropped);
        }
        notify(
            &fill(s.n_connect_failed, &[("name", &label)]),
            msg.trim(),
            &ctx.icons.path("error"),
            true,
        );
    }
}

fn do_disconnect(ctx: Arc<Ctx>, h: Handle<Tray>, tx: Sender<Cmd>) {
    let res = ctx.cli.stop();
    let up = ctx.cli.status();
    let s = strings(&h);
    if !up {
        ctx.save_state(&None);
    }
    // unlock the main loop before the tray shows the result, so a click
    // made as soon as the icon changes is not dropped as "busy"
    let _ = tx.send(Cmd::Done { up });
    h.update(|t| {
        t.busy = None;
        t.connected = up;
        if !up {
            t.state = None;
        }
    });
    if up {
        let msg = match res {
            Ok(out) => crate::cli::Failure::from_output(&out).describe(s),
            Err(f) => f.describe(s),
        };
        notify(s.n_disconnect_failed, &msg, &ctx.icons.path("error"), true);
    } else {
        notify(s.n_disconnected, "", &ctx.icons.path("ghost-off"), false);
    }
}

fn do_refresh(ctx: Arc<Ctx>, h: Handle<Tray>, manual: bool) {
    let res = ctx.cli.fetch_lists();
    let s = strings(&h);
    match res {
        Ok(lists) => {
            if let Err(e) = store::save_lists(&ctx.paths, &lists) {
                eprintln!("cache: {e}");
            }
            let body = fill(
                s.n_lists_counts,
                &[
                    ("countries", &lists.traffic.len().to_string()),
                    ("services", &lists.streaming.len().to_string()),
                    ("torrent", &lists.torrent.len().to_string()),
                ],
            );
            h.update(|t| {
                t.refreshing = false;
                t.lists = Arc::new(lists);
                t.load_flags();
            });
            if manual {
                notify(s.n_lists_updated, &body, &ctx.icons.path("ghost-on"), false);
            }
        }
        Err(e) => {
            let had_lists = h.update(|t| {
                t.refreshing = false;
                !t.lists.traffic.is_empty()
            });
            if manual || had_lists == Some(false) {
                notify(
                    s.n_lists_failed,
                    &e.describe(s),
                    &ctx.icons.path("error"),
                    true,
                );
            }
        }
    }
}

fn do_edit_favorites(ctx: Arc<Ctx>, h: Handle<Tray>) {
    let Some((lists, current, s)) =
        h.update(|t| (Arc::clone(&t.lists), t.favorites.clone(), t.s()))
    else {
        return;
    };
    match dialog::edit(&dialog::program(), &lists, &current, s) {
        dialog::Outcome::Saved(f) if f != current => {
            h.update(|t| t.set_favorites(f));
        }
        dialog::Outcome::Saved(_) | dialog::Outcome::Cancelled => {}
        dialog::Outcome::Missing(e) => {
            notify(
                s.n_dialog_missing,
                &fill(s.n_dialog_missing_body, &[("error", &e)]),
                &ctx.icons.path("error"),
                true,
            );
        }
    }
}

/// Run the indicator until "Quit". Returns the process exit code.
pub fn run() -> i32 {
    let paths = Arc::new(Paths::from_env());
    let config = Config::load(&paths);
    let system_lang = i18n::detect_from_env();
    let s = config.language.resolve(system_lang).strings();

    // single instance
    if let Err(e) = std::fs::create_dir_all(&paths.cache) {
        eprintln!("cache directory {}: {e}", paths.cache.display());
        return 1;
    }
    let lock = match File::create(paths.lock_file()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("lock file: {e}");
            return 1;
        }
    };
    if lock.try_lock().is_err() {
        eprintln!("{}", s.already_running);
        return 0;
    }

    let icons = Arc::new(Icons::from_env());
    if icons.dir.is_none() {
        eprintln!("warning: icon directory not found, using theme icons");
    }
    let ctx = Arc::new(Ctx {
        cli: Cli::new(Cli::locate(), config.connect_timeout),
        paths: Arc::clone(&paths),
        icons: Arc::clone(&icons),
    });
    if !ctx.cli.exists() {
        notify(
            s.n_cli_missing,
            &fill(s.n_cli_missing_body, &[("path", &ctx.cli.bin)]),
            "dialog-error",
            true,
        );
    }

    let status_interval = config.status_interval;
    let (tx, rx) = mpsc::channel();
    let mut tray = Tray {
        tx: tx.clone(),
        paths: Arc::clone(&paths),
        icons,
        config,
        system_lang,
        lists: Arc::new(store::load_lists(&paths)),
        favorites: Favorites::load(&paths.favorites_file()),
        flags: HashMap::new(),
        busy: None,
        frame: 0,
        refreshing: false,
        connected: false, // until the first status check
        state: store::load_state(&paths),
    };
    tray.load_flags();
    // empty cache, or a cache written before a list existed (e.g. torrent)
    let need_lists = tray.lists.incomplete();
    if need_lists {
        tray.refreshing = true;
    }
    let handle = match tray.spawn() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("tray: {e}");
            return 1;
        }
    };

    let status_check = {
        let ctx = Arc::clone(&ctx);
        move |tx: Sender<Cmd>| {
            let ctx = Arc::clone(&ctx);
            std::thread::spawn(move || {
                let _ = tx.send(Cmd::Status {
                    up: ctx.cli.status(),
                });
            });
        }
    };
    status_check(tx.clone());
    if need_lists {
        let (c, h) = (Arc::clone(&ctx), handle.clone());
        std::thread::spawn(move || do_refresh(c, h, false));
    }

    // The loop keeps its own busy/connected copy so idle ticks never touch the
    // tray: every Handle::update re-sends properties + menu over D-Bus.
    let mut busy = false;
    let mut editing = false; // favorites window open (only one at a time)
    let mut connected = false;
    let mut first_status = true;
    // Idle cost: the kernel tells us when a VPN interface appears/disappears
    // (netlink), so nothing runs until something changes. Without netlink
    // (e.g. a restrictive sandbox), fall back to checking every POLL_TICK.
    let watched = {
        let (tx1, tx2) = (tx.clone(), tx.clone());
        netwatch::spawn(
            crate::cli::iface_names,
            move |up| {
                let _ = tx1.send(Cmd::LinkChanged { up });
            },
            move || {
                let _ = tx2.send(Cmd::LinkResync);
            },
        )
    };
    let netlink = watched.is_ok();
    let mut last_iface = watched.unwrap_or_else(|_| vpn_iface_up());
    // a second status check shortly after an interface change, in case the CLI
    // had not settled yet when the first one ran
    let mut next_recheck: Option<Instant> = None;
    // optional periodic `--status` (0 = off, the default: it costs ~200 ms of CPU per call)
    let interval = (!status_interval.is_zero()).then_some(status_interval);
    let idle_tick = || (!netlink).then(|| Instant::now() + POLL_TICK);
    let mut next_tick: Option<Instant> = idle_tick();
    let mut next_status: Option<Instant> = interval.map(|i| Instant::now() + i);
    loop {
        let deadline = if busy {
            next_tick
        } else {
            [next_tick, next_status, next_recheck]
                .into_iter()
                .flatten()
                .min()
        };
        let msg = match deadline {
            Some(d) => rx.recv_timeout(d.saturating_duration_since(Instant::now())),
            // nothing scheduled: sleep until a message arrives
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Cmd::Connect { label, cc, args }) if !busy => {
                busy = true;
                let was = connected;
                handle.update(|t| {
                    t.busy = Some(Busy::Connecting(label.clone()));
                    t.frame = 0;
                });
                next_tick = Some(Instant::now() + BUSY_TICK);
                let (c, h, tx) = (Arc::clone(&ctx), handle.clone(), tx.clone());
                std::thread::spawn(move || do_connect(c, h, tx, label, cc, args, was));
            }
            Ok(Cmd::Disconnect) if !busy && connected => {
                busy = true;
                handle.update(|t| {
                    t.busy = Some(Busy::Disconnecting);
                    t.frame = 0;
                });
                next_tick = Some(Instant::now() + BUSY_TICK);
                let (c, h, tx) = (Arc::clone(&ctx), handle.clone(), tx.clone());
                std::thread::spawn(move || do_disconnect(c, h, tx));
            }
            Ok(Cmd::Refresh) => {
                let start = handle
                    .update(|t| !std::mem::replace(&mut t.refreshing, true))
                    .unwrap_or(false);
                if start {
                    let (c, h) = (Arc::clone(&ctx), handle.clone());
                    std::thread::spawn(move || do_refresh(c, h, true));
                }
            }
            Ok(Cmd::EditFavorites) if !editing => {
                editing = true;
                let (c, h, tx) = (Arc::clone(&ctx), handle.clone(), tx.clone());
                std::thread::spawn(move || {
                    do_edit_favorites(c, h);
                    let _ = tx.send(Cmd::EditDone);
                });
            }
            Ok(Cmd::EditDone) => editing = false,
            Ok(Cmd::Quit) => break,
            Ok(Cmd::Done { up }) => {
                busy = false;
                connected = up;
                // our own (dis)connection changed the interfaces: no extra check needed.
                // With netlink, `last_iface` already followed the events (sysfs can lag);
                // when polling, resynchronise so the next tick doesn't re-check.
                if !netlink {
                    last_iface = vpn_iface_up();
                }
                next_tick = idle_tick();
                next_status = interval.map(|i| Instant::now() + i);
            }
            Ok(Cmd::LinkChanged { up }) => {
                // a VPN interface appearing/disappearing triggers an authoritative check
                if up != last_iface {
                    last_iface = up;
                    if !busy {
                        status_check(tx.clone());
                        next_recheck = Some(Instant::now() + RECHECK_DELAY);
                    }
                }
            }
            Ok(Cmd::LinkResync) => {
                last_iface = vpn_iface_up();
                if !busy {
                    status_check(tx.clone());
                }
            }
            Ok(Cmd::Status { up }) if !busy && (up != connected || first_status) => {
                first_status = false;
                connected = up;
                handle.update(|t| {
                    t.connected = up;
                    if !up && t.state.is_some() {
                        t.state = None;
                        ctx.save_state(&None);
                    }
                });
            }
            Ok(_) => {} // ignored while busy / no change
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                if busy {
                    next_tick = Some(now + BUSY_TICK);
                    handle.update(|t| t.frame = t.frame.wrapping_add(1));
                    continue;
                }
                if next_status.is_some_and(|t| now >= t) {
                    next_status = interval.map(|i| now + i);
                    status_check(tx.clone());
                }
                if next_recheck.is_some_and(|t| now >= t) {
                    next_recheck = None;
                    status_check(tx.clone());
                }
                if next_tick.is_some_and(|t| now >= t) {
                    next_tick = idle_tick();
                    let _ = tx.send(Cmd::LinkChanged { up: vpn_iface_up() }); // polling fallback
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    handle.shutdown().wait();
    0
}
