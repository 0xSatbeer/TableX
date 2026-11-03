//! `--bench-scenario`: starts the app on one scenario, prints one `BenchReport` line and quits.
//! `scripts/perf.sh` runs each scenario and checks the numbers against the budgets.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;

use crate::model::{
    Connection, ConnectionId, ConnectionParams, ConnectionStatus, Secret, SslMode, Workspace,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Scenario {
    Startup,
    Idle,
    BigResult,
    GridScroll,
}

impl Scenario {
    /// The scenario named `name`, else the error the contract prints.
    pub(crate) fn parse(name: Option<&str>) -> Result<Self, String> {
        match name {
            Some("startup") => Ok(Self::Startup),
            Some("idle") => Ok(Self::Idle),
            Some("big_result") => Ok(Self::BigResult),
            Some("grid_scroll") => Ok(Self::GridScroll),
            other => Err(format!(
                "unknown bench scenario: {}",
                other.unwrap_or_default()
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Idle => "idle",
            Self::BigResult => "big_result",
            Self::GridScroll => "grid_scroll",
        }
    }
}

#[derive(serde::Serialize)]
pub(crate) struct BenchReport {
    /// "startup" | "idle" | "big_result" | "grid_scroll"
    pub scenario: &'static str,
    /// Wall-clock Unix epoch ms when the first frame was presented; perf.sh subtracts its spawn time.
    pub first_frame_epoch_ms: u64,
    /// RssAnon in MiB at the scenario's measure point (startup: first frame; idle: after the
    /// 10 s window; big_result: after all tabs settle).
    pub rss_anon_mb: f64,
    /// idle only: frames drawn during 10 s with no input, after a 2 s settle. null otherwise.
    pub idle_frames: Option<u32>,
    /// big_result only: rows held by the PG console's result. Must be 100_000. null otherwise.
    pub rows: Option<usize>,
    /// grid_scroll only: frame intervals while big_events scrolls every frame. null otherwise.
    pub frames: Option<FrameStats>,
}

#[derive(serde::Serialize, Debug, PartialEq)]
pub(crate) struct FrameStats {
    pub frames: usize,
    pub fps: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Intervals over 1.5x the median: the hitches you see as stutter.
    pub slow: usize,
}

/// Stats over frame intervals in ms; `None` for an empty list.
pub(crate) fn frame_stats(mut ms: Vec<f64>) -> Option<FrameStats> {
    if ms.is_empty() {
        return None;
    }
    ms.sort_by(f64::total_cmp);
    let at = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    let p50 = at(0.5);
    Some(FrameStats {
        frames: ms.len(),
        fps: ms.len() as f64 * 1000. / ms.iter().sum::<f64>(),
        p50_ms: p50,
        p95_ms: at(0.95),
        p99_ms: at(0.99),
        max_ms: at(1.),
        slow: ms.iter().filter(|&&m| m > p50 * 1.5).count(),
    })
}

/// Frames the shell has drawn.
static FRAMES: AtomicU32 = AtomicU32::new(0);
static FIRST_FRAME: OnceLock<u64> = OnceLock::new();

/// Counts a frame being drawn; true for the first one.
pub(crate) fn frame_drawn() -> bool {
    FRAMES.fetch_add(1, Ordering::Relaxed) == 0
}

/// Marks the first frame presented, now.
pub(crate) fn first_frame_presented() {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    FIRST_FRAME.get_or_init(|| ms.as_millis() as u64);
}

/// RssAnon in MiB from a `/proc/<pid>/status` text.
pub(crate) fn rss_anon_mb(status: &str) -> Option<f64> {
    let line = status.lines().find_map(|l| l.strip_prefix("RssAnon:"))?;
    let kb: f64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    Some(kb / 1024.)
}

fn rss_now() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    rss_anon_mb(&status).unwrap_or(f64::NAN)
}

/// The `big_result` workspace: a Postgres connection on `pg` (a `postgres://` URL) and a SQLite
/// one on the file at `sqlite`.
pub(crate) fn workspace(pg: Option<String>, sqlite: Option<String>) -> Result<Workspace, String> {
    let pg = pg.ok_or("TABLEX_BENCH_PG is not set")?;
    let sqlite = sqlite.ok_or("TABLEX_BENCH_SQLITE is not set")?;
    let config: tokio_postgres::Config = pg
        .parse()
        .map_err(|e| format!("TABLEX_BENCH_PG is not a postgres URL: {e}"))?;
    let Some(tokio_postgres::config::Host::Tcp(host)) = config.get_hosts().first() else {
        return Err("TABLEX_BENCH_PG needs a TCP host".into());
    };
    let conn = |id, name: &str, params| Connection {
        id: ConnectionId(id),
        name: name.into(),
        color: 0,
        is_production: false,
        status: ConnectionStatus::Disconnected,
        params,
        save_password: false,
        databases: None,
    };
    let pg = ConnectionParams::Postgres {
        host: host.clone(),
        port: config.get_ports().first().copied().unwrap_or(5432),
        user: config.get_user().unwrap_or_default().into(),
        password: config
            .get_password()
            .map(|p| Secret::new(String::from_utf8_lossy(p))),
        database: config.get_dbname().unwrap_or_default().into(),
        ssl: SslMode::Prefer,
    };
    Ok(Workspace {
        id: 1,
        name: "Bench".into(),
        connections: vec![
            conn(1, "bench-pg", pg),
            conn(
                2,
                "bench-sqlite",
                ConnectionParams::Sqlite {
                    path: sqlite.into(),
                },
            ),
        ],
        id_floor: 0,
    })
}

/// Waits for `scenario`'s measure point, prints its report on stdout and quits.
pub(crate) fn run(
    scenario: Scenario,
    window: AnyWindowHandle,
    shell: Entity<crate::shell::Shell>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let timer = |ms| cx.background_executor().timer(Duration::from_millis(ms));
        let first_frame_epoch_ms = loop {
            match FIRST_FRAME.get() {
                Some(ms) => break *ms,
                None => timer(5).await,
            }
        };
        let mut frames = None;
        let (idle_frames, rows) = match scenario {
            Scenario::Startup => (None, None),
            Scenario::GridScroll => {
                timer(1_000).await;
                let grid = shell.read_with(cx, |s, _| s.bench_grid());
                let mut ms = Vec::new();
                let mut last = std::time::Instant::now();
                let end = last + Duration::from_secs(8);
                while last < end {
                    let (tx, rx) = futures::channel::oneshot::channel();
                    _ = window.update(cx, |_, window, cx| {
                        if let Some(grid) = &grid {
                            grid.update(cx, |g, cx| g.bench_step(cx));
                        }
                        window.on_next_frame(move |_, _| _ = tx.send(()));
                    });
                    _ = rx.await;
                    let now = std::time::Instant::now();
                    ms.push((now - last).as_secs_f64() * 1000.);
                    last = now;
                }
                frames = frame_stats(ms.split_off(1));
                (None, None)
            }
            Scenario::Idle => {
                timer(2_000).await;
                let start = FRAMES.load(Ordering::Relaxed);
                timer(10_000).await;
                (Some(FRAMES.load(Ordering::Relaxed) - start), None)
            }
            Scenario::BigResult => loop {
                if let Some(rows) = shell.read_with(cx, |s, cx| s.bench_rows(cx)) {
                    break (None, Some(rows));
                }
                timer(100).await;
            },
        };
        let report = BenchReport {
            scenario: scenario.name(),
            first_frame_epoch_ms,
            rss_anon_mb: rss_now(),
            idle_frames,
            rows,
            frames,
        };
        println!(
            "{}",
            serde_json::to_string(&report).expect("a report serializes")
        );
        cx.update(|cx| cx.quit());
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::{BenchReport, Scenario, rss_anon_mb, workspace};

    #[test]
    fn a_scenario_name_parses_and_anything_else_is_an_error() {
        // (name, scenario or error)
        let cases: [(Option<&str>, Result<Scenario, &str>); 7] = [
            (Some("startup"), Ok(Scenario::Startup)),
            (Some("idle"), Ok(Scenario::Idle)),
            (Some("big_result"), Ok(Scenario::BigResult)),
            (Some("grid_scroll"), Ok(Scenario::GridScroll)),
            (
                Some("big-result"),
                Err("unknown bench scenario: big-result"),
            ),
            (Some("STARTUP"), Err("unknown bench scenario: STARTUP")),
            (None, Err("unknown bench scenario: ")),
        ];
        for (name, want) in cases {
            assert_eq!(
                Scenario::parse(name),
                want.map_err(String::from),
                "{name:?}"
            );
        }
    }

    #[test]
    fn a_report_has_exactly_the_contract_keys_with_null_for_absent_options() {
        // (report, JSON)
        let cases = [
            (
                BenchReport {
                    scenario: "startup",
                    first_frame_epoch_ms: 1_700_000_000_123,
                    rss_anon_mb: 31.5,
                    idle_frames: None,
                    rows: None,
                    frames: None,
                },
                r#"{"scenario":"startup","first_frame_epoch_ms":1700000000123,"rss_anon_mb":31.5,"idle_frames":null,"rows":null,"frames":null}"#,
            ),
            (
                BenchReport {
                    scenario: "idle",
                    first_frame_epoch_ms: 1,
                    rss_anon_mb: 2.0,
                    idle_frames: Some(0),
                    rows: None,
                    frames: None,
                },
                r#"{"scenario":"idle","first_frame_epoch_ms":1,"rss_anon_mb":2.0,"idle_frames":0,"rows":null,"frames":null}"#,
            ),
            (
                BenchReport {
                    scenario: "big_result",
                    first_frame_epoch_ms: 1,
                    rss_anon_mb: 99.25,
                    idle_frames: None,
                    rows: Some(100_000),
                    frames: None,
                },
                r#"{"scenario":"big_result","first_frame_epoch_ms":1,"rss_anon_mb":99.25,"idle_frames":null,"rows":100000,"frames":null}"#,
            ),
        ];
        for (report, want) in cases {
            assert_eq!(serde_json::to_string(&report).unwrap(), want);
        }
    }

    #[test]
    fn frame_stats_take_percentiles_and_count_hitches() {
        assert_eq!(super::frame_stats(Vec::new()), None);
        let mut ms = vec![10.; 98];
        ms.extend([16., 40.]);
        let s = super::frame_stats(ms).unwrap();
        assert_eq!(
            (s.frames, s.p50_ms, s.p99_ms, s.max_ms, s.slow),
            (100, 10., 16., 40., 2)
        );
        assert!((s.fps - 100_000. / 1036.).abs() < 1e-9);
    }

    #[test]
    fn rss_anon_is_read_from_proc_status_in_mib() {
        // (status text, MiB)
        let cases: [(&str, Option<f64>); 5] = [
            (
                "Name:\ttablex\nVmRSS:\t  90000 kB\nRssAnon:\t   40960 kB\nRssFile:\t 1 kB\n",
                Some(40.),
            ),
            ("RssAnon:\t512 kB\n", Some(0.5)),
            ("RssAnon:\t0 kB", Some(0.)),
            ("VmRSS:\t1024 kB\n", None),
            ("RssAnon:\tlots kB\n", None),
        ];
        for (status, want) in cases {
            assert_eq!(rss_anon_mb(status), want, "{status:?}");
        }
    }

    #[test]
    fn the_big_result_workspace_comes_from_the_two_variables() {
        use crate::model::{ConnectionParams, SslMode};
        let pg = |host: &str, port, user: &str, password: Option<&str>, database: &str| {
            Ok(vec![
                format!(
                    "{host}:{port} {user} {password:?} {database} {:?}",
                    SslMode::Prefer
                ),
                "/data/bench.sqlite".to_string(),
            ])
        };
        // (TABLEX_BENCH_PG, TABLEX_BENCH_SQLITE, the two connections or the error)
        let cases: [(Option<&str>, Option<&str>, Result<Vec<String>, &str>); 6] = [
            (
                Some("postgres://u:p@h:1/db"),
                Some("/data/bench.sqlite"),
                pg("h", 1, "u", Some("p"), "db"),
            ),
            (
                Some("postgres://u@localhost/q"),
                Some("/data/bench.sqlite"),
                pg("localhost", 5432, "u", None, "q"),
            ),
            (None, Some("/x"), Err("TABLEX_BENCH_PG is not set")),
            (
                Some("postgres://h/db"),
                None,
                Err("TABLEX_BENCH_SQLITE is not set"),
            ),
            (
                Some("postgres://u@%2Frun%2Fpg/db"),
                Some("/x"),
                Err("TABLEX_BENCH_PG needs a TCP host"),
            ),
            (
                Some("postgres://h:notaport/db"),
                Some("/x"),
                Err("not a URL"),
            ),
        ];
        for (pg_url, sqlite, want) in cases {
            let got = workspace(pg_url.map(String::from), sqlite.map(String::from)).map(|ws| {
                ws.connections
                    .into_iter()
                    .map(|c| match c.params {
                        ConnectionParams::Postgres {
                            host,
                            port,
                            user,
                            password,
                            database,
                            ssl,
                        } => format!(
                            "{host}:{port} {user} {:?} {database} {ssl:?}",
                            password.as_ref().map(|p| p.expose())
                        ),
                        ConnectionParams::Sqlite { path } => path.display().to_string(),
                    })
                    .collect::<Vec<_>>()
            });
            match (got, want) {
                (Ok(got), Ok(want)) => assert_eq!(got, want, "{pg_url:?}"),
                (Err(got), Err("not a URL")) => {
                    assert!(
                        got.starts_with("TABLEX_BENCH_PG is not a postgres URL"),
                        "{got}"
                    )
                }
                (got, want) => assert_eq!(got, want.map_err(String::from), "{pg_url:?}"),
            }
        }
    }
}
