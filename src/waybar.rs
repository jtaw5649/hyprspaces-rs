use crate::hyprctl::{Hyprctl, HyprctlError, HyprctlRunner, WorkspaceInfo};
use crate::paired::normalize_workspace;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeColors {
    pub bright: String,
    pub mid: String,
    pub dim: String,
}

impl ThemeColors {
    pub fn from_foreground(hex: &str) -> Option<Self> {
        let bright = normalize_hex(hex)?;
        let mid = dim_color(&bright, 65)?;
        let dim = dim_color(&bright, 40)?;
        Some(Self { bright, mid, dim })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum WaybarError {
    #[error("failed to read theme css")]
    Io(#[from] std::io::Error),
    #[error("missing foreground color in theme css")]
    MissingForeground,
    #[error("hyprctl failed")]
    Hyprctl(#[from] HyprctlError),
}

pub fn load_theme_colors(path: &Path) -> Result<ThemeColors, WaybarError> {
    let css = std::fs::read_to_string(path)?;
    let foreground = parse_foreground(&css).ok_or(WaybarError::MissingForeground)?;
    ThemeColors::from_foreground(&foreground).ok_or(WaybarError::MissingForeground)
}

pub fn parse_foreground(css: &str) -> Option<String> {
    let needle = "@define-color foreground";
    let line = css
        .lines()
        .find(|line| line.trim_start().starts_with(needle))?;
    let hex = line
        .split_whitespace()
        .find(|part| part.starts_with('#'))
        .map(|value| value.trim_end_matches(';'))?;
    normalize_hex(hex)
}

pub fn occupied_workspaces(workspaces: &[WorkspaceInfo], offset: u32) -> Vec<u32> {
    let mut ids: Vec<u32> = workspaces
        .iter()
        .filter(|workspace| workspace.windows > 0)
        .map(|workspace| {
            if workspace.id > offset {
                workspace.id - offset
            } else {
                workspace.id
            }
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

pub fn render_display(active_workspace: u32, occupied: &[u32], colors: &ThemeColors) -> String {
    let mut output = String::new();
    let glyph = "\u{f14fb}";
    for i in 1..=5 {
        let is_active = i == active_workspace;
        let is_occupied = occupied.contains(&i);
        if is_active {
            output.push_str(&format!(
                "<span foreground='{}'>{}</span>",
                colors.bright, glyph
            ));
        } else if is_occupied {
            output.push_str(&format!("<span foreground='{}'>{}</span>", colors.mid, i));
        } else {
            output.push_str(&format!("<span foreground='{}'>{}</span>", colors.dim, i));
        }
        if i < 5 {
            output.push(' ');
        }
    }
    output
}

pub fn render_json(text: &str) -> String {
    serde_json::json!({
        "text": text,
        "class": "workspaces",
        "markup": true
    })
    .to_string()
}

pub fn render_state(
    active_workspace: u32,
    workspaces: &[WorkspaceInfo],
    offset: u32,
    colors: &ThemeColors,
) -> String {
    let active_normalized = normalize_workspace(active_workspace, offset);
    let occupied = occupied_workspaces(workspaces, offset);
    let display = render_display(active_normalized, &occupied, colors);
    render_json(&display)
}

pub fn state_from_hyprctl<R: HyprctlRunner>(
    hyprctl: &Hyprctl<R>,
    offset: u32,
    colors: &ThemeColors,
) -> Result<String, WaybarError> {
    let active_workspace = hyprctl.active_workspace_id()?;
    let workspaces = hyprctl.workspaces()?;
    Ok(render_state(active_workspace, &workspaces, offset, colors))
}

pub fn should_update(line: &str) -> bool {
    matches!(
        line,
        line if line.starts_with("workspace")
            || line.starts_with("focusedmon")
            || line.starts_with("createworkspace")
            || line.starts_with("destroyworkspace")
            || line.starts_with("openwindow")
            || line.starts_with("closewindow")
            || line.starts_with("movewindow")
    )
}

fn normalize_hex(hex: &str) -> Option<String> {
    let value = hex.trim();
    if value.len() != 7 || !value.starts_with('#') {
        return None;
    }
    if !value.chars().skip(1).all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(value.to_lowercase())
}

fn dim_color(hex: &str, factor: u8) -> Option<String> {
    let normalized = normalize_hex(hex)?;
    let r = u8::from_str_radix(&normalized[1..3], 16).ok()?;
    let g = u8::from_str_radix(&normalized[3..5], 16).ok()?;
    let b = u8::from_str_radix(&normalized[5..7], 16).ok()?;
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        (u16::from(r) * u16::from(factor) / 100) as u8,
        (u16::from(g) * u16::from(factor) / 100) as u8,
        (u16::from(b) * u16::from(factor) / 100) as u8
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        ThemeColors, load_theme_colors, occupied_workspaces, parse_foreground, render_display,
        render_state, should_update, state_from_hyprctl,
    };
    use crate::hyprctl::{Hyprctl, HyprctlRunner, WorkspaceInfo};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::fs;
    use std::rc::Rc;

    #[test]
    fn parses_foreground_color() {
        let css = "@define-color foreground #AABBCC;";

        assert_eq!(parse_foreground(css), Some("#aabbcc".to_string()));
    }

    #[test]
    fn computes_occupied_workspaces() {
        let workspaces = vec![
            WorkspaceInfo { id: 1, windows: 2 },
            WorkspaceInfo { id: 12, windows: 1 },
            WorkspaceInfo { id: 3, windows: 0 },
        ];

        assert_eq!(occupied_workspaces(&workspaces, 10), vec![1, 2]);
    }

    #[test]
    fn renders_display_with_active_and_occupied() {
        let colors = ThemeColors::from_foreground("#ffffff").expect("colors");
        let output = render_display(2, &[1, 3], &colors);

        assert!(output.contains("\u{f14fb}"));
        assert!(output.contains("1"));
        assert!(output.contains("3"));
    }

    #[test]
    fn renders_state_json() {
        let colors = ThemeColors::from_foreground("#ffffff").expect("colors");
        let workspaces = vec![WorkspaceInfo { id: 1, windows: 1 }];

        let json = render_state(1, &workspaces, 10, &colors);

        assert!(json.contains("\"markup\":true"));
        assert!(json.contains("\"class\":\"workspaces\""));
    }

    #[test]
    fn updates_on_waybar_events() {
        assert!(should_update("workspace>>2"));
        assert!(should_update("focusedmon>>DP-1,1"));
        assert!(should_update("createworkspace>>2"));
        assert!(should_update("openwindow>>0x123,2,App,Title"));
        assert!(!should_update("activelayout>>kbd,us"));
    }

    #[test]
    fn loads_theme_colors_from_css_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("style.css");
        fs::write(&path, "@define-color foreground #AABBCC;").expect("write css");

        let colors = load_theme_colors(&path).expect("colors");
        let expected = ThemeColors::from_foreground("#AABBCC").expect("expected");

        assert_eq!(colors, expected);
    }

    #[derive(Clone)]
    struct SequenceRunner {
        responses: Rc<RefCell<VecDeque<String>>>,
        calls: Rc<RefCell<Vec<Vec<String>>>>,
    }

    impl SequenceRunner {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: Rc::new(RefCell::new(responses.into())),
                calls: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl HyprctlRunner for SequenceRunner {
        fn run(&self, args: &[String]) -> Result<String, crate::hyprctl::HyprctlError> {
            self.calls.borrow_mut().push(args.to_vec());
            let response = self.responses.borrow_mut().pop_front().unwrap_or_default();
            Ok(response)
        }
    }

    #[test]
    fn renders_state_from_hyprctl() {
        let colors = ThemeColors::from_foreground("#ffffff").expect("colors");
        let runner = SequenceRunner::new(vec![
            r#"{"id":12}"#.to_string(),
            r#"[{"id":1,"windows":1},{"id":12,"windows":2}]"#.to_string(),
        ]);
        let hyprctl = Hyprctl::new(runner.clone());

        let json = state_from_hyprctl(&hyprctl, 10, &colors).expect("state");

        assert!(json.contains("\"class\":\"workspaces\""));
        let calls = runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec!["-j".to_string(), "activeworkspace".to_string()]
        );
        assert_eq!(calls[1], vec!["-j".to_string(), "workspaces".to_string()]);
    }
}
