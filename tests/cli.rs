use clap::{CommandFactory, Parser};

use hyprspaces::cli::{Cli, Command, PairedCommand, SetupCommand};

#[test]
fn parses_paired_switch() {
    let cli = Cli::try_parse_from(["hyprspaces", "paired", "switch", "3"]).expect("parse");

    match cli.command {
        Command::Paired {
            command: PairedCommand::Switch { workspace },
        } => assert_eq!(workspace, 3),
        _ => panic!("unexpected command"),
    }
}

#[test]
fn parses_paired_grab_rogue() {
    let cli = Cli::try_parse_from(["hyprspaces", "paired", "grab-rogue"]).expect("parse");

    match cli.command {
        Command::Paired {
            command: PairedCommand::GrabRogue,
        } => {}
        _ => panic!("unexpected command"),
    }
}

#[test]
fn parses_setup_migrate_windows() {
    let cli = Cli::try_parse_from(["hyprspaces", "setup", "migrate-windows"]).expect("parse");

    match cli.command {
        Command::Setup {
            command: SetupCommand::MigrateWindows,
        } => {}
        _ => panic!("unexpected command"),
    }
}

#[test]
fn parses_completions_bash() {
    let cli = Cli::try_parse_from(["hyprspaces", "completions", "bash"]);

    assert!(cli.is_ok());
}

#[test]
fn help_mentions_completions() {
    let help = Cli::command().render_long_help().to_string();

    assert!(help.contains("completions"));
}

#[test]
fn parses_status_command() {
    let cli = Cli::try_parse_from(["hyprspaces", "status"]);

    assert!(cli.is_ok());
}
