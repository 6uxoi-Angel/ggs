//! Safe management of zapret domain / IP lists with atomic writes.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::Serialize;

use crate::domain::{normalize_list_entry, DomainError};

/// One managed list file inside `<zapret>/lists`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListSpec {
    pub id: &'static str,
    pub file: &'static str,
    /// What kind of entries the list expects (used for validation hints).
    pub kind: ListKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    Domains,
    Networks,
    Mixed,
}

pub const LISTS: [ListSpec; 5] = [
    ListSpec {
        id: "list-general",
        file: "list-general.txt",
        kind: ListKind::Mixed,
    },
    ListSpec {
        id: "list-exclude",
        file: "list-exclude.txt",
        kind: ListKind::Mixed,
    },
    ListSpec {
        id: "list-google",
        file: "list-google.txt",
        kind: ListKind::Domains,
    },
    ListSpec {
        id: "ipset-all",
        file: "ipset-all.txt",
        kind: ListKind::Networks,
    },
    ListSpec {
        id: "ipset-exclude",
        file: "ipset-exclude.txt",
        kind: ListKind::Networks,
    },
];

pub fn find_spec(id: &str) -> Option<&'static ListSpec> {
    LISTS.iter().find(|spec| spec.id == id)
}

pub fn has_strategies(zapret_dir: &Path) -> bool {
    glob_general_bat(zapret_dir).next().is_some()
}

/// Case-insensitive `general*.bat` glob without pulling in a glob crate.
pub fn glob_general_bat(zapret_dir: &Path) -> impl Iterator<Item = PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(zapret_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .map(|n| {
                                let n = n.to_string_lossy().to_ascii_lowercase();
                                n.starts_with("general") && n.ends_with(".bat")
                            })
                            .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|p| natural_key(&p.file_name().unwrap_or_default().to_string_lossy()));
    found.into_iter()
}

/// Natural sort key: digits compare numerically ("general2" < "general10").
pub fn natural_key(name: &str) -> Vec<NaturalPart> {
    natural_parts(name)
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub enum NaturalPart {
    Text(String),
    Num(u64),
}

fn natural_parts(name: &str) -> Vec<NaturalPart> {
    let lower = name.to_lowercase();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut digits = String::new();
    for ch in lower.chars() {
        if ch.is_ascii_digit() {
            if !current.is_empty() {
                parts.push(NaturalPart::Text(std::mem::take(&mut current)));
            }
            digits.push(ch);
        } else {
            if !digits.is_empty() {
                parts.push(NaturalPart::Num(digits.parse().unwrap_or(u64::MAX)));
                digits.clear();
            }
            current.push(ch);
        }
    }
    if !digits.is_empty() {
        parts.push(NaturalPart::Num(digits.parse().unwrap_or(u64::MAX)));
    }
    if !current.is_empty() {
        parts.push(NaturalPart::Text(current));
    }
    parts
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct BulkReport {
    pub added: Vec<String>,
    pub duplicates: Vec<String>,
    pub invalid: Vec<InvalidEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InvalidEntry {
    pub value: String,
    pub error: String,
}

impl BulkReport {
    pub fn changed(&self) -> bool {
        !self.added.is_empty()
    }
}

/// Edits list files of a single zapret installation. All operations are
/// mutex-guarded and files are rewritten atomically (tmp + rename).
pub struct ListsManager {
    root: Mutex<PathBuf>,
}

impl ListsManager {
    pub fn new(zapret_path: impl Into<PathBuf>) -> Self {
        Self {
            root: Mutex::new(zapret_path.into()),
        }
    }

    pub fn set_root(&self, zapret_path: impl Into<PathBuf>) {
        *self.root.lock() = zapret_path.into();
    }

    pub fn root(&self) -> PathBuf {
        self.root.lock().clone()
    }

    pub fn lists_dir(&self) -> PathBuf {
        self.root.lock().join("lists")
    }

    /// Path of one list file. Caller must hold the root lock (or accept a
    /// racy read) — used internally by the guarded write operations.
    fn lists_dir_no_lock(root: &Path) -> PathBuf {
        root.join("lists")
    }

    pub fn list_path(&self, id: &str) -> Option<PathBuf> {
        find_spec(id).map(|spec| self.lists_dir().join(spec.file))
    }

    pub fn read(&self, id: &str) -> Vec<String> {
        let Some(path) = self.list_path(id) else {
            return Vec::new();
        };
        match fs::read_to_string(&path) {
            Ok(text) => text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn count(&self, id: &str) -> usize {
        self.read(id)
            .iter()
            .filter(|l| !l.starts_with('#') && !l.starts_with(';'))
            .count()
    }

    pub fn contains(&self, id: &str, entry: &str) -> bool {
        let needle = entry.trim().to_ascii_lowercase();
        self.read(id)
            .iter()
            .any(|l| l.trim().to_ascii_lowercase() == needle)
    }

    /// Add many entries at once. Each entry is validated & normalized;
    /// duplicates are reported, not re-added. Comments are appended verbatim.
    pub fn add(&self, id: &str, entries: &[String]) -> Result<BulkReport, DomainError> {
        let Some(spec) = find_spec(id) else {
            return Err(DomainError::CannotParse);
        };
        let guard = self.root.lock(); // serialize writers
        let path = Self::lists_dir_no_lock(&guard).join(spec.file);

        let mut lines: Vec<String> = match fs::read_to_string(&path) {
            Ok(text) => text.lines().map(|l| l.trim_end().to_string()).collect(),
            Err(_) => Vec::new(),
        };
        let mut existing: HashSet<String> = lines
            .iter()
            .map(|l| l.trim().to_ascii_lowercase())
            .collect();

        let mut report = BulkReport::default();
        for raw in entries {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            match normalize_list_entry(raw) {
                Ok(entry) => {
                    let key = entry.to_ascii_lowercase();
                    if existing.contains(&key) {
                        report.duplicates.push(entry);
                        continue;
                    }
                    existing.insert(key);
                    lines.push(entry.clone());
                    report.added.push(entry);
                }
                Err(err) => report.invalid.push(InvalidEntry {
                    value: raw.to_string(),
                    error: err.i18n_key().to_string(),
                }),
            }
        }

        if report.changed() {
            fs::create_dir_all(Self::lists_dir_no_lock(&guard))?;
            atomic_write_lines(&path, &lines)?;
        }
        Ok(report)
    }

    /// Remove a single entry (case-insensitive). Returns true when the file changed.
    pub fn remove(&self, id: &str, entry: &str) -> std::io::Result<bool> {
        let Some(spec) = find_spec(id) else {
            return Ok(false);
        };
        let guard = self.root.lock();
        let path = Self::lists_dir_no_lock(&guard).join(spec.file);
        let Ok(text) = fs::read_to_string(&path) else {
            return Ok(false);
        };
        let needle = entry.trim().to_ascii_lowercase();
        let mut removed = false;
        let kept: Vec<String> = text
            .lines()
            .filter(|line| {
                let matches = line.trim().to_ascii_lowercase() == needle;
                if matches {
                    removed = true;
                }
                !matches
            })
            .map(|l| l.to_string())
            .collect();
        if removed {
            atomic_write_lines(&path, &kept)?;
        }
        Ok(removed)
    }

    /// Remove the entry from every managed list; returns ids of touched lists.
    pub fn remove_from_all(&self, entry: &str) -> std::io::Result<Vec<String>> {
        let mut touched = Vec::new();
        for spec in LISTS {
            if self.remove(spec.id, entry)? {
                touched.push(spec.id.to_string());
            }
        }
        Ok(touched)
    }

    /// Overwrite the whole list (used by the UI bulk editor).
    pub fn replace(&self, id: &str, lines: &[String]) -> std::io::Result<BulkReport> {
        let Some(spec) = find_spec(id) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "unknown list",
            ));
        };
        let guard = self.root.lock();
        let path = Self::lists_dir_no_lock(&guard).join(spec.file);
        let mut report = BulkReport::default();
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        for raw in lines {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            match normalize_list_entry(raw) {
                Ok(entry) => {
                    let key = entry.to_ascii_lowercase();
                    if !seen.insert(key) {
                        report.duplicates.push(entry);
                        continue;
                    }
                    out.push(entry);
                }
                Err(err) => report.invalid.push(InvalidEntry {
                    value: raw.to_string(),
                    error: err.i18n_key().to_string(),
                }),
            }
        }
        report.added = out.clone();
        fs::create_dir_all(Self::lists_dir_no_lock(&guard))?;
        atomic_write_lines(&path, &out)?;
        Ok(report)
    }
}

fn atomic_write_lines(path: &Path, lines: &[String]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile_lite::TempDir, ListsManager) {
        let dir = tempfile_lite::TempDir::new("integra-lists");
        fs::create_dir_all(dir.path().join("lists")).unwrap();
        let mgr = ListsManager::new(dir.path().to_path_buf());
        (dir, mgr)
    }

    #[test]
    fn add_dedups_and_normalizes() {
        let (_dir, mgr) = setup();
        let report = mgr
            .add(
                "list-general",
                &[
                    "https://Example.com/path".into(),
                    "example.com".into(),
                    "пример.рф".into(),
                    "not valid!!".into(),
                ],
            )
            .unwrap();
        assert_eq!(report.added, vec!["example.com", "xn--e1afmkfd.xn--p1ai"]);
        assert_eq!(report.duplicates, vec!["example.com"]);
        assert_eq!(report.invalid.len(), 1);
        assert!(mgr.contains("list-general", "EXAMPLE.com"));
        assert_eq!(mgr.count("list-general"), 2);
    }

    #[test]
    fn remove_and_remove_from_all() {
        let (_dir, mgr) = setup();
        mgr.add("list-general", &["a.example".into()]).unwrap();
        mgr.add("list-exclude", &["a.example".into()]).unwrap();
        let touched = mgr.remove_from_all("A.EXAMPLE").unwrap();
        assert_eq!(touched.len(), 2);
        assert!(!mgr.contains("list-general", "a.example"));
    }

    #[test]
    fn networks_go_to_ipset() {
        let (_dir, mgr) = setup();
        let report = mgr
            .add("ipset-all", &["10.0.0.0/8".into(), "8.8.8.8".into()])
            .unwrap();
        assert_eq!(report.added.len(), 2);
        assert!(mgr.contains("ipset-all", "10.0.0.0/8"));
    }

    #[test]
    fn natural_sort_orders_strategies() {
        // Numbered variants group first (shared "general" text part, then the
        // numeric part), then plain-text names in string order.
        let mut names = vec![
            "general10.bat".to_string(),
            "general FAKE TLS.bat".into(),
            "general.bat".into(),
            "general (ALT).bat".into(),
            "general2.bat".into(),
        ];
        names.sort_by_key(|n| natural_parts(n));
        assert_eq!(
            names,
            vec![
                "general2.bat",
                "general10.bat",
                "general (ALT).bat",
                "general FAKE TLS.bat",
                "general.bat"
            ]
        );
    }
}

/// Minimal temp-dir helper so core has no dev-dependency on tempfile.
pub mod tempfile_lite {
    use std::fs;
    use std::path::{Path, PathBuf};

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{}-{}", prefix, crate::random_hex(6)));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
