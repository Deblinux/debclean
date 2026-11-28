//! Gestione dei kernel installati e dello spazio nella partizione `/boot`.
//!
//! La scansione e la rimozione girano sempre in un thread di lavoro: la UI
//! riceve solo snapshot tramite `Chan`, usando la barra condivisa dell'app.

use crate::{cleaner, ui, util, App, Chan, Job, Msg, ProgressReporter};
use eframe::egui;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct KernelPackage {
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct KernelEntry {
    /// Versione normalizzata (senza suffisso architettura o `-common`).
    pub version: String,
    pub packages: Vec<KernelPackage>,
    /// Kernel in esecuzione o fallback di emergenza: mai selezionabile.
    pub protected: bool,
    /// Motivo della protezione, mostrato sotto il badge verde.
    pub protection: &'static str,
    pub selected: bool,
}

impl KernelEntry {
    pub fn size(&self) -> u64 {
        self.packages.iter().map(|p| p.size).sum()
    }

    pub fn has_image(&self) -> bool {
        self.packages.iter().any(|p| p.name.starts_with("linux-image-"))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub mount: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub running: String,
    pub fallback: String,
    pub kernels: Vec<KernelEntry>,
}

impl Report {
    pub fn selected_packages(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .kernels
            .iter()
            .filter(|k| k.selected && !k.protected)
            .flat_map(|k| k.packages.iter().map(|p| p.name.clone()))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn selected_size(&self) -> u64 {
        self.kernels
            .iter()
            .filter(|k| k.selected && !k.protected)
            .map(|k| k.size())
            .sum()
    }
}

// ---- Utility -----------------------------------------------------------------------------------

/// Confronto naturale: "6.1.0-9" risulta minore di "6.1.0-18".
fn cmp_natural(a: &str, b: &str) -> Ordering {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let (mut i, mut j) = (0usize, 0usize);
    while i < ab.len() && j < bb.len() {
        if ab[i].is_ascii_digit() && bb[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < ab.len() && ab[i].is_ascii_digit() {
                i += 1;
            }
            while j < bb.len() && bb[j].is_ascii_digit() {
                j += 1;
            }
            let na = &a[si..i];
            let nb = &b[sj..j];
            let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            if ab[i] != bb[j] {
                return ab[i].cmp(&bb[j]);
            }
            i += 1;
            j += 1;
        }
    }
    ab.len().cmp(&bb.len())
}

/// Rimuove il suffisso di architettura e `-common` per raggruppare
/// immagini e header della stessa versione kernel.
fn normalize_version(raw: &str) -> String {
    let mut v = raw.strip_suffix("-common").unwrap_or(raw);
    for arch in ["amd64", "i386", "i686", "arm64", "armhf", "ppc64el", "s390x", "riscv64"] {
        let suffix = format!("-{arch}");
        if let Some(stripped) = v.strip_suffix(&suffix) {
            v = stripped;
            break;
        }
    }
    v.to_string()
}

fn version_from_package(name: &str) -> Option<String> {
    let rest = name
        .strip_prefix("linux-image-")
        .or_else(|| name.strip_prefix("linux-headers-"))?;
    let rest = rest.strip_prefix("unsigned-").unwrap_or(rest);
    if !rest.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return None; // Esclude i metapacchetti (linux-image-amd64, ecc.).
    }
    Some(normalize_version(rest))
}

fn boot_usage() -> (String, u64, u64, u64) {
    let mount = if Path::new("/boot").is_dir() { "/boot" } else { "/" };
    let script = format!("df -P -B1 {} 2>/dev/null", util::shq(mount));
    let (ok, out) = util::run_shell(&script, false);
    if ok {
        if let Some(line) = out.lines().nth(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() >= 4 {
                let total = cols[1].parse().unwrap_or(0);
                let used = cols[2].parse().unwrap_or(0);
                let free = cols[3].parse().unwrap_or(0);
                return (mount.to_string(), total, used, free);
            }
        }
    }
    // Fallback per sistemi senza `df -B1`: valori leggibili dall'uomo.
    let (_, out) = util::run_shell(&format!("df -P -h {} 2>/dev/null", util::shq(mount)), false);
    let parse = |s: &str| -> Option<u64> {
        let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        let n: f64 = s[..split].parse().ok()?;
        let mult = match s[split..].trim().to_ascii_lowercase().as_str() {
            "k" | "kb" => 1024.0,
            "m" | "mb" => 1024f64.powi(2),
            "g" | "gb" => 1024f64.powi(3),
            "t" | "tb" => 1024f64.powi(4),
            _ => 1.0,
        };
        Some((n * mult) as u64)
    };
    if let Some(line) = out.lines().nth(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 4 {
            return (
                mount.to_string(),
                parse(cols[1]).unwrap_or(0),
                parse(cols[2]).unwrap_or(0),
                parse(cols[3]).unwrap_or(0),
            );
        }
    }
    (mount.to_string(), 0, 0, 0)
}

fn running_kernel() -> String {
    Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Scansione completa: `/boot`, pacchetti installati, protezioni e dimensioni.
pub fn scan_with<F: FnMut(f32, String)>(mut progress: F) -> Report {
    progress(0.08, crate::i18n::t("Lettura dello spazio su /boot...").to_string());
    let (mount, total, used, free) = boot_usage();

    progress(0.32, crate::i18n::t("Lettura dei pacchetti kernel installati...").to_string());
    let script = "dpkg-query -W -f='${Package}\\t${Version}\\t${Installed-Size}\\t${Status}\\n' 'linux-image-*' 'linux-headers-*' 2>/dev/null";
    let (_, out) = util::run_shell(script, false);
    let mut by_version: BTreeMap<String, Vec<KernelPackage>> = BTreeMap::new();
    for line in out.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 4 || !cols[3].contains("install ok installed") {
            continue;
        }
        let name = cols[0].trim();
        if name.is_empty() {
            continue;
        }
        let Some(version) = version_from_package(name) else { continue };
        let kb: u64 = cols[2].trim().parse().unwrap_or(0);
        by_version.entry(version).or_default().push(KernelPackage {
            name: name.to_string(),
            size: kb.saturating_mul(1024),
        });
    }

    progress(0.72, crate::i18n::t("Individuazione del kernel in uso e del fallback...").to_string());
    let running = running_kernel();
    let running_ver = normalize_version(&running);
    let mut versions: Vec<String> = by_version.keys().cloned().collect();
    versions.sort_by(|a, b| cmp_natural(b, a));

    // Il fallback è il kernel installato più recente diverso da quello in uso.
    let fallback = versions
        .iter()
        .find(|v| *v != &running_ver && by_version.get(*v).map_or(false, |p| p.iter().any(|x| x.name.starts_with("linux-image-"))))
        .cloned()
        .unwrap_or_default();

    let mut kernels = Vec::new();
    for version in versions {
        let Some(packages) = by_version.remove(&version) else { continue };
        let is_running = !running_ver.is_empty() && version == running_ver;
        let is_fallback = !fallback.is_empty() && version == fallback;
        kernels.push(KernelEntry {
            version,
            packages,
            protected: is_running || is_fallback,
            protection: if is_running {
                crate::i18n::t("Kernel attuale (uname -r)")
            } else {
                crate::i18n::t("Fallback di emergenza (secondo kernel più recente)")
            },
            selected: false,
        });
    }

    progress(0.95, crate::i18n::t("Scansione completata").to_string());
    Report {
        mount,
        total,
        used,
        free,
        running,
        fallback,
        kernels,
    }
}

// ---- UI ---------------------------------------------------------------------------------------

impl App {
    /// Avvia la scansione in background con avanzamento condiviso.
    pub(crate) fn kb_scan(&mut self, ctx: &egui::Context) {
        self.spawn(ctx, |ch| {
            let mut prog = ProgressReporter::start(
                &ch,
                "kernel_boot",
                crate::i18n::t("Scansione kernel e /boot in corso..."),
            );
            let report = scan_with(|frac, detail| {
                prog.set(frac, detail.as_str());
                ch.send(Msg::KernelBootProgress { fraction: frac, detail });
            });
            let packages: usize = report.kernels.iter().map(|k| k.packages.len()).sum();
            ch.send(Msg::Log(crate::i18n::tf(
                "✔ /boot ({}): {} usati, {} liberi su {}",
                &[
                    &report.mount,
                    &cleaner::fmt_bytes(report.used),
                    &cleaner::fmt_bytes(report.free),
                    &cleaner::fmt_bytes(report.total),
                ],
            )));
            ch.send(Msg::Log(crate::i18n::tf(
                "✔ {} pacchetti kernel trovati in {} versioni ({} protette)",
                &[
                    &packages.to_string(),
                    &report.kernels.len().to_string(),
                    &report.kernels.iter().filter(|k| k.protected).count().to_string(),
                ],
            )));
            ch.send(Msg::KernelBootScan(report));
            prog.finish(crate::i18n::t("Scansione kernel completata"));
        });
    }

    /// Avvia la rimozione dei kernel selezionati con snapshot dei pacchetti,
    /// conferma e aggiornamento della lista al termine.
    pub(crate) fn kb_remove_selected(&mut self, _ctx: &egui::Context) {
        let targets = self.kb_report.selected_packages();
        if targets.is_empty() {
            return;
        }
        let args = targets.iter().map(|p| util::shq(p)).collect::<Vec<_>>().join(" ");
        let script = format!("apt-get purge -y {args} && update-grub");
        let purge = crate::cmd_job("kernel_boot", script, true);
        let job: Job = Box::new(move |ch: &Chan| {
            purge(ch);
            let mut prog = ProgressReporter::start(
                ch,
                "kernel_boot",
                crate::i18n::t("Aggiornamento elenco kernel..."),
            );
            let fresh = scan_with(|frac, detail| {
                prog.set(0.85 + 0.14 * frac, detail.as_str());
                ch.send(Msg::KernelBootProgress {
                    fraction: 0.85 + 0.14 * frac,
                    detail,
                });
            });
            ch.send(Msg::KernelBootScan(fresh));
            prog.finish(crate::i18n::t("Elenco kernel aggiornato"));
        });
        let mut text = crate::i18n::t(
            "Rimuovere definitivamente i kernel selezionati e aggiornare GRUB? Il kernel attuale e il fallback resteranno installati.",
        )
        .to_string();
        text.push_str("\n\n");
        for name in targets.iter().take(14) {
            text.push_str(&crate::i18n::tf("• {}", &[&name]));
            text.push('\n');
        }
        if targets.len() > 14 {
            text.push_str(&crate::i18n::tf("… +{}", &[&(targets.len() - 14)]));
            text.push('\n');
        }
        self.confirm = Some((text, job));
    }

    pub(crate) fn ui_kernel_boot(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, crate::i18n::t("Kernel & Boot"));
        ui::hint(
            ui,
            crate::i18n::t(
                "Controlla lo spazio della partizione /boot e rimuovi solo i kernel vecchi. Il kernel in esecuzione e il secondo kernel più recente sono protetti e non possono essere selezionati.",
            ),
        );

        let busy = self.busy;
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!busy, egui::Button::new(crate::i18n::t("Scansiona kernel e /boot"))).clicked() {
                self.kb_scan(ctx);
            }
            if ui::primary_button(
                ui,
                crate::i18n::t("Rimuovi kernel selezionati"),
                !busy && self.kb_report.selected_packages().len() > 0,
            )
            .clicked()
            {
                self.kb_remove_selected(ctx);
            }
            if busy {
                ui.spinner();
            }
        });

        if let Some((frac, detail)) = &self.kb_progress {
            ui.add_space(3.0);
            ui::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(crate::i18n::t("Avanzamento")).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(detail.as_str()).size(12.5).color(ui::text_dim()));
                    });
                });
                ui.add_space(2.0);
                ui.add(
                    ui::progress_bar(*frac)
                        .desired_width(ui.available_width())
                        .animate(true)
                        .show_percentage(),
                );
            });
        }

        ui.add_space(5.0);
        ui::card(ui, |ui| {
            ui::section(
                ui,
                "▣",
                crate::i18n::t("Spazio su /boot"),
                crate::i18n::t("Partizione usata dal kernel e dai file di avvio"),
            );
            ui.add_space(3.0);
            if self.kb_report.total == 0 {
                ui.label(
                    egui::RichText::new(crate::i18n::t("Nessun dato su /boot: premi «Scansiona kernel e /boot»."))
                        .color(ui::text_dim()),
                );
            } else {
                let frac = (self.kb_report.used as f32 / self.kb_report.total as f32).clamp(0.0, 1.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(crate::i18n::t("Occupato:")).color(ui::text_dim()));
                    ui.label(egui::RichText::new(cleaner::fmt_bytes(self.kb_report.used)).strong().size(17.0).color(ui::accent_text()));
                    ui.label(egui::RichText::new(crate::i18n::t("Libero:")).color(ui::text_dim()));
                    ui.label(egui::RichText::new(cleaner::fmt_bytes(self.kb_report.free)).strong().size(17.0).color(ui::semantic_text(ui::GOOD)));
                    ui.label(egui::RichText::new(format!("{} · {}", self.kb_report.mount, cleaner::fmt_bytes(self.kb_report.total))).size(12.5).color(ui::text_dim()));
                });
                ui.add_space(3.0);
                ui.add(ui::bar(frac).desired_width(ui.available_width()).show_percentage());
                if self.kb_report.free < 100 * 1024 * 1024 {
                    ui.add_space(3.0);
                    ui.colored_label(
                        ui::semantic_text(ui::WARN),
                        crate::i18n::t("Attenzione: /boot ha meno di 100 MB liberi. Rimuovi i kernel vecchi per evitare errori durante gli aggiornamenti."),
                    );
                }
            }
        });

        ui.add_space(5.0);
        ui::card(ui, |ui| {
            ui::section(
                ui,
                "⚙",
                crate::i18n::t("Kernel installati"),
                crate::i18n::t("Il kernel attuale e il fallback sono protetti"),
            );
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new(crate::i18n::tf(
                    "In esecuzione: {} · Fallback: {}",
                    &[&self.kb_report.running, &self.kb_report.fallback],
                ))
                .size(13.0)
                .color(ui::text_dim()),
            );
            ui.add_space(3.0);

            if self.kb_report.kernels.is_empty() {
                ui.label(
                    egui::RichText::new(crate::i18n::t("Nessun kernel rilevato. Premi «Scansiona kernel e /boot» per analizzare il sistema."))
                        .color(ui::text_dim()),
                );
            } else {
                let report_size = self.kb_report.selected_size();
                ui.label(
                    egui::RichText::new(crate::i18n::tf(
                        "Selezionati: {}",
                        &[&cleaner::fmt_bytes(report_size)],
                    ))
                    .strong()
                    .color(ui::accent_text()),
                );
                ui.add_space(3.0);

                for (idx, entry) in self.kb_report.kernels.iter_mut().enumerate() {
                    ui.push_id(format!("kb-kernel-{idx}"), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_enabled(
                                !busy && !entry.protected,
                                egui::Checkbox::new(
                                    &mut entry.selected,
                                    egui::RichText::new(&entry.version).strong().size(15.0),
                                ),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if entry.protected {
                                    ui::pill(ui, crate::i18n::t("In uso - Protetto"), ui::GOOD);
                                }
                                ui.label(
                                    egui::RichText::new(cleaner::fmt_bytes(entry.size()))
                                        .strong()
                                        .color(ui::accent_text()),
                                );
                            });
                        });
                        ui.label(
                            egui::RichText::new(if entry.protected {
                                crate::i18n::tf(
                                    "{} · {} pacchetti · {}",
                                    &[
                                        &crate::i18n::t(entry.protection),
                                        &entry.packages.len().to_string(),
                                        &cleaner::fmt_bytes(entry.size()),
                                    ],
                                )
                            } else {
                                crate::i18n::tf(
                                    "{} · {} · {} pacchetti",
                                    &[
                                        &crate::i18n::t("Rimovibile"),
                                        &cleaner::fmt_bytes(entry.size()),
                                        &entry.packages.len().to_string(),
                                    ],
                                )
                            })
                            .size(12.5)
                            .color(ui::text_dim()),
                        );
                        ui.collapsing(
                            crate::i18n::tf("Pacchetti di questa versione ({})", &[&entry.packages.len().to_string()]),
                            |ui| {
                                for pkg in &entry.packages {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(pkg.name.as_str()).monospace().size(12.0));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(
                                                egui::RichText::new(cleaner::fmt_bytes(pkg.size))
                                                    .size(12.0)
                                                    .color(ui::accent_text()),
                                            );
                                        });
                                    });
                                }
                            },
                        );
                        ui.add_space(3.0);
                    });
                }
            }
        });
        self.show_out(ui, "kernel_boot", 150.0);
    }
}

