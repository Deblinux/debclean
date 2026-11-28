use crate::i18n::{t, tf, tr};
use crate::{ui, util, App, Job, Msg, ProgressReporter};
use eframe::egui;
use std::fs;
use std::path::Path;
use crate::cleaner;

/// Salva (una sola volta) governor e profilo energetico originali, per poterli ripristinare.
pub const POWER_SAVE_ORIG: &str = "[ -f /var/lib/debutility/power_orig ] || { mkdir -p /var/lib/debutility; \
for f in power_orig epp_orig power_profile_orig; do \
  if [ ! -f \"/var/lib/debutility/$f\" ] && [ -f \"/var/lib/debclean/$f\" ]; then cp \"/var/lib/debclean/$f\" \"/var/lib/debutility/$f\" 2>/dev/null || true; fi; \
done; \
[ -r /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor ] && cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor > /var/lib/debutility/power_orig 2>/dev/null || :; \
[ -r /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference ] && cat /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference > /var/lib/debutility/epp_orig 2>/dev/null || :; \
command -v powerprofilesctl >/dev/null 2>&1 && powerprofilesctl get > /var/lib/debutility/power_profile_orig 2>/dev/null || :; }; ";
///////-------------------------
const SYSFS_GOV_BALANCED: &str = "avail=/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors; \
g=\"\"; \
if [ -r \"$avail\" ]; then \
  for c in schedutil ondemand conservative powersave performance; do \
    if grep -qw \"$c\" \"$avail\"; then g=\"$c\"; break; fi; \
  done; \
else \
  g=\"schedutil\"; \
fi; \
if [ -n \"$g\" ]; then \
  for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do \
    [ -e \"$f\" ] && [ -w \"$f\" ] && { printf '%s\n' \"$g\" > \"$f\" 2>/dev/null || true; }; \
  done; \
fi; \
for f in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do \
  [ -e \"$f\" ] && [ -w \"$f\" ] && { printf '%s\n' balance_performance > \"$f\" 2>/dev/null || true; }; \
done";

const SYSFS_GOV_PERF: &str = "for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do \
  [ -e \"$f\" ] && [ -w \"$f\" ] && { printf '%s\n' performance > \"$f\" 2>/dev/null || true; }; \
done; \
for f in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do \
  [ -e \"$f\" ] && [ -w \"$f\" ] && { printf '%s\n' performance > \"$f\" 2>/dev/null || true; }; \
done";

const SYSFS_GOV_SAVE: &str = "for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do \
  [ -e \"$f\" ] && [ -w \"$f\" ] && { printf '%s\n' powersave > \"$f\" 2>/dev/null || true; }; \
done; \
for f in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do \
  [ -e \"$f\" ] && [ -w \"$f\" ] && { printf '%s\n' power > \"$f\" 2>/dev/null || true; }; \
done";

/////-------------
const SERVICES: [(&str, &str); 10] = [
    ("NetworkManager", "Gestisce la rete e il Wi-Fi"),
    ("networking", "Rete classica di Debian (file /etc/network/interfaces)"),
    ("ssh", "Accesso remoto al computer tramite SSH"),
    ("bluetooth", "Supporto Bluetooth"),
    ("ufw", "Firewall: protegge il PC dalla rete"),
    ("cups", "Gestione delle stampanti"),
    ("avahi-daemon", "Scoperta automatica dei dispositivi in rete"),
    ("cron", "Esecuzione di attività programmate"),
    ("docker", "Contenitori Docker"),
    ("smbd", "Condivisione file di Windows (Samba)"),
];

#[derive(Clone)]
pub struct Found {
    pub path: String,
    pub sel: bool,
}

fn read_trim(p: &str) -> String {
    fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn battery() -> Option<(String, String)> {
    for e in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("BAT") {
            let base = format!("/sys/class/power_supply/{name}");
            let cap = read_trim(&format!("{base}/capacity"));
            let status = read_trim(&format!("{base}/status"));
            return Some((cap, status));
        }
    }
    None
}
#[derive(Clone, Debug)]
struct BatteryHealth {
    name: String,
    health_pct: Option<f64>,
    design_wh: Option<f64>,
    full_wh: Option<f64>,
    cycles: Option<u64>,
}

fn read_u64_trim(p: &str) -> Option<u64> {
    fs::read_to_string(p).ok()?.trim().parse::<u64>().ok()
}

fn battery_health() -> Option<BatteryHealth> {
    for e in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("BAT") {
            let base = format!("/sys/class/power_supply/{name}");
            let cycles = read_u64_trim(&format!("{base}/cycle_count")).filter(|&c| c > 0);
            let volt_v = read_u64_trim(&format!("{base}/voltage_now"))
                .or_else(|| read_u64_trim(&format!("{base}/voltage_min_design")))
                .map(|v| v as f64 / 1_000_000.0);

            let (design_wh, full_wh) = if let (Some(ed), Some(ef)) = (
                read_u64_trim(&format!("{base}/energy_full_design")),
                read_u64_trim(&format!("{base}/energy_full")),
            ) {
                (Some(ed as f64 / 1_000_000.0), Some(ef as f64 / 1_000_000.0))
            } else if let (Some(cd), Some(cf), Some(v)) = (
                read_u64_trim(&format!("{base}/charge_full_design")),
                read_u64_trim(&format!("{base}/charge_full")),
                volt_v,
            ) {
                (
                    Some((cd as f64 / 1_000_000.0) * v),
                    Some((cf as f64 / 1_000_000.0) * v),
                )
            } else {
                (None, None)
            };

            let health_pct = match (design_wh, full_wh) {
                (Some(d), Some(f)) if d > 0.0 => Some(((f / d) * 100.0).clamp(0.0, 100.0)),
                _ => None,
            };

            return Some(BatteryHealth {
                name,
                health_pct,
                design_wh,
                full_wh,
                cycles,
            });
        }
    }
    None
}

fn find_list(script: &str, default_sel: bool) -> Vec<Found> {
    let (_, o) = util::run_shell(script, false);
    o.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| Found { path: l.to_string(), sel: default_sel })
        .collect()
}

pub(crate) fn scan_dirs(hidden: bool) -> Vec<Found> {
    let excl = if hidden { "" } else { "-not -path '*/.*'" };
    find_list(
        &format!(
            "find {} -xdev -mindepth 1 -type d -empty {excl} 2>/dev/null | head -3000",
            util::shq(&util::home())
        ),
        true,
    )
}

pub(crate) fn scan_files(hidden: bool) -> Vec<Found> {
    let excl = if hidden { "" } else { "-not -path '*/.*'" };
    find_list(
        &format!(
            "find {} -xdev -type f -size 0 {excl} -not -name '__init__.py' 2>/dev/null | head -3000",
            util::shq(&util::home())
        ),
        false,
    )
}

/// Rimozione condivisa tra la GUI e la CLI: valida i percorsi, elimina solo
/// cartelle vuote o file da 0 byte e restituisce (rimossi, non rimossi).
pub(crate) fn delete_empty_items<F: FnMut(usize, usize, &str)>(
    dirs: bool,
    items: Vec<String>,
    mut on_progress: F,
) -> (usize, usize) {
    let total = items.len();
    let home = util::home();
    let mut list = items;
    if dirs {
        // prima le cartelle più profonde
        list.sort_by_key(|p| std::cmp::Reverse(p.len()));
    }
    let (mut ok, mut err) = (0, 0);
    for p in &list {
        if !p.starts_with(&home) || p == &home {
            err += 1;
            continue;
        }
        let path = Path::new(p);
        let res = if dirs {
            fs::remove_dir(path)
        } else {
            match path.symlink_metadata() {
                Ok(m) if m.is_file() && m.len() == 0 => fs::remove_file(path),
                _ => Err(std::io::Error::new(std::io::ErrorKind::Other, "non più vuoto")),
            }
        };
        match res {
            Ok(_) => ok += 1,
            Err(_) => err += 1,
        }
        on_progress(ok + err, total, p.as_str());
    }
    let done = tf("Eliminati {} elementi", &[&ok]);
    on_progress(list.len(), total, &done);
    (ok, err)
}

fn delete_job(dirs: bool, hidden: bool, items: Vec<String>) -> Job {
    Box::new(move |ch| {
        let mut prog = ProgressReporter::start(
            &ch,
            "adv_clean",
            t("Eliminazione elementi selezionati in corso..."),
        );
        let (ok, err) = delete_empty_items(dirs, items, |cur, total, p| prog.step(cur, total, p));
        ch.send(Msg::Log(format!("✔ Eliminati {ok} elementi ({err} non rimossi)")));
        prog.set(0.60, t("Nuova scansione in corso..."));
        if dirs {
            ch.send(Msg::AdvDirs(scan_dirs(hidden)));
        } else {
            ch.send(Msg::AdvFiles(scan_files(hidden)));
        }
        prog.finish(t("Eliminazione completata"));
    })
}

impl App {
    pub(crate) fn ui_cleaner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Pulizia");
        ui::sub_tabs(
            ui,
            &mut self.sub,
            &[
                "Pulizia",
                "Pulizia avanzata",
                "Risparmio energetico",
                "Servizi di sistema",
                "Ottimizzazione performance",
                "Log sistema",
            ],
        );
        self.snapshot_toggle(ui);
        match self.sub {
            0 => self.clean_basic(ui, ctx),
            1 => self.clean_advanced(ui, ctx),
            2 => self.power_ui(ui, ctx),
            3 => self.services_ui(ui, ctx),
            4 => self.perf_ui(ui, ctx),
            _ => self.syslog_ui(ui, ctx),
        }
    }

    // ------------------------------------------------------------ Pulizia avanzata
    fn clean_advanced(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            egui::RichText::new(t("Trova e rimuovi le cartelle vuote e i file da 0 byte nella tua cartella personale. Prima di eliminare vedrai sempre l'elenco e dovrai confermare."))
                .size(17.0),
        );
        ui.checkbox(&mut self.adv_hidden, t("Includi anche file e cartelle nascosti (meglio lasciare disattivato)"));
        ui.add_space(3.0);
        self.found_card(ui, ctx, true);
        ui.add_space(4.0);
        self.found_card(ui, ctx, false);
        ui.add_space(4.0);
        self.rc_card(ui, ctx);
        self.flatpak_card(ui, ctx);
        self.orphan_configs_card(ui, ctx);
        
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Ottimizza database SQLite browser")).strong().size(20.0));
            ui.label(t("Esegue VACUUM e REINDEX sui database interni dei browser (Firefox, Chrome, Chromium, Brave) verificando prima che siano chiusi. Velocizza l'avvio e la barra degli indirizzi senza cancellare cronologia o segnalibri."));
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Ottimizza database SQLite browser"))).clicked() {
                self.spawn(ctx, |ch| {
                    let mut prog = ProgressReporter::start(
                        &ch,
                        "adv_clean",
                        t("Ottimizzazione database browser in corso..."),
                    );
                    ch.send(Msg::Log("▶ Avvio ottimizzazione database SQLite dei browser...".into()));
                    for line in cleaner::optimize_browser_databases() {
                        prog.status(line.clone());
                        ch.send(Msg::Log(line));
                    }
                    prog.finish(t("Ottimizzazione completata"));
                });
            }
            if self.busy {
                ui.spinner();
            }
        });
    }

    fn found_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, dirs: bool) {
        let (title, desc, warn) = if dirs {
            (
                "Cartelle vuote",
                "Cartelle che non contengono nulla. Eliminarle non fa perdere dati e rende l'archivio più ordinato.",
                "",
            )
        } else {
            (
                "File vuoti (0 byte)",
                "File che non contengono dati. Spesso sono residui di operazioni interrotte.",
                "Attenzione: alcuni programmi usano file vuoti di proposito. Controlla l'elenco e deseleziona quelli che vuoi tenere.",
            )
        };
        let hidden = self.adv_hidden;
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t(title)).strong().size(20.0));
            ui.label(t(desc));
            if !warn.is_empty() {
                ui.colored_label(ui::semantic_text(ui::WARN), t(warn));
            }
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Cerca"))).clicked() {
                self.spawn(ctx, move |ch| {
                    let mut prog = ProgressReporter::start(
                        &ch,
                        "adv_clean",
                        t("Ricerca di file e cartelle vuote in corso..."),
                    );
                    prog.status(if dirs { t("Ricerca di cartelle vuote...") } else { t("Ricerca di file vuoti...") });
                    let mut l = if dirs { scan_dirs(hidden) } else { scan_files(hidden) };
                    if dirs {
                        let paths: std::collections::HashSet<String> =
                            l.iter().map(|f| f.path.clone()).collect();
                        l.retain(|f| {
                            let mut cur = Path::new(&f.path).parent();
                            while let Some(p) = cur {
                                if paths.contains(p.to_string_lossy().as_ref()) {
                                    return false;
                                }
                                cur = p.parent();
                            }
                            true
                        });
                    }
                    ch.send(Msg::Log(format!("✔ Trovati {} elementi", l.len())));
                    prog.finish(t("Ricerca completata"));
                    ch.send(if dirs { Msg::AdvDirs(l) } else { Msg::AdvFiles(l) });
                });
            }
            
            if self.busy {
                ui.spinner();
            }
        });

        let home = util::home();
        let list = if dirs { &mut self.adv_dirs } else { &mut self.adv_files };
        let selected: Vec<String> = list.iter().filter(|f| f.sel).map(|f| f.path.clone()).collect();
        ui.horizontal(|ui| {
            ui.label(format!("{}: {} / {}", t("Selezionati"), selected.len(), list.len()));
            if ui.small_button(t("Seleziona tutto")).clicked() {
                for f in list.iter_mut() {
                    f.sel = true;
                }
            }
            if ui.small_button(t("Deseleziona tutto")).clicked() {
                for f in list.iter_mut() {
                    f.sel = false;
                }
            }
        });
        egui::ScrollArea::vertical().id_source(if dirs { "adv_d" } else { "adv_f" }).max_height(165.0).show(ui, |ui| {
            for f in list.iter_mut().take(500) {
                let rel = f.path.strip_prefix(&home).unwrap_or(&f.path).to_string();
                ui.checkbox(&mut f.sel, format!("~{rel}"));
            }
        });

        let n = selected.len();
        if ui::primary_button(ui, t("Elimina selezionati"), self.action_enabled() && n > 0).clicked() {
            let mut txt = format!(
                "{} {}\n\n",
                n,
                t(if dirs {
                    "cartelle vuote verranno eliminate definitivamente:"
                } else {
                    "file vuoti verranno eliminati definitivamente:"
                })
            );
            for p in selected.iter().take(12) {
                txt.push_str(&format!("• {p}\n"));
            }
            if n > 12 {
                txt.push_str(&format!("… +{}\n", n - 12));
            }
            txt.push_str(&format!("\n{}", t("Confermi l'eliminazione?")));
            self.confirm_snapshot(txt, delete_job(dirs, hidden, selected));
        }
    }

fn rc_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Residui di configurazione pacchetti (rc)")).strong().size(20.0));
            ui.label(t("Pacchetti già disinstallati in passato che mantengono vecchi file di configurazione sul disco. La loro rimozione pulisce il sistema in sicurezza."));
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔍 Cerca residui rc"))).clicked() {
                self.spawn(ctx, move |ch| {
                    let prog = ProgressReporter::start(
                        &ch,
                        "adv_clean",
                        t("Ricerca pacchetti residui in corso..."),
                    );
                    let (_, output) = util::run_shell("dpkg -l | awk '/^rc/ {print $2}'", false);
                    let list: Vec<(String, bool)> = output.lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .map(|s| (s, true))
                        .collect();
                    ch.send(Msg::Log(format!("✔ Trovati {} pacchetti rc", list.len())));
                    prog.finish(t("Ricerca completata"));
                    ch.send(Msg::RcPkgs(list));
                });
            }
            if self.busy {
                ui.spinner();
            }
        });

        if !self.rc_pkgs.is_empty() {
            ui.horizontal(|ui| {
                let count = self.rc_pkgs.iter().filter(|(_, sel)| *sel).count();
                ui.label(format!("{}: {} / {}", t("Selezionati"), count, self.rc_pkgs.len()));
                if ui.small_button(t("Seleziona tutto")).clicked() {
                    for (_, sel) in &mut self.rc_pkgs {
                        *sel = true;
                    }
                }
                if ui.small_button(t("Deseleziona tutto")).clicked() {
                    for (_, sel) in &mut self.rc_pkgs {
                        *sel = false;
                    }
                }
            });

            egui::ScrollArea::vertical().id_source("rc_pkgs_scroll").max_height(155.0).show(ui, |ui| {
                for (pkg, sel) in &mut self.rc_pkgs {
                    ui.checkbox(sel, pkg.clone());
                }
            });

            let count = self.rc_pkgs.iter().filter(|(_, sel)| *sel).count();
            if ui::primary_button(ui, t("Elimina selezionati"), self.action_enabled() && count > 0).clicked() {
                let pkgs: Vec<String> = self.rc_pkgs.iter().filter(|(_, sel)| *sel).map(|(p, _)| p.clone()).collect();
                let joined = pkgs.join(" ");
                let script = format!("apt-get purge -y {joined} && echo 'Pulizia pacchetti rc completata'");
                let count_str = count.to_string();
                let job: Job = Box::new(move |ch| {
                    let mut prog = ProgressReporter::start(
                        &ch,
                        "adv_clean",
                        t("Purgatura pacchetti residui in corso..."),
                    );
                    ch.send(Msg::Log(format!("▶ Purgatura di {} pacchetti rc...", pkgs.len())));
                    let (ok, out) = util::run_shell(&script, true);
                    let mark = if ok { "✔" } else { "✘" };
                    ch.send(Msg::Log(format!("{mark} {}", util::tail(out.trim(), 3).replace('\n', " | "))));
                    prog.status(t("Aggiornamento elenco residui..."));
                    let (_, output) = util::run_shell("dpkg -l | awk '/^rc/ {print $2}'", false);
                    let list: Vec<(String, bool)> = output.lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .map(|s| (s, true))
                        .collect();
                    prog.finish(t("Purgatura completata"));
                    ch.send(Msg::RcPkgs(list));
                });
                self.confirm_snapshot(tf("Eliminare definitivamente i file di configurazione di {} pacchetti?", &[&count_str]), job);
            }
        }
    }

fn flatpak_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Runtime Flatpak inutilizzati")).strong().size(20.0));
            ui.label(t("Librerie e ambienti di esecuzione di Flatpak che non sono più necessari ad alcuna applicazione installata. Rimuoverli libera spazio su disco in sicurezza."));
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔍 Cerca runtime Flatpak inutilizzati"))).clicked() {
                self.spawn(ctx, move |ch| {
                    let prog = ProgressReporter::start(
                        &ch,
                        "adv_clean",
                        t("Ricerca runtime Flatpak inutilizzati in corso..."),
                    );
                    let (_, output) = util::run_shell("flatpak --version >/dev/null 2>&1 && flatpak uninstall --unused --assumeno 2>/dev/null || echo ''", false);
                    let list: Vec<(String, bool)> = output.lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty() && (s.contains('/') || s.starts_with("org.") || s.starts_with("com.")))
                        .map(|s| (s, true))
                        .collect();
                    ch.send(Msg::Log(format!("✔ Trovati {} elementi Flatpak inutilizzati", list.len())));
                    prog.finish(t("Ricerca completata"));
                    ch.send(Msg::FlatpakUnused(list));
                });
            }
            if self.busy {
                ui.spinner();
            }
        });

        if !self.flatpak_unused.is_empty() {
            ui.horizontal(|ui| {
                let count = self.flatpak_unused.iter().filter(|(_, sel)| *sel).count();
                ui.label(format!("{}: {} / {}", t("Selezionati"), count, self.flatpak_unused.len()));
                if ui.small_button(t("Seleziona tutto")).clicked() {
                    for (_, sel) in &mut self.flatpak_unused { *sel = true; }
                }
                if ui.small_button(t("Deseleziona tutto")).clicked() {
                    for (_, sel) in &mut self.flatpak_unused { *sel = false; }
                }
            });

            egui::ScrollArea::vertical().id_source("flatpak_scroll").max_height(145.0).show(ui, |ui| {
                for (pkg, sel) in &mut self.flatpak_unused {
                    ui.checkbox(sel, pkg.clone());
                }
            });

            let count = self.flatpak_unused.iter().filter(|(_, sel)| *sel).count();
            if ui::primary_button(ui, t("Disinstalla selezionati"), self.action_enabled() && count > 0).clicked() {
                let pkgs: Vec<String> = self.flatpak_unused.iter().filter(|(_, sel)| *sel).map(|(p, _)| p.clone()).collect();
                let joined = pkgs.join(" ");
                let script = format!("flatpak uninstall -y {joined} && echo 'Pulizia Flatpak completata'");
                let count_str = count.to_string();
                let job: Job = Box::new(move |ch| {
                    let mut prog = ProgressReporter::start(
                        &ch,
                        "adv_clean",
                        t("Rimozione runtime Flatpak in corso..."),
                    );
                    ch.send(Msg::Log(format!("▶ Rimozione di {} elementi Flatpak...", pkgs.len())));
                    let (ok, out) = util::run_shell(&script, true);
                    let mark = if ok { "✔" } else { "✘" };
                    ch.send(Msg::Log(format!("{mark} {}", util::tail(out.trim(), 3).replace('\n', " | "))));
                    prog.status(t("Aggiornamento elenco runtime inutilizzati..."));
                    let (_, output) = util::run_shell("flatpak uninstall --unused --assumeno 2>/dev/null || echo ''", false);
                    let list: Vec<(String, bool)> = output.lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty() && (s.contains('/') || s.starts_with("org.") || s.starts_with("com.")))
                        .map(|s| (s, true))
                        .collect();
                    prog.finish(t("Rimozione completata"));
                    ch.send(Msg::FlatpakUnused(list));
                });
                self.confirm_snapshot(tf("Disinstallare permanentemente questi {} elementi Flatpak?", &[&count_str]), job);
            }
        }
    }

fn orphan_configs_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Configurazioni utente orfane")).strong().size(20.0));
            ui.label(t("Cartelle di configurazione in ~/.config appartenenti a programmi che non sono più installati sul computer."));
            ui.colored_label(ui::semantic_text(ui::WARN), t("Attenzione: eliminare una cartella in ~/.config può far perdere impostazioni o preferiti dell'applicazione."));
        });
        
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔍 Cerca configurazioni orfane"))).clicked() {
                self.spawn(ctx, move |ch| {
                    let mut prog = ProgressReporter::start(
                        &ch,
                        "adv_clean",
                        t("Ricerca configurazioni orfane in corso..."),
                    );
                    let home = util::home();
                    let config_dir = format!("{home}/.config");
                    let mut found = Vec::new();
                    let mut checked = 0usize;
                    
                    if let Ok(entries) = std::fs::read_dir(config_dir) {
                        let entries: Vec<_> = entries.flatten().collect();
                        let total = entries.len();
                        for entry in entries {
                            checked += 1;
                            prog.step(checked, total, tf("Verifica di {}", &[&entry.file_name().to_string_lossy()]));
                            if let Ok(name) = entry.file_name().into_string() {
                                let path = entry.path();
                                if path.is_dir() {
                                    let check_script = format!(
                                        "dpkg -l {name} 2>/dev/null | grep -q '^ii' || command -v {name} >/dev/null 2>&1 || flatpak list --app 2>/dev/null | grep -qi {name}",
                                        name = util::shq(&name)
                                    );
                                    let (installed, _) = util::run_shell(&check_script, false);
                                    if !installed {
                                        found.push((path, false)); // PathBuf preservato correttamente
                                    }
                                }
                            }
                        }
                    }
                    let _ = ch.send(Msg::Log(format!("✔ Trovate {} cartelle di configurazione orfane", found.len())));
                    prog.finish(t("Ricerca completata"));
                    let _ = ch.send(Msg::OrphanConfigs(found));
                });
            }
            if self.busy {
                ui.spinner();
            }
        });

        if !self.orphan_configs.is_empty() {
            ui.horizontal(|ui| {
                let count = self.orphan_configs.iter().filter(|(_, sel)| *sel).count();
                ui.label(format!("{}: {} / {}", t("Selezionati"), count, self.orphan_configs.len()));
                if ui.small_button(t("Seleziona tutto")).clicked() {
                    for (_, sel) in &mut self.orphan_configs { *sel = true; }
                }
                if ui.small_button(t("Deseleziona tutto")).clicked() {
                    for (_, sel) in &mut self.orphan_configs { *sel = false; }
                }
            });

            egui::ScrollArea::vertical().id_source("orphan_configs_scroll").max_height(145.0).show(ui, |ui| {
                for (path, sel) in &mut self.orphan_configs {
                    ui.checkbox(sel, path.to_string_lossy().as_ref()); // Conversione PathBuf -> &str per la checkbox
                }
            });

            let count = self.orphan_configs.iter().filter(|(_, sel)| *sel).count();
            if ui::primary_button(ui, t("Elimina selezionati"), self.action_enabled() && count > 0).clicked() {
                let paths: Vec<std::path::PathBuf> = self.orphan_configs.iter().filter(|(_, sel)| *sel).map(|(p, _)| p.clone()).collect(); // Vettore di PathBuf
                let count_str = count.to_string();
                let job: Job = Box::new(move |ch| {
                    let mut ok_count = 0;
                    for p in &paths {
                        if std::fs::remove_dir_all(p).is_ok() {
                            ok_count += 1;
                        }
                    }
                    let _ = ch.send(Msg::Log(format!("✔ Eliminate {ok_count} cartelle di configurazione orfane")));
                    let _ = ch.send(Msg::OrphanConfigs(Vec::new()));
                });
                self.confirm_snapshot(tf("Eliminare definitivamente le cartelle di configurazione di questi {} elementi?", &[&count_str]), job);
            }
        }
    }

    // ------------------------------------------------------------ Risparmio energetico
    fn power_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            egui::RichText::new(t("Allunga la durata della batteria del notebook scegliendo come il processore usa l'energia. Funziona solo sui portatili: un PC fisso non ha la batteria."))
                .size(17.0),
        );
        ui.add_space(3.0);
        let (cap, status) = match battery() {
            Some(b) => b,
            None => {
                ui::card(ui, |ui| {
                    ui.label(
                        egui::RichText::new(t("Questo computer non ha una batteria (è un PC fisso). Il risparmio energetico per la batteria è disponibile solo sui notebook."))
                            .size(18.0)
                            .color(ui::semantic_text(ui::WARN)),
                    );
                });
                return;
            }
        };
        let st_key = match status.as_str() {
            "Charging" => "In carica",
            "Discharging" => "Scarica",
            "Full" => "Carica completa",
            _ => "Stato sconosciuto",
        };
        let frac = cap.parse::<f32>().unwrap_or(0.0) / 100.0;
        ui::card(ui, |ui| {
            ui.add(ui::bar(frac.clamp(0.0, 1.0)).text(format!("{} {}% — {}", t("Batteria"), cap, tr(st_key))));
            let gov = read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor");
            if !gov.is_empty() {
                ui.label(format!("{}: {}", t("Modalità attuale del processore"), gov));
            }
        });
        
        if let Some(bh) = battery_health() {
            ui.add_space(3.0);
            ui::card(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("{} ({})", t("Salute e usura batteria"), bh.name))
                        .strong()
                        .size(18.0),
                );
                if let Some(h) = bh.health_pct {
                    let (giudizio, col) = if h >= 85.0 {
                        (t("Ottima"), ui::GOOD)
                    } else if h >= 70.0 {
                        (t("Discreta / normale usura"), ui::WARN)
                    } else {
                        (t("Usurata o degradata (es. fermo in magazzino)"), egui::Color32::from_rgb(230, 70, 70))
                    };
                    ui.colored_label(
                        col,
                        format!("{}: {:.1}% — {}", t("Salute reale"), h, giudizio),
                    );
                }
                if let (Some(d), Some(f)) = (bh.design_wh, bh.full_wh) {
                    ui.label(format!(
                        "{}: {:.2} Wh / {:.2} Wh {}",
                        t("Capacità massima"),
                        f,
                        d,
                        t("originali")
                    ));
                }
                if let Some(c) = bh.cycles {
                    ui.label(format!("{}: {}", t("Cicli di carica"), c));
                }
            });
        }
        
        ui.add_space(3.0);

        let pp = |profile: &str, sysfs: &str| -> String {
            format!(
                "{POWER_SAVE_ORIG}if command -v powerprofilesctl >/dev/null 2>&1 && powerprofilesctl set {profile} 2>/dev/null; then :; else {sysfs}; fi"
            )
        };
        let profiles: Vec<(&'static str, &'static str, String)> = vec![
            (
                "Bilanciato (standard)",
                "Il computer regola da solo velocità e consumi: veloce quando serve, risparmia quando non fai nulla. È la scelta giusta per l'uso di tutti i giorni.",
                pp("balanced", SYSFS_GOV_BALANCED) + "; echo 'OK'",
            ),
            (
                "Performance",
                "Massima velocità sempre. Utile per giochi, montaggio video o lavori pesanti, ma la batteria si scarica molto più in fretta e il computer si scalda e fa più rumore. Meglio usarla con il caricabatterie collegato.",
                pp("performance", SYSFS_GOV_PERF) + "; echo 'OK'",
            ),
            (
                "Risparmio energetico (Powersave)",
                "Riduce la velocità del processore per consumare meno. La batteria dura di più e il computer resta più fresco, ma i programmi pesanti possono essere più lenti. Ideale quando sei in viaggio.",
                pp("power-saver", SYSFS_GOV_SAVE) + "; echo 'OK'",
            ),
            (
                "Origine",
                "Ripristina le impostazioni che il computer aveva prima di usare DebUtility.",
                format!(
                    "ORIG_DIR=/var/lib/debutility; \
                     if [ ! -f \"$ORIG_DIR/power_orig\" ] && [ ! -s \"$ORIG_DIR/power_profile_orig\" ] && [ ! -s \"$ORIG_DIR/epp_orig\" ] && [ -d /var/lib/debclean ]; then ORIG_DIR=/var/lib/debclean; fi; \
                     if [ -f \"$ORIG_DIR/power_orig\" ] || [ -s \"$ORIG_DIR/power_profile_orig\" ] || [ -s \"$ORIG_DIR/epp_orig\" ]; then \
                       if [ -s \"$ORIG_DIR/power_profile_orig\" ] && command -v powerprofilesctl >/dev/null 2>&1; then \
                         powerprofilesctl set \"$(cat \"$ORIG_DIR/power_profile_orig\")\" 2>/dev/null || true; \
                       fi; \
                       if [ -s \"$ORIG_DIR/power_orig\" ]; then \
                         orig_gov=\"$(cat \"$ORIG_DIR/power_orig\")\"; \
                         for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do \
                           [ -e \"$f\" ] && [ -w \"$f\" ] && {{ printf '%s\\n' \"$orig_gov\" > \"$f\" 2>/dev/null || true; }}; \
                         done; \
                       fi; \
                       if [ -s \"$ORIG_DIR/epp_orig\" ]; then \
                         orig_epp=\"$(cat \"$ORIG_DIR/epp_orig\")\"; \
                         for f in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do \
                           [ -e \"$f\" ] && [ -w \"$f\" ] && {{ printf '%s\\n' \"$orig_epp\" > \"$f\" 2>/dev/null || true; }}; \
                         done; \
                       fi; \
                       echo 'OK'; \
                     else echo 'Nessuna impostazione originale salvata: non hai ancora cambiato profilo.'; fi"
                ),
            ),
        ];
        for (title, desc, script) in profiles {
            ui::card(ui, |ui| {
                ui.label(egui::RichText::new(t(title)).strong().size(20.0));
                ui.label(t(desc));
                ui.add_space(2.0);
                self.run_btn(ui, ctx, t("Applica"), "power", &script, true, None);
            });
            ui.add_space(3.0);
        }
        ui.label(egui::RichText::new(t("Le impostazioni valgono fino al riavvio del computer.")).color(ui::text_dim()));
        self.show_out(ui, "power", 100.0);
    }

    // ------------------------------------------------------------ Servizi di sistema
    fn services_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Cosa sono i servizi")).strong().size(20.0));
            ui.label(t("I servizi (o «demoni») sono programmi che girano sempre in background e gestiscono le funzioni chiave del sistema: la rete, il server SSH, il Bluetooth, il firewall."));
            ui.add_space(2.0);
            ui.label(t("Verifica stato: controlla se il servizio è attivo (running) o fermo (dead), senza modificare nulla."));
            ui.label(t("Avvia: attiva subito il servizio per la sessione corrente (richiede privilegi di amministratore)."));
            ui.label(t("Arresta: ferma il servizio in tempo reale per liberare memoria o interrompere una funzione (richiede privilegi di amministratore)."));
            ui.add_space(2.0);
            ui.colored_label(
                ui::WARN,
                t("Attenzione: fermare servizi critici come networking o NetworkManager può disconnettere temporaneamente il PC da Internet."),
            );
        });
        ui.add_space(3.0);
        egui::ComboBox::from_label(t("Servizi comuni"))
            .selected_text(self.svc_name.clone())
            .width(ui::fit_width(ui, 420.0, 160.0))
            .show_ui(ui, |ui| {
                for (n, d) in SERVICES {
                    if ui.selectable_label(self.svc_name == n, format!("{n} — {}", t(d))).clicked() {
                        self.svc_name = n.to_string();
                    }
                }
            });
        ui.horizontal(|ui| {
            ui.label(t("Oppure scrivi il nome:"));
            ui.text_edit_singleline(&mut self.svc_name);
        });
        let name = self.svc_name.trim().to_string();
        let valid = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "@._-".contains(c));
        let q = util::shq(&name);
        let state = self.out.get("svc").and_then(|o| {
            o.lines()
                .map(|l| l.trim())
                .find(|l| matches!(*l, "active" | "inactive" | "failed" | "activating" | "deactivating" | "unknown"))
                .map(|l| l.to_string())
        });
        match state.as_deref() {
            Some("active") => ui::led(ui, Some(true), t("Servizio attivo (running)")),
            Some(_) => ui::led(ui, Some(false), t("Servizio fermo (dead)")),
            None => ui::led(ui, None, t("Stato sconosciuto: premi «Verifica stato»")),
        }
        ui.add_enabled_ui(valid, |ui| {
            ui.horizontal_wrapped(|ui| {
                self.run_btn(
                    ui, ctx, t("Verifica stato"), "svc",
                    &format!("systemctl is-active {q}; echo; systemctl status {q} --no-pager -l 2>&1 | head -15"),
                    false, None,
                );
                self.run_btn(
                    ui, ctx, t("Avvia servizio"), "svc",
                    &format!("systemctl start {q}; systemctl is-active {q}"),
                    true, None,
                );
                self.run_btn(
                    ui, ctx, t("Arresta servizio"), "svc",
                    &format!("systemctl stop {q}; systemctl is-active {q}"),
                    true,
                    Some(t("Arrestare questo servizio? Se è un servizio critico potresti perdere la connessione a Internet.")),
                );
            });
        });
        self.show_out(ui, "svc", 260.0);
    }

    // ------------------------------------------------------------ Ottimizzazione performance
    fn perf_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card(ui, |ui| {
            ui.label(t("Analizza i punti che possono rallentare il sistema e intervieni con i pulsanti qui sotto."));
            ui.add_space(2.0);
            ui.label(t("Analisi avvio: mostra i servizi che impiegano più tempo a caricarsi all'accensione."));
            ui.label(t("Gestione swap: regola la «swappiness», cioè quanto il sistema usa il disco al posto della RAM."));
            ui.label(t("Manutenzione disco: esegue TRIM (solo SSD) per mantenere alte le velocità di lettura e scrittura."));
            ui.label(t("Processi pesanti: mostra le applicazioni che consumano più CPU e RAM."));
            ui.label(t("Ottimizza tutto: manutenzione completa automatica (TRIM, pulizia cache, ottimizzazione memoria)."));
            ui.add_space(2.0);
            ui.colored_label(ui::semantic_text(ui::WARN), t("Attenzione: alcune operazioni richiedono privilegi di amministratore."));
        });
        ui.add_space(3.0);
        ui.horizontal_wrapped(|ui| {
            self.run_btn(
                ui, ctx, t("Analisi avvio"), "perf",
                "systemd-analyze 2>&1; echo; systemd-analyze blame 2>&1 | head -15",
                false, None,
            );
            self.run_btn(
                ui, ctx, t("Trova processi pesanti"), "perf",
                "ps -eo pid,user:12,pcpu,pmem,comm --sort=-pcpu | head -12; echo; ps -eo pid,user:12,pcpu,pmem,comm --sort=-pmem | head -8",
                false, None,
            );
            self.run_btn(
                ui, ctx, t("Manutenzione disco (TRIM)"), "perf",
                "fstrim -av 2>&1",
                true, None,
            );
        });
        ui.add_space(3.0);
        ui.label(egui::RichText::new(t("Gestione swap")).strong().size(18.0));
        ui.add(egui::Slider::new(&mut self.swappiness, 0..=100).text("vm.swappiness"));
        ui.label(egui::RichText::new(t("Valori bassi usano di più la RAM; valori alti spostano prima i dati sul disco. 10-30 è ideale per la maggior parte dei PC desktop.")).color(ui::text_dim()));
        let n = self.swappiness;
        let swap = format!(
            "sysctl -w vm.swappiness={n} && echo vm.swappiness={n} > /etc/sysctl.d/99-debutility.conf && echo 'OK'"
        );
        let total_gb = self.info.sys.total_memory() / 1024 / 1024 / 1024;
        let tune = if total_gb >= 8 { 10 } else { 30 };
        let all = format!(
            "fstrim -av 2>&1; sync; echo 3 > /proc/sys/vm/drop_caches; sysctl -w vm.swappiness={tune}; sysctl -w vm.vfs_cache_pressure=50; apt-get clean; echo 'OK'"
        );
        ui.horizontal_wrapped(|ui| {
            self.run_btn(ui, ctx, t("Applica swappiness"), "perf", &swap, true, None);
            ui.add_space(5.0);
            self.run_btn(
                ui, ctx, t("Ottimizza tutto"), "perf", &all, true,
                Some(t("Eseguire la manutenzione completa (TRIM, pulizia cache, ottimizzazione memoria)?")),
            );
        });
        self.show_out(ui, "perf", 320.0);
    }

    // ------------------------------------------------------------ Log di sistema
    fn syslog_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            egui::RichText::new(t("Mostra gli ultimi eventi e messaggi di sistema registrati da journalctl. Utile per diagnosticare eventuali problemi."))
                .size(17.0),
        );
        ui.checkbox(&mut self.log_root, t("Usa i privilegi di amministratore (mostra tutti i messaggi)"));
        ui.horizontal(|ui| {
            ui.label(t("Filtro testo:"));
            ui.text_edit_singleline(&mut self.log_filter);
        });
        let filter = self.log_filter.trim().to_string();
        let tail = if filter.is_empty() { String::new() } else { format!(" | grep -i {}", util::shq(&filter)) };
        let root = self.log_root;
        let cmds: [(&'static str, String); 4] = [
            ("Ultimi 100 eventi", format!("journalctl -n 100 --no-pager 2>&1{tail}")),
            ("Solo errori", format!("journalctl -p err -b --no-pager 2>&1 | tail -n 150{tail}")),
            ("Avvio corrente", format!("journalctl -b --no-pager 2>&1 | tail -n 200{tail}")),
            ("Messaggi del kernel", format!("journalctl -k -n 150 --no-pager 2>&1{tail}")),
        ];
        ui.horizontal_wrapped(|ui| {
            for (label, script) in &cmds {
                self.run_btn(ui, ctx, t(*label), "syslog", script, root, None);
            }
        });
        self.show_out(ui, "syslog", 460.0);
        self.solutions_ui(ui, ctx, "syslog", false);
    }
}
