use clap::Parser;

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
fn parses_setup_migrate_windows() {
    let cli = Cli::try_parse_from(["hyprspaces", "setup", "migrate-windows"]).expect("parse");

    match cli.command {
        Command::Setup {
            command: SetupCommand::MigrateWindows,
        } => {}
        _ => panic!("unexpected command"),
    }
}
