use crate::i18n::t;
use crate::{ui, util, App};
use eframe::egui;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct InsecureFile {
    pub path: PathBuf,
    pub current_mode: u32,
    pub recommended_mode: u32,
    pub reason: &'static str,
}

pub fn audit_permissions() -> Vec<InsecureFile> {
    let mut list = Vec::new();
    // `util::home()` risolve l'utente reale anche quando l'app gira con pkexec.
    let home = PathBuf::from(util::home());
    if home.as_os_str().is_empty() {
        return list;
    }

    let ssh_dir = home.join(".ssh");
    // Un symlink ha sempre mode 0777: correggerlo significherebbe cambiare i
    // permessi del bersaglio, non del link. Non toccare link simbolici.
    if ssh_dir
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return list;
    }
    if ssh_dir.exists() {
        if let Ok(md) = ssh_dir.symlink_metadata() {
            let mode = md.permissions().mode() & 0o777;
            if mode != 0o700 {
                list.push(InsecureFile {
                    path: ssh_dir.clone(),
                    current_mode: mode,
                    recommended_mode: 0o700,
                    reason: t("Cartella .ssh deve essere accessibile solo dall'utente (700)"),
                });
            }
        }
        if let Ok(rd) = fs::read_dir(&ssh_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if let Ok(md) = p.symlink_metadata() {
                    if md.file_type().is_symlink() {
                        continue;
                    }
                    let mode = md.permissions().mode() & 0o777;
                    let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    if file_name.ends_with(".pub") {
                        if mode > 0o644 {
                            list.push(InsecureFile {
                                path: p,
                                current_mode: mode,
                                recommended_mode: 0o644,
                                reason: t("Chiave pubblica non deve essere scrivibile da altri (644)"),
                            });
                        }
                    } else if (file_name.starts_with("id_") || file_name == "authorized_keys" || file_name == "config") && mode != 0o600 {
                        list.push(InsecureFile {
                            path: p,
                            current_mode: mode,
                            recommended_mode: 0o600,
                            reason: t("Chiave privata o configurazione SSH deve essere 600"),
                        });
                    }
                }
            }
        }
    }

    // Scansione rapida file 777/666 nella radice della home
    if let Ok(rd) = fs::read_dir(&home) {
        for entry in rd.flatten() {
            let p = entry.path();
            if let Ok(md) = p.symlink_metadata() {
                if md.file_type().is_symlink() {
                    continue;
                }
                let mode = md.permissions().mode() & 0o777;
                if mode == 0o777 || mode == 0o666 {
                    list.push(InsecureFile {
                        path: p,
                        current_mode: mode,
                        recommended_mode: if md.is_dir() { 0o755 } else { 0o644 },
                        reason: t("File con permessi globali di scrittura a chiunque (777/666)"),
                    });
                }
            }
        }
    }

    list
}

pub fn fix_permission(item: &InsecureFile) -> Result<(), std::io::Error> {
    // `set_permissions` segue i symlink: rifiutarli esplicitamente evita di
    // cambiare i permessi del bersaglio (per esempio una chiave privata).
    if fs::symlink_metadata(&item.path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Symlink: correzione non applicata",
        ));
    }
    let perms = fs::Permissions::from_mode(item.recommended_mode);
    fs::set_permissions(&item.path, perms)
}

impl App {
    // ---------------- AUDIT PERMESSI ----------------
    pub(crate) fn ui_audit_perms(&mut self, ui: &mut egui::Ui) {
        ui::card(ui, |ui| {
            ui::section(ui, "🛡", "Audit permessi e file esposti", "");
            ui.label(t("Analizza file e cartelle sensibili (come ~/.ssh) per verificare che non siano leggibili o modificabili da altri utenti del sistema."));
        });
        ui.add_space(3.0);

        let issues = audit_permissions();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{}: {}", t("Problemi rilevati"), issues.len()));
            if !issues.is_empty() && ui::primary_button(ui, t("Correggi tutti i permessi"), self.action_enabled()).clicked() {
                let mut fixed = 0;
                let mut failed = 0;
                for i in &issues {
                    match fix_permission(i) {
                        Ok(()) => fixed += 1,
                        Err(e) => {
                            failed += 1;
                            self.log.push_str(&format!(
                                "✘ {} ({}): {e}\n",
                                t("Impossibile correggere i permessi"),
                                i.path.display()
                            ));
                        }
                    }
                }
                self.log.push_str(&format!("✔ Corretti i permessi di {fixed} elementi.\n"));
                if failed > 0 {
                    self.log.push_str(&format!("✘ {failed} elementi non corretti.\n"));
                }
            }
        });
        ui.add_space(3.0);

        if issues.is_empty() {
            ui::card(ui, |ui| {
                ui.colored_label(ui::semantic_text(ui::GOOD), t("✔ Ottimo! Nessun file sensibile o chiave con permessi aperti rilevati."));
            });
        } else {
            for item in issues {
                ui::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(format!("{:04o} ➔ {:04o}", item.current_mode, item.recommended_mode)).strong().color(ui::semantic_text(ui::WARN)));
                        ui.label(item.path.display().to_string());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(t("Correggi")).clicked() {
                                match fix_permission(&item) {
                                    Ok(()) => self.log.push_str(&format!(
                                        "✔ Permessi corretti: {}\n",
                                        item.path.display()
                                    )),
                                    Err(e) => self.log.push_str(&format!(
                                        "✘ {} ({}): {e}\n",
                                        t("Impossibile correggere i permessi"),
                                        item.path.display()
                                    )),
                                }
                            }
                        });
                    });
                    ui.label(egui::RichText::new(t(item.reason)).size(14.0).color(ui::text_dim()));
                });
                ui.add_space(2.0);
            }
        }
    }

    // ---------------- GESTIONE CHIAVI SSH ----------------
    pub(crate) fn ui_ssh_keys(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card(ui, |ui| {
            ui::section(ui, "🔑", "Gestione chiavi SSH", "");
            ui.label(t("Visualizza le tue chiavi pubbliche da copiare su server remoti o GitHub, oppure crea una nuova coppia ed25519."));
        });
        ui.add_space(3.0);

        let home = util::home();
        let ssh_dir = Path::new(&home).join(".ssh");
        let mut keys = Vec::new();
        if let Ok(rd) = fs::read_dir(&ssh_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "pub").unwrap_or(false) {
                    if let Ok(content) = fs::read_to_string(&p) {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.to_string_lossy().into_owned());
                        keys.push((name, content.trim().to_string()));
                    }
                }
            }
        }

        ui.horizontal_wrapped(|ui| {
            if ui::primary_button(ui, t("➕ Genera nuova chiave ED25519"), self.action_enabled()).clicked() {
                let user = util::username();
                let host = sysinfo::System::host_name().unwrap_or_else(|| "linux".into());
                let comment = format!("{user}@{host}");
                let script = format!(
                    "mkdir -p -m 700 \"$HOME/.ssh\" && ssh-keygen -t ed25519 -C {} -f \"$HOME/.ssh/id_ed25519\" -N ''",
                    util::shq(&comment)
                );
                self.spawn(ctx, crate::cmd_job("ssh", script, false));
            }
            let (_, agent_out) = util::run_shell("ssh-add -l 2>/dev/null", false);
            let agent_active = !agent_out.contains("Could not open a connection") && !agent_out.trim().is_empty();
            let agent_label = if agent_active { t("SSH-Agent attivo") } else { t("SSH-Agent non attivo o vuoto") };
            ui::led(ui, Some(agent_active), agent_label);
        });
        ui.add_space(3.0);

        for (name, pub_content) in keys {
            ui::card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(&name).strong().size(18.0).color(ui::text_primary()));
                    if ui.button(t("📋 Copia chiave pubblica")).clicked() {
                        ctx.copy_text(pub_content.clone());
                        self.log.push_str(&format!("✔ Chiave {name} copiata negli appunti!\n"));
                    }
                });
                ui.add(egui::TextEdit::multiline(&mut pub_content.as_str()).desired_rows(2).desired_width(ui::fit_width(ui, 420.0, 160.0)));
            });
            ui.add_space(3.0);
        }
        self.show_out(ui, "ssh", 100.0);
    }

    // ---------------- SCANNER ROOTKIT / MALWARE ----------------
    pub(crate) fn ui_rootkit(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card(ui, |ui| {
            ui::section(ui, "🔍", "Scanner Rootkit e Malware", "");
            ui.label(t("Esegue una scansione del sistema alla ricerca di rootkit, trojan e file modificati illegalmente usando chkrootkit o rkhunter."));
        });
        ui.add_space(3.0);

        let have_chk = util::run_shell("command -v chkrootkit >/dev/null 2>&1", false).0;
        let have_rkh = util::run_shell("command -v rkhunter >/dev/null 2>&1", false).0;

        if !have_chk && !have_rkh {
            ui::card(ui, |ui| {
                ui.colored_label(ui::semantic_text(ui::WARN), t("Né chkrootkit né rkhunter sono installati nel sistema."));
                if ui::primary_button(ui, t("Installa chkrootkit e rkhunter"), self.action_enabled()).clicked() {
                    self.spawn(ctx, crate::cmd_job("rootkit", "apt-get update && apt-get install -y chkrootkit rkhunter".into(), true));
                }
            });
            self.show_out(ui, "rootkit", 150.0);
            return;
        }

        ui.horizontal_wrapped(|ui| {
            if have_chk && ui.button(t("🔍 Scansione rapida (chkrootkit)")).clicked() {
                self.spawn(ctx, crate::cmd_job("rootkit", "chkrootkit -q 2>&1 | grep -iE 'INFECTED|Vulnerable|Warning' || echo 'Nessuna minaccia evidente rilevata.'".into(), true));
            }
            if have_rkh && ui.button(t("🔬 Scansione completa (rkhunter)")).clicked() {
                self.spawn(ctx, crate::cmd_job("rootkit", "rkhunter --check --sk --rwo 2>&1".into(), true));
            }
            if have_rkh && ui.button(t("Aggiorna firme rkhunter")).clicked() {
                self.spawn(ctx, crate::cmd_job("rootkit", "rkhunter --update && rkhunter --propupd".into(), true));
            }
            if self.busy {
                ui.spinner();
            }
        });
        ui.add_space(3.0);
        self.show_out_follow(ui, "rootkit", 320.0);
    }
}
