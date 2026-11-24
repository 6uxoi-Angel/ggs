//! Быстрая диагностика «что не так с интернетом».
//! Лёгкий аналог wlprobe: DNS, TCP до зарубежных и «белых» хостов,
//! проверка на заморозку по объёму (~16 КБ).

use serde::Serialize;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

#[derive(Serialize)]
pub struct ProbeResult {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Serialize)]
pub struct NetcheckReport {
    pub verdict: String,
    pub tip: String,
    pub probes: Vec<ProbeResult>,
}

fn tcp_probe(name: &str, host: &str, port: u16, timeout: Duration) -> ProbeResult {
    let start = Instant::now();
    let addrs = match (host, port).to_socket_addrs() {
        Ok(a) => a,
        Err(e) => {
            return ProbeResult {
                name: name.into(),
                ok: false,
                detail: format!("DNS: {e}"),
            }
        }
    };
    let Some(addr) = addrs.into_iter().next() else {
        return ProbeResult {
            name: name.into(),
            ok: false,
            detail: "не удалось разрешить имя".into(),
        };
    };
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(_) => {
            let ms = start.elapsed().as_millis();
            ProbeResult {
                name: name.into(),
                ok: true,
                detail: format!("соединение за {ms} мс ({addr})"),
            }
        }
        Err(e) => ProbeResult {
            name: name.into(),
            ok: false,
            detail: e.to_string(),
        },
    }
}

/// Сырой HTTP/1.0-запрос на neverssl.com:80, подсчёт полученных байт.
/// Если поток «замерзает» в районе ~16 КБ — это сигнатура ТСПУ (FREEZE).
fn freeze_probe() -> ProbeResult {
    let name = "заморозка по объёму (~16 КБ)";
    let Ok(mut addrs) = ("neverssl.com", 80).to_socket_addrs() else {
        return ProbeResult {
            name: name.into(),
            ok: false,
            detail: "DNS не резолвится".into(),
        };
    };
    let Some(addr) = addrs.next() else {
        return ProbeResult {
            name: name.into(),
            ok: false,
            detail: "DNS: пусто".into(),
        };
    };
    let Ok(mut sock) = TcpStream::connect_timeout(&addr, Duration::from_secs(6)) else {
        return ProbeResult {
            name: name.into(),
            ok: false,
            detail: "TCP-соединение не установлено".into(),
        };
    };
    let _ = sock.set_read_timeout(Some(Duration::from_secs(6)));
    let _ = sock.write_all(
        b"GET / HTTP/1.0\r\nHost: neverssl.com\r\nUser-Agent: integra-netcheck/3\r\n\r\n",
    );

    let start = Instant::now();
    let mut buf = [0u8; 8192];
    let mut total: u64 = 0;
    loop {
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                total += n as u64;
                if total > 512_000 {
                    break;
                }
            }
            Err(_) => break, // таймаут → поток замер
        }
        if start.elapsed() > Duration::from_secs(12) {
            break;
        }
    }
    let secs = start.elapsed().as_secs_f64();

    if total < 30_000 && secs >= 5.0 {
        ProbeResult {
            name: name.into(),
            ok: false,
            detail: format!(
                "получено ~{total} байт, дальше поток замер — похоже на режим «FREEZE»"
            ),
        }
    } else {
        ProbeResult {
            name: name.into(),
            ok: true,
            detail: format!("получено {total} байт за {secs:.1} с — лимита объёма не видно"),
        }
    }
}

pub fn run() -> NetcheckReport {
    let cf = tcp_probe(
        "TCP → Cloudflare (1.1.1.1:443)",
        "1.1.1.1",
        443,
        Duration::from_secs(5),
    );
    let google = tcp_probe(
        "TCP → Google (google.com:443)",
        "google.com",
        443,
        Duration::from_secs(5),
    );
    let gosuslugi = tcp_probe(
        "TCP → Госуслуги (реестр СЗС)",
        "gosuslugi.ru",
        443,
        Duration::from_secs(5),
    );

    let dns_ok = ("google.com", 443)
        .to_socket_addrs()
        .map(|mut i| i.next().is_some())
        .unwrap_or(false);

    let freeze = freeze_probe();

    let probes = vec![
        cf,
        google,
        gosuslugi,
        ProbeResult {
            name: "DNS (резолв google.com)".into(),
            ok: dns_ok,
            detail: if dns_ok {
                "обычный резолв работает".into()
            } else {
                "резолв не работает или сильно замедлен".into()
            },
        },
        freeze,
    ];

    let foreign_ok = probes[0].ok || probes[1].ok;
    let ru_ok = probes[2].ok;
    let freeze_bad = !probes[4].ok;
    let dns_bad = !probes[3].ok;
    // Windows DNS failure codes (WSAHOST_NOT_FOUND = 11001) seen in the TCP
    // probes mean the DNS resolver is broken even if raw-IP connects work.
    let foreign_dns_err = probes[0].detail.contains("11001") || probes[1].detail.contains("11001");

    let (verdict, tip) = if !foreign_ok && !ru_ok {
        (
            "NO_INTERNET",
            "Сеть недоступна: проверьте Wi-Fi/кабель, роутер и адаптер.",
        )
    } else if !foreign_ok && ru_ok {
        ("WHITELIST_MODE", "Зарубежные адреса не открываются, а «белые» российские — да. Похоже на режим белого списка (фильтрация по IP/ASN): нужен узел на «белом» IP внутри РФ.")
    } else if dns_bad || foreign_dns_err {
        (
            "DNS_FAILURE",
            "Сбой DNS: резолв имён не работает, хотя TCP по IP проходит. Перейдите на DoH/DoT или пропишите надёжный DNS.",
        )
    } else if freeze_bad {
        ("FREEZE_SUSPECTED", "Соединения устанавливаются, но поток замирает после ~16 КБ — сигнатура «заморозки». Лечится цепочкой через РФ-мост/ротацией соединений.")
    } else {
        (
            "CLEAN",
            "Ограничений не обнаружено: DNS работает, соединения открываются, лимита объёма нет.",
        )
    };

    NetcheckReport {
        verdict: verdict.into(),
        tip: tip.into(),
        probes,
    }
}
