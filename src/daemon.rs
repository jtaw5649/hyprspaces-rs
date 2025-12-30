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

pub fn should_rebalance(line: &str) -> bool {
    let name = event_name(line);
    name.starts_with("monitoradded") || name.starts_with("monitorremoved")
}

pub const DEFAULT_REBALANCE_DEBOUNCE: Duration = Duration::from_millis(200);

pub enum DaemonEvent {
    Line { line: String, at: Instant },
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
                    return Ok(DaemonEvent::Line {
                        line: trimmed.to_string(),
                        at: Instant::now(),
                    });
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
                let _ = added_sender.send(DaemonEvent::Line {
                    line: "monitoradded".to_string(),
                    at: Instant::now(),
                });
            });
            let removed_sender = sender.clone();
            listener.add_monitor_removed_handler(move |_| {
                let _ = removed_sender.send(DaemonEvent::Line {
                    line: "monitorremoved".to_string(),
                    at: Instant::now(),
                });
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

pub fn socket2_path(runtime_dir: &str, instance_signature: &str) -> String {
    format!("{}/hypr/{}/.socket2.sock", runtime_dir, instance_signature)
}

pub fn rebalance_batch_for_event(
    primary: &str,
    secondary: &str,
    offset: u32,
    line: &str,
) -> Option<String> {
    if should_rebalance(line) {
        Some(crate::hyprctl::rebalance_batch(primary, secondary, offset))
    } else {
        None
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
    if let Some(batch) = rebalance_batch_for_event(
        &config.primary_monitor,
        &config.secondary_monitor,
        config.paired_offset,
        line,
    ) {
        hyprctl.batch(&batch)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn rebalance_for_event_debounced(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    line: &str,
    debounce: &mut RebalanceDebounce,
) -> Result<bool, HyprctlError> {
    rebalance_for_event_at(hyprctl, config, line, debounce, Instant::now())
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
    debounce: &mut RebalanceDebounce,
    event: DaemonEvent,
) -> Result<bool, HyprctlError> {
    match event {
        DaemonEvent::Line { line, at } => {
            rebalance_for_event_at(hyprctl, config, &line, debounce, at)
        }
        DaemonEvent::Timeout { at } => {
            flush_pending_rebalance_at(hyprctl, config, debounce, at)
        }
        DaemonEvent::Disconnected => Ok(false),
    }
}

fn rebalance_for_event_at(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    line: &str,
    debounce: &mut RebalanceDebounce,
    now: Instant,
) -> Result<bool, HyprctlError> {
    if let Some(batch) = rebalance_batch_for_event(
        &config.primary_monitor,
        &config.secondary_monitor,
        config.paired_offset,
        line,
    ) {
        if debounce.record_event(now) {
            hyprctl.batch(&batch)?;
            Ok(true)
        } else {
            Ok(false)
        }
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
        event_name, flush_pending_rebalance_at, process_event, rebalance_all,
        rebalance_batch_for_event, rebalance_for_event, rebalance_for_event_at,
        should_rebalance, socket2_path, DaemonEvent, EventSource, RebalanceDebounce,
        Socket2EventSource,
    };
    use crate::config::Config;
    use crate::hyprctl::{Hyprctl, HyprctlRunner, rebalance_batch};
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
    }

    impl HyprctlRunner for RecordingRunner {
        fn run(&self, args: &[String]) -> Result<String, crate::hyprctl::HyprctlError> {
            self.calls.borrow_mut().push(args.to_vec());
            Ok("ok".to_string())
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
        };
        let mut debounce = RebalanceDebounce::new(Duration::from_millis(200));
        let start = Instant::now();

        assert!(rebalance_for_event_at(
            &hyprctl,
            &config,
            "monitoradded>>DP-1",
            &mut debounce,
            start,
        )
        .expect("rebalance"));
        assert!(!rebalance_for_event_at(
            &hyprctl,
            &config,
            "monitorremovedv2>>1,DP-1,desc",
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
        };
        let mut debounce = RebalanceDebounce::new(Duration::from_millis(200));
        let start = Instant::now();

        assert!(rebalance_for_event_at(
            &hyprctl,
            &config,
            "monitoradded>>DP-1",
            &mut debounce,
            start,
        )
        .expect("rebalance"));
        assert!(!rebalance_for_event_at(
            &hyprctl,
            &config,
            "monitorremovedv2>>1,DP-1,desc",
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
        };
        let mut debounce = RebalanceDebounce::new(Duration::from_millis(200));
        let start = Instant::now();

        assert!(process_event(
            &hyprctl,
            &config,
            &mut debounce,
            DaemonEvent::Line {
                line: "monitoradded>>DP-1".to_string(),
                at: start,
            },
        )
        .expect("rebalance"));
        assert!(!process_event(
            &hyprctl,
            &config,
            &mut debounce,
            DaemonEvent::Line {
                line: "monitorremovedv2>>1,DP-1,desc".to_string(),
                at: start + Duration::from_millis(50),
            },
        )
        .expect("debounced"));
        assert!(process_event(
            &hyprctl,
            &config,
            &mut debounce,
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
            DaemonEvent::Line { line, .. } => {
                assert_eq!(line, "monitoradded>>DP-1");
            }
            _ => panic!("expected line event"),
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
        };
        let mut debounce = RebalanceDebounce::new(Duration::from_millis(200));
        let start = Instant::now();

        assert!(rebalance_for_event_at(
            &hyprctl,
            &config,
            "monitoradded>>DP-1",
            &mut debounce,
            start,
        )
        .expect("rebalance"));
        assert!(rebalance_for_event_at(
            &hyprctl,
            &config,
            "monitorremovedv2>>1,DP-1,desc",
            &mut debounce,
            start + Duration::from_millis(250),
        )
        .expect("rebalance again"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
    }
}
