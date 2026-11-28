use crate::i18n::{t, tf};
use crate::{ui, util, App, Msg};
use eframe::egui;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct BootService {
    pub name: String,
    pub duration_str: String,
    pub duration_ms: u64,
}

#[derive(Clone, Debug)]
pub struct AutostartItem {
    pub file_path: PathBuf,
    pub name: String,
    pub exec: String,
    pub enabled: bool,
    pub delay_seconds: u32,
}

pub fn read_boot_blame() -> (String, Vec<BootService>) {
    let total_out = Command::new("systemd-analyze").output();
    let total_summary = if let Ok(res) = total_out {
        String::from_utf8_lossy(&res.stdout).trim().to_string()
    } else {
        String::new()
    };

    let mut list = Vec::new();
    let out = Command::new("systemd-analyze")
        .arg("blame")
        .output();

    if let Ok(res) = out {
        let s = String::from_utf8_lossy(&res.stdout);
        for line in s.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let duration_str = parts[0].to_string();
                let name = parts[1..].join(" ");
                let duration_ms = parse_duration_to_ms(&duration_str);
                list.push(BootService {
                    name,
                    duration_str,
                    duration_ms,
                });
            }
        }
    }
    list.sort_by(|a, b| b.duration_ms.cmp(&a.duration_ms));
    (total_summary, list)
}

fn parse_duration_to_ms(s: &str) -> u64 {
    let s = s.trim();
    if s.ends_with("ms") {
        s.trim_end_matches("ms").parse().unwrap_or(0)
    } else if s.ends_with('s') {
        let sec_str = s.trim_end_matches('s');
        (sec_str.parse::<f64>().unwrap_or(0.0) * 1000.0) as u64
    } else {
        0
    }
}

pub fn read_autostart_items() -> Vec<AutostartItem> {
    let mut items = Vec::new();
    // `util::home()` risolve l'utente reale anche sotto pkexec/root.
    let home = util::home();
    if home.is_empty() {
        return items;
    }

    let dirs = vec![
        PathBuf::from(&home).join(".config").join("autostart"),
        PathBuf::from(&home).join(".local").join("share").join("autostart"),
    ];

    for dir in dirs {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("desktop") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        let mut name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                        let mut exec = String::new();
                        let mut enabled = true;
                        let mut delay_seconds = 0u32;

                        for line in content.lines() {
                            let line = line.trim();
                            if let Some(n) = line.strip_prefix("Name=") {
                                name = n.to_string();
                            } else if let Some(e) = line.strip_prefix("Exec=") {
                                let exec_str = e.to_string();
                                if exec_str.starts_with("sleep ") {
                                    if let Some(idx) = exec_str.find("&&") {
                                        let sleep_part = &exec_str[..idx];
                                        if let Ok(sec) = sleep_part.trim_start_matches("sleep ").trim().parse::<u32>() {
                                            delay_seconds = sec;
                                        }
                                        exec = exec_str[idx + 2..].trim().to_string();
                                    } else {
                                        exec = exec_str;
                                    }
                                } else {
                                    exec = exec_str;
                                }
                            } else if let Some(h) = line.strip_prefix("Hidden=") {
                                if h.to_lowercase() == "true" {
                                    enabled = false;
                                }
                            } else if let Some(en) = line.strip_prefix("X-GNOME-Autostart-enabled=") {
                                if en.to_lowercase() == "false" {
                                    enabled = false;
                                }
                            }
                        }

                        items.push(AutostartItem {
                            file_path: path,
                            name,
                            exec,
                            enabled,
                            delay_seconds,
                        });
                    }
                }
            }
        }
    }

    items
}

impl App {
    pub(crate) fn ui_boot(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Gestore Avvio & Autostart Avanzato"));
        ui::hint(
            ui,
            t("Analizza i tempi di boot di systemd e gestisci i programmi all'avvio utente con controllo dei ritardi (delay), ideale per window manager leggeri."),
        );

        ui.add_space(3.0);

        ui.horizontal(|ui| {
            if ui.selectable_label(self.boot_tab == 0, t("⏱ Tempi di Boot (Systemd)")).clicked() {
                self.boot_tab = 0;
            }
            if ui.selectable_label(self.boot_tab == 1, t("🚀 Programmi Autostart & Ritardi")).clicked() {
                self.boot_tab = 1;
                self.autostart_items = read_autostart_items();
            }
        });

        ui.add_space(5.0);

        if self.boot_tab == 0 {
            ui.horizontal(|ui| {
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Aggiorna Analisi Avvio"))).clicked() {
                    self.spawn(ctx, move |ch| {
                        ch.send(Msg::Log("▶ Eseguo systemd-analyze blame...".into()));
                        let (summary, items) = read_boot_blame();
                        ch.send(Msg::BootSummary(summary));
                        ch.send(Msg::BootItems(items));
                        ch.send(Msg::Log("✔ Analisi avvio completata.".into()));
                    });
                }
                if self.busy {
                    ui.spinner();
                }
            });

            ui.add_space(4.0);
            self.show_out_follow(ui, "boot", 80.0);
            ui.add_space(3.0);

            if !self.boot_summary.is_empty() {
                ui::card(ui, |ui| {
                    ui.label(egui::RichText::new(&self.boot_summary).strong().color(crate::ui::accent_text()).size(15.0));
                });
                ui.add_space(3.0);
            }

            ui::card(ui, |ui| {
                ui.label(egui::RichText::new(format!("{} ({})", t("Servizi ordinati per tempo di avvio"), self.boot_items.len())).strong().color(crate::ui::accent_text()));
                ui.add_space(3.0);

                if self.boot_items.is_empty() {
                    ui.label(t("Nessun dato disponibile. Clicca su 'Aggiorna Analisi Avvio' per scansionare il sistema."));
                } else {
                    let items = self.boot_items.clone();
                    let busy = self.busy;

                    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        for item in &items {
                            ui.horizontal(|ui| {
                                let color = if item.duration_ms > 1500 {
                                    egui::Color32::from_rgb(255, 80, 80)
                                } else if item.duration_ms > 700 {
                                    crate::ui::WARN
                                } else {
                                    crate::ui::GOOD
                                };

                                ui.label(egui::RichText::new(&item.duration_str).strong().monospace().color(color));
                                ui.add_space(5.0);
                                ui.label(egui::RichText::new(&item.name).strong().size(14.0));

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let name = item.name.clone();
                                    if ui.add_enabled(!busy, egui::Button::new(egui::RichText::new(t("Disabilita")))).clicked() {
                                        let script = format!("systemctl disable {}", util::shq(&name));
                                        let job = crate::cmd_job("boot", script, true);
                                        self.spawn(ctx, job);
                                    }
                                });
                            });
                            ui.separator();
                        }
                    });
                }
            });

        } else {
            ui::card(ui, |ui| {
                ui.heading(t("Aggiungi nuova applicazione all'avvio con ritardo"));
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label(t("Nome:"));
                    ui.text_edit_singleline(&mut self.new_autostart_name);
                });
                ui.horizontal(|ui| {
                    ui.label(t("Comando:"));
                    ui.text_edit_singleline(&mut self.new_autostart_exec);
                });
                ui.horizontal(|ui| {
                    ui.label(t("Ritardo (secondi):"));
                    ui.text_edit_singleline(&mut self.new_autostart_delay);
                });

                ui.add_space(3.0);
                if ui.button(egui::RichText::new(t("➕ Crea programma in autostart")).strong().size(14.0)).clicked() {
                    self.add_autostart_item();
                }
            });

            ui.add_space(5.0);

            ui::card(ui, |ui| {
                ui.heading(t("Programmi registrati in autostart utente"));
                ui.separator();

                if self.autostart_items.is_empty() {
                    ui.label(t("Nessun programma trovato in ~/.config/autostart/."));
                } else {
                    let items = self.autostart_items.clone();
                    for (i, item) in items.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let mut enabled = item.enabled;
                            if ui.checkbox(&mut enabled, "").changed() {
                                self.toggle_autostart_item(i, enabled);
                            }

                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&item.name).strong());
                                    if item.delay_seconds > 0 {
                                        ui.colored_label(crate::ui::accent_text(), tf("(Ritardo: {}s)", &[&item.delay_seconds]));
                                    }
                                });
                                ui.label(egui::RichText::new(&item.exec).color(crate::ui::text_dim()).size(12.0));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button(t("🗑 Elimina")).clicked() {
                                    self.remove_autostart_item(i);
                                }
                            });
                        });
                        ui.separator();
                    }
                }
            });
        }
    }

    fn add_autostart_item(&mut self) {
        if self.new_autostart_name.is_empty() || self.new_autostart_exec.is_empty() {
            return;
        }

        let home = util::home();
        if home.is_empty() {
            return;
        }

        let autostart_dir = PathBuf::from(&home).join(".config").join("autostart");
        let _ = fs::create_dir_all(&autostart_dir);

        let filename = format!("{}.desktop", self.new_autostart_name.to_lowercase().replace(' ', "_"));
        let file_path = autostart_dir.join(filename);

        let delay: u32 = self.new_autostart_delay.trim().parse().unwrap_or(0);
        let final_exec = if delay > 0 {
            format!("sleep {} && {}", delay, self.new_autostart_exec)
        } else {
            self.new_autostart_exec.clone()
        };

        let content = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            self.new_autostart_name, final_exec
        );

        let _ = fs::write(file_path, content);
        self.autostart_items = read_autostart_items();
        self.new_autostart_name.clear();
        self.new_autostart_exec.clear();
        self.new_autostart_delay.clear();
    }

    fn toggle_autostart_item(&mut self, index: usize, enabled: bool) {
        if let Some(item) = self.autostart_items.get_mut(index) {
            item.enabled = enabled;
            if let Ok(content) = fs::read_to_string(&item.file_path) {
                let mut new_content = String::new();
                for line in content.lines() {
                    if line.starts_with("Hidden=") || line.starts_with("X-GNOME-Autostart-enabled=") {
                        continue;
                    }
                    new_content.push_str(line);
                    new_content.push('\n');
                }
                if !enabled {
                    new_content.push_str("Hidden=true\n");
                    new_content.push_str("X-GNOME-Autostart-enabled=false\n");
                }
                let _ = fs::write(&item.file_path, new_content);
            }
        }
    }

    fn remove_autostart_item(&mut self, index: usize) {
        if index < self.autostart_items.len() {
            let item = self.autostart_items[index].clone();
            let _ = fs::remove_file(item.file_path);
            self.autostart_items = read_autostart_items();
        }
    }
}
