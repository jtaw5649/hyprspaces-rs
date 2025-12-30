use serde::Deserialize;
use std::path::Path;

pub const DEFAULT_PAIRED_OFFSET: u32 = 10;
pub const DEFAULT_WORKSPACE_COUNT: u32 = DEFAULT_PAIRED_OFFSET;

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub primary_monitor: String,
    pub secondary_monitor: String,
    pub paired_offset: u32,
    pub workspace_count: u32,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    primary_monitor: Option<String>,
    secondary_monitor: Option<String>,
    #[serde(default = "default_offset")]
    paired_offset: u32,
    #[serde(default)]
    workspace_count: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid config json: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
}

impl Config {
    pub fn from_json(input: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig = serde_json::from_str(input)?;
        let primary_monitor = raw
            .primary_monitor
            .filter(|value| !value.is_empty())
            .ok_or(ConfigError::MissingField("primary_monitor"))?;
        let secondary_monitor = raw
            .secondary_monitor
            .filter(|value| !value.is_empty())
            .ok_or(ConfigError::MissingField("secondary_monitor"))?;
        let workspace_count = raw.workspace_count.unwrap_or(raw.paired_offset);

        Ok(Self {
            primary_monitor,
            secondary_monitor,
            paired_offset: workspace_count,
            workspace_count,
        })
    }

    pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_json(&contents)
    }
}

impl std::str::FromStr for Config {
    type Err = ConfigError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Config::from_json(input)
    }
}

fn default_offset() -> u32 {
    DEFAULT_PAIRED_OFFSET
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::fs;

    #[test]
    fn parses_config_with_explicit_offset() {
        let input =
            r#"{"primary_monitor":"DP-1","secondary_monitor":"HDMI-A-1","paired_offset":12}"#;

        let config = Config::from_json(input).expect("config should parse");

        assert_eq!(config.primary_monitor, "DP-1");
        assert_eq!(config.secondary_monitor, "HDMI-A-1");
        assert_eq!(config.paired_offset, 12);
        assert_eq!(config.workspace_count, 12);
    }

    #[test]
    fn parses_config_with_workspace_count() {
        let input =
            r#"{"primary_monitor":"DP-1","secondary_monitor":"HDMI-A-1","workspace_count":6}"#;

        let config = Config::from_json(input).expect("config should parse");

        assert_eq!(config.paired_offset, 6);
        assert_eq!(config.workspace_count, 6);
    }

    #[test]
    fn workspace_count_takes_precedence_over_paired_offset() {
        let input = r#"{"primary_monitor":"DP-1","secondary_monitor":"HDMI-A-1","paired_offset":8,"workspace_count":6}"#;

        let config = Config::from_json(input).expect("config should parse");

        assert_eq!(config.paired_offset, 6);
        assert_eq!(config.workspace_count, 6);
    }

    #[test]
    fn parses_config_via_from_str_trait() {
        let input = r#"{"primary_monitor":"DP-1","secondary_monitor":"HDMI-A-1"}"#;

        let config: Config = input.parse().expect("parse via trait");

        assert_eq!(config.primary_monitor, "DP-1");
        assert_eq!(config.secondary_monitor, "HDMI-A-1");
        assert_eq!(config.paired_offset, 10);
        assert_eq!(config.workspace_count, 10);
    }

    #[test]
    fn defaults_offset_when_missing() {
        let input = r#"{"primary_monitor":"DP-1","secondary_monitor":"HDMI-A-1"}"#;

        let config = Config::from_json(input).expect("config should parse");

        assert_eq!(config.paired_offset, 10);
        assert_eq!(config.workspace_count, 10);
    }

    #[test]
    fn defaults_workspace_count_from_paired_offset() {
        let input =
            r#"{"primary_monitor":"DP-1","secondary_monitor":"HDMI-A-1","paired_offset":8}"#;

        let config = Config::from_json(input).expect("config should parse");

        assert_eq!(config.paired_offset, 8);
        assert_eq!(config.workspace_count, 8);
    }

    #[test]
    fn errors_when_primary_missing() {
        let input = r#"{"secondary_monitor":"HDMI-A-1","paired_offset":10}"#;

        let error = Config::from_json(input).expect_err("config should fail");

        assert!(matches!(
            error,
            super::ConfigError::MissingField("primary_monitor")
        ));
    }

    #[test]
    fn errors_when_secondary_missing() {
        let input = r#"{"primary_monitor":"DP-1","paired_offset":10}"#;

        let error = Config::from_json(input).expect_err("config should fail");

        assert!(matches!(
            error,
            super::ConfigError::MissingField("secondary_monitor")
        ));
    }

    #[test]
    fn loads_config_from_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("paired.json");
        let input =
            r#"{"primary_monitor":"DP-1","secondary_monitor":"HDMI-A-1","paired_offset":12}"#;
        fs::write(&path, input).expect("write");

        let config = Config::from_path(&path).expect("config should parse");

        assert_eq!(config.primary_monitor, "DP-1");
        assert_eq!(config.secondary_monitor, "HDMI-A-1");
        assert_eq!(config.paired_offset, 12);
    }

    #[test]
    fn errors_when_config_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.json");

        let error = Config::from_path(&path).expect_err("config should fail");

        assert!(matches!(error, super::ConfigError::Io(_)));
    }
}
