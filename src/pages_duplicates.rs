use crate::i18n::{t, tf};
use crate::{cleaner, quarantine, ui, util, App, Chan, Msg, ProgressReporter};
use eframe::egui;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone)]
pub struct DuplicateGroup {
    pub size: u64,
    pub files: Vec<(PathBuf, bool)>, // (percorso, selezionato per la quarantena)
}

fn partial_hash(path: &Path) -> Option<[u8; 4096]> {
    let mut f = File::open(path).ok()?;
    let mut buf = [0u8; 4096];
    let n = f.read(&mut buf).ok()?;
    if n == 0 {
        return None;
    }
    Some(buf)
}

fn full_sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg("--").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    s.split_whitespace().next().map(|h| h.trim_start_matches('\\').to_string())
}

fn walk_dir(
    dir: &Path,
    skip_hidden: bool,
    min_size: u64,
    by_size: &mut HashMap<u64, Vec<PathBuf>>,
    seen: &mut usize,
    on_progress: &mut dyn FnMut(usize),
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if skip_hidden && name_str.starts_with('.') {
            continue;
        }
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk_dir(&path, skip_hidden, min_size, by_size, seen, on_progress);
        } else if meta.is_file() {
            *seen += 1;
            if *seen % 50 == 0 {
                on_progress(*seen);
            }
            let len = meta.len();
            if len >= min_size {
                by_size.entry(len).or_default().push(path);
            }
        }
    }
}

pub fn scan_duplicates(root: &str, min_kb: u64, skip_hidden: bool) -> Vec<DuplicateGroup> {
    scan_duplicates_with(root, min_kb, skip_hidden, |_, _, _| {})
}

/// Come `scan_duplicates`, con callback `(fase, completati, totale)`.
/// `totale == 0` significa che il numero di elementi non è noto in anticipo.
pub fn scan_duplicates_with<F: FnMut(&str, usize, usize)>(
    root: &str,
    min_kb: u64,
    skip_hidden: bool,
    mut on_progress: F,
) -> Vec<DuplicateGroup> {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut seen = 0usize;
    walk_dir(
        Path::new(root),
        skip_hidden,
        min_kb.saturating_mul(1024),
        &mut by_size,
        &mut seen,
        &mut |n| on_progress(t("Individuazione file"), n, 0),
    );
    on_progress(t("Individuazione file"), seen, seen.max(1));

    let candidates: Vec<(u64, Vec<PathBuf>)> = by_size
        .into_iter()
        .filter(|(_, paths)| paths.len() >= 2)
        .collect();
    let total_head: usize = candidates.iter().map(|(_, p)| p.len()).sum();
    let mut done_head = 0usize;
    let total_hash: usize = total_head;
    let mut done_hash = 0usize;
    let mut groups = Vec::new();

    for (size, paths) in candidates {
        // Fase 2: filtro rapido sui primi 4 KB
        let mut by_head: HashMap<[u8; 4096], Vec<PathBuf>> = HashMap::new();
        for p in paths {
            if let Some(head) = partial_hash(&p) {
                by_head.entry(head).or_default().push(p);
            }
            done_head += 1;
            on_progress(t("Confronto impronte (filtro rapido)"), done_head, total_head);
        }

        // Fase 3: hash completo SHA-256 solo sui candidati reali
        for (_, candidates) in by_head {
            if candidates.len() < 2 {
                done_hash += candidates.len();
                continue;
            }
            let mut by_hash: HashMap<String, Vec<PathBuf>> = HashMap::new();
            for p in candidates {
                if let Some(h) = full_sha256(&p) {
                    by_hash.entry(h).or_default().push(p);
                }
                done_hash += 1;
                on_progress(t("Verifica SHA-256"), done_hash, total_hash);
            }

            for (_, mut dupes) in by_hash {
                if dupes.len() >= 2 {
                    dupes.sort();
                    // Pre-seleziona tutte le copie tranne la prima (originale)
                    let files = dupes
                        .into_iter()
                        .enumerate()
                        .map(|(idx, p)| (p, idx > 0))
                        .collect();
                    groups.push(DuplicateGroup { size, files });
                }
            }
        }
    }

    // Ordina per spazio recuperabile decrescente
    groups.sort_by(|a, b| {
        let save_a = a.size.saturating_mul(a.files.len().saturating_sub(1) as u64);
        let save_b = b.size.saturating_mul(b.files.len().saturating_sub(1) as u64);
        save_b.cmp(&save_a)
    });

    groups
}

impl App {
    pub(crate) fn ui_duplicates(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Ricercatore File Duplicati"));
        ui::hint(
            ui,
            t("Trova ed elimina i file identici (confrontati tramite impronta SHA-256) sparsi nelle tue cartelle per recuperare spazio senza perdere dati."),
        );

        ui::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Cartella da scansionare:"));
                ui.add(egui::TextEdit::singleline(&mut self.dup_dir).desired_width(ui::fit_width(ui, 300.0, 110.0)));
                if ui.button(t("Home")).clicked() {
                    self.dup_dir = util::home();
                }
            });
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(t("Dimensione minima (KB):"));
                ui.add(egui::DragValue::new(&mut self.dup_min_kb).range(1..=1_000_000));
                ui.add_space(5.0);
                ui.checkbox(&mut self.dup_skip_hidden, t("Ignora file e cartelle nascoste (.*)"));
            });
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                if ui::primary_button(ui, t("Cerca duplicati"), self.action_enabled()).clicked() {
                    let dir = self.dup_dir.clone();
                    let min_kb = self.dup_min_kb;
                    let skip = self.dup_skip_hidden;
                    self.spawn(ctx, move |ch: &Chan| {
                        let mut prog = ProgressReporter::start(
                            &ch,
                            "duplicates",
                            t("Ricerca file duplicati in corso..."),
                        );
                        ch.send(Msg::Log(format!("▶ Scansione duplicati in {dir}...")));
                        let found = scan_duplicates_with(&dir, min_kb, skip, |phase, cur, total| {
                            prog.step(cur, total, phase)
                        });
                        ch.send(Msg::Log(format!("✔ Trovati {} gruppi di file duplicati.", found.len())));
                        ch.send(Msg::Duplicates(found));
                        prog.finish(t("Ricerca duplicati completata"));
                    });
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });

        ui.add_space(4.0);

        if self.dup_groups.is_empty() {
            ui::card(ui, |ui| {
                ui.label(t("Nessun file duplicato in elenco. Avvia una scansione per iniziare."));
            });
            return;
        }

        // Calcolo spazio totale selezionato
        let mut selected_count = 0usize;
        let mut selected_bytes = 0u64;
        let mut groups_fully_selected = 0usize;
        for g in &self.dup_groups {
            let all_sel = g.files.iter().all(|(_, s)| *s);
            if all_sel {
                groups_fully_selected += 1;
            }
            for (i, (_, sel)) in g.files.iter().enumerate() {
                // se tutte le copie sono spuntate, la prima viene comunque conservata
                if *sel && !(all_sel && i == 0) {
                    selected_count += 1;
                    selected_bytes += g.size;
                }
            }
        }

        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(tf(
                        "{}: {} file ({})",
                        &[&t("Selezionati per la quarantena"), &selected_count, &cleaner::fmt_bytes(selected_bytes)]
                    ))
                    .strong()
                    .color(ui::accent_text()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            self.action_enabled() && selected_count > 0,
                            egui::Button::new(
                                egui::RichText::new(t("Sposta selezionati in quarantena"))
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            )
                            .fill(ui::BAD),
                        )
                        .clicked()
                    {
                        let mut to_delete = Vec::new();
                        for g in &self.dup_groups {
                            // non cancellare mai tutte le copie di un file: la prima viene salvata
                            let all_sel = g.files.iter().all(|(_, s)| *s);
                            for (i, (p, sel)) in g.files.iter().enumerate() {
                                if *sel && !(all_sel && i == 0) {
                                    to_delete.push(p.clone());
                                }
                            }
                        }
                        let msg = tf(
                            "Spostare {} file duplicati ({}) in quarantena? Potrai ripristinarli entro {} giorni.",
                            &[&to_delete.len(), &cleaner::fmt_bytes(selected_bytes), &quarantine::RETENTION_DAYS],
                        );
                        let dir = self.dup_dir.clone();
                        let min_kb = self.dup_min_kb;
                        let skip = self.dup_skip_hidden;
                        let job = Box::new(move |ch: &Chan| {
                            let mut prog = ProgressReporter::start(
                                &ch,
                                "quarantine",
                                t("Spostamento duplicati in quarantena in corso..."),
                            );
                            match quarantine::quarantine_files(&to_delete, "File duplicati", false) {
                                Ok((moved, failed)) => {
                                    ch.send(Msg::QuarantineMessage(
                                        tf("Spostati {} file in quarantena, {} non spostati.", &[&moved, &failed]),
                                        failed > 0,
                                    ));
                                }
                                Err(e) => ch.send(Msg::QuarantineMessage(e, true)),
                            }
                            ch.send(Msg::QuarantineItems(quarantine::load().unwrap_or_default()));
                            let found = scan_duplicates_with(&dir, min_kb, skip, |phase, cur, tot| {
                                prog.step(cur, tot, phase)
                            });
                            ch.send(Msg::Duplicates(found));
                            prog.finish(t("Spostamento completato"));
                        });
                        self.confirm = Some((msg, job));
                    }
                });
            });
        });

        if groups_fully_selected > 0 {
            ui.colored_label(
                ui::WARN,
                format!(
                    "{} {}",
                    groups_fully_selected,
                    t("gruppi hanno tutte le copie spuntate: la prima di ogni gruppo verrà conservata.")
                ),
            );
        }

        ui.add_space(3.0);

        egui::ScrollArea::vertical().max_height(285.0).show(ui, |ui| {
            for (idx, g) in self.dup_groups.iter_mut().enumerate() {
                ui::card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(tf(
                                "{} #{} — {} ({} copie)",
                                &[&t("Gruppo"), &(idx + 1), &cleaner::fmt_bytes(g.size), &g.files.len()]
                            ))
                            .strong()
                            .color(ui::accent_text()),
                        );
                    });
                    ui.add_space(2.0);
                    for (path, sel) in &mut g.files {
                        let label = path.to_string_lossy().to_string();
                        ui.checkbox(sel, label.as_str());
                    }
                });
                ui.add_space(3.0);
            }
        });
    }
}
