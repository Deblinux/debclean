use crate::i18n::t;
use crate::{ui, App};
use eframe::egui;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct BatteryHealthInfo {
    pub name: String,
    pub status: String,
    pub cycle_count: Option<u32>,
    pub design_capacity_wh: f32,
    pub full_charge_capacity_wh: f32,
    pub health_percent: f32,
    pub estimated_remaining_life: String,
}

#[derive(Clone, Debug)]
pub struct StorageHealthInfo {
    pub device: String,
    pub model: String,
    pub is_nvme: bool,
    pub is_rotational: bool,
    pub wear_level_percent: Option<u8>,
    pub power_on_hours: Option<u64>,
    pub data_written_tb: Option<f64>,
    pub temp_celsius: Option<i32>,
    pub status_summary: String,
}

/// Raccoglie i dati di salute delle batterie da /sys/class/power_supply/
fn scan_batteries() -> Vec<BatteryHealthInfo> {
    let mut batteries = Vec::new();
    let base = Path::new("/sys/class/power_supply");
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            let supply_type = fs::read_to_string(path.join("type")).unwrap_or_default();
            if supply_type.trim() != "Battery" {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            let status = fs::read_to_string(path.join("status"))
                .unwrap_or_else(|_| "Unknown".to_string())
                .trim()
                .to_string();

            let cycle_count = fs::read_to_string(path.join("cycle_count"))
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .filter(|&c| c > 0);

            // I valori possono essere espressi in microWatt-ora (energy_*) o microAmpere-ora (charge_*)
            let (design_raw, full_raw) = if path.join("energy_full_design").exists() {
                let d = fs::read_to_string(path.join("energy_full_design"))
                    .ok()
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .unwrap_or(0.0);
                let f = fs::read_to_string(path.join("energy_full"))
                    .ok()
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .unwrap_or(0.0);
                (d / 1_000_000.0, f / 1_000_000.0)
            } else {
                let d = fs::read_to_string(path.join("charge_full_design"))
                    .ok()
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .unwrap_or(0.0);
                let f = fs::read_to_string(path.join("charge_full"))
                    .ok()
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .unwrap_or(0.0);
                // Conversione approssimativa (mAh -> Wh stimati se manca voltaggio)
                let volt = fs::read_to_string(path.join("voltage_min_design"))
                    .ok()
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .unwrap_or(11_400_000.0)
                    / 1_000_000.0;
                ((d / 1_000_000.0) * volt, (f / 1_000_000.0) * volt)
            };

            let health_percent = if design_raw > 0.0 {
                ((full_raw / design_raw) * 100.0).clamp(0.0, 100.0)
            } else {
                100.0
            };

            let estimated_remaining_life = if health_percent >= 85.0 {
                t("Ottimo stato: autonomia vicina ai valori di fabbrica.").to_string()
            } else if health_percent >= 70.0 {
                t("Usura moderata: prestare attenzione nei prossimi 12 mesi.").to_string()
            } else {
                t("Degrado critico: consigliata la sostituzione della batteria.").to_string()
            };

            batteries.push(BatteryHealthInfo {
                name,
                status,
                cycle_count,
                design_capacity_wh: design_raw,
                full_charge_capacity_wh: full_raw,
                health_percent,
                estimated_remaining_life,
            });
        }
    }
    batteries
}

/// Raccoglie i dati dei dischi NVMe e SSD

/// Raccoglie i dati dei dischi NVMe, SSD e HDD
fn scan_storage() -> Vec<StorageHealthInfo> {
    let mut disks = Vec::new();
    let block_dir = Path::new("/sys/block");
    if let Ok(entries) = fs::read_dir(block_dir) {
        for entry in entries.flatten() {
            let dev_name = entry.file_name().to_string_lossy().to_string();
            // Filtra solo dischi fisici reali (nvme*, sd*)
            if !dev_name.starts_with("nvme") && !dev_name.starts_with("sd") {
                continue;
            }

            let sys_path = entry.path();
            let model = fs::read_to_string(sys_path.join("device/model"))
                .unwrap_or_else(|_| dev_name.clone())
                .trim()
                .to_string();

            let is_nvme = dev_name.starts_with("nvme");
            
            // Controlla se è un HDD meccanico (1 = rotazionale/HDD, 0 = SSD/Flash)
            let is_rotational = fs::read_to_string(sys_path.join("queue/rotational"))
                .ok()
                .and_then(|s| s.trim().parse::<u8>().ok())
                .unwrap_or(0) == 1;

            let dev_path = format!("/dev/{}", dev_name);

            // Lettura dati SMART tramite smartctl
            let mut wear_level = None;
            let mut power_on_hours = None;
            let mut data_written_tb = None;
            let mut temp_celsius = None;

            if let Ok(output) = Command::new("smartctl")
                .args(["-A", "-j", &dev_path])
                .output()
            {
                if let Ok(json_str) = String::from_utf8(output.stdout) {
                    // Parsing usura (comune su NVMe e alcuni SSD moderni)
                    if let Some(pos) = json_str.find("\"percentage_used\":") {
                        let sub = &json_str[pos + 18..];
                        if let Some(val) = sub
                            .split(|c: char| !c.is_ascii_digit())
                            .find(|s| !s.is_empty())
                        {
                            wear_level = val.parse::<u8>().ok();
                        }
                    }
                    // Ore di attività
                    if let Some(pos) = json_str.find("\"power_on_time\":") {
                        let sub = &json_str[pos..];
                        if let Some(h_pos) = sub.find("\"hours\":") {
                            let h_sub = &sub[h_pos + 8..];
                            if let Some(val) = h_sub
                                .split(|c: char| !c.is_ascii_digit())
                                .find(|s| !s.is_empty())
                            {
                                power_on_hours = val.parse::<u64>().ok();
                            }
                        }
                    }
                    // Terabyte scritti (TBW)
                    if let Some(pos) = json_str.find("\"data_units_written\":") {
                        let sub = &json_str[pos + 21..];
                        if let Some(val) = sub
                            .split(|c: char| !c.is_ascii_digit())
                            .find(|s| !s.is_empty())
                        {
                            if let Ok(units) = val.parse::<f64>() {
                                data_written_tb = Some((units * 512_000.0) / 1_000_000_000_000.0);
                            }
                        }
                    }
                    // Temperatura
                    if let Some(pos) = json_str.find("\"temperature\":") {
                        let sub = &json_str[pos..];
                        if let Some(c_pos) = sub.find("\"current\":") {
                            let c_sub = &sub[c_pos + 10..];
                            if let Some(val) = c_sub
                                .split(|c: char| !c.is_ascii_digit())
                                .find(|s| !s.is_empty())
                            {
                                temp_celsius = val.parse::<i32>().ok();
                            }
                        }
                    }
                }
            }

            // Generazione dello stato basata sui dati effettivi reperiti
            let status_summary = if is_rotational {
                if power_on_hours.is_some() || temp_celsius.is_some() {
                    t("Disco meccanico (HDD): monitoraggio operativo attivo.").to_string()
                } else {
                    t("Periferica di archiviazione o lettore di schede (dati SMART non supportati).").to_string()
                }
            } else {
                match wear_level {
                    Some(w) if w >= 90 => t("ATTENZIONE: Usura celle critica (>90%). Eseguire subito un backup!").to_string(),
                    Some(w) if w >= 70 => t("Usura avanzata: monitorare frequentemente i dati SMART.").to_string(),
                    Some(_) => t("Stato di salute eccellente: nessun degrado anomalo.").to_string(),
                    None => {
                        if power_on_hours.is_some() || temp_celsius.is_some() {
                            t("SSD operativo: parametri di temperatura e ore letti correttamente.").to_string()
                        } else {
                            t("Dati SMART non accessibili o periferica priva di sensori SMART.").to_string()
                        }
                    }
                }
            };

            disks.push(StorageHealthInfo {
                device: dev_path,
                model,
                is_nvme,
                is_rotational,
                wear_level_percent: wear_level,
                power_on_hours,
                data_written_tb,
                temp_celsius,
                status_summary,
            });
        }
    }
    disks
}

impl App {
    pub(crate) fn ui_hardware_health(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui::title(ui, "Guardiano Predittivo");

        ui::card(ui, |ui| {
            ui::section(ui, "🛡", "Guardiano Predittivo", "Stato di salute di batteria e archiviazione");
            ui.label(
                egui::RichText::new(t(
                    "Analisi predittiva dello stato di salute di SSD/NVMe e batteria: calcola l'usura reale delle celle, i cicli di carica e i Terabyte scritti per prevenire guasti hardware.",
                ))
                .size(15.0)
                .color(ui::text_dim()),
            );
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui::section(ui, "🔋", "STATO DI DEGRADO BATTERIA", "Capacità, cicli e salute stimata");
            let batteries = scan_batteries();
            if batteries.is_empty() {
                ui.label(t("Nessuna batteria rilevata (sistema desktop o sensore non disponibile)."));
            } else {
                for (idx, bat) in batteries.into_iter().enumerate() {
                    if idx > 0 {
                        ui.add_space(6.0);
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(format!("🔋 {}", bat.name)).strong().size(17.0));
                        ui.label(egui::RichText::new(format!("({})", bat.status)).color(ui::text_dim()));
                    });
                    ui.add_space(3.0);
                    let health_color = if bat.health_percent >= 85.0 {
                        egui::Color32::from_rgb(70, 200, 110)
                    } else if bat.health_percent >= 70.0 {
                        ui::WARN
                    } else {
                        egui::Color32::from_rgb(230, 70, 70)
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(t("Salute reale:"));
                        ui.label(egui::RichText::new(format!("{:.1}%", bat.health_percent)).strong().size(16.0).color(health_color));
                    });
                    let progress = (bat.health_percent / 100.0).clamp(0.0, 1.0);
                    let pct_txt = format!(
                        "{:.1} Wh / {:.1} Wh ({})",
                        bat.full_charge_capacity_wh,
                        bat.design_capacity_wh,
                        t("Capacità attuale vs Originale")
                    );
                    let text_color = ui::progress_text_color(health_color);
                    ui.scope(|ui| {
                        ui.visuals_mut().override_text_color = Some(text_color);
                        ui.visuals_mut().selection.stroke.color = text_color;
                        ui.add(egui::ProgressBar::new(progress).fill(health_color).text(pct_txt));
                    });
                    ui.add_space(2.0);
                    if let Some(cycles) = bat.cycle_count {
                        ui.label(format!("{}: {}", t("Cicli di carica completati"), cycles));
                    }
                    ui.label(egui::RichText::new(&bat.estimated_remaining_life).italics().color(health_color));
                }
            }
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui::section(ui, "💾", "ANALISI PREDITTIVA SSD E NVME", "Usura, dati scritti e temperatura");
            let disks = scan_storage();
            if disks.is_empty() {
                ui.label(t("Nessun disco SSD o NVMe rilevato."));
            } else {
                for (idx, disk) in disks.into_iter().enumerate() {
                    if idx > 0 {
                        ui.add_space(6.0);
                    }
                    ui.horizontal_wrapped(|ui| {
                        let tag = if disk.is_nvme { "NVMe" } else if disk.is_rotational { "HDD" } else { "SSD" };
                        ui.label(egui::RichText::new(format!("💾 {} ({})", disk.model, disk.device)).strong().size(17.0));
                        ui.label(egui::RichText::new(format!("[{}]", tag)).color(ui::text_dim()));
                    });
                    ui.add_space(3.0);
                    if let Some(wear) = disk.wear_level_percent {
                        let remaining_health = 100u8.saturating_sub(wear);
                        let color = if remaining_health >= 30 {
                            egui::Color32::from_rgb(70, 200, 110)
                        } else if remaining_health >= 10 {
                            ui::WARN
                        } else {
                            egui::Color32::from_rgb(230, 70, 70)
                        };
                        let txt = format!(
                            "{}: {}% ({}: {}%)",
                            t("Vita utile residua stimata"),
                            remaining_health,
                            t("Usura"),
                            wear
                        );
                        let text_color = ui::progress_text_color(color);
                        ui.scope(|ui| {
                            ui.visuals_mut().override_text_color = Some(text_color);
                            ui.visuals_mut().selection.stroke.color = text_color;
                            ui.add(egui::ProgressBar::new(remaining_health as f32 / 100.0).fill(color).text(txt));
                        });
                    }
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        if let Some(hours) = disk.power_on_hours {
                            ui.label(format!("{}: {} h", t("Ore di attività"), hours));
                        }
                        if let Some(tbw) = disk.data_written_tb {
                            ui.label(format!("• {}: {:.2} TB", t("Dati scritti (TBW)"), tbw));
                        }
                        if let Some(temp) = disk.temp_celsius {
                            ui.label(format!("• {}: {} °C", t("Temperatura"), temp));
                        }
                    });
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(&disk.status_summary).italics());
                }
            }
        });
    }
}
