//! Domain, URL and IP normalization.

use std::net::IpAddr;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum DomainError {
    #[error("enter a domain or address")]
    Empty,
    #[error("cannot parse host from input")]
    CannotParse,
    #[error("IP addresses are not allowed here")]
    IpNotAllowed,
    #[error("invalid domain name")]
    InvalidDomain,
    #[error("invalid IP or CIDR")]
    InvalidIpOrCidr,
    #[error("input/output error: {0}")]
    Io(String),
}

impl From<std::io::Error> for DomainError {
    fn from(value: std::io::Error) -> Self {
        DomainError::Io(value.to_string())
    }
}

impl DomainError {
    /// i18n key used by the frontend to render a localized message.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            DomainError::Empty => "core.enter_domain",
            DomainError::CannotParse => "core.cannot_parse_domain",
            DomainError::IpNotAllowed => "core.ip_not_allowed",
            DomainError::InvalidDomain => "core.invalid_domain",
            DomainError::InvalidIpOrCidr => "core.invalid_ip_or_cidr",
            DomainError::Io(_) => "common.error",
        }
    }
}

/// Normalize a raw user input (URL, host, optionally IP) into a canonical form.
///
/// * strips scheme, path, query, fragment, userinfo and port;
/// * lowercases and strips trailing dots;
/// * applies IDNA (`пример.рф` → `xn--e1afmkfd.xn--p1ai`);
/// * with `allow_ip` parses and canonicalizes IP literals (IPv6 compressed).
pub fn normalize_host(raw: &str, allow_ip: bool) -> Result<String, DomainError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(DomainError::Empty);
    }

    // Strip scheme.
    let without_scheme = match raw.find("://") {
        Some(idx) => &raw[idx + 3..],
        None => raw.strip_prefix("//").unwrap_or(raw),
    };

    // Strip userinfo.
    let authority = match without_scheme.rfind('@') {
        Some(idx) => &without_scheme[idx + 1..],
        None => without_scheme,
    };

    // Cut path/query/fragment.
    let authority = authority
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(authority);

    let host = if let Some(rest) = authority.strip_prefix('[') {
        // IPv6 literal: [::1]:443
        match rest.find(']') {
            Some(idx) => &rest[..idx],
            None => return Err(DomainError::CannotParse),
        }
    } else {
        // Strip :port from host:port (only one colon allowed outside brackets).
        match authority.rsplit_once(':') {
            Some((maybe_host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => maybe_host,
            _ => authority,
        }
    };

    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return Err(DomainError::CannotParse);
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if !allow_ip {
            return Err(DomainError::IpNotAllowed);
        }
        return Ok(ip.to_string());
    }
    if !allow_ip && host.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b':') {
        return Err(DomainError::IpNotAllowed);
    }

    // IDNA conversion for unicode domains.
    let ascii = idna::domain_to_ascii(&host).map_err(|_| DomainError::InvalidDomain)?;
    validate_domain_labels(&ascii)?;
    Ok(ascii)
}

fn validate_domain_labels(host: &str) -> Result<(), DomainError> {
    if host.is_empty() || host.len() > 253 {
        return Err(DomainError::InvalidDomain);
    }
    let mut label_count = 0usize;
    for label in host.split('.') {
        label_count += 1;
        if label.is_empty() || label.len() > 63 {
            return Err(DomainError::InvalidDomain);
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(DomainError::InvalidDomain);
        }
        if !label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(DomainError::InvalidDomain);
        }
    }
    if label_count < 2 {
        return Err(DomainError::InvalidDomain);
    }
    Ok(())
}

/// Validate a list entry: either a normalized domain or an IP/CIDR network.
/// Returns the canonical string form.
pub fn normalize_list_entry(raw: &str) -> Result<String, DomainError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(DomainError::Empty);
    }
    // Comment lines pass through unchanged (zapret lists support # comments).
    if raw.starts_with('#') || raw.starts_with(';') {
        return Ok(raw.to_string());
    }
    // Networks first (10.0.0.0/8). An invalid network falls through so URLs
    // with a path (https://example.com/x) still normalize into a domain.
    if raw.contains('/') {
        if let Ok(net) = raw.parse::<ipnet::IpNet>() {
            return Ok(net.to_string());
        }
    }
    if let Ok(ip) = raw.parse::<IpAddr>() {
        return Ok(ip.to_string());
    }
    normalize_host(raw, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_urls_and_hosts() {
        assert_eq!(normalize_host("https://Example.COM/path?q=1", false).unwrap(), "example.com");
        assert_eq!(normalize_host("HTTP://sub.domain.org:8080/x#y", false).unwrap(), "sub.domain.org");
        assert_eq!(normalize_host("discord.com.", false).unwrap(), "discord.com");
        assert_eq!(normalize_host("user:pass@host.example:443", false).unwrap(), "host.example");
    }

    #[test]
    fn normalizes_idna() {
        assert_eq!(normalize_host("https://пример.рф", false).unwrap(), "xn--e1afmkfd.xn--p1ai");
    }

    #[test]
    fn handles_ips() {
        assert_eq!(normalize_host("1.2.3.4", true).unwrap(), "1.2.3.4");
        assert_eq!(normalize_host("[2001:0DB8::0001]:443", true).unwrap(), "2001:db8::1");
        assert_eq!(normalize_host("1.2.3.4", false), Err(DomainError::IpNotAllowed));
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(normalize_host("", true), Err(DomainError::Empty));
        assert_eq!(normalize_host("not a domain", false), Err(DomainError::InvalidDomain));
        assert_eq!(normalize_host("-bad.com", false), Err(DomainError::InvalidDomain));
        assert_eq!(normalize_host("localhost", false), Err(DomainError::InvalidDomain));
    }

    #[test]
    fn list_entries() {
        assert_eq!(normalize_list_entry("10.0.0.0/8").unwrap(), "10.0.0.0/8");
        assert_eq!(normalize_list_entry("192.168.1.5").unwrap(), "192.168.1.5");
        assert_eq!(normalize_list_entry("HTTPS://Example.com ").unwrap(), "example.com");
        assert_eq!(normalize_list_entry("# comment").unwrap(), "# comment");
        assert!(normalize_list_entry("999.1.1.1/99").is_err());
    }

    #[test]
    fn list_entries_urls_with_paths() {
        // Regression: URLs must not be mistaken for CIDR networks.
        assert_eq!(normalize_list_entry("https://example.com/path?q=1").unwrap(), "example.com");
        assert_eq!(normalize_list_entry("http://sub.example.org/a/b/").unwrap(), "sub.example.org");
        assert_eq!(normalize_list_entry("example.com/path").unwrap(), "example.com");
    }
}
