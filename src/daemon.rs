use crate::config::Config;
use crate::hyprctl::{Hyprctl, HyprctlError, HyprctlRunner};

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

pub fn rebalance_all<R: HyprctlRunner>(
    hyprctl: &Hyprctl<R>,
    config: &Config,
) -> Result<(), HyprctlError> {
    let batch = crate::hyprctl::rebalance_batch(
        &config.primary_monitor,
        &config.secondary_monitor,
        config.paired_offset,
    );
    hyprctl.batch(&batch).map(|_| ())
}

pub fn rebalance_for_event<R: HyprctlRunner>(
    hyprctl: &Hyprctl<R>,
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

#[cfg(test)]
mod tests {
    use super::{
        event_name, rebalance_all, rebalance_batch_for_event, rebalance_for_event,
        should_rebalance, socket2_path,
    };
    use crate::config::Config;
    use crate::hyprctl::{Hyprctl, HyprctlRunner, rebalance_batch};
    use std::cell::RefCell;
    use std::rc::Rc;

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
}
