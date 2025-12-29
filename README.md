# Hyprspaces

Coming soon.

## Mission

Hyprspaces aims to make dual-monitor and multi-monitor paired workspaces feel like a native Hyprland feature.

## Waybar integration (manual)

Hyprspaces will not modify Waybar configs during install/uninstall. If you want a Waybar module, add it manually:

```json
"custom/workspaces": {
    "exec": "hyprspaces waybar",
    "return-type": "json",
    "format": "{}",
    "on-scroll-up": "hyprspaces paired cycle prev",
    "on-scroll-down": "hyprspaces paired cycle next"
}
```

The module expects JSON output for Waybar custom modules (see Waybar docs) and should preserve the existing examples.
