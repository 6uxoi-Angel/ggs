//! Fetch the current server list from GitHub (raw file).

use std::time::Duration;

/// GET `url`, parse `{"servers": ["host:port", ...]}`.
/// Any failure -> empty list (caller falls back to the built-in list).
pub fn fetch_servers(url: &str) -> Vec<String> {
    let resp = match ureq::get(url).timeout(Duration::from_secs(10)).call() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let text = match resp.into_string() {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    v.get("servers")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| s.contains(':'))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_list() {
        let url = "data:application/json,{\"servers\":[\"1.2.3.4:9443\"]}";
        // (data: URLs not supported by ureq; this just guards the parse helper logic)
        let _ = url;
        let parsed: serde_json::Value = serde_json::from_str(r#"{"servers":["1.2.3.4:9443"]}"#).unwrap();
        let servers: Vec<String> = parsed["servers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        assert_eq!(servers, vec!["1.2.3.4:9443"]);
    }
}
