use crate::i18n::t;
use std::fs;
use std::process::Command;

#[derive(Clone)]
pub struct TelemetryItem {
    pub name: String,
    pub description: String,
    pub kind: String, // "service" o "domain"
    pub target: String,
    pub active: bool,
}

/// Scansiona demoni di telemetria o crash reporter noti attivi sul sistema
pub fn scan_telemetry_daemons() -> Vec<TelemetryItem> {
    let mut items = Vec::new();

    // Lista di demoni noti di telemetria / crash reporting su Linux
    let known_daemons = vec![
        ("whoopsie", t("Ubuntu Crash Report Daemon (invia crash report automaticamente)")),
        ("apport", t("Apport Crash Collector (raccoglie e invia segnalazioni di errori)")),
        ("telemetry", t("Servizio generico di telemetria di terze parti")),
        ("collector", t("Demone di raccolta dati diagnostici")),
    ];

    for (svc, desc) in known_daemons {
        let check = Command::new("systemctl")
            .args(["is-active", svc])
            .output();
        
        let is_running = match check {
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim() == "active",
            Err(_) => false,
        };

        if is_running {
            items.push(TelemetryItem {
                name: svc.to_string(),
                description: desc.to_string(),
                kind: "service".to_string(),
                target: svc.to_string(),
                active: true,
            });
        }
    }

    items
}

/// Blocca un dominio di telemetria inserendolo in /etc/hosts puntando a 127.0.0.1
pub fn block_domain(domain: &str) -> bool {
    let domain = domain.trim();
    if domain.is_empty()
        || !domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return false;
    }
    let hosts_path = "/etc/hosts";
    if let Ok(content) = fs::read_to_string(hosts_path) {
        // Confronto per campo esatto: "example.com" non deve bloccare "notexample.com".
        let already = content.lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next().is_some()
                && fields.any(|host| host == domain)
        });
        if already {
            return true; // Già bloccato
        }
    }
    let entry = format!("\n127.0.0.1 {domain}\n::1 {domain}\n");
    // `run_shell(..., true)` elevazione già con pkexec: `sudo` qui è ridondante
    // e fallisce sulle installazioni minime che non lo includono.
    let (ok, _) = crate::util::run_shell(
        &format!("printf '%s' {} >> {}", crate::util::shq(&entry), hosts_path),
        true,
    );
    ok
}

/// Disattiva un demone di telemetria tramite systemctl
pub fn disable_service(service_name: &str) -> bool {
    let name = service_name.trim();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'@'))
    {
        return false;
    }
    let q = crate::util::shq(name);
    let script = format!("systemctl stop {q} && systemctl disable {q}");
    let (ok, _) = crate::util::run_shell(&script, true);
    ok
}
