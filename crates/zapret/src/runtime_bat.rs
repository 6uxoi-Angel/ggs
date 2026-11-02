//! Hidden runtime BAT generation.
//!
//! Original Flowseal strategies use `start "..." /min winws.exe ...` which
//! spawns a visible console window. We never modify the originals: instead a
//! runtime copy (`.integra-runtime-<profile>.bat`) is created where `start` is
//! removed so `winws.exe` inherits our hidden `cmd.exe`, and
//! `NO_UPDATE_CHECK=1` is injected after `@echo off`.

use std::fs;
use std::path::Path;

use encoding_rs::{IBM866 as CP866, WINDOWS_1251 as CP1251};
use regex::Regex;

use crate::ZapretError;

/// Read a BAT file, detecting a suitable encoding for lossless re-write.
pub fn read_batch_text(path: &Path) -> std::io::Result<(String, &'static str)> {
    let raw = fs::read(path)?;
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        // Drop the BOM on purpose: cmd.exe may treat it as part of @echo.
        let text = String::from_utf8_lossy(&raw[3..]).into_owned();
        return Ok((text, "utf-8"));
    }
    if let Ok(text) = String::from_utf8(raw.clone()) {
        return Ok((text, "utf-8"));
    }
    // cp866 (OEM Russian) then cp1251 — mirrors the Python implementation.
    for (encoding, name) in [(CP866, "cp866"), (CP1251, "cp1251")] {
        let (text, _, had_errors) = encoding.decode(&raw);
        if !had_errors {
            return Ok((text.into_owned(), name));
        }
    }
    Ok((raw.iter().map(|&b| b as char).collect(), "latin-1"))
}

fn encode_batch_text(text: &str, encoding: &str) -> Vec<u8> {
    match encoding {
        "cp866" => CP866.encode(text).0.into_owned(),
        "cp1251" => CP1251.encode(text).0.into_owned(),
        _ => text.as_bytes().to_vec(),
    }
}

/// Build the hidden runtime copy of a strategy BAT.
pub fn build_hidden_runtime_batch(source: &Path, destination: &Path) -> Result<(), ZapretError> {
    let (text, encoding) = read_batch_text(source)?;

    let start_quoted = Regex::new(
        r#"(?i)^(?P<indent>\s*)start\s+"[^"]*"\s+(?:(?:/min|/b|/wait)\s+)*(?P<exe>"[^"]*winws\.exe")(?P<rest>.*)$"#,
    )
    .expect("valid regex");
    let start_bare = Regex::new(
        r#"(?i)^(?P<indent>\s*)start\s+(?:(?:/min|/b|/wait)\s+)*(?P<exe>"?[^"\s]*winws\.exe"?)(?P<rest>.*)$"#,
    )
    .expect("valid regex");

    let mut output = String::new();
    let mut replaced = false;
    let mut inserted_no_update = false;

    for line in text.split_inclusive('\n') {
        let ending = if line.ends_with("\r\n") {
            "\r\n"
        } else if line.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let body = &line[..line.len() - ending.len()];

        if !inserted_no_update && body.trim().eq_ignore_ascii_case("@echo off") {
            output.push_str(body);
            output.push_str(ending);
            output.push_str("@set \"NO_UPDATE_CHECK=1\"");
            output.push_str(ending);
            inserted_no_update = true;
            continue;
        }

        let matched = start_quoted
            .captures(body)
            .or_else(|| start_bare.captures(body));
        if !replaced {
            if let Some(caps) = matched {
                if body.to_lowercase().contains("winws.exe") {
                    output.push_str(&caps["indent"]);
                    output.push_str(&caps["exe"]);
                    output.push_str(&caps["rest"]);
                    output.push_str(ending);
                    replaced = true;
                    continue;
                }
            }
        }
        output.push_str(body);
        output.push_str(ending);
    }

    if !replaced {
        return Err(ZapretError::BatchInvalid);
    }

    fs::write(destination, encode_batch_text(&output, encoding))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_start_line_and_injects_env() {
        let dir = integra_core::lists::tempfile_lite::TempDir::new("integra-bat");
        let source = dir.path().join("general.bat");
        let dest = dir.path().join(".integra-runtime-test.bat");
        fs::write(
            &source,
            "@echo off\r\nset ARGS=--filter=udp\r\nstart \"zapret\" /min \"%~dp0bin\\winws.exe\" %ARGS%\r\npause\r\n",
        )
        .unwrap();

        build_hidden_runtime_batch(&source, &dest).unwrap();
        let text = fs::read_to_string(&dest).unwrap();

        assert!(
            text.contains("@set \"NO_UPDATE_CHECK=1\""),
            "env must be injected"
        );
        assert!(!text.contains("start "), "start must be removed");
        assert!(
            text.contains("\"%~dp0bin\\winws.exe\" %ARGS%"),
            "direct invocation preserved"
        );
        assert!(text.contains("\r\n"), "CRLF endings preserved");
    }

    #[test]
    fn quoted_flags_and_bare_forms() {
        let dir = integra_core::lists::tempfile_lite::TempDir::new("integra-bat2");
        let source = dir.path().join("general.bat");
        let dest = dir.path().join("out.bat");
        fs::write(
            &source,
            "@echo off\nstart /min bin\\winws.exe --filter=tcp\necho done\n",
        )
        .unwrap();
        build_hidden_runtime_batch(&source, &dest).unwrap();
        let text = fs::read_to_string(&dest).unwrap();
        assert!(text.contains("bin\\winws.exe --filter=tcp"));
        assert!(!text.contains("start"));
    }

    #[test]
    fn fails_without_winws_line() {
        let dir = integra_core::lists::tempfile_lite::TempDir::new("integra-bat3");
        let source = dir.path().join("general.bat");
        let dest = dir.path().join("out.bat");
        fs::write(&source, "@echo off\necho hello\n").unwrap();
        assert!(matches!(
            build_hidden_runtime_batch(&source, &dest),
            Err(ZapretError::BatchInvalid)
        ));
    }

    #[test]
    fn cp866_roundtrip() {
        let dir = integra_core::lists::tempfile_lite::TempDir::new("integra-bat4");
        let source = dir.path().join("general.bat");
        let dest = dir.path().join("out.bat");
        // "Запуск" in cp866 + a start line.
        let mut bytes: Vec<u8> = "@echo off\r\nrem ".bytes().collect();
        bytes.extend_from_slice(&CP866.encode("Запуск").0);
        bytes.extend_from_slice(b"\r\nstart \"z\" /min \"bin\\winws.exe\" --x\r\n");
        fs::write(&source, &bytes).unwrap();

        build_hidden_runtime_batch(&source, &dest).unwrap();
        let (text, encoding) = read_batch_text(&dest).unwrap();
        assert_eq!(encoding, "cp866");
        assert!(text.contains("Запуск"), "cyrillic comment survives");
        assert!(!text.contains("start "));
    }
}
