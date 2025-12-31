use crate::config::Config;
use crate::hyprctl::{HyprlandIpc, paired_switch_batch_with_focus};
use crate::paired::{CycleDirection, cycle_target, normalize_workspace};
use crate::setup::migration_targets;

fn focus_monitor_for_active_workspace(config: &Config, active_workspace: u32) -> &str {
    if active_workspace > config.paired_offset {
        &config.secondary_monitor
    } else {
        &config.primary_monitor
    }
}

fn paired_switch_with_focus(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    workspace: u32,
    focus_monitor: &str,
) -> Result<(), crate::hyprctl::HyprctlError> {
    let batch = paired_switch_batch_with_focus(
        &config.primary_monitor,
        &config.secondary_monitor,
        workspace,
        config.paired_offset,
        focus_monitor,
    );
    hyprctl.batch(&batch)?;
    Ok(())
}

pub fn paired_switch(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    workspace: u32,
) -> Result<(), crate::hyprctl::HyprctlError> {
    let active_workspace = hyprctl.active_workspace_id()?;
    let focus_monitor = focus_monitor_for_active_workspace(config, active_workspace);
    paired_switch_with_focus(hyprctl, config, workspace, focus_monitor)
}

pub fn paired_cycle(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    direction: CycleDirection,
) -> Result<(), crate::hyprctl::HyprctlError> {
    let active_workspace = hyprctl.active_workspace_id()?;
    let focus_monitor = focus_monitor_for_active_workspace(config, active_workspace);
    let base = normalize_workspace(active_workspace, config.paired_offset);
    let target = cycle_target(base, config.paired_offset, direction, config.wrap_cycling);
    paired_switch_with_focus(hyprctl, config, target, focus_monitor)
}

pub fn paired_move_window(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
    workspace: u32,
) -> Result<(), crate::hyprctl::HyprctlError> {
    let normalized = normalize_workspace(workspace, config.paired_offset);
    let active_workspace = hyprctl.active_workspace_id()?;
    let focus_monitor = focus_monitor_for_active_workspace(config, active_workspace);
    let mut target = normalized;
    if active_workspace > config.paired_offset {
        target += config.paired_offset;
    }
    hyprctl.dispatch("movetoworkspacesilent", &target.to_string())?;
    paired_switch_with_focus(hyprctl, config, normalized, focus_monitor)
}

pub fn migrate_windows(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
) -> Result<usize, crate::hyprctl::HyprctlError> {
    let clients = hyprctl.clients()?;
    let targets = migration_targets(&clients, config.paired_offset);
    for (address, target) in &targets {
        hyprctl.dispatch(
            "movetoworkspacesilent",
            &format!("{target},address:{address}"),
        )?;
    }
    Ok(targets.len())
}

pub fn grab_rogue_windows(
    hyprctl: &dyn HyprlandIpc,
    config: &Config,
) -> Result<usize, crate::hyprctl::HyprctlError> {
    let clients = hyprctl.clients()?;
    let targets = migration_targets(&clients, config.workspace_count);
    for (address, target) in &targets {
        hyprctl.dispatch(
            "movetoworkspacesilent",
            &format!("{target},address:{address}"),
        )?;
    }
    Ok(targets.len())
}

#[cfg(test)]
mod tests {
    use super::{grab_rogue_windows, migrate_windows, paired_cycle, paired_move_window};
    use crate::config::Config;
    use crate::hyprctl::{Hyprctl, HyprctlRunner};
    use crate::paired::CycleDirection;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone)]
    struct ScriptedRunner {
        active_id: u32,
        clients_json: String,
        calls: Rc<RefCell<Vec<Vec<String>>>>,
    }

    impl ScriptedRunner {
        fn new(active_id: u32, clients_json: &str) -> Self {
            Self {
                active_id,
                clients_json: clients_json.to_string(),
                calls: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl HyprctlRunner for ScriptedRunner {
        fn run(&self, args: &[String]) -> Result<String, crate::hyprctl::HyprctlError> {
            self.calls.borrow_mut().push(args.to_vec());
            if args == ["-j".to_string(), "activeworkspace".to_string()] {
                return Ok(format!(r#"{{"id":{}}}"#, self.active_id));
            }
            if args == ["-j".to_string(), "clients".to_string()] {
                return Ok(self.clients_json.clone());
            }
            Ok("ok".to_string())
        }
    }

    fn config() -> Config {
        Config {
            primary_monitor: "DP-1".to_string(),
            secondary_monitor: "HDMI-A-1".to_string(),
            paired_offset: 10,
            workspace_count: 10,
            wrap_cycling: true,
        }
    }

    #[test]
    fn cycles_to_next_workspace() {
        let runner = ScriptedRunner::new(12, "[]");
        let hyprctl = Hyprctl::new(runner.clone());

        paired_cycle(&hyprctl, &config(), CycleDirection::Next).expect("cycle");

        let calls = runner.calls.borrow();
        assert!(calls.iter().any(|call| {
            call == &vec![
                "--batch".to_string(),
                "dispatch focusmonitor DP-1 ; dispatch workspace 3 ; dispatch focusmonitor HDMI-A-1 ; dispatch workspace 13".to_string(),
            ]
        }));
    }

    #[test]
    fn moves_window_and_switches_pair() {
        let runner = ScriptedRunner::new(12, "[]");
        let hyprctl = Hyprctl::new(runner.clone());

        paired_move_window(&hyprctl, &config(), 2).expect("move");

        let calls = runner.calls.borrow();
        assert!(calls.iter().any(|call| {
            call == &vec![
                "dispatch".to_string(),
                "movetoworkspacesilent".to_string(),
                "12".to_string(),
            ]
        }));
    }

    #[test]
    fn move_window_keeps_focus_on_secondary_monitor() {
        let runner = ScriptedRunner::new(12, "[]");
        let hyprctl = Hyprctl::new(runner.clone());

        paired_move_window(&hyprctl, &config(), 2).expect("move");

        let calls = runner.calls.borrow();
        assert!(calls.iter().any(|call| {
            call == &vec![
                "--batch".to_string(),
                "dispatch focusmonitor DP-1 ; dispatch workspace 2 ; dispatch focusmonitor HDMI-A-1 ; dispatch workspace 12".to_string(),
            ]
        }));
    }

    #[test]
    fn migrates_windows_from_secondary() {
        let clients_json = r#"[{"address":"0x123","workspace":{"id":12}},{"address":"0x456","workspace":{"id":1}}]"#;
        let runner = ScriptedRunner::new(1, clients_json);
        let hyprctl = Hyprctl::new(runner.clone());

        let migrated = migrate_windows(&hyprctl, &config()).expect("migrate");

        assert_eq!(migrated, 1);
        let calls = runner.calls.borrow();
        assert!(calls.iter().any(|call| {
            call == &vec![
                "dispatch".to_string(),
                "movetoworkspacesilent".to_string(),
                "2,address:0x123".to_string(),
            ]
        }));
    }

    #[test]
    fn grabs_rogue_windows_from_secondary_range() {
        let clients_json = r#"[{"address":"0x123","workspace":{"id":12}},{"address":"0x456","workspace":{"id":1}}]"#;
        let runner = ScriptedRunner::new(1, clients_json);
        let hyprctl = Hyprctl::new(runner.clone());

        let migrated = grab_rogue_windows(&hyprctl, &config()).expect("grab");

        assert_eq!(migrated, 1);
        let calls = runner.calls.borrow();
        assert!(calls.iter().any(|call| {
            call == &vec![
                "dispatch".to_string(),
                "movetoworkspacesilent".to_string(),
                "2,address:0x123".to_string(),
            ]
        }));
    }
}
