//! Version parsing/comparison for zapret maintenance metadata.

use std::fmt;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version(pub Vec<u64>);

impl Version {
    /// Extract the first version-looking substring: `v1.6.2`, `1.6`, `2.0.1-beta`.
    pub fn parse(text: &str) -> Option<Version> {
        let re = Regex::new(r"(?i)\b(?:v(?:ersion)?\s*)?(\d+(?:\.\d+)+)(?:[-+][A-Za-z0-9.-]+)?\b").ok()?;
        let caps = re.captures(text)?;
        let parts = caps[1]
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>();
        if parts.is_empty() {
            None
        } else {
            Some(Version(parts))
        }
    }

    /// `latest > installed` with zero-padding (`1.6` == `1.6.0`).
    pub fn is_newer(installed: &str, latest: &str) -> bool {
        let (Some(a), Some(b)) = (Version::parse(installed), Version::parse(latest)) else {
            return false;
        };
        let width = a.0.len().max(b.0.len());
        let pad = |v: &Version| -> Vec<u64> {
            let mut out = v.0.clone();
            out.resize(width, 0);
            out
        };
        pad(&b) > pad(&a)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let joined: Vec<String> = self.0.iter().map(|p| p.to_string()).collect();
        write!(f, "{}", joined.join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_variants() {
        assert_eq!(Version::parse("v1.6.2").unwrap().to_string(), "1.6.2");
        assert_eq!(Version::parse("version 2.0").unwrap().to_string(), "2.0");
        assert_eq!(Version::parse("zapret-1.6.2-rc1").unwrap().to_string(), "1.6.2");
        assert!(Version::parse("no digits").is_none());
    }

    #[test]
    fn comparisons() {
        assert!(Version::is_newer("1.6.1", "1.6.2"));
        assert!(Version::is_newer("1.6", "1.6.1"));
        assert!(!Version::is_newer("1.6.2", "1.6.2"));
        assert!(!Version::is_newer("2.0", "1.9.9"));
        assert!(!Version::is_newer("garbage", "1.0"));
    }
}
