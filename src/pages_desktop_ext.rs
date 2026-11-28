use crate::i18n::{t, tf};
use crate::{ui, util, App};
use eframe::egui;

#[derive(Clone)]
pub struct DesktopExt {
    pub uuid: String,
    pub name: String,
    pub enabled: bool,
}

pub fn read_gnome_extensions() -> Vec<DesktopExt> {
    let mut list = Vec::new();
    let (_, o) = util::run_shell("gnome-extensions list -d 2>/dev/null", false);
    let mut cur_uuid = String::new();
    let mut cur_name = String::new();
    let mut cur_state = false;

    for l in o.lines() {
        let t = l.trim();
        if !t.starts_with("Name:") && !t.starts_with("State:") && !t.starts_with("Description:") && !t.is_empty() && !l.starts_with(' ') {
            if !cur_uuid.is_empty() {
                list.push(DesktopExt {
                    uuid: cur_uuid.clone(),
                    name: if cur_name.is_empty() { cur_uuid.clone() } else { cur_name.clone() },
                    enabled: cur_state,
                });
            }
            cur_uuid = t.to_string();
            cur_name.clear();
            cur_state = false;
        } else if let Some(n) = t.strip_prefix("Name:") {
            cur_name = n.trim().to_string();
        } else if let Some(s) = t.strip_prefix("State:") {
            cur_state = s.contains("ENABLED") || s.contains("ACTIVE");
        }
    }
    if !cur_uuid.is_empty() {
        list.push(DesktopExt {
            uuid: cur_uuid.clone(),
            name: if cur_name.is_empty() { cur_uuid } else { cur_name },
            enabled: cur_state,
        });
    }
    list
}

impl App {
    pub(crate) fn ui_desktop_ext(&mut self, ui: &mut egui::Ui) {
        let de = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_uppercase();
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Estensioni Desktop")).strong().size(20.0));
            let de = if de.is_empty() { t("Sconosciuto") } else { de.as_str() };
            ui.label(tf("Ambiente desktop rilevato: {}", &[&de]));
        });
        ui.add_space(3.0);

        if de.contains("GNOME") {
            let exts = read_gnome_extensions();
            if exts.is_empty() {
                ui.label(t("Nessuna estensione GNOME trovata o comando 'gnome-extensions' non presente."));
                return;
            }

            for e in exts {
                ui::card(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut state = e.enabled;
                        if ui.checkbox(&mut state, egui::RichText::new(&e.name).strong().size(17.0)).changed() {
                            let action = if state { "enable" } else { "disable" };
                            let _ = util::run_shell(&format!("gnome-extensions {action} {}", util::shq(&e.uuid)), false);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(&e.uuid).size(13.0).color(ui::text_dim()));
                        });
                    });
                });
                ui.add_space(2.0);
            }
        } else {
            ui::card(ui, |ui| {
                ui.label(t("La gestione estensioni con toggle rapido è attualmente supportata nativamente per GNOME Shell."));
            });
        }
    }
}
