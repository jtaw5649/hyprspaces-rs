use crate::paired::normalize_workspace;
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
#[error("hyprctl command failed")]
pub struct HyprctlError;

pub trait HyprctlRunner {
    fn run(&self, args: &[String]) -> Result<String, HyprctlError>;
}

pub struct Hyprctl<R> {
    runner: R,
}

impl<R> Hyprctl<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }
}

impl<R: HyprctlRunner> Hyprctl<R> {
    pub fn batch(&self, batch: &str) -> Result<String, HyprctlError> {
        let args = vec!["--batch".to_string(), batch.to_string()];
        self.runner.run(&args)
    }

    pub fn active_workspace_id(&self) -> Result<u32, HyprctlError> {
        let args = vec!["-j".to_string(), "activeworkspace".to_string()];
        let output = self.runner.run(&args)?;
        let workspace: ActiveWorkspace = serde_json::from_str(&output).map_err(|_| HyprctlError)?;
        Ok(workspace.id)
    }

    pub fn dispatch(&self, dispatcher: &str, argument: &str) -> Result<String, HyprctlError> {
        let args = vec![
            "dispatch".to_string(),
            dispatcher.to_string(),
            argument.to_string(),
        ];
        self.runner.run(&args)
    }

    pub fn reload(&self) -> Result<String, HyprctlError> {
        let args = vec!["reload".to_string()];
        self.runner.run(&args)
    }

    pub fn monitors(&self) -> Result<Vec<MonitorInfo>, HyprctlError> {
        let args = vec!["-j".to_string(), "monitors".to_string()];
        let output = self.runner.run(&args)?;
        let monitors: Vec<MonitorInfo> = serde_json::from_str(&output).map_err(|_| HyprctlError)?;
        Ok(monitors)
    }

    pub fn workspaces(&self) -> Result<Vec<WorkspaceInfo>, HyprctlError> {
        let args = vec!["-j".to_string(), "workspaces".to_string()];
        let output = self.runner.run(&args)?;
        let workspaces: Vec<WorkspaceInfo> =
            serde_json::from_str(&output).map_err(|_| HyprctlError)?;
        Ok(workspaces)
    }

    pub fn clients(&self) -> Result<Vec<ClientInfo>, HyprctlError> {
        let args = vec!["-j".to_string(), "clients".to_string()];
        let output = self.runner.run(&args)?;
        let clients: Vec<ClientInfo> = serde_json::from_str(&output).map_err(|_| HyprctlError)?;
        Ok(clients)
    }
}

pub struct SystemHyprctlRunner {
    program: String,
}

impl SystemHyprctlRunner {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
        }
    }
}

impl HyprctlRunner for SystemHyprctlRunner {
    fn run(&self, args: &[String]) -> Result<String, HyprctlError> {
        let output = Command::new(&self.program)
            .args(args)
            .output()
            .map_err(|_| HyprctlError)?;
        if !output.status.success() {
            return Err(HyprctlError);
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }
}

#[derive(Debug, Deserialize)]
struct ActiveWorkspace {
    id: u32,
}

#[derive(Debug, Deserialize)]
pub struct MonitorInfo {
    pub name: String,
    pub x: i32,
    pub id: i32,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceInfo {
    pub id: u32,
    pub windows: u32,
}

#[derive(Debug, Deserialize)]
pub struct ClientInfo {
    pub address: String,
    pub workspace: WorkspaceRef,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceRef {
    pub id: u32,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct HyprctlBatch {
    commands: Vec<String>,
}

impl HyprctlBatch {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn dispatch(&mut self, dispatcher: &str, argument: &str) {
        self.commands
            .push(format!("dispatch {} {}", dispatcher, argument));
    }

    pub fn to_argument(&self) -> String {
        self.commands.join(" ; ")
    }
}

pub fn paired_switch_batch(primary: &str, secondary: &str, workspace: u32, offset: u32) -> String {
    let normalized = normalize_workspace(workspace, offset);
    let secondary_workspace = normalized + offset;
    let mut batch = HyprctlBatch::new();

    batch.dispatch("focusmonitor", secondary);
    batch.dispatch("workspace", &secondary_workspace.to_string());
    batch.dispatch("focusmonitor", primary);
    batch.dispatch("workspace", &normalized.to_string());

    batch.to_argument()
}

pub fn rebalance_batch(primary: &str, secondary: &str, offset: u32) -> String {
    let mut batch = HyprctlBatch::new();

    for workspace_id in 1..=offset {
        batch.dispatch(
            "moveworkspacetomonitor",
            &format!("{workspace_id} {primary}"),
        );
    }

    for workspace_id in (offset + 1)..=(offset * 2) {
        batch.dispatch(
            "moveworkspacetomonitor",
            &format!("{workspace_id} {secondary}"),
        );
    }

    batch.to_argument()
}

#[cfg(test)]
mod tests {
    use super::{
        Hyprctl, HyprctlBatch, HyprctlRunner, SystemHyprctlRunner, paired_switch_batch,
        rebalance_batch,
    };
    use std::cell::RefCell;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::rc::Rc;

    #[test]
    fn batch_builds_dispatch_commands() {
        let mut batch = HyprctlBatch::new();
        batch.dispatch("focusmonitor", "HDMI-A-1");
        batch.dispatch("workspace", "13");

        assert_eq!(
            batch.to_argument(),
            "dispatch focusmonitor HDMI-A-1 ; dispatch workspace 13"
        );
    }

    #[test]
    fn paired_switch_batch_normalizes_workspace() {
        let batch = paired_switch_batch("DP-1", "HDMI-A-1", 12, 10);

        assert_eq!(
            batch,
            "dispatch focusmonitor HDMI-A-1 ; dispatch workspace 12 ; dispatch focusmonitor DP-1 ; dispatch workspace 2"
        );
    }

    #[test]
    fn rebalance_batch_moves_workspaces_by_offset() {
        let batch = rebalance_batch("DP-1", "HDMI-A-1", 2);

        assert_eq!(
            batch,
            "dispatch moveworkspacetomonitor 1 DP-1 ; dispatch moveworkspacetomonitor 2 DP-1 ; dispatch moveworkspacetomonitor 3 HDMI-A-1 ; dispatch moveworkspacetomonitor 4 HDMI-A-1"
        );
    }

    #[derive(Clone, Default)]
    struct RecordingRunner {
        calls: Rc<RefCell<Vec<Vec<String>>>>,
    }

    impl HyprctlRunner for RecordingRunner {
        fn run(&self, args: &[String]) -> Result<String, super::HyprctlError> {
            self.calls.borrow_mut().push(args.to_vec());
            Ok("ok".to_string())
        }
    }

    #[test]
    fn batch_executes_hyprctl_with_argument() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());

        hyprctl
            .batch("dispatch workspace 1")
            .expect("batch should succeed");

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            vec!["--batch".to_string(), "dispatch workspace 1".to_string()]
        );
    }

    #[test]
    fn dispatch_runs_hyprctl_dispatch() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());

        hyprctl
            .dispatch("movetoworkspacesilent", "2")
            .expect("dispatch");

        let calls = runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec![
                "dispatch".to_string(),
                "movetoworkspacesilent".to_string(),
                "2".to_string()
            ]
        );
    }

    #[test]
    fn reload_runs_hyprctl_reload() {
        let runner = RecordingRunner::default();
        let hyprctl = Hyprctl::new(runner.clone());

        hyprctl.reload().expect("reload");

        let calls = runner.calls.borrow();
        assert_eq!(calls[0], vec!["reload".to_string()]);
    }

    #[test]
    fn parses_active_workspace_id_from_json() {
        let runner = StaticRunner::new(r#"{"id":42}"#);
        let hyprctl = Hyprctl::new(runner.clone());

        assert_eq!(hyprctl.active_workspace_id().expect("id"), 42);

        let calls = runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec!["-j".to_string(), "activeworkspace".to_string()]
        );
    }

    #[derive(Clone)]
    struct StaticRunner {
        response: String,
        calls: Rc<RefCell<Vec<Vec<String>>>>,
    }

    impl StaticRunner {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
                calls: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl HyprctlRunner for StaticRunner {
        fn run(&self, args: &[String]) -> Result<String, super::HyprctlError> {
            self.calls.borrow_mut().push(args.to_vec());
            Ok(self.response.clone())
        }
    }

    #[test]
    fn system_runner_executes_program() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script_path = dir.path().join("hyprctl");
        let script = "#!/usr/bin/env sh\nprintf '%s' \"$*\"\n";
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("set perms");

        let runner = SystemHyprctlRunner::new(script_path.to_string_lossy());
        let output = runner
            .run(&["-j".to_string(), "activeworkspace".to_string()])
            .expect("run");

        assert_eq!(output, "-j activeworkspace");
    }

    #[test]
    fn parses_monitors_from_json() {
        let runner = StaticRunner::new(
            r#"[{"name":"DP-1","x":0,"id":1},{"name":"HDMI-A-1","x":1920,"id":2}]"#,
        );
        let hyprctl = Hyprctl::new(runner.clone());

        let monitors = hyprctl.monitors().expect("monitors");

        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].name, "DP-1");
        assert_eq!(monitors[1].id, 2);

        let calls = runner.calls.borrow();
        assert_eq!(calls[0], vec!["-j".to_string(), "monitors".to_string()]);
    }

    #[test]
    fn parses_workspaces_from_json() {
        let runner = StaticRunner::new(r#"[{"id":1,"windows":2},{"id":12,"windows":0}]"#);
        let hyprctl = Hyprctl::new(runner.clone());

        let workspaces = hyprctl.workspaces().expect("workspaces");

        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].id, 1);
        assert_eq!(workspaces[1].windows, 0);

        let calls = runner.calls.borrow();
        assert_eq!(calls[0], vec!["-j".to_string(), "workspaces".to_string()]);
    }

    #[test]
    fn parses_clients_from_json() {
        let runner = StaticRunner::new(
            r#"[{"address":"0x123","workspace":{"id":12}},{"address":"0x456","workspace":{"id":1}}]"#,
        );
        let hyprctl = Hyprctl::new(runner.clone());

        let clients = hyprctl.clients().expect("clients");

        assert_eq!(clients.len(), 2);
        assert_eq!(clients[0].address, "0x123");
        assert_eq!(clients[1].workspace.id, 1);

        let calls = runner.calls.borrow();
        assert_eq!(calls[0], vec!["-j".to_string(), "clients".to_string()]);
    }
}
