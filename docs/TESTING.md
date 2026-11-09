# Tests

| Suite | Command | What it covers | Count |
|---|---|---|---|
| Unit | `make test-unit` | pure logic in `src/*` (parsing, i18n, config, favorites, favorites window, icons, netlink, menu model) | 113 |
| End-to-end | `make test-e2e` | the real binary driven over D-Bus | 71 |
| Package | `make test-package` | the release ZIP | 21 checks |
| Installer | `make test-install` | `install.sh` / `uninstall.sh` | 23 |

`make test-all` runs everything, plus rustfmt and clippy.

## How the end-to-end tests work

`tests/support/mod.rs` gives each test its own sandbox:

- **D-Bus:** a private session bus (`dbus-daemon --session`). Tests never see your desktop's bus.
- **Tray host:** a fake host (`org.kde.StatusNotifierWatcher`) that the indicator registers with.
- **Fake commands:** `cyberghostvpn`, `sudo`, `pkexec`, `notify-send` and `zenity` scripts. They log every call
  (arguments stay intact, so a name with spaces is checked as one argument). The fake CLI's behaviour
  is set per test through files (`connect_mode=fail|slow|hang|silent`, `stop_mode`,
  `lists_mode=garbage|traffic_only|torrent_down`,
  `status_mode`, `connected`, `sudo_denied`).
- **Clean environment:** the app starts with an empty environment except for the variables the test sets
  (`HOME`, the `XDG_*` directories, `LANG`/`LANGUAGE`/…), so the host's locale and files don't leak in.

The test acts like a panel. It reads the menu with dbusmenu `GetLayout`, clicks with `Event`, and reads
the icon from the `IconName` property. Then it checks what the user would see and what the app did on
disk and on the command line.

The tests run in parallel and need no root or VPN. Your real tray and any running indicator are not
touched.

## Scenarios

### Startup (`tests/e2e_startup.rs`)

| Scenario | Expected |
|---|---|
| No cache | lists fetched from the CLI, countries sorted, streaming grouped by country (unknown country codes shown by code), cache written, no notification |
| Lists cached | menu built from the cache, no CLI list call |
| Already connected, unknown country | colored ghost, "Connected", Disconnect enabled |
| Already connected, last country saved | that flag, "Connected: Switzerland", tooltip/title |
| Saved country but not connected | grey ghost, stale state cleared |
| Corrupted `lists.json` / `state.json` / `config.json` | starts with defaults, lists re-fetched |
| Second instance | exits 0 with "already running" (translated), first keeps running |
| Quit menu item | clean exit (0), lock released, can start again |
| `--version`, `-h` | version line; help mentions usage, config keys, env, languages |
| Unknown argument | exit 2, error message |
| No icon directory | theme icon `network-vpn`, menu without flags, no crash |
| Icon names | absolute paths (avoids the theme's generic `flag` icon) |
| Idle, disconnected / connected / after a connect+disconnect | over 4 s: no command run, ≤ 2 wakeups, 0 CPU, ≤ 4 threads; only one `--status` at startup |
| `status_interval` set | periodic `--status` checks happen |
| CLI missing | critical "CyberGhost not found" + list failure explaining why, menu shows "Loading…" |

### Connections (`tests/e2e_connection.rs`)

| Scenario | Expected |
|---|---|
| Connect to a country | status "Connecting to …", menu locked, hourglass animates (both frames), then the flag, "Connected: …", notification with the flag, state saved, `sudo -n` used, no `--stop` |
| Connect to a streaming service | `--streaming "Netflix US"` passed as one argument, flag of its country |
| Country without a flag | "?" flag in the menu and as the icon |
| CLI prints an error (exit code 0) | critical notification with the relevant line (not the traceback), grey ghost, state cleared |
| CLI prints nothing | notification says "no output" |
| CLI hangs | killed after `connect_timeout`, "Timed out (1 s)", menu usable again |
| Switch country while connected | `--stop` then `--connect`, in that order |
| Switch fails | message adds "(the previous connection was closed)" |
| Disconnect | grey ghost, notification, state cleared, Disconnect disabled |
| Disconnect fails | critical notification with the CLI error, flag and state kept |
| Clicks while busy (stale menu ids) | ignored: one `--connect`, no `--stop` |
| Disconnect while not connected | ignored |
| No sudo rule | falls back to `pkexec env SUDO_USER=… HOME=…`, connection works |
| Disconnected from a terminal | detected by the status check (`status_interval` in the test, a netlink interface event in real use), icon and state updated, no notification |
| Connected from a terminal | colored ghost |
| Unrecognised `--status` output | no crash, valid state |

### Torrent and favorites (`tests/e2e_favorites.rs`)

| Scenario | Expected |
|---|---|
| Connect to a torrent server | `--torrent --country-code NL --connect`, flag, "Connected: Netherlands (Torrent)" |
| Torrent while busy | Torrent and Favorites menus locked like the others |
| Favorites menu | grouped by country (sorted), sub-list only with several favorites (Standard, Torrent, services), single entry "Netherlands  (Torrent)", flags |
| Stars in the lists | ★ after favorites in Countries, Streaming and Torrent only, per server type |
| Connect from Favorites | right CLI arguments for streaming and torrent favorites, right status |
| ★ entry | connects, favorites untouched, no window |
| Favorites window | every server listed (13 rows), favorites ticked and first, then by country; checklist arguments, translated title and columns |
| Tick in the window | favorites saved in window order, Favorites menu and ★ updated, never connects |
| Untick | removed, the others keep their order, France drops to a single entry |
| OK with nothing ticked / Cancel | everything removed / nothing changed |
| Click "Edit favorites…" twice | one window at a time, can be reopened once closed |
| Window during a connection | allowed, the connection finishes normally |
| Favorite no longer in the lists | shown in the window (ticked) so it can be removed |
| zenity missing | critical notification with the install command, the app keeps working |
| Restart | favorites kept |
| Favorite of a country no longer listed | kept, shown by code with its flag |
| Corrupted `favorites.json` | ignored |
| French | "Favoris", "Modifier les favoris…", window title and columns in French, "Connecté : France (Torrent)" |

### Lists and language (`tests/e2e_lists_language.rs`)

| Scenario | Expected |
|---|---|
| Manual refresh | "Lists updated: 5 countries, 5 streaming services, 3 torrent countries", menu and cache updated |
| Refresh fails | critical notification ("country list: …"), old lists and cache kept, can retry |
| Streaming list fails | reported as "streaming list: …" |
| Torrent list fails | reported as "torrent list: …" |
| Cache from an older version (no torrent list) | completed silently at startup |
| First fetch fails | notification, "Loading…" placeholders, a later refresh fixes it |
| `LANG=fr_FR.UTF-8` | French menu, status and title |
| `LANG=en_GB` / `de_DE` / no locale at all | English |
| `LANGUAGE=fr:en` with `LANG=en_US` | French (`LANGUAGE` wins) |
| `LC_ALL=C` with `LANGUAGE=fr` | English (C locale ignores `LANGUAGE`) |
| `LC_MESSAGES=fr` with `LANG=en` | French |
| Language menu | "System default (Français)", English, Français; the current choice is checked |
| Choose a language | applies immediately, saved, kept after a restart; "System default" goes back to the locale |
| Forced language | notifications and error details in that language |
| Invalid `language` in config | system language |
| Changing language | other settings in `config.json` are kept |

### Unit tests (`src/*`)

| Module | Covers |
|---|---|
| `cli` | table parsing (borders, garbage, CRLF, names with spaces/`&`), error-line selection (tracebacks, case, truncation, empty), `--status` parsing, connect confirmation, sudo/pkexec command lines, translated failure messages, `run()` (stdout+stderr, missing program, timeout + kill, output larger than the pipe buffer), VPN interface detection |
| `i18n` | locale parsing (`fr`, `fr_CA`, `fr_FR@euro`, `fr-CH`, case), precedence `LC_ALL` > `LC_MESSAGES` > `LANG`, `LANGUAGE` lists, C/POSIX, empty variables, preference codes, **every language complete with the same placeholders as English** |
| `store` | XDG paths (relative values ignored), config defaults/validation per field/roundtrip/corruption, lists and state roundtrip and validation, atomic writes |
| `icons` | every bundled icon is a 64 px RGBA square, status icons and common flags present, directory lookup order (override → next to the binary → `./assets`), "?" flag for unknown/invalid codes, theme fallback, 16 px downscale (averaging, transparency, rejects non-RGBA, non-square, odd sizes, tiny, garbage) |
| `netwatch` | rtnetlink parsing (NEWLINK/DELLINK, interface name, other attributes and message types skipped, truncated and garbage data never panic), VPN interface tracking from events (removal seen before sysfs updates, several VPN interfaces, unrelated interfaces ignored), socket subscription |
| `favorites` | add/remove/toggle, no duplicates, grouping by country and server order, unknown country codes, CLI arguments per server type, labels, JSON roundtrip and invalid entries, load/save/corruption |
| `dialog` | rows (every server, favorites ticked and first, unlisted favorites kept, translated), zenity stdin format and arguments, newline-safe, applying the result (order kept, add, remove, nothing ticked, garbage ignored), OK vs Cancel, missing program |
| `tray` | menu structure in English/French, sorting and grouping, placeholders, every item sends the right command, torrent list, favorites grouping, ★ markers, "Edit favorites…" (enabled while busy), `set_favorites` saves, busy/refreshing states, status labels, icon per state and animation frame, language menu and persistence, `_` escaping |

### Package (`tests/package_test.sh`)

ZIP name and single top directory, `.sha256` and `SHA256SUMS` (covering every file), executables, README and
icon licence, binary statically linked / x86-64 / stripped / right version, icons identical to `assets/`,
scripts identical to `packaging/` and valid bash, no stray or unexpected files, size < 10 MB.

### Installer (`tests/install_test.sh`)

Each test runs in a temporary `HOME` with fake `sudo`, `visudo`, `pkill`, `busctl`, `setsid` and `id`:
`/etc` is never written and running indicators are never killed.

| Scenario | Expected |
|---|---|
| Default install | binary (755) and icons (644), uninstaller, autostart entry (`Exec`, `Icon`, custom CLI path), sudoers rule validated with `visudo` then installed, manifest, `pkill -x` (never `-f`), not started without a display |
| `--no-autostart --no-sudoers` | nothing else created, no sudo call |
| `--prefix ./rel`, `--prefix=…`, run from `/` | correct absolute paths |
| Reinstall | stale icons removed, manifest without duplicates, existing sudo rule detected |
| Uninstall | removes exactly what was installed (incl. sudoers via sudo), keeps other files and the cache |
| `uninstall --purge` | also removes the cache |
| Uninstall from the release dir / with nothing installed | finds the install / fails cleanly |
| CLI missing | warning, no sudoers rule |
| zenity missing | install refused before anything is written, with the install commands |
| Rule already configured | detected, `visudo` not run |
| Not interactive, no `--yes` | rule skipped with a warning |
| `visudo` rejects the rule | not installed, not in the manifest |
| CLI is a symlink | rule allows both the link and its target |
| Per-user install as root / `--system` as a user | refused |
| No tray host | warning with instructions |
| Graphical session / `--no-start` | started with `setsid` / not started |
| `--help`, bad option, `--prefix` without a value, script outside a release | help / exit 2 / error / refused |

## Not covered automatically

- **The real desktop panel** (XFCE indicator plugin) rendering the menu and icons, including how its
  font draws ★, and the real zenity window. The tests check the D-Bus protocol the panel uses, but not the pixels.
- **Real VPN interfaces appearing/disappearing** (creating a `tun` interface needs root). The event
  parsing and tracking are unit-tested; detection was checked by hand: a connect or `--stop` from a
  terminal is shown within ~0.02 s.
- **The real CyberGhost CLI and network.** Its output formats are captured in the fake CLI and in the unit
  test fixtures. If CyberGhost changes its output, update those fixtures.
- **`--system` installs as root**, which would need a disposable machine or a container.
