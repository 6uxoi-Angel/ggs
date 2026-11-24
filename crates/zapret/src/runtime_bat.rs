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
    // Last resort: neither UTF-8, cp866 nor cp1251 decoded cleanly.
    //
    // The old fallback smuggled raw bytes through `chars` (latin-1 style,
    // `b as char`) while the writer emitted the string back as UTF-8 —
    // silently DOUBLING every non-ASCII byte and corrupting Cyrillic text.
    // Instead we decode lossily up front and the writer emits the string's
    // own UTF-8 bytes (`str::as_bytes`): every char is encoded exactly once,
    // so the written bytes always match the declared "utf-8" encoding and no
    // byte-doubling or per-char truncation can occur.
    Ok((String::from_utf8_lossy(&raw).into_owned(), "utf-8"))
}

fn encode_batch_text(text: &str, encoding: &str) -> Vec<u8> {
    match encoding {
        "cp866" => CP866.encode(text).0.into_owned(),
        "cp1251" => CP1251.encode(text).0.into_owned(),
        // "utf-8" (and any unknown tag): the string's own UTF-8 bytes —
        // never a per-char `as u8` cast, which would silently truncate
        // chars >= U+0100 and double every multi-byte sequence.
        _ => text.as_bytes().to_vec(),
    }
}

/// Build the hidden runtime copy of a strategy BAT.
pub fn build_hidden_runtime_batch(source: &Path, destination: &Path) -> Result<(), ZapretError> {
    let (text, encoding) = read_batch_text(source)?;

    let start_quoted = Regex::new(
        // A quoted title may be followed by a quoted OR bare winws.exe path
        // (`start "z" /min winws.exe` broke the old quoted-only pattern with
        // an unrecoverable BatchInvalid).
        r#"(?i)^(?P<indent>\s*)start\s+"[^"]*"\s+(?:(?:/min|/b|/wait)\s+)*(?P<exe>"?[^"\s]*winws\.exe"?)(?P<rest>.*)$"#,
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

/// Extract numbered menu entries from a zapret batch menu script
/// (`service.bat`).
///
/// Recognizes the shapes the upstream scripts use, e.g.:
/// * `echo [1] General.bat`
/// * `echo 1. General (ALT).bat`
/// * `echo  2) general2.bat`
///
/// Returns `(menu_number, entry_text)` pairs in file order. Used instead of
/// guessing an index from a directory listing — the sorted listing order is
/// NOT the menu order the script actually presents.
pub fn parse_menu_entries(path: &Path) -> std::io::Result<Vec<(u32, String)>> {
    let (text, _encoding) = read_batch_text(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix('@').unwrap_or(line).trim();
        // Case-insensitive `echo` prefix (skips `@echo off` and non-echo
        // lines such as `choice` / `set`). `get(..4)` avoids slicing panics
        // on multi-byte (non-ASCII) lines.
        let Some(prefix) = line.get(..4) else {
            continue;
        };
        if !prefix.eq_ignore_ascii_case("echo") {
            continue;
        }
        let rest = line[4..].trim_start();
        // "echo off" / "echo ON" and bare `echo.` separators are not entries.
        if rest.is_empty() {
            continue;
        }
        if let Some(entry) = split_menu_entry(rest) {
            out.push(entry);
        }
    }
    Ok(out)
}

/// Split `1. text` / `[2] text` / `3) text` / `4: text` into (number, text).
fn split_menu_entry(rest: &str) -> Option<(u32, String)> {
    let rest = rest.trim_start();
    if let Some(after) = rest.strip_prefix('[') {
        let close = after.find(']')?;
        let num: u32 = after[..close].trim().parse().ok()?;
        return Some((num, after[close + 1..].trim().to_string()));
    }
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let sep = rest.as_bytes()[digits];
    if sep != b'.' && sep != b')' && sep != b':' {
        return None;
    }
    let num: u32 = rest[..digits].parse().ok()?;
    Some((num, rest[digits + 1..].trim().to_string()))
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

    #[test]
    fn utf8_fallback_never_doubles_bytes() {
        // A valid-UTF-8 file that also decodes as cp866 is picked up by the
        // cp866 branch; the UTF-8 branches must re-emit the exact bytes.
        let dir = integra_core::lists::tempfile_lite::TempDir::new("integra-bat5");
        let source = dir.path().join("general.bat");
        let dest = dir.path().join("out.bat");
        fs::write(&source, "@echo off\r\nstart \"z\" /min winws.exe\r\n").unwrap();
        build_hidden_runtime_batch(&source, &dest).unwrap();
        let (text, encoding) = read_batch_text(&dest).unwrap();
        assert_eq!(encoding, "utf-8");
        assert_eq!(encode_batch_text(&text, encoding), fs::read(&dest).unwrap());
    }

    #[test]
    fn parses_menu_entries_from_bat() {
        let dir = integra_core::lists::tempfile_lite::TempDir::new("integra-bat6");
        let menu = dir.path().join("service.bat");
        fs::write(
            &menu,
            "@echo off\r\n\
             echo General (main menu)\r\n\
             echo.\r\n\
             echo [1] general.bat\r\n\
             echo 2. General (ALT).bat\r\n\
             echo  3) general2.bat\r\n\
             echo 4: general10.bat\r\n\
             choice /c 12345 /n\r\n",
        )
        .unwrap();
        let entries = parse_menu_entries(&menu).unwrap();
        assert_eq!(
            entries,
            vec![
                (1, "general.bat".to_string()),
                (2, "General (ALT).bat".to_string()),
                (3, "general2.bat".to_string()),
                (4, "general10.bat".to_string()),
            ]
        );
    }
}
