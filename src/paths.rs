use std::path::{Path, PathBuf};

pub fn config_dir(home: &Path, xdg_config: Option<&Path>) -> PathBuf {
    xdg_config
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
}

pub fn config_path(home: &Path, xdg_config: Option<&Path>) -> PathBuf {
    config_dir(home, xdg_config)
        .join("hyprspaces")
        .join("paired.json")
}

pub fn hypr_config_dir(home: &Path, xdg_config: Option<&Path>) -> PathBuf {
    config_dir(home, xdg_config).join("hypr")
}

#[cfg(test)]
mod tests {
    use super::{config_dir, config_path, hypr_config_dir};
    use std::path::PathBuf;

    #[test]
    fn uses_xdg_config_when_provided() {
        let home = PathBuf::from("/home/jtaw");
        let xdg = PathBuf::from("/tmp/config");

        assert_eq!(config_dir(&home, Some(&xdg)), PathBuf::from("/tmp/config"));
    }

    #[test]
    fn defaults_to_home_config_dir() {
        let home = PathBuf::from("/home/jtaw");

        assert_eq!(config_dir(&home, None), PathBuf::from("/home/jtaw/.config"));
    }

    #[test]
    fn builds_hyprspaces_config_path() {
        let home = PathBuf::from("/home/jtaw");

        assert_eq!(
            config_path(&home, None),
            PathBuf::from("/home/jtaw/.config/hyprspaces/paired.json")
        );
    }

    #[test]
    fn builds_hypr_config_dir() {
        let home = PathBuf::from("/home/jtaw");

        assert_eq!(
            hypr_config_dir(&home, None),
            PathBuf::from("/home/jtaw/.config/hypr")
        );
    }
}
