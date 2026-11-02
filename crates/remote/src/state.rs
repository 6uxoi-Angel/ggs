use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Blocks {
    /// app name -> resolved exe path
    #[serde(default)]
    pub apps: BTreeMap<String, String>,
    #[serde(default)]
    pub sites: Vec<String>,
}

impl Blocks {
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.apps.len() + self.sites.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Serialize, Deserialize)]
pub struct State {
    pub id: String,
    pub blocks: Blocks,
}

impl State {
    pub fn load(dir: &str) -> Self {
        let path = Path::new(dir).join("state.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<State>(&text) {
                return s;
            }
        }
        let s = State {
            id: uuid::Uuid::new_v4().to_string(),
            blocks: Blocks::empty(),
        };
        let _ = s.save(dir);
        s
    }

    pub fn save(&self, dir: &str) -> Result<(), String> {
        let path = Path::new(dir).join("state.json");
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = Path::new(dir).join("state.json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        Ok(())
    }
}
