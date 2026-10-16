//! Strategy discovery: `general*.bat` files with category classification
//! compatible with the classic Integra UI.

use std::path::{Path, PathBuf};

use serde::Serialize;

use integra_core::lists::glob_general_bat;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyCategory {
    Basic,
    Simple,
    Advanced,
    Tls,
}

#[derive(Debug, Clone, Serialize)]
pub struct Strategy {
    pub file: String,
    pub name: String,
    pub category: StrategyCategory,
    pub path: PathBuf,
}

/// List every `general*.bat` in the zapret folder, naturally sorted.
pub fn discover(zapret_dir: &Path) -> Vec<Strategy> {
    glob_general_bat(zapret_dir)
        .map(|path| {
            let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let category = classify(&name);
            Strategy { file, name, category, path }
        })
        .collect()
}

pub fn classify(name: &str) -> StrategyCategory {
    let upper = name.to_uppercase();
    if upper.contains("FAKE TLS") || upper.contains("FAKE_TLS") {
        StrategyCategory::Tls
    } else if upper.contains("SIMPLE") {
        StrategyCategory::Simple
    } else if ["ALT", "ADV", "FAKE"].iter().any(|m| upper.contains(m)) {
        StrategyCategory::Advanced
    } else {
        StrategyCategory::Basic
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_matches_legacy_rules() {
        assert_eq!(classify("general FAKE TLS"), StrategyCategory::Tls);
        assert_eq!(classify("general_fake_tls_alt"), StrategyCategory::Tls);
        assert_eq!(classify("general simple"), StrategyCategory::Simple);
        assert_eq!(classify("general ALT"), StrategyCategory::Advanced);
        assert_eq!(classify("general ADV2"), StrategyCategory::Advanced);
        assert_eq!(classify("general FAKE"), StrategyCategory::Advanced);
        assert_eq!(classify("general"), StrategyCategory::Basic);
    }

    #[test]
    fn discover_finds_and_sorts_bats() {
        let dir = integra_core::lists::tempfile_lite::TempDir::new("integra-strat");
        std::fs::write(dir.path().join("general10.bat"), "@echo off").unwrap();
        std::fs::write(dir.path().join("general2.bat"), "@echo off").unwrap();
        std::fs::write(dir.path().join("general FAKE TLS.bat"), "@echo off").unwrap();
        std::fs::write(dir.path().join("other.bat"), "@echo off").unwrap();

        let strategies = discover(dir.path());
        let names: Vec<&str> = strategies.iter().map(|s| s.file.as_str()).collect();
        assert_eq!(
            names,
            vec!["general2.bat", "general10.bat", "general FAKE TLS.bat"],
            "only general*.bat, natural-sorted (numbers before longer text)"
        );
        assert_eq!(strategies[2].category, StrategyCategory::Tls);
    }
}
