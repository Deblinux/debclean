use crate::config_parsers;
use crate::i18n::{self, t, tf, tr};
use crate::{restore_delete_job, restore_scan_job, ui, util, App, Msg};
use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Config {
    pub quick_clean: bool,
    pub lang: usize,
    pub dark_mode: bool,
    /// Controllo aggiornamenti GitHub all'avvio della GUI (attivo di default).
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { 
            quick_clean: false, 
            lang: i18n::detect(),
            dark_mode: true,
            check_updates: true,
        }
    }
}

impl Config {

fn path() -> PathBuf {
        PathBuf::from(util::home()).join(".config/debutility/config")
    }

    fn legacy_path() -> PathBuf {
        PathBuf::from(util::home()).join(".config/debclean/config")
    }

    fn read_text() -> String {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::read_to_string(Self::legacy_path()).unwrap_or_default()
            }
            Err(_) => String::new(),
        }
    }

    pub fn load() -> Self {
        let text = Self::read_text();
        let lang = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("lang="))
            .and_then(i18n::index_of)
            .unwrap_or_else(i18n::detect);
        let quick_clean = text.lines().any(|l| l.trim() == "quick_clean=1");
        let dark_mode = if text.contains("dark_mode=") {
            text.lines().any(|l| l.trim() == "dark_mode=1")
        } else {
            true
        };
        // Attivo di default: disattivato solo se l'utente ha salvato `check_updates=0`.
        let check_updates = if text.contains("check_updates=") {
            text.lines().any(|l| l.trim() == "check_updates=1")
        } else {
            true
        };
        Config { quick_clean, lang, dark_mode, check_updates }
    }

    pub fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, format!(
                "quick_clean={}\nlang={}\ndark_mode={}\ncheck_updates={}\n",
                if self.quick_clean { 1 } else { 0 },
                i18n::code_of(self.lang),
                if self.dark_mode { 1 } else { 0 },
                if self.check_updates { 1 } else { 0 }
            ));

        if let (Ok(uid), Ok(gid)) = (std::env::var("PKEXEC_UID"), std::env::var("PKEXEC_GID")) {
            let _ = std::process::Command::new("chown")
                .arg(format!("{}:{}", uid, gid))
                .arg(&p)
                .status();
        }
    }

    pub fn reset() {
        let _ = std::fs::remove_file(Self::path());
        let _ = std::fs::remove_file(Self::legacy_path());
    }
}

/// Valida un file sorgente APT risolvendo symlink e `..`.
///
/// Sono ammessi solo `/etc/apt/sources.list` e file regolari direttamente in
/// `/etc/apt/sources.list.d/`. Impedisce che percorsi come
/// `/etc/apt/sources.list.d/../../etc/shadow` vengano modificati come root.
fn canonical_apt_source(path: &str) -> Option<PathBuf> {
    let canon = std::fs::canonicalize(path).ok()?;
    if !canon.is_file() {
        return None;
    }
    if canon == Path::new("/etc/apt/sources.list") {
        return Some(canon);
    }
    let dir = std::fs::canonicalize("/etc/apt/sources.list.d").ok()?;
    if canon.parent() == Some(dir.as_path()) {
        Some(canon)
    } else {
        None
    }
}

impl App {
    // ----- Sicurezza e firewall
pub(crate) fn ui_firewall(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Sicurezza e firewall"));
        ui::sub_tabs(
            ui,
            &mut self.sub,
            &["Firewall UFW", "Audit permessi", "Chiavi SSH", "Scanner Rootkit"],
        );

        match self.sub {
            0 => {
                ui::hint(ui, t("UFW blocca i tentativi di accesso non autorizzati dalla rete."));
                let pre = "command -v ufw >/dev/null 2>&1 || { echo 'UFW non installato: usa il pulsante Installa UFW'; exit 1; }; ";
                ui.horizontal_wrapped(|ui| {
                    self.run_btn(ui, ctx, t("Stato"), "ufw", &format!("{pre}ufw status verbose"), true, None);
                    self.run_btn(ui, ctx, t("🛡 Attiva firewall"), "ufw", &format!("{pre}ufw --force enable"), true, None);
                    self.run_btn(ui, ctx, t("Disattiva firewall"), "ufw", &format!("{pre}ufw disable"), true,
                        Some(t("Disattivare il firewall? Il PC sarà più esposto sulla rete.")));
                    self.run_btn(ui, ctx, t("Installa UFW"), "ufw", "apt-get -y install ufw", true, None);
                });
                self.show_out(ui, "ufw", 300.0);
            }
            1 => {
                self.ui_audit_perms(ui);
            }
            2 => {
                self.ui_ssh_keys(ui, ctx);
            }
            3 => {
                self.ui_rootkit(ui, ctx);
            }
            _ => {}
        }
    }

    // ----- Backup
    pub(crate) fn ui_backup(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Gestione backup");
        ui::sub_tabs(ui, &mut self.sub, &["File e cartelle", "Pacchetti di sistema", "Punti di ripristino"]);
        let user = util::shq(&util::username());
        let dst = util::shq(self.backup_dst.trim());
        match self.sub {
            0 => {
                ui::hint(
                ui,
                "Copia le tue cartelle importanti (documenti, foto...) in un'altra posizione, meglio se su un disco esterno. Indica la sorgente e la destinazione e premi «Avvia backup».",
            );
                ui.horizontal(|ui| {
                    ui.label(t("Sorgente:  "));
                    ui.add(egui::TextEdit::singleline(&mut self.backup_src).desired_width(ui::fit_width(ui, 420.0, 160.0)));
                });
                ui.horizontal(|ui| {
                    ui.label(t("Destinazione:"));
                    ui.add(egui::TextEdit::singleline(&mut self.backup_dst).desired_width(ui::fit_width(ui, 420.0, 160.0)));
                });
                let src = util::shq(self.backup_src.trim().trim_end_matches('/'));
                let dst = util::shq(self.backup_dst.trim());
                let script = format!(
                    "mkdir -p {dst} && if command -v rsync >/dev/null 2>&1; then rsync -a {src} {dst}/; else cp -a {src} {dst}/; fi && echo 'Backup completato'"
                );
                let ok = !self.backup_src.trim().is_empty() && !self.backup_dst.trim().is_empty();
                ui.add_enabled_ui(ok, |ui| {
                    self.run_btn(ui, ctx, t("💾 Avvia backup"), "backup", &script, false, None);
                });
            }
            1 => {
                ui::hint(
                ui,
                "Salva l'elenco dei programmi installati e dei repository, così dopo una reinstallazione puoi ripristinare tutto con pochi clic. Non salva i tuoi file personali: per quelli usa «File e cartelle».",
            );
                ui.horizontal(|ui| {
                    ui.label(t("Cartella dei backup:"));
                    ui.add(egui::TextEdit::singleline(&mut self.backup_dst).desired_width(ui::fit_width(ui, 420.0, 160.0)));
                });
                ui.horizontal_wrapped(|ui| {
                    self.run_btn(
                        ui, ctx, t("Salva elenco pacchetti"), "backup",
                        &format!("mkdir -p {dst} && dpkg --get-selections > {dst}/pacchetti.list && apt-mark showmanual > {dst}/manuali.list && echo 'Elenco salvato'"),
                        false, None,
                    );
                    self.run_btn(
                        ui, ctx, t("Ripristina pacchetti"), "backup",
                        &format!("xargs -a {dst}/manuali.list apt-get -y install"),
                        true, Some(t("Reinstallare i pacchetti dell'elenco salvato?")),
                    );
                    self.run_btn(
                        ui, ctx, t("Backup repository"), "backup",
                        &format!("mkdir -p {dst} && tar czf {dst}/repository.tar.gz $(for p in /etc/apt/sources.list /etc/apt/sources.list.d /etc/apt/trusted.gpg.d /etc/apt/keyrings; do [ -e \"$p\" ] && printf '%s ' \"$p\"; done) && chown -R {user} {dst} && echo 'Backup repository creato'"),
                        true, None,
                    );
                    self.run_btn(
                        ui, ctx, t("Ripristina repository"), "backup",
                        &format!("set -e; A={dst}/repository.tar.gz; [ -f \"$A\" ] || {{ echo 'Archivio non trovato' >&2; exit 1; }}; BAD=$(tar tzf \"$A\" | grep -v -E '^etc/apt/(sources\\.list$|sources\\.list\\.d/|trusted\\.gpg\\.d/|keyrings/)' || true); [ -z \"$BAD\" ] || {{ echo \"Archivio non valido: $BAD\" >&2; exit 1; }}; tar xzf \"$A\" -C / && apt-get update"),
                        true, Some(t("Ripristinare repository e chiavi dal backup? Le sorgenti attuali verranno sovrascritte.")),
                    );
                });
            }
            _ => {
                ui::hint(
                    ui,
                    "Qui vedi gli snapshot di Timeshift e le cartelle di ripristino presenti sul sistema. Puoi selezionarne uno o più oppure rimuoverli tutti: prima della cancellazione viene chiesta conferma e l'operazione usa pkexec. Premi «Aggiorna elenco» per cercare i punti disponibili.",
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔄 Aggiorna elenco"))).clicked() {
                        self.restore_msg = None;
                        self.spawn(ctx, restore_scan_job());
                    }
                    if ui.add_enabled(self.action_enabled() && !self.restore_points.is_empty(), egui::Button::new(t("☑ Seleziona tutto"))).clicked() {
                        self.restore_selected = self.restore_points.iter().map(|p| p.id.clone()).collect();
                    }
                    if ui.add_enabled(self.action_enabled() && !self.restore_selected.is_empty(), egui::Button::new(t("☐ Deseleziona tutto"))).clicked() {
                        self.restore_selected.clear();
                    }
                });
                if let Some((msg, is_error)) = &self.restore_msg {
                    ui.add_space(3.0);
                    ui.label(
                        egui::RichText::new(msg.as_str())
                            .size(12.5)
                            .color(if *is_error { ui::semantic_text(ui::BAD) } else { ui::accent_text() }),
                    );
                }

                ui.add_space(4.0);
                let points = self.restore_points.clone();
                let mut selected = self.restore_selected.clone();
                let mut only: Option<String> = None;
                ui::card(ui, |ui| {
                    if points.is_empty() {
                        ui.label(t("Nessun punto di ripristino trovato. Premi «Aggiorna elenco» per cercare snapshot Timeshift e cartelle di ripristino."));
                    } else {
                        ui.label(tf("Punti di ripristino rilevati: {}", &[&points.len()]));
                        ui.add_space(3.0);
                        egui::ScrollArea::vertical()
                            .id_salt("restore_points_scroll")
                            .max_height(210.0)
                            .show(ui, |ui| {
                                egui::Grid::new("restore_points_grid")
                                    .num_columns(5)
                                    .min_col_width(0.0)
                                    .max_col_width((ui.available_width() * 0.34).max(80.0))
                                    .spacing([6.0, 4.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        ui.strong(t("Selez."));
                                        ui.strong(t("Nome"));
                                        ui.strong(t("Data"));
                                        ui.strong(t("Tipo"));
                                        ui.strong(t("Dimensione"));
                                        ui.end_row();
                                        for p in &points {
                                            let mut checked = selected.iter().any(|id| id == &p.id);
                                            if ui.checkbox(&mut checked, "").clicked() {
                                                if checked {
                                                    if !selected.contains(&p.id) {
                                                        selected.push(p.id.clone());
                                                    }
                                                } else {
                                                    selected.retain(|id| id != &p.id);
                                                }
                                            }
                                            // Cliccando il nome si seleziona solo quel punto
                                            // e lo si propone per il ripristino singolo.
                                            if ui.button(p.name.as_str()).clicked() {
                                                only = Some(p.id.clone());
                                            }
                                            ui.label(p.date.as_str());
                                            let kind = tr(&p.kind);
                                            let type_label = if p.snapshot_type.is_empty() {
                                                kind
                                            } else {
                                                format!("{} · {}", kind, p.snapshot_type)
                                            };
                                            ui.label(type_label);
                                            ui.label(p.size.as_str());
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                });
                self.restore_selected = selected;
                if let Some(id) = only {
                    self.restore_selected = vec![id.clone()];
                    self.snap_name = id;
                }

                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let count = self.restore_selected.len();
                    let label = tf("🗑 Rimuovi selezionati ({})", &[&count]);
                    if ui.add_enabled(self.action_enabled() && count > 0, egui::Button::new(label)).clicked() {
                        let chosen: Vec<_> = self.restore_points
                            .iter()
                            .filter(|p| self.restore_selected.contains(&p.id))
                            .cloned()
                            .collect();
                        let text = tf(
                            "Rimuovere {} punti di ripristino selezionati? L'operazione è irreversibile e richiede l'autorizzazione di amministratore.",
                            &[&chosen.len()],
                        );
                        self.confirm = Some((text, restore_delete_job(chosen)));
                    }
                    if ui.add_enabled(self.action_enabled() && !self.restore_points.is_empty(), egui::Button::new(t("🗑 Rimuovi tutto"))).clicked() {
                        let all = self.restore_points.clone();
                        let text = tf(
                            "Rimuovere TUTTI i {} punti di ripristino? L'operazione è irreversibile e richiede l'autorizzazione di amministratore.",
                            &[&all.len()],
                        );
                        self.confirm = Some((text, restore_delete_job(all)));
                    }
                });

                ui.add_space(6.0);
                ui::card(ui, |ui| {
                    ui.label(t("Creazione e ripristino (Timeshift):"));
                    let pre = "command -v timeshift >/dev/null 2>&1 || { echo 'Timeshift non installato: usa il pulsante Installa Timeshift'; exit 1; }; ";
                    ui.horizontal_wrapped(|ui| {
                        self.run_btn(ui, ctx, t("➕ Crea punto di ripristino"), "restore",
                            &format!("{pre}timeshift --create --comments 'DebUtility' --scripted"), true, None);
                        self.run_btn(ui, ctx, t("Installa Timeshift"), "restore",
                            "apt-get -y install timeshift", true, None);
                    });
                    ui.horizontal(|ui| {
                        ui.label(t("Nome snapshot da ripristinare:"));
                        ui.text_edit_singleline(&mut self.snap_name);
                    });
                    let name = self.snap_name.trim().to_string();
                    let valid = crate::restore_points::valid_id(&name);
                    let script = format!("{pre}timeshift --restore --snapshot {} --scripted --yes", util::shq(&name));
                    ui.add_enabled_ui(valid, |ui| {
                        self.run_btn(ui, ctx, t("⏪ Ripristina punto"), "restore", &script, true,
                            Some(t("Ripristinare il sistema a questo punto? Le modifiche successive andranno perse e verrà richiesto un riavvio.")));
                    });
                });
            }
        }
        if self.sub == 2 {
            self.show_out(ui, "restore", 170.0);
        } else {
            self.show_out(ui, "backup", 260.0);
        }
    }

    // ----- Log debug
    pub(crate) fn ui_log(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Log debug");
        ui.horizontal(|ui| {
            if ui.button(if self.log_pause { t("▶ Riprendi") } else { t("⏸ Pausa") }).clicked() {
                self.log_pause = !self.log_pause;
                if self.log_pause {
                    self.log_frozen = self.log.clone();
                }
            }
            if ui.button(t("🧽 Pulisci")).clicked() {
                self.log.clear();
                self.log_frozen.clear();
            }
            self.run_btn(ui, ctx, t("Errori di sistema (journal)"), "journal",
                "journalctl -p err -b --no-pager | tail -n 100", true, None);
        });
        ui.label(t("Operazioni di DebUtility:"));
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.label(t("Dimensione massima log journald:"));
            ui.add(egui::Slider::new(&mut self.journal_max_mb, 50..=2000).suffix(" MB"));
            let script = format!("journalctl --vacuum-size={}M", self.journal_max_mb);
            self.run_btn(ui, ctx, t("🧹 Riduci log journald"), "journal", &script, true, None);
        });
        ui.add_space(2.0);
        let txt = if self.log_pause { &self.log_frozen } else { &self.log };
        ui::output_box(ui, txt, 300.0);
        ui.label(t("Errori del journal:"));
        self.show_out(ui, "journal", 260.0);
        self.solutions_ui(ui, ctx, "journal", true);
    }

    // ----- File shredder
    pub(crate) fn ui_shredder(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Distruggi file sicuro");
        ui.label(t("Sovrascrive più volte il file con dati casuali e zeri, poi lo elimina. L'operazione è irreversibile."));
        ui.add_space(2.0);
        ui.label(egui::RichText::new(t("File e cartelle da distruggere (uno per riga):")).strong());
        ui.add(egui::TextEdit::multiline(&mut self.shred_path).desired_rows(4).desired_width(ui::fit_width(ui, 420.0, 160.0)));
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("📄 Scegli file..."))).clicked() {
                self.bg(ctx, |ch| ch.send(Msg::Picked(util::pick_paths(false))));
            }
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("📁 Scegli cartella..."))).clicked() {
                self.bg(ctx, |ch| ch.send(Msg::Picked(util::pick_paths(true))));
            }
            if ui.button(t("Svuota elenco")).clicked() {
                self.shred_path.clear();
            }
        });
        ui.add_space(3.0);
        ui.add(egui::Slider::new(&mut self.shred_passes, 1..=35).text(t("Passaggi di sovrascrittura")));

        let home = util::home();
        let paths: Vec<String> =
            self.shred_path.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        let bad = paths.iter().find(|p| {
            let path = Path::new(p.as_str());
            // `shred` segue i symlink: non accettarli come destinazione.
            let md = match std::fs::symlink_metadata(path) {
                Ok(md) if !md.file_type().is_symlink() => md,
                _ => return true,
            };
            if !md.is_file() && !md.is_dir() {
                return true;
            }
            // Confronta i percorsi risolti: un link interno in un genitore
            // punta comunque a una destinazione diversa da quella protetta.
            let canon = path.canonicalize().ok();
            let home_canon = Path::new(&home).canonicalize().ok();
            let protected = match (&canon, &home_canon) {
                (Some(c), Some(h)) => c == h || c == Path::new("/"),
                (Some(c), None) => c == Path::new("/"),
                (None, _) => p.as_str() == home.as_str(),
            };
            !(p.starts_with('/') && p.len() > 1 && !protected)
        });
        if let Some(b) = bad {
            ui.colored_label(ui::semantic_text(ui::WARN), t("Indica un percorso assoluto esistente (la radice e la cartella home sono protette)."));
            ui.label(egui::RichText::new(b.as_str()).color(ui::text_dim()));
        }
        let safe = !paths.is_empty() && bad.is_none();
        let n = self.shred_passes;
        let list = paths.iter().map(|p| util::shq(p)).collect::<Vec<_>>().join(" ");
        let script = format!(
            "for P in {list}; do if [ -d \"$P\" ]; then find \"$P\" -type f -exec shred -n {n} -z -u {{}} + && rm -rf \"$P\"; else shred -v -n {n} -z -u \"$P\"; fi; done; echo 'Distruzione completata'"
        );
        ui.add_space(2.0);
        if ui.add_enabled(safe && self.action_enabled(), egui::Button::new(t("🗑 Distruggi"))).clicked() {
            let job = crate::cmd_job("shred", script, false);
            let msg = tf("Distruggere DEFINITIVAMENTE {} elementi? Non sarà possibile recuperarli.", &[&paths.len()]);
            self.confirm = Some((msg, job));
        }
        ui.label(
            egui::RichText::new(t("Nota: su SSD e file system con journaling/snapshot (btrfs, ZFS) la sovrascrittura non offre garanzie complete."))
                .color(crate::ui::text_dim()),
        );
        self.show_out(ui, "shred", 200.0);
    }

    // ----- Impostazioni
    pub(crate) fn ui_settings(&mut self, ui: &mut egui::Ui) {
        ui::title(ui, "Impostazioni");
        ui.heading(t("Lingua"));
        let mut lang = self.cfg.lang;
        // Lingue e nomi arrivano da lang/langs.tsv: nessun indice o conteggio hardcoded.
        egui::ComboBox::from_label(t("Lingua dell'interfaccia"))
            .selected_text(i18n::name_of(lang))
            .show_ui(ui, |ui| {
                for (i, _) in i18n::langs().iter().enumerate() {
                    ui.selectable_value(&mut lang, i, i18n::name_of(i));
                }
            });
        if lang != self.cfg.lang {
            self.cfg.lang = lang;
            i18n::set_lang(lang);
            self.cfg.save();
        }
        ui.add_space(4.0);
        ui.heading(t("Opzioni generali"));
        if ui
            .checkbox(&mut self.cfg.quick_clean, t("Pulizia rapida della cache utente all'avvio"))
            .changed()
        {
            self.cfg.save();
        }
        if ui
            .checkbox(&mut self.cfg.dark_mode, t("Tema scuro dell'interfaccia"))
            .changed()
        {
            self.cfg.save();
        }
        if ui
            .checkbox(&mut self.cfg.check_updates, t("Controlla aggiornamenti all'avvio"))
            .changed()
        {
            self.cfg.save();
        }
        ui.add_space(4.0);
        ui.heading(t("Password amministratore"));
        ui.label(t(
            "Per sicurezza DebUtility non salva la password di root: le operazioni privilegiate usano pkexec \
             e il sistema chiede l'autorizzazione tramite la finestra di Polkit.",
        ));

ui.add_space(4.0);
        ui.heading(t("Profilo di Configurazione"));
        ui.label(t("Esporta o importa le preferenze e le configurazioni di DebUtility in un file TOML."));
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui.button(t("💾 Esporta profilo")).clicked() {
                let out = std::process::Command::new("zenity")
                    .args(["--file-selection", "--save", "--confirm-overwrite", "--filename=debutility-profile.toml"])
                    .output();
                if let Ok(res) = out {
                    if res.status.success() {
                        let path_str = String::from_utf8_lossy(&res.stdout).trim().to_string();
                        if !path_str.is_empty() {
                            let profile = crate::profile::UserProfile {
                                version: 1,
                                custom_blocked_domains: vec![],
                                cleaning_paths: vec![],
                                telemetry_guard_enabled: self.cfg.dark_mode,
                            };
                            match crate::profile::export_profile_to_file(&profile, std::path::Path::new(&path_str)) {
                                Ok(_) => self.log.push_str(&format!("✔ Profilo esportato in: {path_str}\n")),
                                Err(e) => self.log.push_str(&format!("✘ Errore esportazione: {e}\n")),
                            }
                        }
                    }
                }
            }

            if ui.button(t("📂 Importa profilo")).clicked() {
                let out = std::process::Command::new("zenity")
                    .args(["--file-selection"])
                    .output();
                if let Ok(res) = out {
                    if res.status.success() {
                        let path_str = String::from_utf8_lossy(&res.stdout).trim().to_string();
                        if !path_str.is_empty() {
                            match crate::profile::import_profile_from_file(std::path::Path::new(&path_str)) {
                                Ok(_profile) => {
                                    self.log.push_str(&format!("✔ Profilo importato da: {path_str}\n"));
                                }
                                Err(e) => self.log.push_str(&format!("✘ Errore importazione: {e}\n")),
                            }
                        }
                    }
                }
            }
        });

        ui.add_space(4.0);
        if ui.button(t("↺ Reimposta configurazione")).clicked() {
            Config::reset();
            self.cfg = Config::default();
            i18n::set_lang(self.cfg.lang);
        }
    }
    // ----- Kernel orfani con anteprima spazio
pub fn ui_orphan_kernels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
    ui::title(ui, t("Kernel Orfani e Anteprima Spazio"));
    ui::hint(
        ui,
        t("Individua i kernel installati ma non in uso, calcola lo spazio disco occupato da ciascuno e mostra l'anteprima totale in MB/GB prima della rimozione."),
    );

    ui.add_space(3.0);
    if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔍 Scansiona kernel e spazio"))).clicked() {
        self.spawn(ctx, move |ch| {
            ch.send(Msg::Log("▶ Scansione kernel installati e calcolo spazio su disco...".into()));
            let running = std::process::Command::new("uname")
                .arg("-r")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();

            let out = std::process::Command::new("dpkg-query")
                .args(["-W", "-f=${Package}\t${Installed-Size}\t${Status}\n", "linux-image-*", "linux-headers-*"])
                .output();

            // Versione "base" del kernel in uso (senza il suffisso finale, es. "-amd64"):
            // serve a proteggere anche i pacchetti -common e gli header del kernel attivo.
            let base = running
                .rsplit_once('-')
                .map(|(b, _)| b.to_string())
                .unwrap_or_else(|| running.clone());
            let mut map: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
            if let Ok(res) = out {
                for line in String::from_utf8_lossy(&res.stdout).lines() {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() >= 3 && parts[2].contains("installed") {
                        let pkg = parts[0].trim();
                        let kb: u64 = parts[1].trim().parse().unwrap_or(0);
                        let ver = pkg
                            .trim_start_matches("linux-image-")
                            .trim_start_matches("linux-headers-")
                            .trim_start_matches("unsigned-");
                        // Solo pacchetti con numero di versione (esclude i metapacchetti
                        // come linux-image-amd64) e mai quelli del kernel in uso.
                        let is_versioned = ver.chars().next().map_or(false, |c| c.is_ascii_digit());
                        if !running.is_empty()
                            && is_versioned
                            && !running.starts_with(ver)
                            && !ver.starts_with(&running)
                            && !ver.starts_with(&base)
                        {
                            let slot = map.entry(ver.to_string()).or_insert(0);
                            *slot = slot.saturating_add(kb.saturating_mul(1024));
                        }
                    }
                }
            }
            let list: Vec<(String, u64, bool)> = map.into_iter().map(|(v, b)| (v, b, false)).collect();
            ch.send(Msg::OrphanKernels(list.clone()));
            ch.send(Msg::Log(format!("✔ Trovati {} kernel non in uso.", list.len())));
            let summary: String = list
                .iter()
                .map(|(v, bytes, _)| format!("{v}: {:.1} MB", *bytes as f64 / (1024.0 * 1024.0)))
                .collect::<Vec<_>>()
                .join(" | ");
            if !summary.is_empty() {
                ch.send(Msg::Log(format!("Kernel rilevati -> {summary}")));
            }
        });
    }

    ui.add_space(4.0);
    let selected_bytes: u64 = self.orphan_kernels.iter().filter(|(_, _, s)| *s).map(|(_, b, _)| *b).sum();
    let total_bytes: u64 = self.orphan_kernels.iter().map(|(_, b, _)| *b).sum();

    ui::card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(t("Spazio liberabile selezionato:")).strong().size(16.0));
            let mb = selected_bytes as f64 / (1024.0 * 1024.0);
            let gb = selected_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            ui.label(
                egui::RichText::new(format!("{mb:.1} MB ({gb:.2} GB)"))
                    .strong()
                    .size(18.0)
                    .color(ui::accent_text()),
            );
        });
        ui.add_space(3.0);
        let frac = if total_bytes > 0 {
            (selected_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ui.add(
            ui::bar(frac)
                .fill(ui::ACCENT)
                .desired_width(ui.available_width())
                .text(t("Quota spazio kernel selezionato")),
        );
    });

    ui.add_space(4.0);
    ui::card(ui, |ui| {
        if self.orphan_kernels.is_empty() {
            ui.label(t("Nessun kernel orfano caricato nella lista interattiva. Premi 'Scansiona kernel e spazio' per analizzare il sistema."));
        } else {
            for (ver, bytes, sel) in &mut self.orphan_kernels {
                let mb = *bytes as f64 / (1024.0 * 1024.0);
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(sel, egui::RichText::new(ver.as_str()).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("{mb:.1} MB")).color(ui::accent_text()));
                    });
                });
            }
            ui.add_space(3.0);
            let targets: Vec<String> = self
                .orphan_kernels
                .iter()
                .filter(|(_, _, s)| *s)
                .map(|(v, _, _)| {
                    format!(
                        "{} {}",
                        util::shq(&format!("linux-image-{v}*")),
                        util::shq(&format!("linux-headers-{v}*"))
                    )
                })
                .collect();
            if !targets.is_empty() {
                let script = format!("apt-get purge -y {} && update-grub", targets.join(" "));
                self.run_btn(
                    ui,
                    ctx,
                    t("🗑 Rimuovi kernel selezionati"),
                    "orphan_kernels",
                    &script,
                    true,
                    Some(t("Rimuovere definitivamente i kernel selezionati e aggiornare GRUB?")),
                );
            }
        }
    });
    self.show_out(ui, "orphan_kernels", 200.0);
}
// ----- Monitor Connessioni Attive (TCP/UDP Socket Viewer)
    pub fn ui_net_sockets(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Monitor Connessioni Attive (TCP/UDP)"));
        ui::hint(
            ui,
            t("Visualizza le socket TCP/UDP attive in ascolto o connesse e il relativo processo/programma per individuare comunicazioni di rete inattese."),
        );

        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔄 Aggiorna connessioni"))).clicked() {
                self.spawn(ctx, move |ch| {
                    ch.send(Msg::Log("▶ Lettura socket TCP/UDP attive e processi associati...".into()));
                    let out = std::process::Command::new("sh")
                        .arg("-c")
                        .arg("ss -tupn 2>/dev/null || netstat -tupn 2>/dev/null")
                        .output();

                    // Stati riconosciuti sia da `ss` sia da `netstat`.
                    const STATES: &[&str] = &[
                        "LISTEN", "ESTAB", "UNCONN", "TIME-WAIT", "CLOSE-WAIT", "SYN-SENT",
                        "SYN-RECV", "FIN-WAIT-1", "FIN-WAIT-2", "LAST-ACK", "CLOSING", "CLOSED",
                    ];
                    let mut rows: Vec<(String, String, String, String, String)> = Vec::new();
                    if let Ok(res) = out {
                        for line in String::from_utf8_lossy(&res.stdout).lines().skip(1) {
                            let cols: Vec<&str> = line.split_whitespace().collect();
                            if cols.len() < 4 {
                                continue;
                            }
                            let proto = cols[0].to_string();
                            if !proto.starts_with("tcp") && !proto.starts_with("udp") {
                                continue;
                            }

                            // Local/remote sono i primi due token in forma `indirizzo:porta`.
                            // Funziona sia per `ss` sia per `netstat`, TCP e UDP: prima il
                            // codice usava indici diversi per UDP e mostrava Recv-Q come indirizzo.
                            let mut addrs = cols.iter().skip(1).copied().filter(|c| c.contains(':'));
                            let local = addrs.next().unwrap_or("-").to_string();
                            let remote = addrs.next().unwrap_or("-").to_string();

                            let state = cols
                                .iter()
                                .skip(1)
                                .copied()
                                .find(|c| STATES.contains(c))
                                .unwrap_or(if proto.starts_with("udp") { "UNCONN" } else { "-" })
                                .to_string();

                            let proc_col = cols
                                .iter()
                                .find(|c| c.contains("users:(") || c.contains('/'))
                                .copied()
                                .unwrap_or(t("sconosciuto / permessi limitati"))
                                .to_string();
                            rows.push((proto, state, local, remote, proc_col));
                        }
                    }

                    let total = rows.len();
                    ch.send(Msg::NetSockets(rows));
                    ch.send(Msg::Log(format!("✔ Rilevate {total} socket attive.")));
                });
            }

            ui.label(t("Filtro:"));
            ui.add(egui::TextEdit::singleline(&mut self.net_socket_filter).desired_width(220.0));
            if !self.net_socket_filter.is_empty() && ui.button("✖").clicked() {
                self.net_socket_filter.clear();
            }
            if self.busy {
                ui.spinner();
            }
        });

        ui.add_space(4.0);
        ui::card(ui, |ui| {
            let filter = self.net_socket_filter.trim().to_lowercase();
            let filtered: Vec<&(String, String, String, String, String)> = self
                .net_sockets
                .iter()
                .filter(|(proto, state, local, remote, proc_info)| {
                    filter.is_empty()
                        || proto.to_lowercase().contains(&filter)
                        || state.to_lowercase().contains(&filter)
                        || local.to_lowercase().contains(&filter)
                        || remote.to_lowercase().contains(&filter)
                        || proc_info.to_lowercase().contains(&filter)
                })
                .collect();

            if filtered.is_empty() {
                ui.label(t("Nessuna connessione caricata o corrispondente al filtro. Premi 'Aggiorna connessioni'."));
            } else {
                ui.label(
                    egui::RichText::new(format!("{}: {}", t("Socket visualizzate"), filtered.len()))
                        .strong()
                        .color(ui::accent_text()),
                );
                ui.add_space(3.0);
                egui::ScrollArea::vertical().id_salt("net_sockets_scroll").max_height(195.0).show(ui, |ui| {
                    egui::Grid::new("net_sockets_grid")
                        .num_columns(5)
                        .min_col_width(0.0)
                        .max_col_width((ui.available_width() * 0.30).max(70.0))
                        .spacing([6.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong(t("Proto"));
                            ui.strong(t("Stato"));
                            ui.strong(t("Locale"));
                            ui.strong(t("Remoto"));
                            ui.strong(t("Processo / PID"));
                            ui.end_row();

                            for (proto, state, local, remote, proc_info) in filtered {
                                ui.label(proto.as_str());
                                ui.label(state.as_str());
                                ui.label(egui::RichText::new(local.as_str()).monospace());
                                ui.label(egui::RichText::new(remote.as_str()).monospace());
                                ui.label(egui::RichText::new(proc_info.as_str()).strong());
                                ui.end_row();
                            }
                        });
});
        }
    });
}

// ----- Gestore Chiavi GPG e Repository PPA con Verifica Stato
pub fn ui_gpg_keys(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
    ui::title(ui, t("Gestore Chiavi GPG e Repository PPA"));
    ui::hint(
        ui,
        t("Verifica lo stato di validità delle chiavi GPG dei repository e PPA, evidenzia quelle scadute o prossime alla scadenza e consente il rinnovo automatico."),
    );

    ui.add_space(3.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔍 Verifica chiavi GPG"))).clicked() {
            self.spawn(ctx, move |ch| {
                ch.send(Msg::Log("▶ Scansione chiavi GPG di repository e PPA in corso...".into()));
                let now_epoch: i64 = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let soon_secs: i64 = 30 * 86400;

                let mut key_files: Vec<std::path::PathBuf> = Vec::new();
                for dir in ["/etc/apt/keyrings", "/etc/apt/trusted.gpg.d"] {
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for e in entries.flatten() {
                            let p = e.path();
                            if p.is_file() {
                                key_files.push(p);
                            }
                        }
                    }
                }
                let legacy_ring = std::path::PathBuf::from("/etc/apt/trusted.gpg");
                if legacy_ring.is_file() {
                    key_files.push(legacy_ring);
                }

                // Tuple: (key_id_or_fpr, owner_or_file, expiry_label, status_code, source_path)
                let mut rows: Vec<(String, String, String, String, String)> = Vec::new();
                for path in key_files {
                    let out = std::process::Command::new("gpg")
                        .args(["--batch", "--with-colons", "--fixed-list-mode", "--show-keys"])
                        .arg(&path)
                        .output();
                    if let Ok(res) = out {
                        let mut cur_fpr = String::new();
                        let mut cur_exp = String::new();
                        let mut cur_uid = String::new();
                        let mut has_pub = false;

                        let flush_key = |fpr: &str, uid: &str, exp_s: &str, p: &std::path::Path, rows: &mut Vec<(String, String, String, String, String)>| {
                            if fpr.is_empty() {
                                return;
                            }
                            let (status, exp_label) = if exp_s.is_empty() {
                                ("VALID".to_string(), t("Nessuna scadenza").to_string())
                            } else if let Ok(ts) = exp_s.parse::<i64>() {
                                let diff = ts - now_epoch;
                                let date_str = std::process::Command::new("date")
                                    .args(["-d", &format!("@{ts}"), "+%Y-%m-%d"])
                                    .output()
                                    .ok()
                                    .and_then(|o| String::from_utf8(o.stdout).ok())
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .unwrap_or_else(|| ts.to_string());
                                if diff < 0 {
                                    ("EXPIRED".to_string(), tf("Scaduta ({})", &[&date_str]))
                                } else if diff <= soon_secs {
                                    let days = (diff / 86400).max(0);
                                    ("EXPIRING".to_string(), tf("In scadenza tra {}g ({})", &[&days, &date_str]))
                                } else {
                                    ("VALID".to_string(), tf("Valida fino al {}", &[&date_str]))
                                }
                            } else {
                                ("VALID".to_string(), exp_s.to_string())
                            };

                            let short_id = if fpr.len() > 16 { &fpr[fpr.len() - 16..] } else { fpr };
                            let label_uid = if uid.is_empty() {
                                p.file_name().and_then(|n| n.to_str()).unwrap_or("keyring").to_string()
                            } else {
                                uid.to_string()
                            };
                            rows.push((
                                short_id.to_string(),
                                label_uid,
                                exp_label,
                                status,
                                p.to_string_lossy().to_string(),
                            ));
                        };

                        for line in String::from_utf8_lossy(&res.stdout).lines() {
                            let f: Vec<&str> = line.split(':').collect();
                            match f.first().copied() {
                                Some("pub") => {
                                    if has_pub {
                                        flush_key(&cur_fpr, &cur_uid, &cur_exp, &path, &mut rows);
                                    }
                                    has_pub = true;
                                    cur_fpr.clear();
                                    cur_uid.clear();
                                    cur_exp = f.get(6).copied().unwrap_or("").trim().to_string();
                                }
                                Some("fpr") if cur_fpr.is_empty() => {
                                    cur_fpr = f.get(9).copied().unwrap_or("").trim().to_string();
                                }
                                Some("uid") if cur_uid.is_empty() => {
                                    cur_uid = f.get(9).copied().unwrap_or("").trim().to_string();
                                }
                                _ => {}
                            }
                        }
                        if has_pub {
                            flush_key(&cur_fpr, &cur_uid, &cur_exp, &path, &mut rows);
                        }
                    }
                }

                let expired_cnt = rows.iter().filter(|(_, _, _, st, _)| st == "EXPIRED").count();
                let soon_cnt = rows.iter().filter(|(_, _, _, st, _)| st == "EXPIRING").count();
                let total_cnt = rows.len();
                ch.send(Msg::GpgKeys(rows));
                ch.send(Msg::Log(format!(
                    "✔ {}",
                    tf(
                        "Analizzate {} chiavi GPG ({} scadute, {} in scadenza).",
                        &[&total_cnt, &expired_cnt, &soon_cnt],
                    )
                )));
            });
        }

        let problem_ids: Vec<String> = self
            .gpg_keys
            .iter()
            .filter(|(_, _, _, st, _)| st == "EXPIRED" || st == "EXPIRING")
            .map(|(kid, _, _, _, _)| kid.clone())
            .collect();

        if !problem_ids.is_empty() {
            let ids_joined = problem_ids.join(" ");
            let renew_script = format!(
                "KS_LIST='hkps://keyserver.ubuntu.com:443 hkps://keys.openpgp.org hkps://pgp.mit.edu'; \
                for K in {ids_joined}; do \
                    echo \"--- Rinnovo chiave: $K ---\"; \
                    OK=0; \
                    for KS in $KS_LIST; do \
                        if gpg --batch --keyserver \"$KS\" --recv-keys \"$K\" >/dev/null 2>&1 || gpg --batch --keyserver \"$KS\" --refresh-keys \"$K\" >/dev/null 2>&1; then \
                            echo \"OK da $KS per $K\"; \
                            OK=1; \
                            break; \
                        fi; \
                    done; \
                    if [ \"$OK\" -eq 0 ]; then \
                        echo \"AVVISO: chiave $K non trovata o non rinnovata sui keyserver pubblici\"; \
                        continue; \
                    fi; \
                    for D in /etc/apt/keyrings /etc/apt/trusted.gpg.d; do \
                        [ -d \"$D\" ] || continue; \
                        for F in \"$D\"/*; do \
                            [ -f \"$F\" ] || continue; \
                            if gpg --batch --show-keys \"$F\" 2>/dev/null | grep -qi \"$K\"; then \
                                if file -b \"$F\" 2>/dev/null | grep -qi 'PGP public key block'; then \
                                    gpg --batch --armor --export \"$K\" > \"$F.new\" && mv \"$F.new\" \"$F\" && echo \"Aggiornato file ASC: $F\"; \
                                else \
                                    gpg --batch --export \"$K\" > \"$F.new\" && mv \"$F.new\" \"$F\" && echo \"Aggiornato file GPG: $F\"; \
                                fi; \
                            fi; \
                        done; \
                    done; \
                done; \
                apt-get update"
            );
            self.run_btn(
                ui,
                ctx,
                t("🔁 Rinnova chiavi scadute/in scadenza"),
                "gpg_keys",
                &renew_script,
                true,
                Some(t("Scaricare/aggiornare dai keyserver le chiavi GPG scadute o in scadenza, aggiornare i file keyring APT e rigenerare l'indice?")),
            );
        }

        if self.busy {
            ui.spinner();
        }
    });

    ui.add_space(4.0);
    ui::card(ui, |ui| {
        if self.gpg_keys.is_empty() {
            ui.label(t("Nessuna chiave GPG verificata. Premi 'Verifica chiavi GPG' per controllare la scadenza."));
        } else {
            let mut renew_single_script: Option<String> = None;
            let mut action_script: Option<(String, String)> = None;

            egui::ScrollArea::both().id_salt("gpg_keys_scroll").max_height(185.0).show(ui, |ui| {
                egui::Grid::new("gpg_keys_grid")
                    .num_columns(5)
                    .min_col_width(0.0)
                    .max_col_width((ui.available_width() * 0.32).max(80.0))
                    .spacing([6.0, 5.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong(t("Stato"));
                        ui.strong(t("Key ID"));
                        ui.strong(t("Identità / File"));
                        ui.strong(t("Scadenza"));
                        ui.strong(t("Azione"));
                        ui.end_row();

                        for (kid, uid, exp_lbl, status, src_path) in &self.gpg_keys {
                            let (badge, color) = match status.as_str() {
                                "EXPIRED" => (t("SCADUTA"), ui::BAD),
                                "EXPIRING" => (t("IN SCADENZA"), ui::WARN),
                                _ => (t("VALIDA"), ui::GOOD),
                            };
                            ui.label(egui::RichText::new(badge).strong().color(color));
                            ui.label(egui::RichText::new(kid.as_str()).monospace());
                            ui.label(egui::RichText::new(uid.as_str())).on_hover_text(src_path.as_str());
                            ui.label(egui::RichText::new(exp_lbl.as_str()).color(color));

                            if status == "EXPIRED" || status == "EXPIRING" {
                                ui.horizontal(|ui| {
                                    if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Rinnova"))).clicked() {
                                        let qk = util::shq(kid);
                                        renew_single_script = Some(format!(
                                            "KS_LIST='hkps://keyserver.ubuntu.com:443 hkps://keys.openpgp.org hkps://pgp.mit.edu'; \
                                            echo \"--- Rinnovo chiave: {qk} ---\"; \
                                            OK=0; \
                                            for KS in $KS_LIST; do \
                                                if gpg --batch --keyserver \"$KS\" --recv-keys {qk} >/dev/null 2>&1 || gpg --batch --keyserver \"$KS\" --refresh-keys {qk} >/dev/null 2>&1; then \
                                                    echo \"OK da $KS per {qk}\"; \
                                                    OK=1; \
                                                    break; \
                                                fi; \
                                            done; \
                                            if [ \"$OK\" -eq 1 ]; then \
                                                for D in /etc/apt/keyrings /etc/apt/trusted.gpg.d; do \
                                                    [ -d \"$D\" ] || continue; \
                                                    for F in \"$D\"/*; do \
                                                        [ -f \"$F\" ] || continue; \
                                                        if gpg --batch --show-keys \"$F\" 2>/dev/null | grep -qi {qk}; then \
                                                            if file -b \"$F\" 2>/dev/null | grep -qi 'PGP public key block'; then \
                                                                gpg --batch --armor --export {qk} > \"$F.new\" && mv \"$F.new\" \"$F\" && echo \"Aggiornato file ASC: $F\"; \
                                                            else \
                                                                gpg --batch --export {qk} > \"$F.new\" && mv \"$F.new\" \"$F\" && echo \"Aggiornato file GPG: $F\"; \
                                                            fi; \
                                                        fi; \
                                                    done; \
                                                done; \
                                            else \
                                                echo \"AVVISO: chiave {qk} non trovata o non rinnovata sui keyserver pubblici\"; \
                                            fi; \
                                            apt-get update"
                                        ));
                                    }
                                    if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Disabilita"))).clicked() {
                                        let qp = util::shq(src_path);
                                        // Solo cifre esadecimali: viene interpolato in un programma awk.
                                        let kid_hex: String = kid
                                            .chars()
                                            .filter(|c| c.is_ascii_hexdigit())
                                            .collect();
                                        let dis_script = format!(
                                            "mkdir -p /etc/apt/keyrings.disabled /etc/apt/trusted.gpg.d.disabled && \
                                            if [ -f {qp} ] && [ {qp} != \"/etc/apt/trusted.gpg\" ]; then \
                                                BN=$(basename {qp}); \
                                                DIR=$(dirname {qp}); \
                                                if [ \"$DIR\" = \"/etc/apt/keyrings\" ]; then \
                                                    mv {qp} \"/etc/apt/keyrings.disabled/$BN\" && echo \"Spostato file chiave in /etc/apt/keyrings.disabled/$BN\"; \
                                                elif [ \"$DIR\" = \"/etc/apt/trusted.gpg.d\" ]; then \
                                                    mv {qp} \"/etc/apt/trusted.gpg.d.disabled/$BN\" && echo \"Spostato file chiave in /etc/apt/trusted.gpg.d.disabled/$BN\"; \
                                                fi; \
                                            fi && \
                                            KEYBASE=$(basename {qp}); \
                                            for SF in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do \
                                                [ -f \"$SF\" ] && [ ! -L \"$SF\" ] || continue; \
                                                if grep -qF \"$KEYBASE\" \"$SF\" 2>/dev/null; then \
                                                    awk -v key=\"$KEYBASE\" -v kid=\"{kid_hex}\" '{{ hide = ($0 ~ /^[[:space:]]*#/); hit = (index($0, key) > 0) || (length(kid) > 0 && index($0, kid) > 0); if (!hide && hit) print \"# \" $0; else print }}' \"$SF\" > \"$SF.debutility.tmp\" && cat \"$SF.debutility.tmp\" > \"$SF\" && rm -f \"$SF.debutility.tmp\" && echo \"Commentate solo le righe del repository in: $SF\"; \
                                                fi; \
                                            done && \
                                            apt-get update"
                                        );
                                        let confirm_msg = tf(
                                            "Disabilitare la chiave {} e commentare i repository associati in /etc/apt/sources.list*?",
                                            &[kid],
                                        );
                                        action_script = Some((confirm_msg, dis_script));
                                    }
                                });
                            } else {
                                ui.label("-");
                            }
                            ui.end_row();
                        }
                    });
            });

            if let Some(cmd) = renew_single_script {
                let job = crate::cmd_job("gpg_keys", cmd, true);
                self.spawn(ctx, job);
            }
            if let Some((msg, cmd)) = action_script {
                let job = crate::cmd_job("gpg_keys", cmd, true);
                self.confirm = Some((msg, job));
            }
        }
    });

    ui.add_space(4.0);
    ui::card(ui, |ui| {
        ui.strong(t("Chiavi e repository disabilitati"));
        ui.add_space(3.0);

        let mut disabled_keys: Vec<(String, String, String)> = Vec::new();
        for (dis_dir, active_dir) in [
            ("/etc/apt/keyrings.disabled", "/etc/apt/keyrings"),
            ("/etc/apt/trusted.gpg.d.disabled", "/etc/apt/trusted.gpg.d"),
        ] {
            if let Ok(entries) = std::fs::read_dir(dis_dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_file() {
                        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                        let target = format!("{active_dir}/{fname}");
                        disabled_keys.push((fname, p.to_string_lossy().to_string(), target));
                    }
                }
            }
        }
        disabled_keys.sort_by(|a, b| a.0.cmp(&b.0));

        if disabled_keys.is_empty() {
            ui.label(t("Nessuna chiave o repository disabilitato trovato in .disabled."));
        } else {
            let mut reenable_action: Option<(String, String)> = None;
            egui::Grid::new("gpg_disabled_grid")
                .num_columns(3)
                .spacing([12.0, 5.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong(t("File chiave"));
                    ui.strong(t("Percorso disabilitato"));
                    ui.strong(t("Azione"));
                    ui.end_row();

                    for (fname, dis_path, active_path) in &disabled_keys {
                        ui.label(egui::RichText::new(fname.as_str()).monospace());
                        ui.label(egui::RichText::new(dis_path.as_str()).color(ui::text_dim()));
                        if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Riabilita"))).clicked() {
                            let qf = util::shq(fname);
                            let qdis = util::shq(dis_path);
                            let qact = util::shq(active_path);
                            let en_script = format!(
                                "mkdir -p /etc/apt/keyrings /etc/apt/trusted.gpg.d && \
                                if [ -f {qdis} ]; then \
                                    mv {qdis} {qact} && echo \"Ripristinato file chiave in: \" {qact}; \
                                fi && \
                                KEYBASE={qf}; \
                                for SF in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do \
                                    [ -f \"$SF\" ] && [ ! -L \"$SF\" ] || continue; \
                                    if grep -qF \"$KEYBASE\" \"$SF\" 2>/dev/null; then \
                                        awk -v key=\"$KEYBASE\" '{{ line=$0; if (line ~ /^[[:space:]]*#/ && index(line, key) > 0) {{ sub(/^[[:space:]]*#[[:space:]]*/, \"\", line); match($0, /^[[:space:]]*/); print substr($0, 1, RLENGTH) line }} else print }}' \"$SF\" > \"$SF.debutility.tmp\" && cat \"$SF.debutility.tmp\" > \"$SF\" && rm -f \"$SF.debutility.tmp\" && echo \"Decommentate solo le righe del repository in: $SF\"; \
                                    fi; \
                                done && \
                                apt-get update"
                            );
                            let confirm_msg = tf(
                                "Riabilitare la chiave {} e decommentare i repository associati in /etc/apt/sources.list*?",
                                &[fname],
                            );
                            reenable_action = Some((confirm_msg, en_script));
                        }
                        ui.end_row();
                    }
                });

            if let Some((msg, cmd)) = reenable_action {
                let job = crate::cmd_job("gpg_keys", cmd, true);
                self.confirm = Some((msg, job));
            }
        }
    });

    self.show_out(ui, "gpg_keys", 180.0);
}
// ----- Audit Repository APT (Duplicati, Signed-By rotti, Disabilitati)
    pub fn ui_apt_sources_audit(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Audit Repository APT"));
        ui::hint(
            ui,
            t("Scansiona /etc/apt/sources.list e /etc/apt/sources.list.d/* per individuare repository duplicati, Signed-By mancanti o non validi e sorgenti disabilitate."),
        );

        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔍 Scansiona repository APT"))).clicked() {
                self.spawn(ctx, move |ch| {
                    ch.send(Msg::Log("▶ Scansione sorgenti APT in /etc/apt/sources.list*...".into()));

                    let mut files: Vec<std::path::PathBuf> = Vec::new();
                    let main_list = std::path::PathBuf::from("/etc/apt/sources.list");
                    if main_list.is_file() {
                        files.push(main_list);
                    }
                    if let Ok(entries) = std::fs::read_dir("/etc/apt/sources.list.d") {
                        let mut sub_files: Vec<std::path::PathBuf> = entries
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| {
                                p.is_file()
                                    && matches!(
                                        p.extension().and_then(|s| s.to_str()),
                                        Some("list") | Some("sources")
                                    )
                            })
                            .collect();
                        sub_files.sort();
                        files.extend(sub_files);
                    }

                    let mut seen_repos: std::collections::HashSet<String> = std::collections::HashSet::new();
                    let mut issues = 0usize;
                    let mut report = String::new();

                    for fpath in files {
                        let Ok(content) = std::fs::read_to_string(&fpath) else {
                            continue;
                        };
                        let fstr = fpath.to_string_lossy().to_string();

                        for (idx, raw_line) in content.lines().enumerate() {
                            let line_no = idx + 1;
                            let trimmed = raw_line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }

                            // Righe commentate con #
                            if config_parsers::is_disabled_repo_line(trimmed) {
                                issues += 1;
                                let line_msg = tf("⚠ [DISABILITATO] {}:{} -> {}", &[&fstr, &line_no, &trimmed]);
                                report.push_str(&line_msg);
                                report.push('\n');
                                ch.send(Msg::Log(line_msg));
                                continue;
                            }

                            // Verifica Signed-By se presente nella riga
                            if let Some(pos) = trimmed.find("signed-by=") {
                                let after = &trimmed[pos + "signed-by=".len()..];
                                let key_path = after
                                    .split(|c: char| c == ' ' || c == ']' || c == ',')
                                    .next()
                                    .unwrap_or("")
                                    .trim();
                                if key_path.is_empty() || !std::path::Path::new(key_path).is_file() {
                                    issues += 1;
                                    let line_msg = tf("✘ [SIGNED-BY ROTTO] {}:{} -> file mancante: {}", &[&fstr, &line_no, &key_path]);
                                    report.push_str(&line_msg);
                                    report.push('\n');
                                    ch.send(Msg::Log(line_msg));
                                } else {
                                    let ok = std::process::Command::new("gpg")
                                        .args(["--batch", "--show-keys", key_path])
                                        .output()
                                        .map(|o| o.status.success() && !o.stdout.is_empty())
                                        .unwrap_or(false);
                                    if !ok {
                                        issues += 1;
                                        let line_msg = tf("✘ [KEYRING NON VALIDO] {}:{} -> {}", &[&fstr, &line_no, &key_path]);
                                        report.push_str(&line_msg);
                                        report.push('\n');
                                        ch.send(Msg::Log(line_msg));
                                    }
                                }
                            }

                            // Estrai chiave normalizzata per rilevare duplicati
                            let norm_repo = config_parsers::normalize_repo_line(trimmed);

                            if let Some(repo_key) = norm_repo {
                                if !seen_repos.insert(repo_key.clone()) {
                                    issues += 1;
                                    let line_msg = tf("⚠ [DUPLICATO] {}:{} -> {}", &[&fstr, &line_no, &repo_key]);
                                    report.push_str(&line_msg);
                                    report.push('\n');
                                    ch.send(Msg::Log(line_msg));
                                }
                            }
                        }
                    }

                    if issues == 0 {
                        let ok_msg = t("✔ Nessuna anomalia rilevata nei file sorgente APT.");
                        report.push_str(ok_msg);
                        report.push('\n');
                        ch.send(Msg::Log(ok_msg.into()));
                    } else {
                        ch.send(Msg::Log(format!("✔ Scansione completata: {issues} segnalazioni trovate.")));
                    }
                    ch.send(Msg::Out("apt_sources_audit", report));
                    
                });
            }

            if self.busy {
                ui.spinner();
            }
        });

        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui.strong(t("Intervento rapido su file o riga sorgente APT"));
            ui.add_space(3.0);

            ui.horizontal_wrapped(|ui| {
                ui.label(t("File sorgente:"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.apt_audit_file)
                        .desired_width(ui::fit_width(ui, 320.0, 120.0))
                        .hint_text(t("/etc/apt/sources.list.d/esempio.list")),
                );
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Pattern / Riga:"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.apt_audit_pattern)
                        .desired_width(ui::fit_width(ui, 320.0, 120.0))
                        .hint_text(t("es. ppa.launchpadcontent.net o buster")),
                );
            });

            let f_trimmed = self.apt_audit_file.trim();
            let p_trimmed = self.apt_audit_pattern.trim();
            let valid_path = canonical_apt_source(f_trimmed);
            let can_act = valid_path.is_some() && !p_trimmed.is_empty() && self.action_enabled();
            let canon_file = valid_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();

            ui.add_space(3.0);
            ui.horizontal(|ui| {
                let qf = util::shq(&canon_file);
                let qp = util::shq(p_trimmed);

                if ui.add_enabled(can_act, egui::Button::new(t("💬 Commenta righe"))).clicked() {
                    let cmd = format!(
                        "grep -nF {qp} {qf} >/dev/null 2>&1 && sed -i -E \"s/^([[:space:]]*[^#].*$(printf '%s' {qp} | sed 's/[][\\\\/.*^$]/\\\\&/g'))/# \\\\1/\" {qf} && apt-get update"
                    );
                    let msg = tf("Commentare le righe corrispondenti a '{}' in {} e aggiornare APT?", &[&p_trimmed, &canon_file]);
                    self.confirm = Some((msg, crate::cmd_job("apt_sources_audit", cmd, true)));
                }

                if ui.add_enabled(can_act, egui::Button::new(t("🗑 Rimuovi righe"))).clicked() {
                    let cmd = format!(
                        "grep -nF {qp} {qf} >/dev/null 2>&1 && sed -i -E \"/\\b$(printf '%s' {qp} | sed 's/[][\\\\/.*^$]/\\\\&/g')\\b/d\" {qf} && apt-get update"
                    );
                    let msg = tf("Rimuovere definitivamente le righe corrispondenti a '{}' da {} e aggiornare APT?", &[&p_trimmed, &canon_file]);
                    self.confirm = Some((msg, crate::cmd_job("apt_sources_audit", cmd, true)));
                }
            });
        });

        self.show_out(ui, "apt_sources_audit", 200.0);
    }
}
