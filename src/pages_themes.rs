use crate::i18n::{t, tf};
use crate::{ui, App, Chan, Job, Msg};
use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct CustomItem {
    pub name: String,
    pub path: PathBuf,
    pub item_type: ThemeType,
    pub selected: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeType {
    Font,
    Icon,
    Theme,
}

impl ThemeType {
    pub fn label(&self) -> &'static str {
        match self {
            ThemeType::Font => t("Font (Caratteri)"),
            ThemeType::Icon => t("Icone"),
            ThemeType::Theme => t("Tema GTK / Cursore"),
        }
    }
}

/// Scansiona le directory utente per font, icone e temi
pub fn scan_user_themes() -> Vec<CustomItem> {
    let mut list = Vec::new();
    let home = crate::util::home();

    // 1. Font (~/.local/share/fonts, ~/.fonts)
    let font_dirs = [
        format!("{}/.local/share/fonts", home),
        format!("{}/.fonts", home),
    ];
    for d in &font_dirs {
        let p = Path::new(d);
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() || is_font_file(&path) {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        list.push(CustomItem {
                            name: name.to_string(),
                            path,
                            item_type: ThemeType::Font,
                            selected: false,
                        });
                    }
                }
            }
        }
    }

    // 2. Icone (~/.local/share/icons, ~/.icons)
    let icon_dirs = [
        format!("{}/.local/share/icons", home),
        format!("{}/.icons", home),
    ];
    for d in &icon_dirs {
        let p = Path::new(d);
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        list.push(CustomItem {
                            name: name.to_string(),
                            path,
                            item_type: ThemeType::Icon,
                            selected: false,
                        });
                    }
                }
            }
        }
    }

    // 3. Temi GTK (~/.local/share/themes, ~/.themes)
    let theme_dirs = [
        format!("{}/.local/share/themes", home),
        format!("{}/.themes", home),
    ];
    for d in &theme_dirs {
        let p = Path::new(d);
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        list.push(CustomItem {
                            name: name.to_string(),
                            path,
                            item_type: ThemeType::Theme,
                            selected: false,
                        });
                    }
                }
            }
        }
    }

    list
}

fn is_font_file(p: &Path) -> bool {
    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        matches!(ext.to_lowercase().as_str(), "ttf" | "otf" | "woff" | "woff2")
    } else {
        false
    }
}

/// Copia una cartella di icone/temi senza seguire symlink.
fn copy_theme_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry.file_type().map_err(|e| format!("{}: {e}", from.display()))?;
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            copy_theme_dir(&from, &to)?;
        } else if ft.is_file() {
            std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        }
    }
    Ok(())
}

/// Installa i file scelti nella home dell'utente: font in
/// `~/.local/share/fonts`, icone/temi in `~/.local/share/icons|themes`.
pub fn install_files(paths: &[String]) -> Result<Vec<String>, String> {
    let home = crate::util::home();
    if home.is_empty() {
        return Err(t("Home utente non disponibile.").to_string());
    }
    let mut logs = Vec::new();
    let mut fonts_updated = false;

    for p in paths {
        let src = PathBuf::from(p);
        let md = std::fs::symlink_metadata(&src).map_err(|e| format!("{p}: {e}"))?;
        if md.file_type().is_symlink() {
            return Err(tf("Symlink non supportato: {}", &[p]));
        }
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| t("Nome file non valido.").to_string())?;

        if md.is_file() && is_font_file(&src) {
            let dir = PathBuf::from(&home).join(".local/share/fonts");
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let dest = dir.join(name);
            if dest.exists() {
                logs.push(tf("Già presente, saltato: {}", &[&dest.display().to_string()]));
                continue;
            }
            std::fs::copy(&src, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            fonts_updated = true;
            logs.push(tf("✔ Font installato: {}", &[&name]));
        } else if md.is_dir() {
            let index = std::fs::read_to_string(src.join("index.theme")).unwrap_or_default();
            let is_icon = index.contains("[Icon Theme]");
            let base = PathBuf::from(&home)
                .join(".local/share")
                .join(if is_icon { "icons" } else { "themes" });
            std::fs::create_dir_all(&base).map_err(|e| format!("{}: {e}", base.display()))?;
            let dest = base.join(name);
            if dest.exists() {
                logs.push(tf("Già presente, saltato: {}", &[&dest.display().to_string()]));
                continue;
            }
            copy_theme_dir(&src, &dest)?;
            logs.push(tf("✔ Elemento installato: {}", &[&name]));
        } else {
            logs.push(tf("✘ Tipo di file non supportato: {}", &[p]));
        }
    }

    if fonts_updated {
        match std::process::Command::new("fc-cache").arg("-fv").output() {
            Ok(out) if out.status.success() => logs.push(t("✔ Cache font aggiornata.").to_string()),
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                logs.push(tf(
                    "⚠ Cache font non aggiornata: {}",
                    &[&crate::util::tail(&err, 1)],
                ));
            }
            Err(e) => logs.push(tf("⚠ Cache font non aggiornata: {}", &[&e.to_string()])),
        }
    }
    Ok(logs)
}

pub fn remove_items(items: &[CustomItem]) -> Result<(), String> {
    let mut updated_fonts = false;
    for item in items {
        if item.path.exists() {
            if item.path.is_dir() {
                std::fs::remove_dir_all(&item.path).map_err(|e| e.to_string())?;
            } else {
                std::fs::remove_file(&item.path).map_err(|e| e.to_string())?;
            }
            if item.item_type == ThemeType::Font {
                updated_fonts = true;
            }
        }
    }
    if updated_fonts {
        let _ = std::process::Command::new("fc-cache").arg("-fv").status();
    }
    Ok(())
}

impl App {
    pub fn ui_themes(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Gestione Font e Temi / Icone Utente");
        ui::hint(
            ui,
            "Installa o rimuovi rapidamente pacchetti di icone, temi GTK/cursore e font personali in ~/.local/share e ~/.icons. L'installazione di font aggiornerà automaticamente la cache di sistema (fc-cache).",
        );

        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.busy, egui::Button::new(t("Scansiona elementi utente"))).clicked() {
                let items = scan_user_themes();
                self.theme_items = items;
                self.log.push_str("✔ Scansione font, icone e temi utente completata.\n");
            }
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Installa nuovo Font/Tema..."))).clicked() {
                let ctx_c = ctx.clone();
                let tx_c = self.tx.clone();
                std::thread::spawn(move || {
                    let start_dir = format!("--filename={}/", crate::util::home());
                    let out = std::process::Command::new("zenity")
                        .args(["--file-selection", "--multiple", start_dir.as_str()])
                        .output();
                    match out {
                        Ok(res) if res.status.success() => {
                            if let Ok(paths_str) = String::from_utf8(res.stdout) {
                                let paths: Vec<String> = paths_str.split('|').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                                let _ = tx_c.send(Msg::ThemesPicked(Some(paths)));
                                ctx_c.request_repaint();
                            }
                        }
                        Ok(_) => {} // annullo: nessun cambiamento
                        Err(e) => {
                            let msg = format!("{}: {e}", t("Nessun selettore di file trovato: installa zenity (sudo apt install zenity)"));
                            let _ = tx_c.send(Msg::Log(msg));
                            ctx_c.request_repaint();
                        }
                    }
                });
            }
            if self.busy {
                ui.spinner();
            }
        });

        ui.add_space(5.0);

        ui::card(ui, |ui| {
            if self.theme_items.is_empty() {
                ui.label(t("Nessun elemento personalizzato trovato o scansione non avviata. Clicca su 'Scansiona elementi utente'."));
            } else {
                egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    for item in &mut self.theme_items {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut item.selected, "");
                            let icon = match item.item_type {
                                ThemeType::Font => "🔤",
                                ThemeType::Icon => "🎨",
                                ThemeType::Theme => "🪟",
                            };
                            ui.label(icon);
                            ui.label(egui::RichText::new(&item.name).strong().size(15.0));
                            ui.label(egui::RichText::new(format!("[{}]", t(item.item_type.label()))).color(ui::text_dim()));
                        });
                        ui.add_space(2.0);
                    }
                });

                ui.add_space(4.0);
                if ui::primary_button(ui, t("Rimuovi elementi selezionati"), self.action_enabled()).clicked() {
                    let targets: Vec<CustomItem> = self.theme_items.iter().filter(|i| i.selected).cloned().collect();
                    if !targets.is_empty() {
                        let job: Job = Box::new(move |ch: &Chan| {
                            ch.send(Msg::Log("▶ Rimozione elementi grafici utente in corso...".into()));
                            match remove_items(&targets) {
                                Ok(_) => ch.send(Msg::Log("✔ Rimozione completata con successo.".into())),
                                Err(e) => ch.send(Msg::Log(format!("✘ Errore rimozione: {e}"))),
                            }
                            ch.send(Msg::Themes(scan_user_themes()));
                        });
                        self.spawn(ctx, job);
                    }
                }
            }
        });
    }
}
