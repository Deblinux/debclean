use crate::i18n::{t, tf};
use crate::util::run_shell;
use sysinfo::Disks;

#[derive(Clone, Copy, PartialEq)]
pub enum Status {
    Ok,
    Warn,
    Problem,
}

#[derive(Clone)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    /// Comando (eseguito come root) che corregge il problema, se esiste.
    pub fix: Option<&'static str>,
}

fn mk(name: &'static str, status: Status, detail: impl Into<String>, fix: Option<&'static str>) -> Check {
    let mut d: String = detail.into();
    if d.chars().count() > 400 {
        d = d.chars().take(400).collect::<String>() + "…";
    }
    Check { name, status, detail: d, fix }
}

fn cmd_exists(c: &str) -> bool {
    run_shell(&format!("command -v {c} >/dev/null 2>&1"), false).0
}

pub fn run_checks() -> Vec<Check> {
    let mut v = Vec::new();

    // 1. Stato del database dpkg (installazioni interrotte)
    if cmd_exists("dpkg") {
        let (_, o) = run_shell("dpkg --audit", false);
        if o.trim().is_empty() {
            v.push(mk("Database dpkg", Status::Ok, "Nessun pacchetto in stato incoerente", None));
        } else {
            v.push(mk("Database dpkg", Status::Problem, o.trim(), Some("dpkg --configure -a")));
        }
    }

    // 2. Dipendenze rotte
    if cmd_exists("apt-get") {
        // simulazione: non richiede privilegi di root
        let (ok, o) = run_shell("apt-get -s -f install 2>&1", false);
        if ok && o.lines().any(|l| l.starts_with("0 upgraded, 0 newly installed, 0 to remove")) {
            v.push(mk("Dipendenze APT", Status::Ok, "Nessuna dipendenza rotta", None));
        } else if o.contains("Permission denied") {
            v.push(mk("Dipendenze APT", Status::Warn, "Impossibile verificare senza privilegi: usa Correggi dipendenze rotte", None));
        } else {
            v.push(mk("Dipendenze APT", Status::Problem, o.trim(), Some("apt-get -y -f install")));
        }
    }

    // 3. Pacchetti rimossi con configurazioni residue
    let (_, o) = run_shell("dpkg -l 2>/dev/null | awk '/^rc/{print $2}'", false);
    let n = o.lines().filter(|l| !l.trim().is_empty()).count();
    if n == 0 {
        v.push(mk("Configurazioni residue", Status::Ok, "Nessuna configurazione orfana", None));
    } else {
        v.push(mk(
            "Configurazioni residue",
            Status::Warn,
            tf("{} pacchetti rimossi hanno ancora file di configurazione", &[&n]),
            Some("dpkg -l | awk '/^rc/{print $2}' | xargs -r dpkg --purge"),
        ));
    }

    // 4. Servizi systemd falliti
    if cmd_exists("systemctl") {
        let (_, o) = run_shell("systemctl --failed --no-legend --plain 2>/dev/null", false);
        if o.trim().is_empty() {
            v.push(mk("Servizi systemd", Status::Ok, "Nessun servizio in errore", None));
        } else {
            v.push(mk(
                "Servizi systemd",
                Status::Problem,
                o.trim(),
                Some("systemctl --failed --no-legend --plain | awk '{print $1}' | xargs -r systemctl restart; systemctl reset-failed"),
            ));
        }
    }

    // 5. Spazio su disco della radice
    let disks = Disks::new_with_refreshed_list();
    if let Some(d) = disks.iter().find(|d| d.mount_point().to_str() == Some("/")) {
        let total = d.total_space().max(1) as f64;
        let used = (d.total_space() - d.available_space()) as f64 / total * 100.0;
        let st = if used >= 90.0 {
            Status::Problem
        } else if used >= 80.0 {
            Status::Warn
        } else {
            Status::Ok
        };
        let pct = format!("{used:.0}%");
        let hint = if st == Status::Ok { "" } else { t(" — usa la scheda Pulizia per liberare spazio") };
        v.push(mk("Spazio su disco (/)", st, tf("Occupato {}{}", &[&pct, &hint]), None));
    }

    // 6. Sincronizzazione orologio
    if cmd_exists("timedatectl") {
        let (_, o) = run_shell("timedatectl show -p NTPSynchronized --value 2>/dev/null", false);
        match o.trim() {
            "yes" => v.push(mk("Orologio di sistema", Status::Ok, "Sincronizzato via NTP", None)),
            "no" => v.push(mk(
                "Orologio di sistema",
                Status::Warn,
                "Non sincronizzato",
                Some("timedatectl set-ntp true"),
            )),
            _ => {}
        }
    }

    // 7. Pacchetti bloccati (hold)
    if cmd_exists("apt-mark") {
        let (_, o) = run_shell("apt-mark showhold", false);
        if !o.trim().is_empty() {
            v.push(mk(
                "Pacchetti bloccati",
                Status::Warn,
                tf("In hold: {}", &[&o.split_whitespace().collect::<Vec<_>>().join(", ")]),
                Some("apt-mark showhold | awk '{print $1 \" install\"}' | dpkg --set-selections"),
            ));
        }
    }

    // 8. Riavvio necessario
    if std::path::Path::new("/var/run/reboot-required").exists() {
        v.push(mk("Riavvio", Status::Warn, "Un aggiornamento richiede il riavvio del sistema", None));
    }

    v
}
