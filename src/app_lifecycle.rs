use crate::i18n::{t, tf, tr};
use crate::{ui, App};
use eframe::egui;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ResidualArtifact {
    pub path: PathBuf,
    pub artifact_type: String, // "Config (/etc)", "Regola Udev", "Servizio / Cron"
    pub selected: bool,
}

/// Nomi (minuscoli) che non devono mai essere proposti per la rimozione:
/// file e cartelle fondamentali per il funzionamento del sistema.
const PROTECTED_NAMES: &[&str] = &[
    "passwd", "shadow", "group", "gshadow", "fstab", "hosts", "hostname", "resolv.conf",
    "sudoers", "sudoers.d", "ssh", "systemd", "apt", "default", "init.d", "x11", "network",
    "dpkg", "pam.d", "pam.conf", "security", "ld.so.conf", "ld.so.conf.d", "udev", "dbus-1",
    "polkit-1", "cron.d", "cron.daily", "grub.d", "modprobe.d", "sysctl.d", "sysctl.conf",
    "os-release", "machine-id", "kernel", "initramfs-tools", "alternatives", "ssl",
    "ca-certificates", "ca-certificates.conf", "networkmanager", "pulse", "xdg", "profile",
    "profile.d", "bash.bashrc", "environment", "fonts", "mime.types", "services", "protocols",
    "login.defs", "nsswitch.conf", "locale.gen", "localtime", "timezone", "issue", "motd",
];

/// Lunghezza minima della ricerca: con 1-2 caratteri quasi tutto in /etc corrisponderebbe.
const MIN_QUERY_LEN: usize = 3;

fn is_protected(name_lower: &str) -> bool {
    PROTECTED_NAMES.contains(&name_lower)
}

/// Scansiona il file system alla ricerca di residui associati a una determinata app o pacchetto.
/// Nessun elemento viene preselezionato: è l'utente a scegliere cosa eliminare.
pub fn scan_surgical_residuals(query: &str) -> Vec<ResidualArtifact> {
    let mut residuals = Vec::new();
    let q = query.trim().to_lowercase();
    if q.chars().count() < MIN_QUERY_LEN {
        return residuals;
    }

    // (cartella, etichetta del tipo di residuo)
    let targets: [(&str, &str); 7] = [
        ("/etc", t("Config (/etc)")),
        ("/lib/udev/rules.d", t("Regola Udev")),
        ("/etc/udev/rules.d", t("Regola Udev")),
        ("/etc/dbus-1/system.d", t("Servizio / Cron")),
        ("/etc/polkit-1/actions", t("Servizio / Cron")),
        ("/etc/cron.d", t("Servizio / Cron")),
        ("/usr/share/polkit-1/actions", t("Servizio / Cron")),
    ];

    for (dir, label) in targets {
        if let Ok(entries) = fs::read_dir(Path::new(dir)) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.contains(&q) && !is_protected(&name) {
                    residuals.push(ResidualArtifact {
                        path: entry.path(),
                        artifact_type: label.to_string(),
                        selected: false,
                    });
                }
            }
        }
    }

    residuals
}

impl App {
    pub(crate) fn ui_app_lifecycle(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui::title(ui, t("Disinstallazione Chirurgica & Sandbox"));
        self.snapshot_toggle(ui);

        ui::card(ui, |ui| {
            ui.label(
                egui::RichText::new(t(
                    "Monitora e pulisce a fondo l'intero ciclo di vita di un'applicazione. Rimuove non solo il pacchetto ma anche tutti i frammenti invisibili sparsi nel sistema (regole udev, file in /etc, registrazioni D-Bus e policy Polkit).",
                ))
                .size(16.0),
            );
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Nome pacchetto o app da bonificare:")).strong());
                ui.text_edit_singleline(&mut self.app_lifecycle_query);
                if ui::primary_button(ui, t("Cerca Residui Invisibili"), !self.busy).clicked() {
                    let query = self.app_lifecycle_query.clone();
                    self.app_lifecycle_residuals = scan_surgical_residuals(&query);
                }
            });
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.label(
                egui::RichText::new(t("Elementi e residui invisibili rilevati:"))
                    .strong()
                    .size(16.0)
                    .color(ui::accent_text()),
            );
            ui.add_space(3.0);

            if self.app_lifecycle_residuals.is_empty() {
                ui.label(t("Nessun residuo trovato o scansione non ancora avviata. Inserisci un nome e cerca."));
            } else {
                egui::ScrollArea::vertical().max_height(225.0).show(ui, |ui| {
                    for item in &mut self.app_lifecycle_residuals {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut item.selected, "");
                            ui.label(egui::RichText::new(tr(&item.artifact_type)).strong().color(ui::semantic_text(ui::WARN)));
                            ui.label(item.path.to_string_lossy().as_ref());
                        });
                        ui.add_space(2.0);
                    }
                });

                ui.add_space(4.0);
                let n_sel = self.app_lifecycle_residuals.iter().filter(|i| i.selected).count();
                if ui
                    .add_enabled(
                        self.action_enabled() && n_sel > 0,
                        egui::Button::new(
                            egui::RichText::new(t("Esegui Bonifica Chirurgica Selezionati")).color(ui::semantic_text(ui::BAD)),
                        ),
                    )
                    .clicked()
                {
                    let paths_to_remove: Vec<String> = self
                        .app_lifecycle_residuals
                        .iter()
                        .filter(|i| i.selected)
                        .map(|i| i.path.to_string_lossy().to_string())
                        .collect();

                    // Percorsi quotati per la shell; eseguito come amministratore (pkexec)
                    let args = paths_to_remove
                        .iter()
                        .map(|p| crate::util::shq(p))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let script = format!("rm -rf -- {args}");
                    let job = crate::cmd_job("app_lifecycle", script, true);
                    let msg = format!(
                        "{}\n{}",
                        tf("Eliminare definitivamente {} elementi di sistema?", &[&paths_to_remove.len()]),
                        paths_to_remove.join("\n")
                    );
                    self.confirm_snapshot(msg, job);
                }
                ui.add_space(3.0);
                ui.label(t("Dopo l'eliminazione premi di nuovo «Cerca Residui Invisibili» per aggiornare l'elenco."));
            }
        });
    }
}
