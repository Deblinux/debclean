use crate::i18n::t;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct SnapApp {
    pub name: String,
    pub version: String,
    pub revision: String,
    pub publisher: String,
}

/// Restituisce l'elenco dei pacchetti Snap installati sul sistema.
pub fn get_installed_snaps() -> Result<Vec<SnapApp>, &'static str> {
    let output = match Command::new("snap").arg("list").output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(t("Snap non è installato sul sistema."));
        }
        Err(_) => {
            return Err(t("Impossibile eseguire il comando snap."));
        }
    };

    if !output.status.success() {
        return Err(t("Il servizio snapd non è attivo sul sistema."));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut apps = Vec::new();

    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            apps.push(SnapApp {
                name: parts[0].to_string(),
                version: parts[1].to_string(),
                revision: parts[2].to_string(),
                publisher: parts[3].to_string(),
            });
        }
    }

    Ok(apps)
}
/// Disinstalla un pacchetto Snap richiedendo i privilegi di amministratore.
pub fn remove_snap(package_name: &str) -> Result<(), String> {
    let output = Command::new("pkexec")
        .arg("snap")
        .arg("remove")
        .arg(package_name)
        .output()
        .map_err(|e| format!("Execution error: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Removal error: {}", stderr.trim()))
    }
}

/// Aggiorna un pacchetto Snap specifico o tutti i pacchetti se non viene specificato il nome.
pub fn refresh_snap(package_name: Option<&str>) -> Result<(), String> {
    let mut cmd = Command::new("pkexec");
    cmd.arg("snap").arg("refresh");
    if let Some(pkg) = package_name {
        cmd.arg(pkg);
    }

    let output = cmd.output()
        .map_err(|e| format!("Execution error: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Update error: {}", stderr.trim()))
    }
}
