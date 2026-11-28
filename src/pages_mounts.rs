use crate::i18n::t;
use crate::{ui, util, App, Msg};
use eframe::egui;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct MountEntry {
    pub device: String,
    pub mountpoint: String,
    pub fstype: String,
    pub size: String,
    pub removable: bool,
    pub mounted: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NetShareKind {
    Smb,
    Nfs,
    Sshfs,
}

impl NetShareKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Smb => "SMB / CIFS (Cartella Windows / NAS)",
            Self::Nfs => "NFS (Cartella di rete Linux / NAS)",
            Self::Sshfs => "SSHFS (Cartella remota via SSH)",
        }
    }

    pub fn required_tool(self) -> &'static str {
        match self {
            Self::Smb => "mount.cifs",
            Self::Nfs => "mount.nfs",
            Self::Sshfs => "sshfs",
        }
    }

    pub fn apt_package(self) -> &'static str {
        match self {
            Self::Smb => "cifs-utils",
            Self::Nfs => "nfs-common",
            Self::Sshfs => "sshfs",
        }
    }
}

#[derive(Clone)]
pub struct MountsState {
    pub entries: Vec<MountEntry>,
    pub share_kind: NetShareKind,
    pub host: String,
    pub remote_path: String,
    pub local_name: String,
    pub username: String,
}

impl Default for MountsState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            share_kind: NetShareKind::Smb,
            host: String::new(),
            remote_path: String::new(),
            local_name: "condivisione_rete".into(),
            username: String::new(),
        }
    }
}

pub fn read_mounts() -> Vec<MountEntry> {
    let (_, out) = util::run_shell(
        "lsblk -rpo NAME,MOUNTPOINT,FSTYPE,SIZE,RM -e 7,11 2>/dev/null",
        false,
    );
    let mut list = Vec::new();
    for line in out.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 4 {
            let device = cols[0].to_string();
            let mp = if cols[1] == "-" { "" } else { cols[1] };
            let fstype = cols[2].to_string();
            let size = cols[3].to_string();
            if device == "NAME" || fstype == "FSTYPE" {
                continue;
            }
            let removable = cols.get(4).copied() == Some("1");
            if fstype.is_empty() && mp.is_empty() {
                continue;
            }
            list.push(MountEntry {
                device,
                mountpoint: mp.to_string(),
                fstype,
                size,
                removable,
                mounted: !mp.is_empty(),
            });
        }
    }
    list
}

fn sanitize_folder_name(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "condivisione_rete".to_string()
    } else {
        cleaned
    }
}

impl App {
    pub(crate) fn load_mounts(&self, ctx: &egui::Context) {
        self.bg(ctx, |ch| {
            ch.send(Msg::Mounts(read_mounts()));
        });
    }

    pub(crate) fn ui_mounts(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Dischi, USB e Condivisioni di Rete");
        ui::hint(
            ui,
            "Monta o smonta in sicurezza chiavette USB, partizioni e cartelle di rete (SMB, NFS o SSHFS) senza usare il terminale.",
        );

        ui::sub_tabs(
            ui,
            &mut self.sub,
            &["Dischi e USB", "Condivisioni di rete (SMB / NFS / SSHFS)"],
        );

        match self.sub {
            0 => self.ui_local_mounts(ui, ctx),
            _ => self.ui_network_shares(ui, ctx),
        }
    }

    fn ui_local_mounts(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card(ui, |ui| {
            ui::section(ui, "💾", "Dischi e USB", "Gestisci montaggio e smontaggio sicuro dei dispositivi");
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔄 Aggiorna dispositivi"))).clicked() {
                    self.load_mounts(ctx);
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });

        ui.add_space(4.0);

        if self.mounts_state.entries.is_empty() {
            ui::card(ui, |ui| {
                ui.label(t("Nessun dispositivo caricabile o elenco non ancora aggiornato. Premi «Aggiorna dispositivi»."));
            });
        } else {
            let entries = self.mounts_state.entries.clone();
            for item in entries {
                ui::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui::led(ui, Some(item.mounted), "");
                        ui.label(egui::RichText::new(&item.device).strong().size(17.0));
                        if !item.fstype.is_empty() {
                            ui.label(egui::RichText::new(format!("[{}]", item.fstype)).color(ui::text_dim()));
                        }
                        if !item.size.is_empty() {
                            ui.label(egui::RichText::new(&item.size).color(ui::accent_text()));
                        }
                        if item.removable {
                            ui.colored_label(ui::semantic_text(ui::WARN), t("Rimob./USB"));
                        }
                        let is_protected =
                            item.mountpoint == "/" || item.mountpoint == "[SWAP]" || item.fstype == "swap";
                        let dev_q = util::shq(&item.device);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if item.mounted {
                                if is_protected {
                                    ui.label(egui::RichText::new(t("Sistema")).color(ui::text_dim()));
                                }
                                if ui
                                    .add_enabled(
                                        self.action_enabled() && !is_protected,
                                        egui::Button::new(
                                        egui::RichText::new(t("⏏ Smonta in sicurezza")).color(egui::Color32::from_rgb(20, 20, 20)),
)
                                     .fill(ui::WARN),
                                    )
                                    .clicked()
                                {
                                    let script = format!(
                                        "if command -v udisksctl >/dev/null 2>&1; then udisksctl unmount -b {dev_q} --no-user-interaction; else umount {dev_q}; fi"
                                    );
                                    self.spawn(ctx, move |ch| {
                                        let job = crate::cmd_job("mounts", script, true);
                                        job(ch);
                                        ch.send(Msg::Mounts(read_mounts()));
                                    });
                                }
                            } else if ui::primary_button(ui, t("📌 Monta"), self.action_enabled()).clicked() {
                                let script = format!(
                                    "if command -v udisksctl >/dev/null 2>&1; then udisksctl mount -b {dev_q} --no-user-interaction; else mkdir -p /mnt/debutility_mount && mount {dev_q} /mnt/debutility_mount; fi"
                                );
                                self.spawn(ctx, move |ch| {
                                    let job = crate::cmd_job("mounts", script, true);
                                    job(ch);
                                    ch.send(Msg::Mounts(read_mounts()));
                                });
                            }
                        });
                    });

                    let mp_text = if item.mounted {
                        format!("{}: {}", t("Montato in"), item.mountpoint)
                    } else {
                        t("Non montato").to_string()
                    };
                    ui.label(egui::RichText::new(mp_text).size(14.0).color(ui::text_dim()));
                });
                ui.add_space(4.0);
            }
        }

        ui::card(ui, |ui| {
            ui::section(ui, "▣", "Output", "Risultato delle operazioni di montaggio");
            self.show_out(ui, "mounts", 180.0);
        });
    }

    fn ui_network_shares(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let kind = self.mounts_state.share_kind;
        let tool = kind.required_tool();
        let pkg = kind.apt_package();
        let have_tool = util::run_shell(&format!("command -v {} >/dev/null 2>&1", util::shq(tool)), false).0;

        ui::card(ui, |ui| {
            ui::section(ui, "🔗", "Condivisioni di rete", "Configura e monta SMB, NFS o SSHFS");
            ui.label(egui::RichText::new(t("Tipo di condivisione")).strong());
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.mounts_state.share_kind, NetShareKind::Smb, "SMB / CIFS");
                ui.selectable_value(&mut self.mounts_state.share_kind, NetShareKind::Nfs, "NFS");
                ui.selectable_value(&mut self.mounts_state.share_kind, NetShareKind::Sshfs, "SSHFS");
            });

            if !have_tool {
                ui.add_space(4.0);
                ui.colored_label(
                    ui::semantic_text(ui::WARN),
                    format!(
                        "{} ({pkg}) {}",
                        t("Strumento di rete non installato"),
                        t("necessario per questo protocollo.")
                    ),
                );
                let install_script = format!("apt-get update && apt-get install -y {}", util::shq(pkg));
                self.run_btn(
                    ui,
                    ctx,
                    t("Installa supporto condivisione"),
                    "mounts",
                    &install_script,
                    true,
                    None,
                );
            }

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Server / IP:"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.mounts_state.host)
                        .hint_text(t("es. 192.168.1.50 o nas.local"))
                        .desired_width(ui::fit_width(ui, 240.0, 110.0)),
                );
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Cartella remota:"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.mounts_state.remote_path)
                        .hint_text(t("es. /Documenti o Media"))
                        .desired_width(ui::fit_width(ui, 240.0, 110.0)),
                );
            });
            if kind != NetShareKind::Nfs {
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Utente (opzionale):"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.mounts_state.username)
                            .hint_text(t("es. mario"))
                            .desired_width(ui::fit_width(ui, 200.0, 100.0)),
                    );
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Nome cartella locale:"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.mounts_state.local_name)
                        .desired_width(ui::fit_width(ui, 200.0, 100.0)),
                );
            });

            let folder = sanitize_folder_name(&self.mounts_state.local_name);
            let local_mp = format!("{}/Mounts/{folder}", util::home());
            ui.label(
                egui::RichText::new(format!("{}: {local_mp}", t("Punto di montaggio")))
                    .size(13.5)
                    .color(ui::text_dim()),
            );

            let host_clean = self.mounts_state.host.trim().to_string();
            let rem_clean = self.mounts_state.remote_path.trim().trim_start_matches('/').to_string();
            let user_clean = self.mounts_state.username.trim().to_string();
            let valid = !host_clean.is_empty() && !rem_clean.is_empty() && have_tool;

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let q_mp = util::shq(&local_mp);
                let mount_cmd = match kind {
                    NetShareKind::Smb => {
                        let unc = format!("//{host_clean}/{rem_clean}");
                        let opts = if user_clean.is_empty() {
                            "guest,iocharset=utf8".to_string()
                        } else {
                            format!("username={},iocharset=utf8", user_clean)
                        };
                        format!(
                            "mkdir -p {q_mp} && mount -t cifs {} {q_mp} -o {}",
                            util::shq(&unc),
                            util::shq(&opts)
                        )
                    }
                    NetShareKind::Nfs => {
                        let src = format!("{host_clean}:/{rem_clean}");
                        format!("mkdir -p {q_mp} && mount -t nfs {} {q_mp}", util::shq(&src))
                    }
                    NetShareKind::Sshfs => {
                        let target = if user_clean.is_empty() {
                            format!("{host_clean}:/{rem_clean}")
                        } else {
                            format!("{user_clean}@{host_clean}:/{rem_clean}")
                        };
                        format!(
                            "mkdir -p {q_mp} && sshfs {} {q_mp}",
                            util::shq(&target)
                        )
                    }
                };

                let needs_priv = kind != NetShareKind::Sshfs;
                if ui.add_enabled(valid && self.action_enabled(), egui::Button::new(t("🔗 Monta cartella di rete"))).clicked() {
                    self.spawn(ctx, crate::cmd_job("mounts", mount_cmd, needs_priv));
                }

                if Path::new(&local_mp).exists()
                    && ui.add_enabled(self.action_enabled(), egui::Button::new(t("⏏ Smonta cartella"))).clicked()
                {
                    let umount_cmd = format!(
                        "fusermount -u {q_mp} 2>/dev/null || umount {q_mp}"
                    );
                    self.spawn(ctx, crate::cmd_job("mounts", umount_cmd, needs_priv));
                }
            });
        });

        ui.add_space(3.0);
        self.show_out(ui, "mounts", 200.0);
    }
}
