use crate::config::Config;
use crate::hyprctl::{HyprlandIpc, HyprctlError};
use std::io::{self, BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

#[cfg(feature = "native-ipc")]
use hyprland::instance::Instance;

pub fn event_name(line: &str) -> &str {
    match line.split_once(">>") {
        Some((name, _)) => name,
        None => line,
    }
}

pub const DEFAULT_REBALANCE_DEBOUNCE: Duration = Duration::from_millis(200);
pub const DEFAULT_FOCUS_SWITCH_DEBOUNCE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorEventKind {
    Added,
    Removed,
}

#[derive(Debug, Clone)]
pub struct FocusEvent {
    pub at: Instant,
    pub workspace_id: Option<u32>,
    pub window_address: Option<String>,
}

pub enum DaemonEvent {
    Focus(FocusEvent),
    Monitor { kind: MonitorEventKind, at: Instant },
    Timeout { at: Instant },
    Disconnected,
}

pub enum EventSourceKind {
    Socket2,
    #[cfg(feature = "native-ipc")]
    Native,
}

pub trait EventSource {
    fn next_event(&mut self) -> io::Result<DaemonEvent>;
}

pub struct Socket2EventSource {
    reader: BufReader<UnixStream>,
    line: String,
}

impl Socket2EventSource {
    pub fn new(stream: UnixStream, timeout: Duration) -> io::Result<Self> {
        stream.set_read_timeout(Some(timeout))?;
        Ok(Self {
            reader: BufReader::new(stream),
            line: String::new(),
        })
    }
}

impl EventSource for Socket2EventSource {
    fn next_event(&mut self) -> io::Result<DaemonEvent> {
        loop {
            self.line.clear();
            match self.reader.read_line(&mut self.line) {
                Ok(0) => return Ok(DaemonEvent::Disconnected),
                Ok(_) => {
                    let trimmed = self.line.trim_end();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Some(event) = parse_socket2_event(trimmed, Instant::now()) {
                        return Ok(event);
                    }
                    continue;
                }
                Err(err)
                    if err.kind() == io::ErrorKind::TimedOut
                        || err.kind() == io::ErrorKind::WouldBlock =>
                {
                    return Ok(DaemonEvent::Timeout { at: Instant::now() });
                }
                Err(err) => return Err(err),
            }
        }
    }
}

#[cfg(feature = "native-ipc")]
pub struct NativeEventSource {
    receiver: std::sync::mpsc::Receiver<DaemonEvent>,
    timeout: Duration,
}

#[cfg(feature = "native-ipc")]
impl NativeEventSource {
    pub fn new(timeout: Duration) -> Result<Self, HyprctlError> {
        let instance =
            Instance::from_current_env().map_err(|err| HyprctlError::Native(err.to_string()))?;
        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let mut listener = hyprland::event_listener::EventListener::new();
            let added_sender = sender.clone();
            listener.add_monitor_added_handler(move |_| {
                let _ = added_sender.send(DaemonEvent::Monitor {
                    kind: MonitorEventKind::Added,
                    at: Instant::now(),
                });
            });
            let removed_sender = sender.clone();
            listener.add_monitor_removed_handler(move |_| {
                let _ = removed_sender.send(DaemonEvent::Monitor {
                    kind: MonitorEventKind::Removed,
                    at: Instant::now(),
                });
            });
            let workspace_sender = sender.clone();
            listener.add_workspace_changed_handler(move |workspace| {
                let workspace_id = workspace_id_from_native(workspace.id);
                if let Some(workspace_id) = workspace_id {
                    let _ = workspace_sender.send(DaemonEvent::Focus(FocusEvent {
                        at: Instant::now(),
                        workspace_id: Some(workspace_id),
                        window_address: None,
                    }));
                }
            });
            let window_sender = sender.clone();
            listener.add_active_window_changed_handler(move |window| {
                let address = window.map(|window| window.address.to_string());
                if address.is_none() {
                    return;
                }
                let _ = window_sender.send(DaemonEvent::Focus(FocusEvent {
                    at: Instant::now(),
                    workspace_id: None,
                    window_address: address,
                }));
            });
            let monitor_sender = sender.clone();
            listener.add_active_monitor_changed_handler(move |monitor| {
                let workspace_id = monitor
                    .workspace_name
                    .as_ref()
                    .and_then(workspace_id_from_workspace_type);
                if let Some(workspace_id) = workspace_id {
                    let _ = monitor_sender.send(DaemonEvent::Focus(FocusEvent {
                        at: Instant::now(),
                        workspace_id: Some(workspace_id),
                        window_address: None,
                    }));
                }
            });
            let _ = listener.instance_start_listener(&instance);
            let _ = sender.send(DaemonEvent::Disconnected);
        });

        Ok(Self { receiver, timeout })
    }
}

#[cfg(feature = "native-ipc")]
impl EventSource for NativeEventSource {
    fn next_event(&mut self) -> io::Result<DaemonEvent> {
        match self.receiver.recv_timeout(self.timeout) {
            Ok(event) => Ok(event),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Ok(DaemonEvent::Timeout { at: Instant::now() })
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Ok(DaemonEvent::Disconnected)
            }
        }
    }
}

pub struct RebalanceDebounce {
    min_interval: Duration,
    last_rebalance: Option<Instant>,
    last_event: Option<Instant>,
    pending: bool,
}

impl RebalanceDebounce {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_rebalance: None,
            last_event: None,
            pending: false,
        }
    }

    fn record_event(&mut self, now: Instant) -> bool {
        self.last_event = Some(now);
        if self.should_run_now(now) {
            self.last_rebalance = Some(now);
            self.pending = false;
            true
        } else {
            self.pending = true;
            false
        }
    }

    fn flush(&mut self, now: Instant) -> bool {
        if !self.pending {
            return false;
        }
        let last_event = match self.last_event {
            Some(last_event) => last_event,
            None => return false,
        };
        if now.duration_since(last_event) < self.min_interval {
            return false;
        }
        if !self.should_run_now(now) {
            return false;
        }
        self.pending = false;
        self.last_rebalance = Some(now);
        true
    }

    fn should_run_now(&self, now: Instant) -> bool {
        match self.last_rebalance {
            None => true,
            Some(last) => now.duration_since(last) >= self.min_interval,
        }
    }
}

pub struct FocusSwitchDebounce {
    min_interval: Duration,
    last_switch: Option<Instant>,
    last_workspace: Option<u32>,
}

impl FocusSwitchDebounce {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_switch: None,
            last_workspace: None,
        }
    }

    fn should_switch(&mut self, now: Instant, workspace: u32) -> bool {
        let recent_same_workspace = match (self.last_switch, self.last_workspace) {
            (Some(last_switch), Some(last_workspace)) if last_workspace == workspace => {
                now.duration_since(last_switch) < self.min_interval
            }
            _ => false,
        };
        if recent_same_workspace {
            return false;
        }
        self.last_switch = Some(now);
        self.last_workspace = Some(workspace);
        true
    }
}

pub fn socket2_path(runtime_dir: &str, instance_signature: &str) -> String {
    format!("{}/hypr/{}/.socket2.sock", runtime_dir, instance_signature)
}

fn parse_workspace_id_from_name(name: &str) -> Option<u32> {
    name.parse().ok()
}

fn parse_first_field(payload: &str) -> Option<u32> {
    payload
        .split_once(',')
        .and_then(|(first, _)| first.parse().ok())
}

fn parse_second_field(payload: &str) -> Option<u32> {
    payload
        .split_once(',')
        .and_then(|(_, second)| second.parse().ok())
}

fn parse_socket2_event(line: &str, at: Instant) -> Option<DaemonEvent> {
    let (name, payload) = line.split_once(">>")?;
    match name {
        "monitoradded" | "monitoraddedv2" => Some(DaemonEvent::Monitor {
            kind: MonitorEventKind::Added,
            at,
        }),
        "monitorremoved" | "monitorremovedv2" => Some(DaemonEvent::Monitor {
            kind: MonitorEventKind::Removed,
            at,
        }),
        "workspacev2" => parse_first_field(payload).map(|workspace_id| {
            DaemonEvent::Focus(FocusEvent {
                at,
                workspace_id: Some(workspace_id),
                window_address: None,
            })
        }),
        "workspace" => parse_workspace_id_from_name(payload).map(|workspace_id| {
            DaemonEvent::Focus(FocusEvent {
                at,
                workspace_id: Some(workspace_id),
                window_address: None,
            })
        }),
        "focusedmonv2" => parse_second_field(payload).map(|workspace_id| {
            DaemonEvent::Focus(FocusEvent {
                at,
                workspace_id: Some(workspace_id),
                window_address: None,
            })
        }),
        "focusedmon" => parse_second_field(payload).map(|workspace_id| {
            DaemonEvent::Focus(FocusEvent {
                at,
                workspace_id: Some(workspace_id),
                window_address: None,
            })
        }),
        "activewindowv2" => {
            let address = payload.trim();
            if address.is_empty() {
                None
            } else {
                Some(DaemonEvent::Focus(FocusEvent {
                    at,
                    workspace_id: None,
                    window_address: Some(address.to_string()),
                }))
            }
        }
        _ => None,
    }
}

#[cfg(feature = "native-ipc")]
fn workspace_id_from_native(id: hyprland::shared::WorkspaceId) -> Option<u32> {
    if id <= 0 {
        return None;
    }
    u32::try_from(id).ok()
}

#[cfg(feature = "native-ipc")]
fn workspace_id_from_workspace_type(
    workspace: &hyprland::shared::WorkspaceType,
) -> Option<u32> {
    match workspace {
        hyprland::shared::WorkspaceType::Regular(name) => parse_workspace_id_from_name(name),
        hyprland::shared::WorkspaceType::Special(_) => None,
    }
}

pub fn should_rebalance(line: &str) -> bool {
    matches!(
        parse_socket2_event(line, Instant::now()),
        Some(DaemonEvent::Monitor { .. })
    )
}

pub fn rebalance_batch_for_event(
    primary: &str,
    secondary: &str,
    offset: u32,
    line: &str,
) -> Option<String> {
    match parse_socket2_event(line, Instant::now()) {
        Some(DaemonEvent::Monitor { .. }) => {
            Some(crate::hyprctl::rebalance_batch(primary, secondary, offset))
        }
        _ => None,
    }
}

pub fn rebalance_all(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
) -> Result<(), HyprctlError> {
    let batch = crate::hyprctl::rebalance_batch(
        &config.primary_monitor,
        &config.secondary_monitor,
        config.paired_offset,
    );
    hyprctl.batch(&batch).map(|_| ())
}

pub fn rebalance_for_event(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    line: &str,
) -> Result<bool, HyprctlError> {
    if !matches!(
        parse_socket2_event(line, Instant::now()),
        Some(DaemonEvent::Monitor { .. })
    ) {
        return Ok(false);
    }
    let batch = crate::hyprctl::rebalance_batch(
        &config.primary_monitor,
        &config.secondary_monitor,
        config.paired_offset,
    );
    hyprctl.batch(&batch)?;
    Ok(true)
}

pub fn focus_switch_for_event(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    line: &str,
    debounce: &mut FocusSwitchDebounce,
) -> Result<bool, HyprctlError> {
    focus_switch_for_event_at(hyprctl, config, line, debounce, Instant::now())
}

pub fn focus_switch_for_event_at(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    line: &str,
    debounce: &mut FocusSwitchDebounce,
    now: Instant,
) -> Result<bool, HyprctlError> {
    let focus = match parse_socket2_event(line, now) {
        Some(DaemonEvent::Focus(focus)) => focus,
        _ => return Ok(false),
    };
    focus_switch_for_focus_event_at(hyprctl, config, &focus, debounce)
}

fn focus_switch_for_focus_event_at(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    focus: &FocusEvent,
    debounce: &mut FocusSwitchDebounce,
) -> Result<bool, HyprctlError> {
    let workspace_id = if let Some(workspace_id) = focus.workspace_id {
        Some(workspace_id)
    } else if let Some(address) = focus.window_address.as_deref() {
        let clients = hyprctl.clients()?;
        clients
            .iter()
            .find(|client| client.address == address)
            .map(|client| client.workspace.id)
    } else {
        None
    };
    let workspace_id = match workspace_id {
        Some(workspace_id) if workspace_id > 0 => workspace_id,
        _ => return Ok(false),
    };
    let base_workspace =
        crate::paired::normalize_workspace(workspace_id, config.paired_offset);
    if !debounce.should_switch(focus.at, base_workspace) {
        return Ok(false);
    }
    let batch = crate::hyprctl::paired_switch_batch(
        &config.primary_monitor,
        &config.secondary_monitor,
        workspace_id,
        config.paired_offset,
    );
    hyprctl.batch(&batch)?;
    Ok(true)
}

pub fn rebalance_for_event_debounced(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    line: &str,
    debounce: &mut RebalanceDebounce,
) -> Result<bool, HyprctlError> {
    let event = match parse_socket2_event(line, Instant::now()) {
        Some(DaemonEvent::Monitor { kind, at }) => (kind, at),
        _ => return Ok(false),
    };
    rebalance_for_event_at(hyprctl, config, event.0, debounce, event.1)
}

pub fn flush_pending_rebalance(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    debounce: &mut RebalanceDebounce,
) -> Result<bool, HyprctlError> {
    flush_pending_rebalance_at(hyprctl, config, debounce, Instant::now())
}

pub fn process_event(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    rebalance_debounce: &mut RebalanceDebounce,
    focus_debounce: &mut FocusSwitchDebounce,
    event: DaemonEvent,
) -> Result<bool, HyprctlError> {
    match event {
        DaemonEvent::Focus(focus) => {
            let mut did_work = false;
            if focus_switch_for_focus_event_at(hyprctl, config, &focus, focus_debounce)? {
                did_work = true;
            }
            Ok(did_work)
        }
        DaemonEvent::Monitor { kind, at } => {
            let mut did_work = false;
            if rebalance_for_event_at(hyprctl, config, kind, rebalance_debounce, at)? {
                did_work = true;
            }
            Ok(did_work)
        }
        DaemonEvent::Timeout { at } => {
            flush_pending_rebalance_at(hyprctl, config, rebalance_debounce, at)
        }
        DaemonEvent::Disconnected => Ok(false),
    }
}

fn rebalance_for_event_at(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    kind: MonitorEventKind,
    debounce: &mut RebalanceDebounce,
    now: Instant,
) -> Result<bool, HyprctlError> {
    let batch = match kind {
        MonitorEventKind::Added | MonitorEventKind::Removed => {
            crate::hyprctl::rebalance_batch(
                &config.primary_monitor,
                &config.secondary_monitor,
                config.paired_offset,
            )
        }
    };
    if debounce.record_event(now) {
        hyprctl.batch(&batch)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn flush_pending_rebalance_at(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    debounce: &mut RebalanceDebounce,
    now: Instant,
) -> Result<bool, HyprctlError> {
    if debounce.flush(now) {
        let batch = crate::hyprctl::rebalance_batch(
            &config.primary_monitor,
            &config.secondary_monitor,
            config.paired_offset,
        );
        hyprctl.batch(&batch)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        event_name, flush_pending_rebalance_at, focus_switch_for_event_at, process_event,
        rebalance_all, rebalance_batch_for_event, rebalance_for_event, rebalance_for_event_at,
        should_rebalance, socket2_path, DaemonEvent, EventSource, FocusSwitchDebounce,
        MonitorEventKind, RebalanceDebounce, Socket2EventSource,
    };
    use crate::config::Config;
    use crate::hyprctl::{Hyprctl, HyprctlRunner, paired_switch_batch, rebalance_batch};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    #[test]
    fn extracts_event_name_from_socket2_line() {
        assert_eq!(event_name("monitoradded>>DP-1"), "monitoradded");
        assert_eq!(
            event_name("monitorremovedv2>>1,DP-1,desc"),
            "monitorremovedv2"
        );
        assert_eq!(event_name("focusedmon>>DP-1,1"), "focusedmon");
    }

    #[test]
    fn leaves_event_name_when_separator_missing() {
        assert_eq!(event_name("monitoradded"), "monitoradded");
    }

    #[test]
    fn rebalance_on_monitor_add_remove() {
        assert!(should_rebalance("monitoradded>>DP-1"));
        assert!(should_rebalance("monitorremovedv2>>1,DP-1,desc"));
        assert!(!should_rebalance("focusedmon>>DP-1,1"));
    }

    #[test]
    fn switches_pair_on_focusedmonv2_event() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };
        let mut debounce = FocusSwitchDebounce::new(Duration::from_millis(100));

        assert!(focus_switch_for_event_at(
            &hyprctl,
            &config,
            "focusedmonv2>>DP-1,3",
            &mut debounce,
            Instant::now(),
        )
        .expect("switch"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            vec![
                "--batch".to_string(),
                paired_switch_batch("DP-1", "HDMI-A-1", 3, 2)
            ]
        );
    }

    #[test]
    fn switches_pair_on_activewindowv2_event() {
        let runner = RecordingRunner::with_clients(
            r#"[{"address":"0x123","workspace":{"id":4}}]"#,
        );
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };
        let mut debounce = FocusSwitchDebounce::new(Duration::from_millis(100));

        assert!(focus_switch_for_event_at(
            &hyprctl,
            &config,
            "activewindowv2>>0x123",
            &mut debounce,
            Instant::now(),
        )
        .expect("switch"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[1],
            vec![
                "--batch".to_string(),
                paired_switch_batch("DP-1", "HDMI-A-1", 4, 2)
            ]
        );
    }

    #[test]
    fn debounces_repeated_focus_events() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };
        let mut debounce = FocusSwitchDebounce::new(Duration::from_millis(100));
        let start = Instant::now();

        assert!(focus_switch_for_event_at(
            &hyprctl,
            &config,
            "focusedmonv2>>DP-1,3",
            &mut debounce,
            start,
        )
        .expect("switch"));
        assert!(!focus_switch_for_event_at(
            &hyprctl,
            &config,
            "focusedmonv2>>DP-1,3",
            &mut debounce,
            start + Duration::from_millis(10),
        )
        .expect("debounced"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn debounces_paired_focus_events() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };
        let mut debounce = FocusSwitchDebounce::new(Duration::from_millis(100));
        let start = Instant::now();

        assert!(focus_switch_for_event_at(
            &hyprctl,
            &config,
            "focusedmonv2>>DP-1,3",
            &mut debounce,
            start,
        )
        .expect("switch"));
        assert!(!focus_switch_for_event_at(
            &hyprctl,
            &config,
            "focusedmonv2>>DP-1,1",
            &mut debounce,
            start + Duration::from_millis(10),
        )
        .expect("debounced"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn builds_socket2_path() {
        let path = socket2_path("/run/user/1000", "abc");

        assert_eq!(path, "/run/user/1000/hypr/abc/.socket2.sock");
    }

    #[test]
    fn rebalance_batch_only_on_monitor_events() {
        let expected = rebalance_batch("DP-1", "HDMI-A-1", 2);

        assert_eq!(
            rebalance_batch_for_event("DP-1", "HDMI-A-1", 2, "monitoradded>>DP-1"),
            Some(expected.clone())
        );
        assert_eq!(
            rebalance_batch_for_event("DP-1", "HDMI-A-1", 2, "focusedmon>>DP-1,1"),
            None
        );
    }

    #[derive(Clone, Default)]
    struct RecordingRunner {
        calls: Rc<RefCell<Vec<Vec<String>>>>,
        clients_json: Option<String>,
    }

    impl HyprctlRunner for RecordingRunner {
        fn run(&self, args: &[String]) -> Result<String, crate::hyprctl::HyprctlError> {
            self.calls.borrow_mut().push(args.to_vec());
            if args == ["-j".to_string(), "clients".to_string()] {
                return match self.clients_json.as_ref() {
                    Some(payload) => Ok(payload.clone()),
                    None => Ok("ok".to_string()),
                };
            }
            Ok("ok".to_string())
        }
    }

    impl RecordingRunner {
        fn with_clients(clients_json: &str) -> Self {
            Self {
                calls: Rc::new(RefCell::new(Vec::new())),
                clients_json: Some(clients_json.to_string()),
            }
        }
    }

    #[test]
    fn rebalance_all_runs_batch() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };

        rebalance_all(&hyprctl, &config).expect("rebalance");

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            vec![
                "--batch".to_string(),
                rebalance_batch("DP-1", "HDMI-A-1", 2)
            ]
        );
    }

    #[test]
    fn rebalance_for_event_runs_only_on_monitor_events() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };

        assert!(rebalance_for_event(&hyprctl, &config, "monitoradded>>DP-1").expect("rebalance"));
        assert!(!rebalance_for_event(&hyprctl, &config, "focusedmon>>DP-1,1").expect("skip"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn debounces_rebalance_events_within_window() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };
        let mut debounce = RebalanceDebounce::new(Duration::from_millis(200));
        let start = Instant::now();

        assert!(rebalance_for_event_at(
            &hyprctl,
            &config,
            MonitorEventKind::Added,
            &mut debounce,
            start,
        )
        .expect("rebalance"));
        assert!(!rebalance_for_event_at(
            &hyprctl,
            &config,
            MonitorEventKind::Removed,
            &mut debounce,
            start + Duration::from_millis(50),
        )
        .expect("debounced"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn flushes_pending_rebalance_after_burst() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };
        let mut debounce = RebalanceDebounce::new(Duration::from_millis(200));
        let start = Instant::now();

        assert!(rebalance_for_event_at(
            &hyprctl,
            &config,
            MonitorEventKind::Added,
            &mut debounce,
            start,
        )
        .expect("rebalance"));
        assert!(!rebalance_for_event_at(
            &hyprctl,
            &config,
            MonitorEventKind::Removed,
            &mut debounce,
            start + Duration::from_millis(50),
        )
        .expect("debounced"));

        assert!(flush_pending_rebalance_at(
            &hyprctl,
            &config,
            &mut debounce,
            start + Duration::from_millis(260),
        )
        .expect("flush"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn process_event_flushes_pending_rebalance() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };
        let mut debounce = RebalanceDebounce::new(Duration::from_millis(200));
        let mut focus_debounce = FocusSwitchDebounce::new(Duration::from_millis(100));
        let start = Instant::now();

        assert!(process_event(
            &hyprctl,
            &config,
            &mut debounce,
            &mut focus_debounce,
            DaemonEvent::Monitor {
                kind: MonitorEventKind::Added,
                at: start,
            },
        )
        .expect("rebalance"));
        assert!(!process_event(
            &hyprctl,
            &config,
            &mut debounce,
            &mut focus_debounce,
            DaemonEvent::Monitor {
                kind: MonitorEventKind::Removed,
                at: start + Duration::from_millis(50),
            },
        )
        .expect("debounced"));
        assert!(process_event(
            &hyprctl,
            &config,
            &mut debounce,
            &mut focus_debounce,
            DaemonEvent::Timeout {
                at: start + Duration::from_millis(260),
            },
        )
        .expect("flush"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn socket2_event_source_reads_lines() {
        let (mut writer, reader) = UnixStream::pair().expect("pair");
        let mut source =
            Socket2EventSource::new(reader, Duration::from_secs(1)).expect("source");

        writer
            .write_all(b"monitoradded>>DP-1\n")
            .expect("write line");

        let event = source.next_event().expect("event");
        match event {
            DaemonEvent::Monitor { kind, .. } => {
                assert_eq!(kind, MonitorEventKind::Added);
            }
            _ => panic!("expected monitor event"),
        }
    }

    #[test]
    fn socket2_event_source_reports_disconnect() {
        let (writer, reader) = UnixStream::pair().expect("pair");
        let mut source =
            Socket2EventSource::new(reader, Duration::from_secs(1)).expect("source");

        drop(writer);

        let event = source.next_event().expect("event");
        assert!(matches!(event, DaemonEvent::Disconnected));
    }

    #[test]
    fn socket2_event_source_reports_timeout() {
        let (_writer, reader) = UnixStream::pair().expect("pair");
        let mut source =
            Socket2EventSource::new(reader, Duration::from_millis(10)).expect("source");

        let event = source.next_event().expect("event");
        assert!(matches!(event, DaemonEvent::Timeout { .. }));
    }

    #[test]
    fn allows_rebalance_after_debounce_window() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());
        let config = Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 2,
            workspace_count: 2,
            wrap_cycling: true,
        };
        let mut debounce = RebalanceDebounce::new(Duration::from_millis(200));
        let start = Instant::now();

        assert!(rebalance_for_event_at(
            &hyprctl,
            &config,
            MonitorEventKind::Added,
            &mut debounce,
            start,
        )
        .expect("rebalance"));
        assert!(rebalance_for_event_at(
            &hyprctl,
            &config,
            MonitorEventKind::Removed,
            &mut debounce,
            start + Duration::from_millis(250),
        )
        .expect("rebalance again"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
    }
}
