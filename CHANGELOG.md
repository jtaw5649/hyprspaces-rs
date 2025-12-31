# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `--ipc` flag with `hyprctl` default plus a `native-ipc` feature gate for the hyprland-rs backend.
- Native IPC daemon event listening (hyprland-rs) when built with `--features native-ipc`.
- `completions` subcommand for generating bash/zsh/fish scripts.
- `status` subcommand for daemon/config/pairing visibility.
- `workspace_count` config option (preferred) with setup-generated bindings/rules based on the configured count.
- `paired grab-rogue` command to return windows from the secondary range to their paired primary workspaces.
- `wrap_cycling` config option to disable wrap-around when cycling paired workspaces.

### Changed
- Hyprctl errors now carry command, status, and JSON context for easier debugging.
- Daemon monitor rebalance is debounced with a trailing flush to avoid missed topology updates.
- Default paired offset is centralized for consistent config and setup behavior.
- CI runs feature tests for `native-ipc` builds alongside default tests.

### Fixed

## [1.0.0] - 2025-12-30

### Added
- CLI for paired workspace management (`paired switch`, `paired cycle`, `paired move-window`)
- Daemon mode for automatic workspace rebalancing on monitor events
- Waybar module with button generation and theme customization
- Setup commands for Hyprland integration (`setup install`, `setup uninstall`)
- Configuration via `~/.config/hyprspaces/paired.json`
- AUR package for Arch Linux installation
