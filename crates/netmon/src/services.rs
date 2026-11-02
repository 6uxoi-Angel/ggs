//! Well-known port labels shown in the tracker UI.

pub fn label_for_port(port: u16) -> Option<&'static str> {
    Some(match port {
        20 | 21 => "FTP",
        22 => "SSH",
        25 => "SMTP",
        53 => "DNS",
        80 => "HTTP",
        123 => "NTP",
        443 => "HTTPS",
        444 => "SNPP",
        465 => "SMTPS",
        587 => "SMTP-sub",
        993 => "IMAPS",
        995 => "POP3S",
        1935 => "RTMP",
        3478 => "STUN",
        3479 => "STUN",
        3480 => "STUN-TLS",
        3481 => "STUN-TLS",
        4380..=4384 => "Steam",
        5222 => "XMPP",
        5223 => "XMPP-SSL",
        8080 => "HTTP-alt",
        8443 => "HTTPS-alt",
        4433 => "QUIC-alt",
        27000..=27099 => "Steam",
        27900..=27999 => "Game",
        28910 => "Game",
        3074 => "Xbox/UDP",
        3702 => "WS-Discovery",
        p if (26000..=27000).contains(&p) => "Game",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(label_for_port(443), Some("HTTPS"));
        assert_eq!(label_for_port(27015), Some("Steam"));
        assert_eq!(label_for_port(12345), None);
    }
}
