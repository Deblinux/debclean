use crate::i18n::{t, tf};
use crate::{ui, util, App, Msg, ServiceSortKey};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::process::Command;

#[derive(Clone)]
pub struct SystemdService {
    pub name: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub description: String,
    pub is_masked: bool,
}

pub fn read_services() -> Vec<SystemdService> {
    let mut list = Vec::new();
    
    // 1. Legge i servizi standard (attivi, inattivi, ecc.)
    let out = Command::new("systemctl")
        .args(["list-units", "--type=service", "--all", "--no-legend", "--no-pager"])
        .output();

    if let Ok(res) = out {
        let s = String::from_utf8_lossy(&res.stdout);
        for line in s.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let name = parts[0].to_string();
                let load_state = parts[1].to_string();
                let active_state = parts[2].to_string();
                let sub_state = parts[3].to_string();
                let description = if parts.len() > 4 {
                    parts[4..].join(" ")
                } else {
                    String::new()
                };
                let is_masked = load_state == "masked";
                list.push(SystemdService {
                    name,
                    load_state,
                    active_state,
                    sub_state,
                    description,
                    is_masked,
                });
            }
        }
    }

    // 2. Aggiunge i servizi mascherati che non compaiono in list-units
    let out_masked = Command::new("systemctl")
        .args(["list-unit-files", "--type=service", "--state=masked", "--no-legend", "--no-pager"])
        .output();

    if let Ok(res) = out_masked {
        let s = String::from_utf8_lossy(&res.stdout);
        for line in s.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(name) = parts.first() {
                let name = name.to_string();
                if !list.iter().any(|svc| svc.name == name) {
                    list.push(SystemdService {
                        name,
                        load_state: "masked".to_string(),
                        active_state: "inactive".to_string(),
                        sub_state: "dead".to_string(),
                        description: t("Servizio mascherato dal sistema").to_string(),
                        is_masked: true,
                    });
                }
            }
        }
    }

    list
}

impl App {
    pub(crate) fn ui_services(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Gestore Servizi Systemd"));
        ui::hint(
            ui,
            t("Visualizza, avvia, arresta o sblocca i servizi di sistema (inclusi quelli mascherati)."),
        );

        ui.add_space(3.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Ricarica elenco"))).clicked() {
                self.spawn(ctx, move |ch| {
                    ch.send(Msg::Services(read_services()));
                });
            }
            if self.busy {
                ui.spinner();
            }
        });

        ui.add_space(4.0);
        self.show_out_follow(ui, "services", 100.0);
        ui.add_space(3.0);

        if self.services_list.is_empty() {
            ui::card(ui, |ui| {
                ui.label(t("Nessun servizio caricato. Premi 'Ricarica elenco' per scansionare il sistema."));
            });
            return;
        }

        let total = self.services_list.len();
        let filter = self.svc_filter.to_lowercase();
        let mut filtered: Vec<SystemdService> = self
            .services_list
            .iter()
            .filter(|s| {
                filter.is_empty()
                    || s.name.to_lowercase().contains(&filter)
                    || s.description.to_lowercase().contains(&filter)
                    || s.active_state.to_lowercase().contains(&filter)
            })
            .cloned()
            .collect();

        filtered.sort_by(|a, b| {
            let ord = match self.svc_sort.key {
                ServiceSortKey::Name => ui::cmp_alpha(&a.name, &b.name),
                ServiceSortKey::State => ui::cmp_alpha(&a.active_state, &b.active_state)
                    .then(ui::cmp_alpha(&a.sub_state, &b.sub_state)),
                ServiceSortKey::Description => ui::cmp_alpha(&a.description, &b.description),
            };
            match self.svc_sort.dir {
                ui::SortDir::Asc => ord,
                ui::SortDir::Desc => ord.reverse(),
            }
        });

        let shown = filtered.len();
        ui::filter_bar(ui, &mut self.svc_filter, shown, total);

        let visible_names: Vec<String> = filtered.iter().map(|s| s.name.clone()).collect();
        let selected_visible = visible_names.iter().filter(|n| self.svc_selected.contains(*n)).count();
        let sel_action = ui::selection_controls(ui, selected_visible, shown);
        match sel_action {
            ui::SelectionAction::All => {
                for n in &visible_names {
                    if !self.svc_selected.contains(n) {
                        self.svc_selected.push(n.clone());
                    }
                }
            }
            ui::SelectionAction::NoneSelected => {
                self.svc_selected.retain(|n| !visible_names.contains(n));
            }
            ui::SelectionAction::None => {}
        }

        let mut action_cmd: Option<(String, String, bool)> = None;
        let mut bulk_action: Option<&'static str> = None;

        let w = ui.available_width();
        let sel_w = 32.0;
        let state_w = (w * 0.12).clamp(72.0, 110.0);
        let name_w = (w * 0.28).clamp(120.0, 260.0);
        let act_w = (w * 0.22).clamp(140.0, 220.0);

        TableBuilder::new(ui)
            .id_salt("services_table")
            .striped(true)
            .vscroll(true)
            .max_scroll_height(320.0)
            .auto_shrink([false, false])
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(sel_w))
            .column(Column::initial(name_w).at_least(100.0))
            .column(Column::initial(state_w).at_least(64.0))
            .column(Column::remainder().at_least(120.0))
            .column(Column::initial(act_w).at_least(120.0))
            .header(22.0, |mut header| {
                header.col(|ui| { ui::table_head(ui, "✓"); });
                header.col(|ui| { ui::sort_header(ui, t("Servizio"), ServiceSortKey::Name, &mut self.svc_sort); });
                header.col(|ui| { ui::sort_header(ui, t("Stato"), ServiceSortKey::State, &mut self.svc_sort); });
                header.col(|ui| { ui::sort_header(ui, t("Descrizione"), ServiceSortKey::Description, &mut self.svc_sort); });
                header.col(|ui| { ui::table_head(ui, t("Azioni")); });
            })
            .body(|body| {
                body.rows(24.0, filtered.len(), |mut row| {
                    let svc = &filtered[row.index()];
                    let name = svc.name.clone();
                    row.col(|ui| {
                        let mut checked = self.svc_selected.contains(&name);
                        if ui.checkbox(&mut checked, "").changed() {
                            if checked {
                                if !self.svc_selected.contains(&name) {
                                    self.svc_selected.push(name.clone());
                                }
                            } else {
                                self.svc_selected.retain(|x| x != &name);
                            }
                        }
                    });
                    row.col(|ui| {
                        ui.label(egui::RichText::new(&svc.name).strong());
                    });
                    row.col(|ui| {
                        if svc.is_masked {
                            ui.colored_label(ui::semantic_text(ui::BAD), t("[MASCHERATO]"));
                        } else {
                            let color = match svc.active_state.as_str() {
                                "active" => ui::semantic_text(ui::GOOD),
                                "failed" => ui::semantic_text(ui::BAD),
                                _ => ui::text_dim(),
                            };
                            ui.label(egui::RichText::new(&svc.active_state).color(color));
                        }
                    });
                    row.col(|ui| {
                        ui.label(egui::RichText::new(&svc.description).size(12.0).color(ui::text_dim()));
                    });
                    row.col(|ui| {
                        if svc.is_masked {
                            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Smaschera e Attiva")).small()).clicked() {
                                let script = format!("systemctl unmask {} && systemctl enable --now {}", util::shq(&name), util::shq(&name));
                                action_cmd = Some(("services".into(), script, true));
                            }
                        } else {
                            let is_active = svc.active_state == "active";
                            let lbl = if is_active { t("Ferma") } else { t("Avvia") };
                            if ui.add_enabled(self.action_enabled(), egui::Button::new(lbl).small()).clicked() {
                                let action = if is_active { "stop" } else { "start" };
                                let script = format!("systemctl {} {}", action, util::shq(&name));
                                action_cmd = Some(("services".into(), script, true));
                            }
                        }
                        if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Log")).small()).clicked() {
                            let script = format!("journalctl -u {} -n 50 --no-pager", util::shq(&name));
                            action_cmd = Some(("services".into(), script, false));
                        }
                    });
                });
            });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let n = self.svc_selected.iter().filter(|x| visible_names.contains(*x)).count();
            if ui.add_enabled(n > 0 && self.action_enabled(), egui::Button::new(tf("Avvia selezionati ({})", &[&n.to_string()]))).clicked() {
                bulk_action = Some("start");
            }
            if ui.add_enabled(n > 0 && self.action_enabled(), egui::Button::new(tf("Ferma selezionati ({})", &[&n.to_string()]))).clicked() {
                bulk_action = Some("stop");
            }
            if ui.small_button(t("Deseleziona tutto")).clicked() {
                self.svc_selected.retain(|n| !visible_names.contains(n));
            }
        });

        if let Some(action) = bulk_action {
            let targets: Vec<String> = self
                .svc_selected
                .iter()
                .filter(|n| visible_names.contains(*n))
                .cloned()
                .collect();
            if !targets.is_empty() {
                let args = targets.iter().map(|s| util::shq(s)).collect::<Vec<_>>().join(" ");
                let script = format!("systemctl {action} {args}");
                let job = crate::cmd_job("services", script, true);
                self.spawn(ctx, job);
                self.svc_selected.retain(|n| !targets.contains(n));
            }
        }

        if let Some((_key, script, privileged)) = action_cmd {
            let job = crate::cmd_job("services", script, privileged);
            self.spawn(ctx, job);
        }
    }
}
