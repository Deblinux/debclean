use crate::i18n::t;
use crate::{ui, util, App};
use eframe::egui;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

#[derive(Clone)]
pub struct AppImageItem {
    pub name: String,
    pub path: PathBuf,
    pub is_executable: bool,
    pub is_integrated: bool,
    pub desktop_file: PathBuf,
}

pub fn scan_appimages() -> Vec<AppImageItem> {
    let mut list = Vec::new();
    let home = util::home();
    let search_dirs = [
        PathBuf::from(&home).join("Applications"),
        PathBuf::from(&home).join("AppImages"),
        PathBuf::from(&home).join(".local/bin"),
        PathBuf::from(&home).join("Scaricati"),
        PathBuf::from(&home).join("Downloads"),
    ];

    let desktop_dir = PathBuf::from(&home).join(".local/share/applications");

    for dir in &search_dirs {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() {
                    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                    if ext == "appimage" {
                        let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("AppImage").to_string();
                        let md = p.symlink_metadata().ok();
                        let is_exec = md.map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false);
                        let dt_name = format!("appimage_{}.desktop", name.replace(' ', "_"));
                        let dt_path = desktop_dir.join(&dt_name);
                        let is_integrated = dt_path.exists();

                        list.push(AppImageItem {
                            name,
                            path: p,
                            is_executable: is_exec,
                            is_integrated,
                            desktop_file: dt_path,
                        });
                    }
                }
            }
        }
    }
    list
}

pub fn integrate_appimage(item: &AppImageItem) -> Result<(), String> {
    let home = util::home();
    let desktop_dir = PathBuf::from(&home).join(".local/share/applications");
    fs::create_dir_all(&desktop_dir).map_err(|e| e.to_string())?;

    // Rendi eseguibile
    if let Ok(md) = item.path.symlink_metadata() {
        let mut perms = md.permissions();
        perms.set_mode(perms.mode() | 0o755);
        let _ = fs::set_permissions(&item.path, perms);
    }

    let p_str = item.path.to_str().ok_or(t("Percorso non valido"))?;
    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={}\n\
         Exec=\"{}\"\n\
         Icon=application-x-executable\n\
         Terminal=false\n\
         Categories=Utility;\n",
        item.name, p_str
    );
    fs::write(&item.desktop_file, content).map_err(|e| e.to_string())?;
    let apps = format!("{}/.local/share/applications", util::home());
    let _ = util::run_shell(&format!("update-desktop-database {} 2>/dev/null", util::shq(&apps)), false);
    Ok(())
}

impl App {
    pub(crate) fn ui_appimages(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui::title(ui, t("AppImage"));
        ui::card(ui, |ui| {
            ui::section(ui, "◆", "Gestione AppImage", "");
            ui.label(t("Rileva le applicazioni AppImage in ~/Applications, ~/AppImages o ~/.local/bin e le integra nel menu delle applicazioni di sistema."));
        });
        ui.add_space(3.0);

        let items = scan_appimages();
        if items.is_empty() {
            ui::card(ui, |ui| {
                ui.label(t("Nessuna AppImage trovata. Copia i tuoi file .AppImage in ~/Applications o ~/.local/bin."));
            });
            return;
        }

        for item in items {
            ui::card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(&item.name).strong().size(18.0).color(ui::text_primary()));
                    if item.is_integrated {
                        ui::pill(ui, t("nel menu"), ui::GOOD);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if item.is_integrated {
                            if ui.button(t("Rimuovi dal menu")).clicked() {
                                let _ = fs::remove_file(&item.desktop_file);
                                let apps = format!("{}/.local/share/applications", util::home());
                                let _ = util::run_shell(&format!("update-desktop-database {} 2>/dev/null", util::shq(&apps)), false);
                            }
                        } else {
                            if ui::primary_button(ui, t("Integra nel menu"), true).clicked() {
                                let _ = integrate_appimage(&item);
                            }
                        }

                        if ui.button(t("▶ Avvia")).clicked() {
                            let p = item.path.clone();
                            std::thread::spawn(move || {
                                let _ = std::process::Command::new(p).spawn();
                            });
                        }
                    });
                });
                ui.label(egui::RichText::new(item.path.display().to_string()).size(13.0).color(ui::text_dim()));
            });
            ui.add_space(2.0);
        }
    }
}
