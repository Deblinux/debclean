//! Quarantena temporanea dei singoli file rimossi dalle pulizie.
//
// Non è uno snapshot né un backup di cartelle: ogni voce riguarda un solo file,
// con metadati minimi e scadenza automatica. Percorsi e scrittura atomica
// riutilizzano `util::state_dir()` / `util::write_atomic()`, già usati dall'app.

use crate::i18n::{t, tf, tr};
use crate::{cleaner, ui, util, App, Chan, Job, Msg, ProgressReporter};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Giorni di conservazione prima dell'eliminazione automatica.
pub const RETENTION_DAYS: u64 = 7;
const DAY_SECS: u64 = 86_400;
const MANIFEST_NAME: &str = "quarantine.toml";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    /// Percorso del file prima dello spostamento.
    pub original_path: String,
    /// Nome del file nella cartella di quarantena (usa solo l'ID, mai il percorso).
    pub stored_name: String,
    /// Data leggibile dello spostamento.
    pub date: String,
    /// Timestamp epoch, usato per la scadenza.
    pub epoch: u64,
    pub size: u64,
    /// Modulo che ha rimosso il file (chiave tradotta con `tr`).
    pub module: String,
    /// True per i file che richiedono pkexec (per esempio i backup in /etc).
    pub privileged: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    items: Vec<Item>,
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Come `pages_sec::Config::save`: ripristina la proprietà dell'utente reale
/// quando DebUtility è avviato tramite pkexec.
fn chown_to_user(path: &Path) {
    if let (Ok(uid), Ok(gid)) = (
        std::env::var("PKEXEC_UID"),
        std::env::var("PKEXEC_GID"),
    ) {
        let _ = std::process::Command::new("chown")
            .arg(format!("{uid}:{gid}"))
            .arg(path)
            .status();
    }
}

/// Cartella privata dedicata, sotto lo stato già usato dall'applicazione.
pub fn dir() -> Result<PathBuf, String> {
    let path = util::state_dir()?.join("quarantine");
    fs::create_dir_all(&path)
        .map_err(|e| tf("Impossibile creare la cartella di quarantena ({}): {}", &[&path.display().to_string(), &e.to_string()]))?;
    chown_to_user(&path);
    Ok(path)
}

fn manifest_path() -> Result<PathBuf, String> {
    Ok(dir()?.join(MANIFEST_NAME))
}

pub fn load() -> Result<Vec<Item>, String> {
    let path = manifest_path()?;
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| {
        tf(
            "Impossibile leggere la quarantena ({}): {}",
            &[&path.display().to_string(), &e.to_string()],
        )
    })?;
    let manifest: Manifest = toml::from_str(&text).map_err(|e| {
        tf(
            "Manifesto della quarantena non valido ({}): {}",
            &[&path.display().to_string(), &e.to_string()],
        )
    })?;
    Ok(manifest.items)
}

fn save(items: &[Item]) -> Result<(), String> {
    let path = manifest_path()?;
    let manifest = Manifest { version: 1, items: items.to_vec() };
    let text = toml::to_string_pretty(&manifest)
        .map_err(|e| tf("Impossibile serializzare la quarantena: {}", &[&e.to_string()]))?;
    util::write_atomic(&path, &text)?;
    chown_to_user(&path);
    Ok(())
}

/// Percorso sicuro del file conservato: il manifest è un file di testo, quindi
/// non deve poter indicare cartelle arbitrarie tramite `stored_name`.
fn stored_path(item: &Item) -> Result<PathBuf, String> {
    let name = Path::new(&item.stored_name)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("");
    if name.is_empty() || name != item.stored_name || name == "." || name == ".." {
        return Err(t("Nome di quarantena non valido.").to_string());
    }
    Ok(dir()?.join(name))
}

fn move_file(src: &Path, dst: &Path, privileged: bool) -> Result<(), String> {
    if privileged {
        let script = format!(
            "mv -f -- {} {}",
            util::shq(&src.to_string_lossy()),
            util::shq(&dst.to_string_lossy())
        );
        let (ok, out) = util::run_shell(&script, true);
        return if ok {
            Ok(())
        } else {
            Err(util::tail(out.trim(), 2).replace('\n', " | "))
        };
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(src, dst).map_err(|e| {
                tf(
                    "Impossibile copiare {} in quarantena: {}",
                    &[&src.display().to_string(), &e.to_string()],
                )
            })?;
            fs::remove_file(src).map_err(|e| {
                tf(
                    "Impossibile rimuovere l'originale {}: {}",
                    &[&src.display().to_string(), &e.to_string()],
                )
            })?;
            Ok(())
        }
    }
}

/// Sposta in quarantena solo file regolari, escludendo la quarantena stessa.
/// Restituisce `(spostati, non_spostati)`.
pub fn quarantine_files(
    paths: &[PathBuf],
    module: &str,
    privileged: bool,
) -> Result<(usize, usize), String> {
    let store = dir()?;
    let mut items = load()?;
    let stamp = now_epoch();
    let pid = std::process::id();
    let mut seq = 0u64;
    let (mut moved, mut failed) = (0usize, 0usize);

    for src in paths {
        if src.starts_with(&store) {
            continue;
        }
        let meta = match fs::symlink_metadata(src) {
            Ok(m) if m.is_file() => m,
            _ => {
                failed += 1;
                continue;
            }
        };
        let id = loop {
            let candidate = format!("{stamp}-{pid}-{seq}");
            seq += 1;
            if !store.join(&candidate).exists() {
                break candidate;
            }
        };
        let dst = store.join(&id);
        if move_file(src, &dst, privileged).is_err() {
            failed += 1;
            continue;
        }
        items.push(Item {
            id: id.clone(),
            original_path: src.to_string_lossy().to_string(),
            stored_name: id,
            date: util::timestamp(),
            epoch: stamp,
            size: meta.len(),
            module: module.to_string(),
            privileged,
        });
        if let Err(e) = save(&items) {
            // Il file è già stato spostato: riportarlo indietro evita di
            // perdere irrimediabilmente un elemento non registrato nel manifest.
            let _ = move_file(&dst, src, privileged);
            return Err(e);
        }
        moved += 1;
    }
    Ok((moved, failed))
}

/// Giorni interi ancora disponibili, arrotondati per eccesso: con 1 secondo
/// di vita residua il file è ancora valido per oggi, non "scaduto".
pub fn days_left(item: &Item) -> u64 {
    let end = item.epoch.saturating_add(RETENTION_DAYS * DAY_SECS);
    let left = end.saturating_sub(now_epoch());
    left / DAY_SECS + u64::from(left % DAY_SECS > 0)
}

pub fn is_expired(item: &Item) -> bool {
    now_epoch() >= item.epoch.saturating_add(RETENTION_DAYS * DAY_SECS)
}

fn remove_stored(item: &Item) -> Result<(), String> {
    let path = stored_path(item)?;
    if !path.exists() {
        return Ok(());
    }
    if item.privileged {
        let script = format!("rm -f -- {}", util::shq(&path.to_string_lossy()));
        let (ok, out) = util::run_shell(&script, true);
        if ok {
            Ok(())
        } else {
            Err(util::tail(out.trim(), 2).replace('\n', " | "))
        }
    } else {
        fs::remove_file(&path)
            .map_err(|e| tf("Impossibile eliminare il file in quarantena: {}", &[&e.to_string()]))
    }
}

/// Ripristina un file nel percorso originale. Non sovrascrive un file esistente.
pub fn restore(id: &str) -> Result<String, String> {
    let store = dir()?;
    let mut items = load()?;
    let idx = items
        .iter()
        .position(|i| i.id == id)
        .ok_or_else(|| t("Elemento non trovato nella quarantena.").to_string())?;
    let item = items[idx].clone();
    let stored = stored_path(&item)?;
    if !stored.is_file() {
        return Err(t("File in quarantena mancante.").to_string());
    }
    let original = PathBuf::from(&item.original_path);
    if !original.is_absolute()
        || original.starts_with(&store)
        || original.components().any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(t("Percorso originale non valido.").to_string());
    }
    // Un symlink pendente esiste come voce di directory: `exists()` lo segue
    // e lo considererebbe assente, consentendo una sovrascrittura.
    if fs::symlink_metadata(&original).is_ok() {
        return Err(tf(
            "Esiste già un file nel percorso originale: {}",
            &[&original.display().to_string()],
        ));
    }
    if let Some(parent) = original.parent() {
        if item.privileged {
            // Le voci privilegiate spesso puntano a /etc: la cartella padre
            // va creata con elevazione, non con i permessi dell'utente.
            let script = format!("mkdir -p -- {}", util::shq(&parent.to_string_lossy()));
            let (ok, out) = util::run_shell(&script, true);
            if !ok {
                return Err(util::tail(out.trim(), 2).replace('\n', " | "));
            }
        } else {
            fs::create_dir_all(parent).map_err(|e| {
                tf(
                    "Impossibile ricreare la cartella di destinazione: {}",
                    &[&e.to_string()],
                )
            })?;
        }
    }
    move_file(&stored, &original, item.privileged)?;
    items.remove(idx);
    if let Err(e) = save(&items) {
        // Rollback: il file deve restare in quarantena se il manifesto non
        // può essere aggiornato, altrimenti non è più ripristinabile.
        let _ = move_file(&original, &stored, item.privileged);
        return Err(e);
    }
    Ok(tf("Ripristinato: {}", &[&original.display().to_string()]))
}

/// Eliminazione definitiva di una singola voce.
pub fn delete_one(id: &str) -> Result<String, String> {
    let mut items = load()?;
    let idx = items
        .iter()
        .position(|i| i.id == id)
        .ok_or_else(|| t("Elemento non trovato nella quarantena.").to_string())?;
    let item = items[idx].clone();
    // Aggiorna prima il manifest: se la rimozione del file fallisce si può
    // reinserire la voce; se si cancellasse prima e `save` fallisse, il file
    // resterebbe sul disco senza più modo di essere ripristinato dalla UI.
    items.remove(idx);
    save(&items)?;
    if let Err(e) = remove_stored(&item) {
        items.push(item.clone());
        let _ = save(&items);
        return Err(e);
    }
    Ok(tf("Eliminato definitivamente: {}", &[&item.original_path]))
}

/// Elimina automaticamente le voci più vecchie di `RETENTION_DAYS` giorni.
pub fn purge_expired() -> Result<usize, String> {
    let mut items = load()?;
    let expired: Vec<Item> = items.iter().filter(|i| is_expired(*i)).cloned().collect();
    if expired.is_empty() {
        return Ok(0);
    }
    // Aggiorna prima il manifesto: se la scrittura fallisce i file restano
    // dov'erano e restano ripristinabili dalla UI.
    items.retain(|i| !is_expired(i));
    save(&items)?;

    let mut removed = 0usize;
    let mut readded = false;
    for item in expired {
        match remove_stored(&item) {
            Ok(()) => removed += 1,
            Err(_) => {
                // Il file non si può cancellare: lo rimettiamo nel manifesto
                // invece di lasciarlo orfano sul disco.
                items.push(item);
                readded = true;
            }
        }
    }
    if readded {
        save(&items)?;
    }
    Ok(removed)
}

impl App {
    /// Ricarica l'elenco dal manifest, senza duplicare lo stato in memoria.
    pub(crate) fn refresh_quarantine(&mut self) {
        match load() {
            Ok(items) => self.quarantine_items = items,
            Err(e) => self.log.push_str(&format!("✘ {e}\n")),
        }
    }

    pub(crate) fn ui_quarantine(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Quarantena / Ripristino"));
        ui::hint(
            ui,
            t("I singoli file rimossi da File duplicati e dai backup di configurazione in Residui di sistema restano qui per 7 giorni. Puoi ripristinarli nel percorso originale o eliminarli definitivamente. Non è uno snapshot di sistema."),
        );

        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                let (badge, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                ui::icon_badge(ui.painter(), badge, ui::WARN, "↩", 15.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(t("Conservazione temporanea dei file")).strong());
                    ui.label(
                        egui::RichText::new(tf("Scadenza automatica dopo {} giorni", &[&RETENTION_DAYS]))
                            .size(12.5)
                            .color(ui::text_dim()),
                    );
                });
            });
            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("Aggiorna elenco")).clicked() {
                    self.refresh_quarantine();
                }
                if ui
                    .add_enabled(self.action_enabled(), egui::Button::new(t("Elimina elementi scaduti")))
                    .clicked()
                {
                    self.spawn(ctx, move |ch: &Chan| {
                        let prog = ProgressReporter::start(
                            &ch,
                            "quarantine",
                            t("Pulizia della quarantena in corso..."),
                        );
                        match purge_expired() {
                            Ok(n) => ch.send(Msg::QuarantineMessage(
                                tf("Eliminati {} elementi scaduti.", &[&n]),
                                false,
                            )),
                            Err(e) => ch.send(Msg::QuarantineMessage(e, true)),
                        }
                        match load() {
                            Ok(items) => ch.send(Msg::QuarantineItems(items)),
                            Err(e) => ch.send(Msg::Log(format!("✘ {e}"))),
                        }
                        prog.finish(t("Pulizia della quarantena completata"));
                    });
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });

        if let Some((msg, is_error)) = &self.quarantine_msg {
            ui.add_space(3.0);
            ui.colored_label(
                if *is_error { ui::semantic_text(ui::BAD) } else { ui::semantic_text(ui::GOOD) },
                msg.as_str(),
            );
        }

        ui.add_space(4.0);
        if self.quarantine_items.is_empty() {
            ui::card(ui, |ui| {
                ui.label(t("Nessun file in quarantena."));
            });
            return;
        }

        ui.label(
            egui::RichText::new(tf("{} file in quarantena", &[&self.quarantine_items.len()]))
                .strong()
                .color(ui::accent_text()),
        );
        ui.add_space(3.0);

        let mut restore_id: Option<String> = None;
        let mut delete_request: Option<(String, String)> = None;

        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for item in &self.quarantine_items {
                ui::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let left = days_left(item);
                        if left == 0 {
                            ui::led(ui, Some(false), t("Scaduto"));
                        } else if left <= 2 {
                            ui::led(ui, None, t("In scadenza"));
                        } else {
                            ui::led(ui, Some(true), t("Valido"));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(if left == 0 {
                                    t("Scadenza: oggi")
                                } else {
                                    t("Scadenza tra giorni")
                                })
                                .size(12.0)
                                .color(ui::text_dim()),
                            );
                            if left > 0 {
                                ui.label(
                                    egui::RichText::new(left.to_string())
                                        .strong()
                                        .color(ui::accent_text()),
                                );
                            }
                        });
                    });
                    ui.add_space(2.0);
                    egui::Grid::new(format!("q_{}", item.id))
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            ui::kv(ui, "Percorso originale", &item.original_path);
                            ui::kv(ui, "Modulo di origine", &tr(&item.module));
                            ui::kv(ui, "Data", &item.date);
                            ui::kv(ui, "Dimensione", &cleaner::fmt_bytes(item.size));
                        });
                    ui.add_space(3.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(self.action_enabled(), egui::Button::new(t("Ripristina")))
                            .on_hover_text(t("Riporta il file nel percorso originale"))
                            .clicked()
                        {
                            restore_id = Some(item.id.clone());
                        }
                        if ui::danger_button(ui, t("Elimina definitivamente"), self.action_enabled())
                            .on_hover_text(t("Elimina il file dalla quarantena senza possibilità di recupero"))
                            .clicked()
                        {
                            delete_request = Some((
                                item.id.clone(),
                                tf(
                                    "Eliminare definitivamente {}? L'operazione non è reversibile.",
                                    &[&item.original_path],
                                ),
                            ));
                        }
                    });
                });
                ui.add_space(3.0);
            }
        });

        if let Some(id) = restore_id {
            // `bg()` non imposta `busy`: due ripristini potevano sovrapporsi e
            // sovrascrivere il manifest. Qui serve la stessa serializzazione
            // delle altre operazioni sulla quarantena.
            self.spawn(ctx, move |ch: &Chan| {
                match restore(&id) {
                    Ok(msg) => ch.send(Msg::QuarantineMessage(msg, false)),
                    Err(e) => ch.send(Msg::QuarantineMessage(e, true)),
                }
                match load() {
                    Ok(items) => ch.send(Msg::QuarantineItems(items)),
                    Err(e) => ch.send(Msg::Log(format!("✘ {e}"))),
                }
            });
        }
        if let Some((id, text)) = delete_request {
            let job: Job = Box::new(move |ch: &Chan| {
                match delete_one(&id) {
                    Ok(msg) => ch.send(Msg::QuarantineMessage(msg, false)),
                    Err(e) => ch.send(Msg::QuarantineMessage(e, true)),
                }
                match load() {
                    Ok(items) => ch.send(Msg::QuarantineItems(items)),
                    Err(e) => ch.send(Msg::Log(format!("✘ {e}"))),
                }
            });
            self.confirm = Some((text, job));
        }
    }
}
