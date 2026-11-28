use crate::i18n::{t, tf};
use crate::{ui, util, App, Chan, Msg, ProgressReporter};
use eframe::egui;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct VpnProfile {
    pub name: String,
    pub kind: String, // "WireGuard" o "OpenVPN"
    pub active: bool,
    pub path: Option<PathBuf>,
}

pub fn read_vpns() -> Vec<VpnProfile> {
    let mut list = Vec::new();

    // 1. Cerca configurazioni WireGuard in /etc/wireguard/
    if let Ok(rd) = fs::read_dir("/etc/wireguard") {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().map(|e| e == "conf").unwrap_or(false) {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    // `wg show` richiede root: chiamarlo qui, ad ogni frame della
                    // UI, aprirebbe ripetutamente la finestra pkexec. La directory
                    // `wireguard` sotto /sys/class/net è leggibile da chiunque e
                    // indica un'interfaccia WG realmente attiva (non una qualsiasi
                    // interfaccia con lo stesso nome).
                    let active = Path::new("/sys/class/net")
                        .join(stem)
                        .join("wireguard")
                        .exists();
                    list.push(VpnProfile {
                        name: stem.to_string(),
                        kind: "WireGuard".into(),
                        active,
                        path: Some(p),
                    });
                }
            }
        }
    }

    // 2. Cerca VPN NetworkManager (WireGuard / OpenVPN)
    let (_, o) = util::run_shell("nmcli -t -f NAME,TYPE,ACTIVE connection show 2>/dev/null", false);
    for l in o.lines() {
        // `nmcli -t` usa ':' come separatore: si legge da destra per non
        // troncare i nomi che contengono ':' (escapato da nmcli come `\:`).
        let mut parts = l.rsplitn(3, ':');
        let act_s = parts.next().unwrap_or("");
        let ty = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let act = act_s == "yes";
        if ty == "wireguard" || ty == "vpn" {
            let kind = if ty == "wireguard" { "WireGuard (NM)" } else { "OpenVPN (NM)" };
            if let Some(existing) = list.iter_mut().find(|v| v.name == name) {
                // Un profilo NetworkManager con lo stesso nome di un file
                // WireGuard va gestito con nmcli; si conserva lo stato attivo.
                existing.active |= act;
                existing.kind = kind.into();
                existing.path = None;
            } else {
                list.push(VpnProfile {
                    name: name.to_string(),
                    kind: kind.into(),
                    active: act,
                    path: None,
                });
            }
        }
    }
    list
}

impl App {
    pub(crate) fn ui_vpn(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Configuratore VPN rapido")).strong().size(20.0));
            ui.label(t("Importa configurazioni WireGuard (.conf) o OpenVPN (.ovpn) e attivale/disattivale con un clic."));
        });
        ui.add_space(3.0);

        ui.horizontal(|ui| {
            if ui::primary_button(ui, t("📥 Importa file .conf / .ovpn"), self.action_enabled()).clicked() {
                // `spawn` imposta `busy`: import e connessioni non possono
                // sovrapporsi generando più finestre pkexec.
                self.spawn(ctx, |ch: &Chan| {
                    let mut prog = ProgressReporter::start(
                        ch,
                        "vpn",
                        t("Configurazione VPN in corso..."),
                    );
                    if let Some(files) = util::pick_paths(false) {
                        for f in files {
                            let path = Path::new(&f);
                            let ext = path
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                                .to_lowercase();
                            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("vpn");
                            let q_f = util::shq(&f);
                            let q_stem = util::shq(stem);
                            prog.status(tf("Importazione di {}", &[&f]));

                            if ext == "conf" {
                                // Copia sicura in /etc/wireguard/ con permessi 600
                                let script = format!(
                                    "mkdir -p /etc/wireguard && cp {q_f} /etc/wireguard/{q_stem}.conf && chmod 600 /etc/wireguard/{q_stem}.conf"
                                );
                                let (ok, out) = util::run_shell(&script, true);
                                ch.send(Msg::Log(if ok {
                                    tf("✔ Configurazione VPN importata: {}", &[&stem])
                                } else {
                                    tf(
                                        "✘ Importazione VPN non riuscita: {} ({})",
                                        &[&stem, &util::tail(out.trim(), 1)],
                                    )
                                }));
                            } else if ext == "ovpn" {
                                let script = format!("nmcli connection import type openvpn file {q_f}");
                                let (ok, out) = util::run_shell(&script, false);
                                ch.send(Msg::Log(if ok {
                                    tf("✔ Configurazione VPN importata: {}", &[&stem])
                                } else {
                                    tf(
                                        "✘ Importazione VPN non riuscita: {} ({})",
                                        &[&stem, &util::tail(out.trim(), 1)],
                                    )
                                }));
                            } else {
                                ch.send(Msg::Log(tf("✘ Tipo di file non supportato: {}", &[&f])));
                            }
                        }
                    }
                    ch.send(Msg::Vpns(read_vpns()));
                    prog.finish(t("Completato"));
                });
            }
            if ui.button(t("🔄 Ricarica VPN")).clicked() {
                self.load_vpn(ctx);
            }
            if self.busy {
                ui.spinner();
            }
        });
        ui.add_space(3.0);

        // `read_vpns()` esegue `nmcli`: mai a ogni frame. La lista è aggiornata
        // da `load_vpn()` all'ingresso nella pagina, dal pulsante Ricarica e
        // al termine di import/connessione.
        let vpns = self.vpn_list.clone();
        self.show_out(ui, "vpn", 100.0);
        if vpns.is_empty() {
            ui::card(ui, |ui| {
                ui.label(t("Nessuna VPN configurata. Importa un file WireGuard o OpenVPN per iniziare."));
            });
            return;
        }

        for v in vpns {
            ui::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui::led(ui, Some(v.active), "");
                    ui.label(egui::RichText::new(&v.name).strong().size(18.0));
                    ui.label(egui::RichText::new(format!("({})", v.kind)).color(ui::text_dim()));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let name_q = util::shq(&v.name);
                        if v.active {
                            if ui.add(egui::Button::new(t("Disconnetti")).fill(ui::BAD)).clicked() {
                                let script = if v.kind == "WireGuard" {
                                    format!("wg-quick down {name_q}")
                                } else {
                                    format!("nmcli connection down {name_q}")
                                };
                                let inner = crate::cmd_job("vpn", script, v.kind == "WireGuard");
                                self.spawn(ctx, Box::new(move |ch: &Chan| {
                                    inner(ch);
                                    ch.send(Msg::Vpns(read_vpns()));
                                }));
                            }
                        } else {
                            if ui::primary_button(ui, t("Connetti"), true).clicked() {
                                let script = if v.kind == "WireGuard" {
                                    format!("wg-quick up {name_q}")
                                } else {
                                    format!("nmcli connection up {name_q}")
                                };
                                let inner = crate::cmd_job("vpn", script, v.kind == "WireGuard");
                                self.spawn(ctx, Box::new(move |ch: &Chan| {
                                    inner(ch);
                                    ch.send(Msg::Vpns(read_vpns()));
                                }));
                            }
                        }
                    });
                });
            });
            ui.add_space(2.0);
        }
    }

    pub(crate) fn load_vpn(&mut self, ctx: &egui::Context) {
        // Lettura in background: `nmcli` può bloccarsi se NetworkManager è
        // occupato e non va eseguita sul thread della UI.
        self.bg(ctx, |ch| ch.send(Msg::Vpns(read_vpns())));
        ctx.request_repaint();
    }
}
