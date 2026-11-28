use crate::i18n::{t, tf};
use crate::{ui, util, App, Msg};
use eframe::egui;
use std::collections::HashSet;

#[derive(Clone)]
pub struct SynapticPkg {
    pub name: String,
    pub installed: bool,
    pub desc: String,
}

/// Cerca nei repository APT e incrocia i risultati con i pacchetti installati via dpkg.
pub fn search_apt(query: &str) -> Vec<SynapticPkg> {
    let (_, inst_out) = util::run_shell("dpkg-query -W -f='${Package}\n' 2>/dev/null", false);
    let installed_set: HashSet<String> = inst_out.lines().map(|s| s.trim().to_string()).collect();

    let clean_q = query.replace('\'', "").replace('"', "");
    let cmd = if clean_q.trim().is_empty() {
        "apt-cache search . | sort | head -n 150".to_string()
    } else {
        format!("apt-cache search '{}' | sort | head -n 150", clean_q.trim())
    };

    let (_, out) = util::run_shell(&cmd, false);
    let mut results = Vec::new();

    for line in out.lines() {
        if let Some((name, desc)) = line.split_once(" - ") {
            let pkg_name = name.trim().to_string();
            let is_inst = installed_set.contains(&pkg_name);
            results.push(SynapticPkg {
                name: pkg_name,
                installed: is_inst,
                desc: desc.trim().to_string(),
            });
        }
    }
    results
}

impl App {
    pub(crate) fn synaptic_search(&self, ctx: &egui::Context, query: String) {
        self.bg(ctx, move |ch| {
            ch.send(Msg::Log(format!("▶ Ricerca pacchetti APT: '{}'", query)));
            let list = search_apt(&query);
            ch.send(Msg::SynapticPkgs(list));
        });
    }

    pub(crate) fn ui_synaptic(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Gestore Pacchetti (Synaptic)");
        ui::hint(
            ui,
            "Esplora, cerca, installa o rimuovi pacchetti DEB dai repository ufficiali. Clicca su un pacchetto per leggerne i dettagli.",
        );

        // --- Barra di ricerca e filtri stile Synaptic ---
        ui::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t("Cerca:")).strong());
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.pkg_filter)
                        .hint_text(t("Es. vlc, gimp, curl, htop..."))
                        .desired_width(ui::fit_width(ui, 280.0, 110.0)),
                );
                let enter_pressed = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                if ui::primary_button(ui, t("Cerca"), self.action_enabled()).clicked() || enter_pressed {
                    let q = self.pkg_filter.clone();
                    self.synaptic_search(ctx, q);
                }

                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Aggiorna lista APT"))).clicked() {
                    let job = crate::cmd_job("synaptic_action", "apt-get update".to_string(), true);
                    self.spawn(ctx, job);
                }
            });

            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Mostra:")).color(ui::text_dim()));
                ui.selectable_value(&mut self.synaptic_filter_mode, 0, t("Tutti"));
                ui.selectable_value(&mut self.synaptic_filter_mode, 1, t("Solo installati"));
                ui.selectable_value(&mut self.synaptic_filter_mode, 2, t("Non installati"));
            });
        });

        ui.add_space(3.0);

        // --- Tabella lista pacchetti ---
        let mut action_cmd: Option<(String, String)> = None;
        let mut load_detail_pkg: Option<(usize, String)> = None;

        ui::card(ui, |ui| {
            // Larghezze proporzionali: le righe non sbordano più su schermi
            // stretti e i pulsanti restano sempre raggiungibili.
            let row_w = ui.available_width();
            let status_w = (row_w * 0.10).clamp(54.0, 70.0);
            let name_w = (row_w * 0.26).clamp(90.0, 220.0);
            let desc_w = ((row_w - status_w - name_w - 90.0) * 0.72).max(80.0);
            let desc_chars = ((desc_w / 7.0) as usize).max(14);
            // Intestazione colonne
            ui.horizontal_wrapped(|ui| {
                ui.add_sized([status_w, 20.0], egui::Label::new(egui::RichText::new(t("Stato")).strong()));
                ui.add_sized([name_w, 20.0], egui::Label::new(egui::RichText::new(t("Pacchetto")).strong()));
                ui.add_sized([desc_w, 20.0], egui::Label::new(egui::RichText::new(t("Descrizione")).strong()));
                ui.label(egui::RichText::new(t("Azioni")).strong());
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("synaptic_pkg_list")
                .max_height(185.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let filtered: Vec<(usize, &SynapticPkg)> = self
                        .synaptic_pkgs
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| match self.synaptic_filter_mode {
                            1 => p.installed,
                            2 => !p.installed,
                            _ => true,
                        })
                        .collect();

                    if filtered.is_empty() {
                        ui.label(egui::RichText::new(t("Nessun pacchetto trovato con i filtri attuali.")).color(ui::text_dim()));
                    }

                    for (idx, pkg) in filtered {
                        let is_selected = self.synaptic_selected == Some(idx);

                        ui.horizontal_wrapped(|ui| {
                            // Spia stato: verde se installato, grigio se disponibile
                            let status_color = if pkg.installed { ui::GOOD } else { egui::Color32::GRAY };
                            let status_txt = if pkg.installed { t("● Inst.") } else { t("○ Disp.") };
                            ui.add_sized(
                                [status_w, 24.0],
                                egui::Label::new(egui::RichText::new(status_txt).color(status_color).strong()),
                            );

                            // Nome pacchetto cliccabile per selezionarlo
                            let name_resp = ui.add_sized(
                                [name_w, 24.0],
                                egui::SelectableLabel::new(is_selected, egui::RichText::new(&pkg.name).strong()),
                            );
                            if name_resp.clicked() {
                                load_detail_pkg = Some((idx, pkg.name.clone()));
                            }

                            // Descrizione troncata in base allo spazio disponibile
                            let short_desc: String = pkg.desc.chars().take(desc_chars).collect();
                            ui.add_sized(
                                [desc_w, 24.0],
                                egui::Label::new(egui::RichText::new(short_desc).color(ui::text_dim())),
                            );

                            // Pulsanti Installa / Rimuovi / Purga direttamente sulla riga
                            if !pkg.installed {
                                if ui::primary_button(ui, t("Installa"), self.action_enabled()).clicked() {
                                    action_cmd = Some((
                                        tf("Vuoi installare il pacchetto '{}'?", &[&pkg.name]),
                                        format!("DEBIAN_FRONTEND=noninteractive apt-get install -y {}", pkg.name),
                                    ));
                                }
                            } else {
                                if ui
                                    .add_enabled(self.action_enabled(), egui::Button::new(t("Rimuovi")))
                                    .clicked()
                                {
                                    action_cmd = Some((
                                        tf("Vuoi rimuovere '{}'?", &[&pkg.name]),
                                        format!("DEBIAN_FRONTEND=noninteractive apt-get remove -y {}", pkg.name),
                                    ));
                                }
                                if ui
                                    .add_enabled(
                                        self.action_enabled(),
                                        egui::Button::new(egui::RichText::new(t("Purga")).color(egui::Color32::WHITE))
                                            .fill(ui::BAD),
                                    )
                                    .clicked()
                                {
                                    action_cmd = Some((
                                        tf("Vuoi eliminare '{}' e tutti i suoi file di configurazione?", &[&pkg.name]),
                                        format!("DEBIAN_FRONTEND=noninteractive apt-get purge -y {}", pkg.name),
                                    ));
                                }
                            }
                        });
                        ui.separator();
                    }
                });
        });

        // Caricamento dettagli pacchetto selezionato
        if let Some((idx, pkg_name)) = load_detail_pkg {
            self.synaptic_selected = Some(idx);
            let (_, info) = util::run_shell(&format!("apt-cache show {} | head -n 35", pkg_name), false);
            self.synaptic_details = info;
        }

        // Esecuzione comando con conferma
        if let Some((confirm_msg, script)) = action_cmd {
            let job = crate::cmd_job("synaptic_action", script, true);
            self.confirm = Some((confirm_msg, job));
        }

        ui.add_space(3.0);

        // --- Pannello inferiore: Dettagli pacchetto & Log installazione ---
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(idx) = self.synaptic_selected {
                    if let Some(p) = self.synaptic_pkgs.get(idx) {
                        ui.label(egui::RichText::new(tf("Dettagli pacchetto: {}", &[&p.name])).strong().color(ui::accent_text()));
                    }
                } else {
                    ui.label(egui::RichText::new(t("Dettagli pacchetto (clicca un nome sopra) e Log APT")).strong());
                }
            });
            ui.add_space(2.0);
            if !self.synaptic_details.is_empty() {
                ui::output_box(ui, &self.synaptic_details, 110.0);
                ui.add_space(3.0);
            }
            self.show_out_follow(ui, "synaptic_action", 100.0);
        });
    }
}
