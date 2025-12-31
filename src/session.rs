use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::hyprctl::{
    ClientInfo, HyprctlBatch, HyprctlError, HyprlandIpc, MonitorInfo, WorkspaceInfo,
};
use crate::paired::normalize_workspace;

const SESSION_VERSION: u32 = 1;

#[derive(thiserror::Error, Debug)]
pub enum SessionError {
    #[error("io error")]
    Io(#[from] std::io::Error),
    #[error("json error")]
    Json(#[from] serde_json::Error),
    #[error("hyprctl error")]
    Hyprctl(#[from] HyprctlError),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub version: u32,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    pub paired_offset: u32,
    pub workspace_count: u32,
    pub focus: SnapshotFocus,
    pub monitors: Vec<SnapshotMonitor>,
    pub workspaces: Vec<SnapshotWorkspace>,
    pub clients: Vec<SnapshotClient>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotFocus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    pub workspace_id: u32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotMonitor {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotWorkspace {
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    pub windows: u32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotClient {
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    pub workspace_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    pub paired_slot: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    Auto,
    Same,
    Cold,
}

impl SessionSnapshot {
    pub fn from_state(
        config: &Config,
        signature: Option<String>,
        active_workspace_id: u32,
        monitors: Vec<MonitorInfo>,
        workspaces: Vec<WorkspaceInfo>,
        clients: Vec<ClientInfo>,
    ) -> Self {
        let focus_monitor = workspaces
            .iter()
            .find(|workspace| workspace.id == active_workspace_id)
            .and_then(|workspace| workspace.monitor.clone());

        let snapshot_monitors = monitors
            .into_iter()
            .map(|monitor| SnapshotMonitor {
                id: monitor.id,
                name: monitor.name,
            })
            .collect();

        let snapshot_workspaces = workspaces
            .into_iter()
            .map(|workspace| SnapshotWorkspace {
                id: workspace.id,
                name: workspace.name,
                monitor: workspace.monitor,
                windows: workspace.windows,
            })
            .collect();

        let snapshot_clients = clients
            .into_iter()
            .map(|client| {
                let paired_slot = if is_special_workspace_name(client.workspace.name.as_deref()) {
                    client.workspace.id
                } else {
                    normalize_workspace(client.workspace.id, config.paired_offset)
                };
                SnapshotClient {
                    address: client.address,
                    class: client.class,
                    title: client.title,
                    initial_class: client.initial_class,
                    initial_title: client.initial_title,
                    app_id: client.app_id,
                    pid: client.pid,
                    workspace_id: client.workspace.id,
                    workspace_name: client.workspace.name,
                    paired_slot,
                }
            })
            .collect();

        SessionSnapshot {
            version: SESSION_VERSION,
            created_at: epoch_seconds(),
            signature,
            paired_offset: config.paired_offset,
            workspace_count: config.workspace_count,
            focus: SnapshotFocus {
                monitor: focus_monitor,
                workspace_id: active_workspace_id,
            },
            monitors: snapshot_monitors,
            workspaces: snapshot_workspaces,
            clients: snapshot_clients,
        }
    }
}

pub fn session_path(base_dir: &Path, override_path: Option<&Path>) -> PathBuf {
    override_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| base_dir.join("sessions").join("latest.json"))
}

pub fn save_session(
    ipc: &dyn HyprlandIpc,
    config: &Config,
    base_dir: &Path,
    override_path: Option<&Path>,
) -> Result<PathBuf, SessionError> {
    let snapshot = SessionSnapshot::from_state(
        config,
        current_signature(),
        ipc.active_workspace_id()?,
        ipc.monitors()?,
        ipc.workspaces()?,
        ipc.clients()?,
    );
    let path = session_path(base_dir, override_path);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(&snapshot)?;
    fs::write(&path, contents)?;

    Ok(path)
}

pub fn restore_session(
    ipc: &dyn HyprlandIpc,
    config: &Config,
    base_dir: &Path,
    override_path: Option<&Path>,
    mode: RestoreMode,
) -> Result<(), SessionError> {
    let path = session_path(base_dir, override_path);
    let contents = fs::read_to_string(&path)?;
    let snapshot: SessionSnapshot = serde_json::from_str(&contents)?;
    let current_clients = ipc.clients()?;
    let signature = current_signature();
    let batch = restore_batch(&snapshot, mode, signature.as_deref(), &current_clients, config);

    let argument = batch.to_argument();
    if !argument.is_empty() {
        ipc.batch(&argument)?;
    }

    Ok(())
}

pub fn restore_batch(
    snapshot: &SessionSnapshot,
    mode: RestoreMode,
    current_signature: Option<&str>,
    current_clients: &[ClientInfo],
    config: &Config,
) -> HyprctlBatch {
    let resolved = resolve_restore_mode(mode, snapshot.signature.as_deref(), current_signature);

    match resolved {
        RestoreMode::Same => restore_same_session(snapshot, current_clients),
        RestoreMode::Cold => restore_cold_session(snapshot, current_clients, config),
        RestoreMode::Auto => HyprctlBatch::new(),
    }
}

fn restore_same_session(snapshot: &SessionSnapshot, current_clients: &[ClientInfo]) -> HyprctlBatch {
    let mut batch = HyprctlBatch::new();
    let mut current_by_address = HashMap::new();

    for client in current_clients {
        current_by_address.insert(
            client.address.as_str(),
            (client.workspace.id, client.workspace.name.as_deref()),
        );
    }

    for client in &snapshot.clients {
        if let Some((current_id, current_name)) = current_by_address.get(client.address.as_str())
            && !snapshot_matches_current(client, *current_id, *current_name)
        {
            let argument = format!("{},address:{}", workspace_target(client), client.address);
            batch.dispatch("movetoworkspacesilent", &argument);
        }
    }

    batch
}

fn restore_cold_session(
    snapshot: &SessionSnapshot,
    current_clients: &[ClientInfo],
    config: &Config,
) -> HyprctlBatch {
    let mut batch = HyprctlBatch::new();
    let mut used_snapshot = HashSet::new();
    let mut matched_addresses = HashSet::new();

    for client in current_clients {
        let mut best = None;
        let mut second_best = 0;

        for (idx, snapshot_client) in snapshot.clients.iter().enumerate() {
            if used_snapshot.contains(&idx) {
                continue;
            }
            let score = match_score(snapshot_client, client);
            if score == 0 {
                continue;
            }

            if let Some((_, best_score)) = best {
                if score > best_score {
                    second_best = second_best.max(best_score);
                    best = Some((idx, score));
                } else if score == best_score {
                    second_best = best_score;
                } else if score > second_best {
                    second_best = score;
                }
            } else {
                best = Some((idx, score));
            }
        }

        if let Some((idx, score)) = best
            && score >= 4
            && score > second_best
        {
            let snapshot_client = &snapshot.clients[idx];
            if client.workspace.id != snapshot_client.workspace_id {
                let argument = format!("{},address:{}", workspace_target(snapshot_client), client.address);
                batch.dispatch("movetoworkspacesilent", &argument);
            }
            used_snapshot.insert(idx);
            matched_addresses.insert(client.address.as_str());
        }
    }

    for client in current_clients {
        if matched_addresses.contains(client.address.as_str()) {
            continue;
        }
        if is_special_workspace_name(client.workspace.name.as_deref()) {
            continue;
        }
        let paired_slot = normalize_workspace(client.workspace.id, config.paired_offset);
        if paired_slot != client.workspace.id {
            let argument = format!("{},address:{}", paired_slot, client.address);
            batch.dispatch("movetoworkspacesilent", &argument);
        }
    }

    batch
}

fn resolve_restore_mode(
    mode: RestoreMode,
    snapshot_signature: Option<&str>,
    current_signature: Option<&str>,
) -> RestoreMode {
    match mode {
        RestoreMode::Auto => {
            if snapshot_signature.is_some() && snapshot_signature == current_signature {
                RestoreMode::Same
            } else {
                RestoreMode::Cold
            }
        }
        other => other,
    }
}

fn match_score(snapshot: &SnapshotClient, client: &ClientInfo) -> u8 {
    let mut score = 0;
    if normalized_eq(&snapshot.app_id, &client.app_id) {
        score += 4;
    }
    if normalized_eq(&snapshot.class, &client.class) {
        score += 3;
    }
    if normalized_eq(&snapshot.initial_class, &client.initial_class) {
        score += 2;
    }
    if normalized_eq(&snapshot.title, &client.title) {
        score += 1;
    }
    score
}

fn workspace_target(snapshot: &SnapshotClient) -> String {
    if is_special_workspace_name(snapshot.workspace_name.as_deref()) {
        snapshot
            .workspace_name
            .clone()
            .unwrap_or_else(|| snapshot.workspace_id.to_string())
    } else {
        snapshot.workspace_id.to_string()
    }
}

fn snapshot_matches_current(
    snapshot: &SnapshotClient,
    current_id: u32,
    current_name: Option<&str>,
) -> bool {
    if is_special_workspace_name(snapshot.workspace_name.as_deref()) {
        snapshot.workspace_name.as_deref() == current_name
    } else {
        snapshot.workspace_id == current_id
    }
}

fn normalized_eq(left: &Option<String>, right: &Option<String>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => normalize_value(left) == normalize_value(right),
        _ => false,
    }
}

fn normalize_value(value: &str) -> String {
    value.trim().to_lowercase()
}

fn is_special_workspace_name(name: Option<&str>) -> bool {
    name.is_some_and(|value| value.starts_with("special:"))
}

fn current_signature() -> Option<String> {
    env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
