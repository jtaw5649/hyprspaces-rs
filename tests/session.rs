use std::path::Path;

use hyprspaces::config::Config;
use hyprspaces::hyprctl::{ClientInfo, MonitorInfo, WorkspaceInfo, WorkspaceRef};
use hyprspaces::session::{restore_batch, session_path, RestoreMode, SessionSnapshot};

fn test_config() -> Config {
    Config {
        primary_monitor: "DP-1".to_string(),
        secondary_monitor: "HDMI-A-1".to_string(),
        paired_offset: 10,
        workspace_count: 10,
        wrap_cycling: true,
    }
}

#[test]
fn session_path_defaults_to_latest() {
    let base = Path::new("/tmp/hyprspaces");

    let path = session_path(base, None);

    assert_eq!(path, base.join("sessions").join("latest.json"));
}

#[test]
fn session_path_uses_override() {
    let base = Path::new("/tmp/hyprspaces");
    let override_path = Path::new("/tmp/custom/session.json");

    let path = session_path(base, Some(override_path));

    assert_eq!(path, override_path);
}

#[test]
fn snapshot_computes_paired_slot_and_focus() {
    let config = test_config();
    let monitors = vec![MonitorInfo {
        name: "HDMI-A-1".to_string(),
        x: 0,
        id: 1,
    }];
    let workspaces = vec![WorkspaceInfo {
        id: 13,
        windows: 1,
        name: Some("13".to_string()),
        monitor: Some("HDMI-A-1".to_string()),
    }];
    let clients = vec![ClientInfo {
        address: "0x123".to_string(),
        workspace: WorkspaceRef {
            id: 13,
            name: Some("13".to_string()),
        },
        class: Some("kitty".to_string()),
        title: Some("term".to_string()),
        initial_class: None,
        initial_title: None,
        app_id: None,
        pid: Some(4242),
    }];

    let snapshot = SessionSnapshot::from_state(
        &config,
        Some("sig".to_string()),
        13,
        monitors,
        workspaces,
        clients,
    );

    assert_eq!(snapshot.focus.workspace_id, 13);
    assert_eq!(snapshot.focus.monitor.as_deref(), Some("HDMI-A-1"));
    assert_eq!(snapshot.clients[0].paired_slot, 3);
}

#[test]
fn restore_same_session_moves_mismatched_clients() {
    let config = test_config();
    let snapshot = SessionSnapshot {
        version: 1,
        created_at: 0,
        signature: Some("sig".to_string()),
        paired_offset: 10,
        workspace_count: 10,
        focus: hyprspaces::session::SnapshotFocus {
            monitor: None,
            workspace_id: 1,
        },
        monitors: Vec::new(),
        workspaces: Vec::new(),
        clients: vec![hyprspaces::session::SnapshotClient {
            address: "0xabc".to_string(),
            class: None,
            title: None,
            initial_class: None,
            initial_title: None,
            app_id: None,
            pid: None,
            workspace_id: 2,
            workspace_name: None,
            paired_slot: 2,
        }],
    };
    let current_clients = vec![ClientInfo {
        address: "0xabc".to_string(),
        workspace: WorkspaceRef { id: 1, name: None },
        class: None,
        title: None,
        initial_class: None,
        initial_title: None,
        app_id: None,
        pid: None,
    }];

    let batch = restore_batch(
        &snapshot,
        RestoreMode::Same,
        Some("sig"),
        &current_clients,
        &config,
    );

    assert_eq!(
        batch.to_argument(),
        "dispatch movetoworkspacesilent 2,address:0xabc"
    );
}

#[test]
fn restore_cold_matches_by_app_id() {
    let config = test_config();
    let snapshot = SessionSnapshot {
        version: 1,
        created_at: 0,
        signature: Some("sig".to_string()),
        paired_offset: 10,
        workspace_count: 10,
        focus: hyprspaces::session::SnapshotFocus {
            monitor: None,
            workspace_id: 1,
        },
        monitors: Vec::new(),
        workspaces: Vec::new(),
        clients: vec![hyprspaces::session::SnapshotClient {
            address: "0xabc".to_string(),
            class: None,
            title: None,
            initial_class: None,
            initial_title: None,
            app_id: Some("org.gnome.Nautilus".to_string()),
            pid: None,
            workspace_id: 4,
            workspace_name: None,
            paired_slot: 4,
        }],
    };
    let current_clients = vec![ClientInfo {
        address: "0xdef".to_string(),
        workspace: WorkspaceRef { id: 1, name: None },
        class: None,
        title: None,
        initial_class: None,
        initial_title: None,
        app_id: Some("org.gnome.Nautilus".to_string()),
        pid: None,
    }];

    let batch = restore_batch(
        &snapshot,
        RestoreMode::Cold,
        Some("sig"),
        &current_clients,
        &config,
    );

    assert_eq!(
        batch.to_argument(),
        "dispatch movetoworkspacesilent 4,address:0xdef"
    );
}
