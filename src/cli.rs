//! Running the CyberGhost CLI and interpreting its output.

use crate::i18n::{Strings, fill};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const DEFAULT_BIN_CANDIDATES: &[&str] = &[
    "/usr/bin/cyberghostvpn",
    "/usr/local/bin/cyberghostvpn",
    "/usr/local/cyberghost/cyberghostvpn",
];

/// Why a command produced no usable output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    Spawn { program: String, reason: String },
    Timeout(u64),
}

/// A failed CLI operation, as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The command ran; this is the most relevant line of its output.
    Output(String),
    Run(RunError),
}

impl Failure {
    pub fn from_output(out: &str) -> Failure {
        Failure::Output(error_summary(out).unwrap_or_default())
    }

    pub fn describe(&self, s: &Strings) -> String {
        match self {
            Failure::Output(line) if line.is_empty() => s.err_no_output.to_string(),
            Failure::Output(line) => line.clone(),
            Failure::Run(RunError::Spawn { program, reason }) => {
                fill(s.err_spawn, &[("program", program), ("reason", reason)])
            }
            Failure::Run(RunError::Timeout(secs)) => {
                fill(s.err_timeout, &[("secs", &secs.to_string())])
            }
        }
    }
}

/// Run a command with a timeout; returns stdout + "\n" + stderr, trimmed.
/// The exit status is ignored: `cyberghostvpn` exits 0 even when it fails.
pub fn run(cmd: &[String], timeout: Duration) -> Result<String, RunError> {
    let mut child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RunError::Spawn {
            program: cmd[0].clone(),
            reason: e.to_string(),
        })?;
    // Read the pipes in threads so a chatty child can't block on a full pipe.
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let t_out = std::thread::spawn(move || read_all(so.as_mut()));
    let t_err = std::thread::spawn(move || read_all(se.as_mut()));
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout(timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                return Err(RunError::Spawn {
                    program: cmd[0].clone(),
                    reason: e.to_string(),
                });
            }
        }
    }
    let out = t_out.join().unwrap_or_default();
    let err = t_err.join().unwrap_or_default();
    Ok([out.trim(), err.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n"))
}

fn read_all<R: std::io::Read>(r: Option<&mut R>) -> String {
    let mut buf = Vec::new();
    if let Some(r) = r {
        let _ = r.read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Command line that runs `bin args…` as root: passwordless sudo when allowed,
/// else pkexec (which keeps the user's HOME so the CLI finds its config).
pub fn root_command(
    sudo_allowed: bool,
    bin: &str,
    args: &[String],
    user: &str,
    home: &str,
) -> Vec<String> {
    let mut c: Vec<String> = if sudo_allowed {
        vec!["sudo".into(), "-n".into(), bin.into()]
    } else {
        vec![
            "pkexec".into(),
            "env".into(),
            format!("SUDO_USER={user}"),
            format!("HOME={home}"),
            bin.into(),
        ]
    };
    c.extend(args.iter().cloned());
    c
}

/// Most relevant line of a CLI output: the last line that looks like an error
/// (Python traceback "File …" lines excluded), else the last line. Max 300 chars.
pub fn error_summary(out: &str) -> Option<String> {
    const KEYS: [&str; 6] = [
        "error",
        "failed",
        "invalid",
        "not found",
        "denied",
        "exception",
    ];
    let lines: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let pick = lines
        .iter()
        .rev()
        .find(|l| {
            let low = l.to_lowercase();
            !l.starts_with("File ") && KEYS.iter().any(|k| low.contains(k))
        })
        .or(lines.last())?;
    Some(pick.chars().take(300).collect())
}

/// `--status` output → connected?  `None` when the output is not recognised.
pub fn parse_status(out: &str) -> Option<bool> {
    let low = out.to_lowercase();
    if low.contains("no vpn connection") {
        Some(false)
    } else if low.contains("vpn connection found") {
        Some(true)
    } else {
        None
    }
}

/// Did `--connect` succeed? The CLI prints this line; its exit code is useless.
pub fn connect_confirmed(out: &str) -> bool {
    out.to_lowercase().contains("connection established")
}

/// Rows of the CLI's ASCII tables: `| 1 | Name | CC |` → (Name, CC).
pub fn parse_table(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let cols: Vec<&str> = l.split('|').map(str::trim).collect();
            let ok = cols.len() == 5
                && cols[0].is_empty()
                && cols[4].is_empty()
                && cols[1].parse::<u32>().is_ok()
                && !cols[2].is_empty()
                && cols[3].len() == 2
                && cols[3].chars().all(|c| c.is_ascii_uppercase());
            ok.then(|| (cols[2].to_string(), cols[3].to_string()))
        })
        .collect()
}

/// Is this the name of a VPN interface (OpenVPN `tun*`, WireGuard `wg*` / `cg*`)?
pub fn is_vpn_iface(name: &str) -> bool {
    name.starts_with("tun") || name.starts_with("wg") || name.starts_with("cg")
}

/// Is a VPN network interface present among these names?
pub fn has_vpn_iface<I: IntoIterator<Item = String>>(names: I) -> bool {
    names.into_iter().any(|n| is_vpn_iface(&n))
}

/// Current network interface names (from /sys/class/net).
pub fn iface_names() -> Vec<String> {
    std::fs::read_dir("/sys/class/net")
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

pub fn vpn_iface_up() -> bool {
    has_vpn_iface(iface_names())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lists {
    /// (country name, code), sorted by name
    pub traffic: Vec<(String, String)>,
    /// (service name, country code), CLI order
    pub streaming: Vec<(String, String)>,
    /// (country name, code) with torrent-optimized servers, sorted by name
    pub torrent: Vec<(String, String)>,
}

impl Lists {
    /// Country names by code, from the country and torrent lists.
    pub fn country_names(&self) -> std::collections::HashMap<String, String> {
        self.torrent
            .iter()
            .chain(self.traffic.iter())
            .map(|(n, c)| (c.clone(), n.clone()))
            .collect()
    }

    /// True when one of the lists is missing (e.g. a cache from an older version).
    pub fn incomplete(&self) -> bool {
        self.traffic.is_empty() || self.streaming.is_empty() || self.torrent.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListsError {
    Traffic(Failure),
    Streaming(Failure),
    Torrent(Failure),
}

impl ListsError {
    pub fn describe(&self, s: &Strings) -> String {
        match self {
            ListsError::Traffic(f) => fill(s.err_country_list, &[("error", &f.describe(s))]),
            ListsError::Streaming(f) => fill(s.err_streaming_list, &[("error", &f.describe(s))]),
            ListsError::Torrent(f) => fill(s.err_torrent_list, &[("error", &f.describe(s))]),
        }
    }
}

/// Wrapper around the `cyberghostvpn` binary.
#[derive(Debug, Clone)]
pub struct Cli {
    pub bin: String,
    pub connect_timeout: Duration,
}

impl Cli {
    pub fn new(bin: String, connect_timeout: Duration) -> Cli {
        Cli {
            bin,
            connect_timeout,
        }
    }

    /// `$CYBERGHOST_BIN`, else the first existing default path.
    pub fn locate() -> String {
        std::env::var("CYBERGHOST_BIN")
            .ok()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| {
                DEFAULT_BIN_CANDIDATES
                    .iter()
                    .find(|p| std::fs::metadata(p).is_ok())
                    .unwrap_or(&DEFAULT_BIN_CANDIDATES[0])
                    .to_string()
            })
    }

    pub fn exists(&self) -> bool {
        std::fs::metadata(&self.bin).is_ok()
    }

    fn user_run(&self, args: &[&str], timeout: Duration) -> Result<String, RunError> {
        let mut c = vec![self.bin.clone()];
        c.extend(args.iter().map(|s| s.to_string()));
        run(&c, timeout)
    }

    fn root_run(&self, args: &[String], timeout: Duration) -> Result<String, RunError> {
        static SUDO_OK: OnceLock<bool> = OnceLock::new();
        // `sudo -n -l <bin>` succeeds only if a NOPASSWD rule allows <bin>.
        let sudo = *SUDO_OK.get_or_init(|| {
            Command::new("sudo")
                .args(["-n", "-l", &self.bin])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        });
        let user = std::env::var("USER").unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        run(&root_command(sudo, &self.bin, args, &user, &home), timeout)
    }

    /// Authoritative connection state; falls back to the interface check
    /// when the output is not recognised or the CLI can't run.
    pub fn status(&self) -> bool {
        self.user_run(&["--status"], Duration::from_secs(15))
            .ok()
            .and_then(|out| parse_status(&out))
            .unwrap_or_else(vpn_iface_up)
    }

    /// Connect with the given CLI arguments. Ok(()) only if the CLI confirmed it.
    pub fn connect(&self, args: &[String]) -> Result<(), Failure> {
        match self.root_run(args, self.connect_timeout) {
            Ok(out) if connect_confirmed(&out) => Ok(()),
            Ok(out) => Err(Failure::from_output(&out)),
            Err(e) => Err(Failure::Run(e)),
        }
    }

    pub fn stop(&self) -> Result<String, Failure> {
        self.root_run(&["--stop".into()], Duration::from_secs(60))
            .map_err(Failure::Run)
    }

    pub fn fetch_lists(&self) -> Result<Lists, ListsError> {
        let t = Duration::from_secs(60);
        let out = self
            .user_run(&["--traffic", "--country-code"], t)
            .map_err(|e| ListsError::Traffic(Failure::Run(e)))?;
        let mut traffic = parse_table(&out);
        if traffic.is_empty() {
            return Err(ListsError::Traffic(Failure::from_output(&out)));
        }
        traffic.sort_by_key(|(n, _)| n.to_lowercase());
        let out = self
            .user_run(&["--streaming", "--country-code"], t)
            .map_err(|e| ListsError::Streaming(Failure::Run(e)))?;
        let streaming = parse_table(&out);
        if streaming.is_empty() {
            return Err(ListsError::Streaming(Failure::from_output(&out)));
        }
        let out = self
            .user_run(&["--torrent", "--country-code"], t)
            .map_err(|e| ListsError::Torrent(Failure::Run(e)))?;
        let mut torrent = parse_table(&out);
        if torrent.is_empty() {
            return Err(ListsError::Torrent(Failure::from_output(&out)));
        }
        torrent.sort_by_key(|(n, _)| n.to_lowercase());
        Ok(Lists {
            traffic,
            streaming,
            torrent,
        })
    }
}

/// CLI arguments for a country connection.
pub fn traffic_args(cc: &str) -> Vec<String> {
    ["--traffic", "--country-code", cc, "--connect"]
        .map(String::from)
        .to_vec()
}

/// CLI arguments for a torrent-optimized connection.
pub fn torrent_args(cc: &str) -> Vec<String> {
    ["--torrent", "--country-code", cc, "--connect"]
        .map(String::from)
        .to_vec()
}

/// CLI arguments for a streaming-service connection (service name stays one argument).
pub fn streaming_args(service: &str, cc: &str) -> Vec<String> {
    ["--streaming", service, "--country-code", cc, "--connect"]
        .map(String::from)
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{EN, FR};

    const TRAFFIC: &str = "\
+-----+----------------------+--------------+
| No. |     Country Name     | Country Code |
+-----+----------------------+--------------+
|  1  |       Andorra        |      AD      |
|  2  | United Arab Emirates |      AE      |
|  8  | Bosnia & Herzegovina |      BA      |
+-----+----------------------+--------------+";

    const STREAMING: &str = "\
+-----+-----------------------+--------------+
| No. |        Service        | Country Code |
+-----+-----------------------+--------------+
|  1  |      Crave TV CA      |      CA      |
|  2  |   NBA League Pass US  |      US      |
|  20 |          HRT          |      HR      |
+-----+-----------------------+--------------+";

    const TRACEBACK: &str = r#"Traceback (most recent call last):
  File "requests/models.py", line 1120, in json
requests.exceptions.JSONDecodeError: Expecting value: line 1 column 1 (char 0)
Connection failed: Expecting value: line 1 column 1 (char 0)"#;

    // ---- parse_table
    #[test]
    fn parses_traffic_table() {
        assert_eq!(
            parse_table(TRAFFIC),
            vec![
                ("Andorra".into(), "AD".into()),
                ("United Arab Emirates".into(), "AE".into()),
                ("Bosnia & Herzegovina".into(), "BA".into()),
            ]
        );
    }

    #[test]
    fn parses_streaming_table_keeping_spaces_in_names() {
        let rows = parse_table(STREAMING);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], ("NBA League Pass US".into(), "US".into()));
        assert_eq!(rows[2], ("HRT".into(), "HR".into()));
    }

    #[test]
    fn table_ignores_header_borders_and_garbage() {
        let s = "| No. | Name | Code |\n+---+\n|  x  | Foo | FR |\n| 1 | Foo | fr |\n\
                 | 1 | Foo | FRA |\n| 1 |  | FR |\n 1 | Foo | FR \nhello | world\n";
        assert!(parse_table(s).is_empty());
        assert!(parse_table("").is_empty());
        assert!(parse_table(TRACEBACK).is_empty());
    }

    #[test]
    fn table_handles_crlf() {
        let rows = parse_table("|  1  | France |  FR  |\r\n|  2  | Spain |  ES  |\r\n");
        assert_eq!(
            rows,
            vec![
                ("France".into(), "FR".into()),
                ("Spain".into(), "ES".into())
            ]
        );
    }

    // ---- error_summary
    #[test]
    fn summary_picks_last_error_line_not_traceback_file_lines() {
        assert_eq!(
            error_summary(TRACEBACK).unwrap(),
            "Connection failed: Expecting value: line 1 column 1 (char 0)"
        );
    }

    #[test]
    fn summary_falls_back_to_last_line() {
        assert_eq!(error_summary("one\n\ntwo\n  ").unwrap(), "two");
    }

    #[test]
    fn summary_of_empty_output_is_none() {
        assert_eq!(error_summary(""), None);
        assert_eq!(error_summary(" \n\t\n"), None);
    }

    #[test]
    fn summary_is_case_insensitive_and_truncated() {
        assert_eq!(
            error_summary("ok\nACCESS DENIED\nbye").unwrap(),
            "ACCESS DENIED"
        );
        let long = format!("error {}", "é".repeat(400));
        let s = error_summary(&long).unwrap();
        assert_eq!(s.chars().count(), 300);
    }

    // ---- status / connect parsing
    #[test]
    fn status_parsing() {
        assert_eq!(parse_status("No VPN connections found."), Some(false));
        assert_eq!(parse_status("VPN connection found."), Some(true));
        assert_eq!(parse_status("Something else"), None);
        assert_eq!(parse_status(""), None);
    }

    #[test]
    fn connect_confirmation() {
        let ok = "Select server ... x\nConnecting ...\nVPN connection established.";
        assert!(connect_confirmed(ok));
        assert!(!connect_confirmed(TRACEBACK));
        assert!(!connect_confirmed(""));
    }

    // ---- failures
    #[test]
    fn failure_descriptions_are_translated() {
        let f = Failure::Run(RunError::Timeout(120));
        assert_eq!(f.describe(&EN), "Timed out (120 s)");
        assert_eq!(f.describe(&FR), "Délai dépassé (120 s)");
        let f = Failure::Run(RunError::Spawn {
            program: "/x".into(),
            reason: "No such file".into(),
        });
        assert_eq!(f.describe(&EN), "Cannot run /x: No such file");
        assert_eq!(Failure::from_output("").describe(&EN), "no output");
        assert_eq!(Failure::from_output("").describe(&FR), "aucune sortie");
        assert_eq!(
            Failure::from_output(TRACEBACK).describe(&FR),
            "Connection failed: Expecting value: line 1 column 1 (char 0)"
        );
    }

    #[test]
    fn lists_error_descriptions() {
        let e = ListsError::Traffic(Failure::Output("Error: boom".into()));
        assert_eq!(e.describe(&EN), "country list: Error: boom");
        let e = ListsError::Streaming(Failure::Output("Error: boom".into()));
        assert_eq!(e.describe(&FR), "liste streaming : Error: boom");
    }

    // ---- command lines
    #[test]
    fn root_command_with_sudo() {
        let c = root_command(true, "/usr/bin/cg", &traffic_args("FR"), "bob", "/home/bob");
        assert_eq!(
            c,
            [
                "sudo",
                "-n",
                "/usr/bin/cg",
                "--traffic",
                "--country-code",
                "FR",
                "--connect"
            ]
        );
    }

    #[test]
    fn root_command_with_pkexec_keeps_user_home() {
        let c = root_command(false, "/usr/bin/cg", &["--stop".into()], "bob", "/home/bob");
        assert_eq!(
            c,
            [
                "pkexec",
                "env",
                "SUDO_USER=bob",
                "HOME=/home/bob",
                "/usr/bin/cg",
                "--stop"
            ]
        );
    }

    #[test]
    fn torrent_args_use_torrent_server_type() {
        assert_eq!(
            torrent_args("NL"),
            ["--torrent", "--country-code", "NL", "--connect"]
        );
    }

    #[test]
    fn torrent_list_error_description() {
        let e = ListsError::Torrent(Failure::Output("Error: boom".into()));
        assert_eq!(e.describe(&EN), "torrent list: Error: boom");
        assert_eq!(e.describe(&FR), "liste torrent : Error: boom");
    }

    #[test]
    fn lists_helpers() {
        let mut l = Lists {
            traffic: vec![("France".into(), "FR".into())],
            streaming: vec![("Netflix US".into(), "US".into())],
            torrent: vec![
                ("Netherlands".into(), "NL".into()),
                ("France".into(), "FR".into()),
            ],
        };
        let n = l.country_names();
        assert_eq!(n["NL"], "Netherlands", "torrent-only country");
        assert_eq!(n["FR"], "France");
        assert!(!l.incomplete());
        l.torrent.clear();
        assert!(l.incomplete(), "cache from before torrent support");
    }

    #[test]
    fn streaming_args_keep_service_as_one_argument() {
        assert_eq!(
            streaming_args("Netflix US", "US"),
            [
                "--streaming",
                "Netflix US",
                "--country-code",
                "US",
                "--connect"
            ]
        );
    }

    // ---- interfaces
    #[test]
    fn vpn_interfaces() {
        let v = |n: &[&str]| has_vpn_iface(n.iter().map(|s| s.to_string()));
        assert!(v(&["lo", "wlp3s0", "tun0"]));
        assert!(v(&["wg0"]));
        assert!(v(&["cg-vpn"]));
        assert!(!v(&["lo", "enp2s0", "wlp3s0", "docker0"]));
        assert!(!v(&[]));
    }

    // ---- run()
    #[test]
    fn run_captures_stdout_and_stderr() {
        let out = run(
            &[
                "sh".into(),
                "-c".into(),
                "echo out; echo err >&2; exit 3".into(),
            ],
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(out, "out\nerr");
    }

    #[test]
    fn run_reports_missing_program() {
        let e = run(&["/nonexistent/prog".into()], Duration::from_secs(5)).unwrap_err();
        assert!(matches!(e, RunError::Spawn { ref program, .. } if program == "/nonexistent/prog"));
    }

    #[test]
    fn run_times_out_and_kills() {
        let t = Instant::now();
        let e = run(&["sleep".into(), "5".into()], Duration::from_millis(300)).unwrap_err();
        assert_eq!(e, RunError::Timeout(0));
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn run_handles_large_output() {
        // more than a pipe buffer (64 KiB) on both streams
        let out = run(
            &[
                "sh".into(),
                "-c".into(),
                "head -c 200000 /dev/zero | tr '\\0' a; head -c 200000 /dev/zero | tr '\\0' b >&2"
                    .into(),
            ],
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(out.len(), 400_001);
    }

    #[test]
    fn missing_cli_operations_fail_cleanly() {
        let cli = Cli::new("/nonexistent/cyberghostvpn".into(), Duration::from_secs(1));
        assert!(!cli.exists());
        match cli.fetch_lists() {
            Err(ListsError::Traffic(Failure::Run(RunError::Spawn { .. }))) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}
