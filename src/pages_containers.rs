use crate::i18n::t;
use crate::{ui, util, App};
use eframe::egui;

#[derive(Clone)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub running: bool,
}

pub fn read_containers(runtime: &str) -> Vec<Container> {
    let mut list = Vec::new();
    let cmd = format!("{runtime} ps -a --format '{{{{.ID}}}}|{{{{.Names}}}}|{{{{.Image}}}}|{{{{.Status}}}}|{{{{.State}}}}' 2>/dev/null");
    let (_, o) = util::run_shell(&cmd, false);
    for l in o.lines() {
        let parts: Vec<&str> = l.split('|').collect();
        if parts.len() >= 5 {
            list.push(Container {
                id: parts[0].to_string(),
                name: parts[1].to_string(),
                image: parts[2].to_string(),
                status: parts[3].to_string(),
                running: parts[4] == "running",
            });
        }
    }
    list
}

impl App {
    pub(crate) fn ui_containers(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Manager Container (Docker / Podman)"));

        let have_docker = util::run_shell("command -v docker >/dev/null 2>&1", false).0;
        let have_podman = util::run_shell("command -v podman >/dev/null 2>&1", false).0;

        if !have_docker && !have_podman {
            ui::card(ui, |ui| {
                ui.colored_label(ui::semantic_text(ui::WARN), t("Né Docker né Podman sono installati nel sistema."));
                ui.label(t("Installa docker.io o podman da terminale per gestire i container da qui."));
            });
            return;
        }

        let runtime = if have_docker { "docker" } else { "podman" };
        ui::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{}: {}", t("Motore in uso"), runtime));
                if ui::primary_button(ui, t("🧹 Pulisci spazio container (Prune)"), self.action_enabled()).clicked() {
                    let script = format!("{runtime} system prune -af --volumes");
                    self.confirm = Some((
                        t("Eliminare tutti i container fermi, volumi e immagini orfane non utilizzate?").into(),
                        crate::cmd_job("containers", script, false),
                    ));
                }
            });
        });
        ui.add_space(3.0);

        let containers = read_containers(runtime);
        if containers.is_empty() {
            ui::card(ui, |ui| {
                ui.label(t("Nessun container trovato su questo host."));
            });
        } else {
            for c in containers {
                ui::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui::led(ui, Some(c.running), "");
                        ui.label(egui::RichText::new(&c.name).strong().size(18.0).color(ui::text_primary()));
                        ui.label(egui::RichText::new(format!("[{}]", c.image)).color(ui::text_dim()));

                        let id_q = util::shq(&c.id);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if c.running {
                                if ui::danger_button(ui, t("Arresta"), true).clicked() {
                                    self.spawn(ctx, crate::cmd_job("containers", format!("{runtime} stop {id_q}"), false));
                                }
                            } else {
                                if ui::primary_button(ui, t("Avvia"), true).clicked() {
                                    self.spawn(ctx, crate::cmd_job("containers", format!("{runtime} start {id_q}"), false));
                                }
                            }
                            if ui.button(t("Log")).clicked() {
                                self.spawn(ctx, crate::cmd_job("containers", format!("{runtime} logs --tail 80 {id_q}"), false));
                            }
                        });
                    });
                    ui.label(egui::RichText::new(&c.status).size(14.0).color(ui::text_dim()));
                });
                ui.add_space(2.0);
            }
        }
        self.show_out(ui, "containers", 200.0);
    }
}
