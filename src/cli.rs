use clap::{Args, Parser, Subcommand, ValueEnum};
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use crate::commands;
use crate::config::{Config, ConfigError};
use crate::daemon;
use crate::hyprctl::{HyprlandIpc, Hyprctl, HyprctlError, SystemHyprctlRunner};
#[cfg(feature = "native-ipc")]
use crate::hyprctl::NativeIpc;
use crate::paired::CycleDirection;
use crate::paths;
use crate::setup::{self, SetupError};
use crate::waybar::{self, WaybarError};

#[derive(Parser, Debug)]
#[command(
    name = "hyprspaces",
    version,
    about = "Paired workspaces for Hyprland."
)]
pub struct Cli {
    #[arg(long, value_enum, default_value_t = IpcBackend::Hyprctl)]
    pub ipc: IpcBackend,
    #[command(subcommand)]
    pub command: Command,
}


#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum IpcBackend {
    Hyprctl,
    Native,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Paired {
        #[command(subcommand)]
        command: PairedCommand,
    },
    Daemon,
    Setup {
        #[command(subcommand)]
        command: SetupCommand,
    },
    Waybar(WaybarArgs),
}

#[derive(Subcommand, Debug)]
pub enum PairedCommand {
    Switch {
        workspace: u32,
    },
    Cycle {
        direction: CycleDirectionArg,
    },
    #[command(name = "move-window")]
    MoveWindow {
        workspace: u32,
    },
}

#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum CycleDirectionArg {
    Next,
    Prev,
}

impl From<CycleDirectionArg> for CycleDirection {
    fn from(value: CycleDirectionArg) -> Self {
        match value {
            CycleDirectionArg::Next => CycleDirection::Next,
            CycleDirectionArg::Prev => CycleDirection::Prev,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum SetupCommand {
    Install(InstallArgs),
    Uninstall,
    #[command(name = "migrate-windows")]
    MigrateWindows,
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    #[arg(long)]
    pub waybar: bool,
}

#[derive(Args, Debug)]
pub struct WaybarArgs {
    #[arg(long)]
    pub enable_waybar: bool,
    #[arg(long, value_name = "PATH")]
    pub theme_css: Option<PathBuf>,
}

#[derive(thiserror::Error, Debug)]
pub enum CliError {
    #[error("missing environment variable: {0}")]
    MissingEnv(&'static str),
    #[error("hyprland socket not found: {0}")]
    MissingSocket(PathBuf),
    #[error("waybar output requires --enable-waybar")]
    WaybarDisabled,
    #[error("native ipc requires --features native-ipc")]
    NativeIpcUnavailable,
    #[error("io error")]
    Io(#[from] io::Error),
    #[error("config error")]
    Config(#[from] ConfigError),
    #[error("setup error")]
    Setup(#[from] SetupError),
    #[error("hyprctl error")]
    Hyprctl(#[from] HyprctlError),
    #[error("waybar error")]
    Waybar(#[from] WaybarError),
}

#[derive(Debug, Clone)]
struct EnvPaths {
    base_dir: PathBuf,
    config_path: PathBuf,
    hypr_config_dir: PathBuf,
    waybar_css: PathBuf,
}

trait DaemonLauncher {
    fn launch(&self, bin_path: &str, base_dir: &Path) -> Result<(), CliError>;
}

struct SystemDaemonLauncher;

impl DaemonLauncher for SystemDaemonLauncher {
    fn launch(&self, bin_path: &str, base_dir: &Path) -> Result<(), CliError> {
        spawn_daemon(bin_path, base_dir)
    }
}

fn spawn_daemon(bin_path: &str, base_dir: &Path) -> Result<(), CliError> {
    let child = ProcessCommand::new(bin_path)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    write_daemon_pid(base_dir, child.id())?;
    Ok(())
}

trait DaemonKiller {
    fn kill(&self, pid: u32) -> Result<(), CliError>;
}

struct SystemDaemonKiller;

impl DaemonKiller for SystemDaemonKiller {
    fn kill(&self, pid: u32) -> Result<(), CliError> {
        kill_pid(pid)
    }
}

trait DaemonPidSource {
    fn pids(&self) -> Result<Vec<u32>, CliError>;
}

struct SystemDaemonPidSource;

impl DaemonPidSource for SystemDaemonPidSource {
    fn pids(&self) -> Result<Vec<u32>, CliError> {
        system_daemon_pids()
    }
}

fn daemon_pid_path(base_dir: &Path) -> PathBuf {
    base_dir.join("daemon.pid")
}

fn write_daemon_pid(base_dir: &Path, pid: u32) -> Result<(), CliError> {
    fs::write(daemon_pid_path(base_dir), format!("{pid}\n"))?;
    Ok(())
}

fn read_daemon_pid(base_dir: &Path) -> Result<Option<u32>, CliError> {
    let path = daemon_pid_path(base_dir);
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)?;
    match contents.trim().parse::<u32>() {
        Ok(pid) => Ok(Some(pid)),
        Err(_) => {
            let _ = fs::remove_file(path);
            Ok(None)
        }
    }
}

fn kill_pid(pid: u32) -> Result<(), CliError> {
    match ProcessCommand::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn stop_daemon(base_dir: &Path) -> Result<(), CliError> {
    let killer = SystemDaemonKiller;
    let pid_source = SystemDaemonPidSource;
    stop_daemon_with_killer(base_dir, &killer, &pid_source)
}

fn stop_daemon_with_killer<L: DaemonKiller, P: DaemonPidSource>(
    base_dir: &Path,
    killer: &L,
    pid_source: &P,
) -> Result<(), CliError> {
    let mut pids = Vec::new();
    if let Some(pid) = read_daemon_pid(base_dir)? {
        pids.push(pid);
    }
    if let Ok(mut extra) = pid_source.pids() {
        pids.append(&mut extra);
    }
    pids.sort_unstable();
    pids.dedup();
    for pid in pids {
        killer.kill(pid)?;
    }
    let path = daemon_pid_path(base_dir);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn system_daemon_pids() -> Result<Vec<u32>, CliError> {
    let mut pids = Vec::new();
    let entries = match fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return Ok(pids),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let file_name = entry.file_name();
        let pid = match file_name.to_string_lossy().parse::<u32>() {
            Ok(pid) => pid,
            Err(_) => continue,
        };
        let cmdline_path = entry.path().join("cmdline");
        let cmdline = match fs::read(&cmdline_path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        let args = parse_cmdline(&cmdline);
        if cmdline_is_daemon(&args) {
            pids.push(pid);
        }
    }
    Ok(pids)
}

fn parse_cmdline(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|segment| !segment.is_empty())
        .map(|segment| String::from_utf8_lossy(segment).to_string())
        .collect()
}

fn cmdline_is_daemon(args: &[String]) -> bool {
    let has_daemon = args.iter().any(|arg| arg == "daemon");
    let has_binary = args
        .first()
        .map(|arg| arg.ends_with("hyprspaces"))
        .unwrap_or(false);
    has_daemon && has_binary
}

impl WaybarArgs {
    fn ensure_enabled(&self) -> Result<(), CliError> {
        if self.enable_waybar {
            Ok(())
        } else {
            Err(CliError::WaybarDisabled)
        }
    }
}

fn build_ipc(backend: IpcBackend) -> Result<Box<dyn HyprlandIpc>, CliError> {
    match backend {
        IpcBackend::Hyprctl => Ok(Box::new(Hyprctl::new(SystemHyprctlRunner::new(
            "hyprctl",
        )))),
        IpcBackend::Native => {
            #[cfg(feature = "native-ipc")]
            {
                Ok(Box::new(NativeIpc::new()))
            }
            #[cfg(not(feature = "native-ipc"))]
            {
                Err(CliError::NativeIpcUnavailable)
            }
        }
    }
}

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let hyprctl = build_ipc(cli.ipc)?;
    let hyprctl = hyprctl.as_ref();
    let paths = env_paths()?;
    let bin_path = bin_path();

    match cli.command {
        Command::Paired { command } => {
            ensure_setup(hyprctl, &paths, &bin_path)?;
            let config = load_config(&paths)?;
            match command {
                PairedCommand::Switch { workspace } => {
                    commands::paired_switch(hyprctl, &config, workspace)?;
                }
                PairedCommand::Cycle { direction } => {
                    commands::paired_cycle(hyprctl, &config, direction.into())?;
                }
                PairedCommand::MoveWindow { workspace } => {
                    commands::paired_move_window(hyprctl, &config, workspace)?;
                }
            }
        }
        Command::Daemon => {
            ensure_setup(hyprctl, &paths, &bin_path)?;
            let config = load_config(&paths)?;
            let socket_path = socket2_path()?;
            ensure_socket(&socket_path)?;
            daemon::rebalance_all(hyprctl, &config)?;
            let stream = std::os::unix::net::UnixStream::connect(&socket_path)?;
            stream.set_read_timeout(Some(daemon::DEFAULT_REBALANCE_DEBOUNCE))?;
            let mut reader = io::BufReader::new(stream);
            let mut debounce =
                daemon::RebalanceDebounce::new(daemon::DEFAULT_REBALANCE_DEBOUNCE);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim_end();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let _ = daemon::rebalance_for_event_debounced(
                            hyprctl,
                            &config,
                            trimmed,
                            &mut debounce,
                        )?;
                    }
                    Err(err)
                        if err.kind() == io::ErrorKind::TimedOut
                            || err.kind() == io::ErrorKind::WouldBlock =>
                    {
                        let _ = daemon::flush_pending_rebalance(
                            hyprctl,
                            &config,
                            &mut debounce,
                        )?;
                    }
                    Err(err) => return Err(err.into()),
                }
            }
        }
        Command::Setup { command } => match command {
            SetupCommand::Install(args) => {
                handle_setup_install(hyprctl, &paths, &bin_path, args.waybar)?;
            }
            SetupCommand::Uninstall => {
                if let Ok(config) = load_config(&paths) {
                    let _ = commands::migrate_windows(hyprctl, &config);
                }
                stop_daemon(&paths.base_dir)?;
                setup::uninstall(&paths.base_dir, &paths.hypr_config_dir)?;
                let _ = hyprctl.reload();
            }
            SetupCommand::MigrateWindows => {
                let config = load_config(&paths)?;
                commands::migrate_windows(hyprctl, &config)?;
            }
        },
        Command::Waybar(args) => {
            args.ensure_enabled()?;
            ensure_setup(hyprctl, &paths, &bin_path)?;
            let config = load_config(&paths)?;
            let theme_path = args.theme_css.unwrap_or(paths.waybar_css);
            let colors = waybar::load_theme_colors(&theme_path)?;
            let socket_path = socket2_path()?;
            ensure_socket(&socket_path)?;
            write_stdout(&waybar::state_from_hyprctl(
                hyprctl,
                config.paired_offset,
                &colors,
            )?)?;
            let stream = std::os::unix::net::UnixStream::connect(&socket_path)?;
            let reader = io::BufReader::new(stream);
            for line in reader.lines() {
                let line = line?;
                if waybar::should_update(&line) {
                    let state =
                        waybar::state_from_hyprctl(hyprctl, config.paired_offset, &colors)?;
                    write_stdout(&state)?;
                }
            }
        }
    }

    Ok(())
}

fn load_config(paths: &EnvPaths) -> Result<Config, CliError> {
    Ok(Config::from_path(&paths.config_path)?)
}

fn handle_setup_install(
    hyprctl: &dyn HyprlandIpc,
    paths: &EnvPaths,
    bin_path: &str,
    waybar: bool,
) -> Result<(), CliError> {
    let launcher = SystemDaemonLauncher;
    handle_setup_install_with_launcher(hyprctl, paths, bin_path, waybar, &launcher)
}

fn handle_setup_install_with_launcher<L: DaemonLauncher>(
    hyprctl: &dyn HyprlandIpc,
    paths: &EnvPaths,
    bin_path: &str,
    waybar: bool,
    launcher: &L,
) -> Result<(), CliError> {
    let monitors = hyprctl.monitors().ok();
    setup::install(
        &paths.base_dir,
        bin_path,
        &paths.hypr_config_dir,
        &paths.config_path,
        monitors.as_deref(),
    )?;
    if waybar {
        setup::install_waybar(&paths.base_dir, bin_path)?;
    }
    let _ = hyprctl.reload();
    launcher.launch(bin_path, &paths.base_dir)?;
    Ok(())
}

fn ensure_setup(
    hyprctl: &dyn HyprlandIpc,
    paths: &EnvPaths,
    bin_path: &str,
) -> Result<(), CliError> {
    if paths.base_dir.join("bindings.conf").exists() {
        return Ok(());
    }
    let monitors = hyprctl.monitors().ok();
    setup::install(
        &paths.base_dir,
        bin_path,
        &paths.hypr_config_dir,
        &paths.config_path,
        monitors.as_deref(),
    )?;
    let _ = hyprctl.reload();
    Ok(())
}

fn bin_path() -> String {
    env::args()
        .next()
        .unwrap_or_else(|| "hyprspaces".to_string())
}

fn env_paths() -> Result<EnvPaths, CliError> {
    let home = env::var("HOME").map_err(|_| CliError::MissingEnv("HOME"))?;
    let home_path = Path::new(&home);
    let xdg_config = env::var("XDG_CONFIG_HOME").ok();
    let xdg_path = xdg_config.as_deref().map(Path::new);
    let config_dir = paths::config_dir(home_path, xdg_path);
    let base_dir = config_dir.join("hyprspaces");
    let config_path = paths::config_path(home_path, xdg_path);
    let hypr_config_dir = paths::hypr_config_dir(home_path, xdg_path);
    let waybar_css = config_dir.join("waybar").join("style.css");

    Ok(EnvPaths {
        base_dir,
        config_path,
        hypr_config_dir,
        waybar_css,
    })
}

fn socket2_path() -> Result<PathBuf, CliError> {
    let runtime_dir =
        env::var("XDG_RUNTIME_DIR").map_err(|_| CliError::MissingEnv("XDG_RUNTIME_DIR"))?;
    let instance = env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .map_err(|_| CliError::MissingEnv("HYPRLAND_INSTANCE_SIGNATURE"))?;
    Ok(PathBuf::from(daemon::socket2_path(&runtime_dir, &instance)))
}

fn ensure_socket(path: &Path) -> Result<(), CliError> {
    let metadata = std::fs::metadata(path).map_err(|_| CliError::MissingSocket(path.into()))?;
    if metadata.file_type().is_socket() {
        Ok(())
    } else {
        Err(CliError::MissingSocket(path.into()))
    }
}

fn write_stdout(line: &str) -> Result<(), CliError> {
    let mut stdout = io::stdout();
    if let Err(err) = writeln!(stdout, "{}", line) {
        if err.kind() == io::ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(err.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use super::{
        Cli, CliError, Command, EnvPaths, SetupCommand, WaybarArgs,
        handle_setup_install_with_launcher,
    };
    use crate::hyprctl::{Hyprctl, HyprctlRunner};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::rc::Rc;

    #[derive(Default)]
    struct RecordingLauncher {
        calls: Rc<RefCell<Vec<String>>>,
    }

    impl super::DaemonLauncher for RecordingLauncher {
        fn launch(&self, bin_path: &str, base_dir: &Path) -> Result<(), CliError> {
            self.calls
                .borrow_mut()
                .push(format!("{}:{}", bin_path, base_dir.display()));
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingKiller {
        calls: Rc<RefCell<Vec<u32>>>,
    }

    impl super::DaemonKiller for RecordingKiller {
        fn kill(&self, pid: u32) -> Result<(), CliError> {
            self.calls.borrow_mut().push(pid);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingPidSource {
        pids: Vec<u32>,
    }

    impl super::DaemonPidSource for RecordingPidSource {
        fn pids(&self) -> Result<Vec<u32>, CliError> {
            Ok(self.pids.clone())
        }
    }

    #[test]
    fn waybar_requires_enable_flag() {
        let args = WaybarArgs {
            theme_css: None,
            enable_waybar: false,
        };

        let err = args.ensure_enabled().expect_err("expected disabled error");

        assert!(matches!(err, CliError::WaybarDisabled));
    }

    #[test]
    fn waybar_allows_enabled_flag() {
        let args = WaybarArgs {
            theme_css: None,
            enable_waybar: true,
        };

        args.ensure_enabled().expect("enabled");
    }

    #[test]
    fn parses_waybar_install_flag() {
        let cli =
            Cli::try_parse_from(["hyprspaces", "setup", "install", "--waybar"]).expect("parse");

        match cli.command {
            Command::Setup {
                command: SetupCommand::Install(args),
            } => assert!(args.waybar),
            _ => panic!("expected setup install"),
        }
    }

    #[derive(Clone)]
    struct SequenceRunner {
        responses: Rc<RefCell<VecDeque<String>>>,
    }

    impl SequenceRunner {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: Rc::new(RefCell::new(responses.into())),
            }
        }
    }

    impl HyprctlRunner for SequenceRunner {
        fn run(&self, args: &[String]) -> Result<String, crate::hyprctl::HyprctlError> {
            let response = self
                .responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_default();
            if args == ["-j".to_string(), "monitors".to_string()]
                || args == ["reload".to_string()]
            {
                return Ok(response);
            }
            Ok(response)
        }
    }

    #[test]
    fn setup_install_writes_waybar_files_when_enabled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base_dir = dir.path().join("hyprspaces");
        let config_path = dir.path().join("paired.json");
        let hypr_dir = dir.path().join("hypr");
        fs::create_dir_all(&hypr_dir).expect("hypr dir");
        fs::write(hypr_dir.join("bindings.conf"), "base\n").expect("bindings");
        fs::write(hypr_dir.join("autostart.conf"), "base\n").expect("autostart");
        fs::write(hypr_dir.join("hyprland.conf"), "base\n").expect("hyprland");

        let monitors = r#"[{"name":"DP-1","x":0,"id":1},{"name":"HDMI-A-1","x":1920,"id":2}]"#;
        let runner = SequenceRunner::new(vec![monitors.to_string(), "ok".to_string()]);
        let hyprctl = Hyprctl::new(runner);
        let paths = EnvPaths {
            base_dir: base_dir.clone(),
            config_path,
            hypr_config_dir: hypr_dir,
            waybar_css: PathBuf::from("unused"),
        };

        let launcher = RecordingLauncher::default();
        handle_setup_install_with_launcher(&hyprctl, &paths, "hyprspaces", true, &launcher)
            .expect("install waybar");

        let waybar_dir = base_dir.join("waybar");
        let config = fs::read_to_string(waybar_dir.join("workspaces.json")).expect("config");
        let json: serde_json::Value = serde_json::from_str(&config).expect("json");
        let exec = json["custom/workspaces"]["exec"]
            .as_str()
            .expect("exec");
        let theme_path = waybar_dir.join("theme.css");
        assert!(waybar_dir.join("workspaces.json").exists());
        assert!(waybar_dir.join("workspaces.css").exists());
        assert!(theme_path.exists());
        assert!(waybar_dir.join("installed.flag").exists());
        assert_eq!(
            exec,
            format!(
                "hyprspaces waybar --enable-waybar --theme-css {}",
                theme_path.display()
            )
        );
    }

    #[test]
    fn setup_install_launches_daemon() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base_dir = dir.path().join("hyprspaces");
        let config_path = dir.path().join("paired.json");
        let hypr_dir = dir.path().join("hypr");
        fs::create_dir_all(&hypr_dir).expect("hypr dir");
        fs::write(hypr_dir.join("bindings.conf"), "base\n").expect("bindings");
        fs::write(hypr_dir.join("autostart.conf"), "base\n").expect("autostart");
        fs::write(hypr_dir.join("hyprland.conf"), "base\n").expect("hyprland");

        let monitors = r#"[{"name":"DP-1","x":0,"id":1},{"name":"HDMI-A-1","x":1920,"id":2}]"#;
        let runner = SequenceRunner::new(vec![monitors.to_string(), "ok".to_string()]);
        let hyprctl = Hyprctl::new(runner);
        let paths = EnvPaths {
            base_dir: base_dir.clone(),
            config_path,
            hypr_config_dir: hypr_dir,
            waybar_css: PathBuf::from("unused"),
        };

        let launcher = RecordingLauncher::default();
        handle_setup_install_with_launcher(&hyprctl, &paths, "hyprspaces", false, &launcher)
            .expect("install");

        let calls = launcher.calls.borrow();
        assert_eq!(
            calls.as_slice(),
            &[format!("hyprspaces:{}", base_dir.display())]
        );
    }

    #[test]
    fn writes_and_reads_daemon_pid() {
        let dir = tempfile::tempdir().expect("tempdir");

        super::write_daemon_pid(dir.path(), 4242).expect("write pid");

        let pid = super::read_daemon_pid(dir.path()).expect("read pid");
        assert_eq!(pid, Some(4242));
    }

    #[test]
    fn stop_daemon_removes_pidfile_and_calls_killer() {
        let dir = tempfile::tempdir().expect("tempdir");
        super::write_daemon_pid(dir.path(), 9001).expect("write pid");
        let killer = RecordingKiller::default();
        let pid_source = RecordingPidSource::default();

        super::stop_daemon_with_killer(dir.path(), &killer, &pid_source).expect("stop daemon");

        let calls = killer.calls.borrow();
        assert_eq!(calls.as_slice(), &[9001]);
        assert!(!super::daemon_pid_path(dir.path()).exists());
    }

    #[test]
    fn cmdline_detects_daemon() {
        let args = vec![
            "/usr/bin/hyprspaces".to_string(),
            "daemon".to_string(),
        ];

        assert!(super::cmdline_is_daemon(&args));
    }

    #[test]
    fn cmdline_ignores_non_daemon() {
        let args = vec![
            "/usr/bin/hyprspaces".to_string(),
            "setup".to_string(),
            "install".to_string(),
        ];

        assert!(!super::cmdline_is_daemon(&args));
    }


    #[test]
    fn ipc_defaults_to_hyprctl() {
        let cli = Cli::try_parse_from(["hyprspaces", "paired", "switch", "1"]).expect("parse");

        assert!(matches!(cli.ipc, super::IpcBackend::Hyprctl));
    }

    #[test]
    fn ipc_parses_explicit_hyprctl() {
        let cli = Cli::try_parse_from([
            "hyprspaces",
            "--ipc",
            "hyprctl",
            "paired",
            "switch",
            "1",
        ])
        .expect("parse");

        assert!(matches!(cli.ipc, super::IpcBackend::Hyprctl));
    }

    #[cfg(not(feature = "native-ipc"))]
    #[test]
    fn ipc_native_requires_feature() {
        let cli = Cli::try_parse_from([
            "hyprspaces",
            "--ipc",
            "native",
            "paired",
            "switch",
            "1",
        ])
        .expect("parse");

        let err = match super::build_ipc(cli.ipc) {
            Ok(_) => panic!("expected native ipc error"),
            Err(err) => err,
        };

        assert!(matches!(err, CliError::NativeIpcUnavailable));
    }

    #[cfg(feature = "native-ipc")]
    #[test]
    fn ipc_native_available_with_feature() {
        let cli = Cli::try_parse_from([
            "hyprspaces",
            "--ipc",
            "native",
            "paired",
            "switch",
            "1",
        ])
        .expect("parse");

        let _ = super::build_ipc(cli.ipc).expect("native ipc");
    }
}
