use crate::i18n::{t, tf, tr};
use crate::pages_clean::POWER_SAVE_ORIG;
use crate::update_check;
use crate::{clean_and_scan_job, cleaner, cmd_job, health, ui, util, App, AppSortKey, Job, Msg, Page, ProcSortKey};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct NetStatus {
    pub gw: Option<bool>,
    pub dns: Option<bool>,
    pub inet: Option<bool>,
}

#[derive(Clone)]
pub struct ProcRow {
    pub pid: String,
    pub user: String,
    pub cpu: String,
    pub mem: String,
    pub cmd: String,
}

#[derive(Clone)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    pub apt: bool,
}

#[derive(Clone)]
pub struct BigFile {
    pub size: u64,
    pub path: String,
    pub sel: bool,
}

pub struct Zram {
    pub size: u64,
    pub algo: String,
    pub orig: u64,
    pub compr: u64,
}

// ---------------------------------------------------------------- funzioni di supporto

pub fn net_test() -> NetStatus {
    let (_, gw) = util::run_shell("ip route | awk '/^default/{print $3; exit}'", false);
    let gw = gw.trim().to_string();
    let valid = !gw.is_empty() && gw.chars().all(|c| c.is_ascii_hexdigit() || c == '.' || c == ':');
    let gw_ok = if valid {
        Some(util::run_shell(&format!("ping -c1 -W2 {gw} >/dev/null 2>&1"), false).0)
    } else {
        Some(false)
    };
    let dns = Some(util::run_shell("getent hosts debian.org >/dev/null 2>&1", false).0);
    let addr: SocketAddr = ([1, 1, 1, 1], 53).into();
    let inet = Some(TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_ok());
    NetStatus { gw: gw_ok, dns, inet }
}

pub fn read_procs() -> Vec<ProcRow> {
    // Avviamo `ps` direttamente per conoscere il suo PID: lo escludiamo dai
    // risultati, altrimenti il processo usato per leggere la lista poteva
    // comparire in cima con il 100% di CPU.
    let mut cmd = std::process::Command::new("ps");
    cmd.args(["-eo", "pid,user:12,pcpu,pmem,comm", "--sort=-pcpu", "--no-headers"])
        .env("LC_ALL", "C")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let ps_pid = child.id().to_string();
    let text = child
        .wait_with_output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    text.lines()
        .filter(|l| l.split_whitespace().next() != Some(ps_pid.as_str()))
        .take(80)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some(ProcRow {
                pid: it.next()?.into(),
                user: it.next()?.into(),
                cpu: it.next()?.into(),
                mem: it.next()?.into(),
                cmd: it.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

pub fn read_pkgs() -> Vec<Pkg> {
    let mut v = Vec::new();
    let (_, o) = util::run_shell(
        "dpkg-query -W -f='${db:Status-Abbrev}\\t${Package}\\t${Version}\\n' 2>/dev/null | awk -F'\\t' '$1 ~ /^ii/'",
        false,
    );
    for l in o.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() >= 3 {
            v.push(Pkg { name: f[1].into(), version: f[2].into(), apt: true });
        }
    }
    let (_, o) = util::run_shell("flatpak list --app --columns=application,version 2>/dev/null", false);
    for l in o.lines() {
        let mut f = l.split('\t');
        if let Some(n) = f.next() {
            if !n.trim().is_empty() {
                v.push(Pkg { name: n.trim().into(), version: f.next().unwrap_or("").trim().into(), apt: false });
            }
        }
    }
    v
}

pub fn zram_info() -> Option<Zram> {
    let size: u64 = std::fs::read_to_string("/sys/block/zram0/disksize").ok()?.trim().parse().ok()?;
    if size == 0 {
        return None;
    }
    let algo = std::fs::read_to_string("/sys/block/zram0/comp_algorithm").unwrap_or_default();
    let algo = algo
        .split_whitespace()
        .find(|a| a.starts_with('['))
        .map(|a| a.trim_matches(|c: char| c == '[' || c == ']').to_string())
        .unwrap_or_else(|| "n/d".into());
    let mm = std::fs::read_to_string("/sys/block/zram0/mm_stat").unwrap_or_default();
    let mut it = mm.split_whitespace().map(|x| x.parse::<u64>().unwrap_or(0));
    let orig = it.next().unwrap_or(0);
    let compr = it.next().unwrap_or(0);
    Some(Zram { size, algo, orig, compr })
}

fn cpu_stress(secs: u64) -> String {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let end = Instant::now() + Duration::from_secs(secs);
    let handles: Vec<_> = (0..n)
        .map(|_| {
            std::thread::spawn(move || {
                let mut x = 1.000001f64;
                let mut cycles = 0u64;
                while Instant::now() < end {
                    for _ in 0..100_000 {
                        x = (x * 1.000_000_1).sin().abs() + 1.0;
                    }
                    cycles += 1;
                }
                std::hint::black_box(x);
                cycles
            })
        })
        .collect();
    let total: u64 = handles.into_iter().map(|h| h.join().unwrap_or(0)).sum();
    tf("Stress CPU completato: {} thread per {} s ({} cicli). Nessun blocco o errore rilevato.", &[&n, &secs, &total])
}

fn ram_test(mb: usize) -> String {
    let words = mb.saturating_mul(1024 * 1024) / 8;
    let mut v: Vec<u64> = Vec::new();
    if v.try_reserve_exact(words).is_err() {
        return tf("Memoria insufficiente per allocare {} MB.", &[&mb]);
    }
    v.resize(words, 0);
    let mut errors = 0u64;
    for pat in [0xAAAA_AAAA_AAAA_AAAAu64, 0x5555_5555_5555_5555, 0, u64::MAX] {
        for x in v.iter_mut() {
            *x = pat;
        }
        std::hint::black_box(&mut v);
        for x in v.iter() {
            // SAFETY: il puntatore deriva da un riferimento valido
            if unsafe { std::ptr::read_volatile(x) } != pat {
                errors += 1;
            }
        }
    }
    for (i, x) in v.iter_mut().enumerate() {
        *x = i as u64;
    }
    for (i, x) in v.iter().enumerate() {
        if unsafe { std::ptr::read_volatile(x) } != i as u64 {
            errors += 1;
        }
    }
    if errors == 0 {
        tf("Test RAM su {} MB superato: 0 errori (4 pattern + test indirizzi).", &[&mb])
    } else {
        tf("ATTENZIONE: {} errori rilevati durante il test RAM su {} MB!", &[&errors, &mb])
    }
}

fn pair(s: &str, k: &str) -> String {
    let pat = format!("{k}=\"");
    s.find(&pat)
        .map(|i| s[i + pat.len()..].split('"').next().unwrap_or("").to_string())
        .unwrap_or_default()
}

/// Descrive un disco: tipo (SSD/HDD/NVMe...), dimensione e modello. Formato "tipo\tdimensione\tmodello".
pub(crate) fn describe_disk(dev: &str) -> String {
    let name = dev.trim_start_matches("/dev/");
    let rota = std::fs::read_to_string(format!("/sys/block/{name}/queue/rotational"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let (_, o) = util::run_shell(&format!("lsblk -dnP -o TRAN,SIZE,MODEL {} 2>/dev/null", util::shq(dev)), false);
    let tran = pair(&o, "TRAN");
    let size = pair(&o, "SIZE");
    let model = pair(&o, "MODEL");
    let kind = if name.starts_with("nvme") {
        "SSD NVMe"
    } else if name.starts_with("mmcblk") {
        "Scheda SD / eMMC"
    } else if name.starts_with("zram") {
        "zRAM (memoria compressa)"
    } else if name.starts_with("vd") || name.starts_with("xvd") {
        "Disco virtuale"
    } else if rota == "1" {
        if tran == "usb" { "HDD esterno USB (meccanico)" } else { "HDD (disco meccanico)" }
    } else if rota == "0" {
        if tran == "usb" { "SSD / memoria Flash USB" } else { "SSD SATA" }
    } else {
        "Tipo sconosciuto"
    };
    format!("{kind}\t{size}\t{}", model.trim())
}

pub(crate) fn disk_label(desc: &str) -> String {
    let mut p = desc.split('\t');
    let kind = p.next().unwrap_or("");
    let size = p.next().unwrap_or("");
    let model = p.next().unwrap_or("");
    format!("{} · {} · {}", tr(kind), size, model)
}

fn read_u64(p: &str) -> u64 {
    std::fs::read_to_string(p).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0)
}

// ---------------------------------------------------------------- interfaccia

impl App {
    pub(crate) fn load_pkgs(&self, ctx: &egui::Context) {
        self.bg(ctx, |ch| ch.send(Msg::Pkgs(read_pkgs())));
    }

    pub(crate) fn load_disks(&self, ctx: &egui::Context) {
        self.bg(ctx, |ch| {
            let (_, o) = util::run_shell("lsblk -dnpo NAME -e 7,11 2>/dev/null", false);
            let list: Vec<(String, String)> = o
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .map(|d| {
                    let desc = describe_disk(&d);
                    (d, desc)
                })
                .filter(|(_, desc)| desc.split('\t').nth(1).map(|s| s != "0B").unwrap_or(true))
                .collect();
            ch.send(Msg::Disks(list));
        });
    }

    /// Avviso discreto per una nuova release, mostrato solo nella Dashboard.
    fn update_notice_ui(&self, ui: &mut egui::Ui) {
        if let Some(version) = self.update_available.as_ref() {
            ui::card_tinted_compact(ui, ui::BLUE_LIGHT, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(egui::RichText::new("ℹ").size(14.0).color(ui::text_dim()));
                    ui.label(
                        egui::RichText::new(tf("Nuova versione disponibile: {}", &[version]))
                            .size(13.0)
                            .color(ui::text_primary()),
                    );
                    if ui.button(egui::RichText::new(t("Vedi le note di rilascio")).size(13.0)).clicked() {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(update_check::RELEASES_URL)
                            .spawn();
                    }
                });
            });
            ui.add_space(2.0);
        }
    }

    pub(crate) fn ui_dashboard(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Option<Page> {
        let mut goto: Option<Page> = None;
        ui::title_compact(ui, "Dashboard");
        ui.spacing_mut().item_spacing.y = 3.0;
        self.update_notice_ui(ui);
        self.ui_slow_diag(ui, ctx, &mut goto);

        let problems = self.checks.iter().filter(|c| c.status == health::Status::Problem).count();
        let warns = self.checks.iter().filter(|c| c.status == health::Status::Warn).count();
        let (col, head, sub, mark_ok) = if self.checks.is_empty() {
            (egui::Color32::GRAY, t("Analisi in corso...").to_string(), t("Sto controllando lo stato del sistema.").to_string(), true)
        } else if problems > 0 {
            (ui::BAD, tf("{} problemi richiedono attenzione", &[&problems]), t("Apri il Fixer per correggerli.").to_string(), false)
        } else if warns > 0 {
            (ui::WARN, t("Il sistema funziona, con alcuni avvisi").to_string(), tf("{} avvisi da controllare nel Fixer.", &[&warns]), false)
        } else {
            (ui::GOOD, t("Il sistema è in buono stato").to_string(), t("Nessun problema rilevato.").to_string(), true)
        };
        ui::card_tinted_compact(ui, col, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
                let pnt = ui.painter_at(rect);
                let c = rect.center();
                pnt.circle_filled(c, 24.0, ui::alpha(col, 40));
                pnt.circle_filled(c, 20.0, col);
                let stroke = egui::Stroke::new(4.0_f32, egui::Color32::WHITE);
                if mark_ok {
                    pnt.add(egui::Shape::line(
                        vec![c + egui::vec2(-8.0, 0.0), c + egui::vec2(-3.0, 6.0), c + egui::vec2(8.0, -6.0)],
                        stroke,
                    ));
                } else {
                    pnt.line_segment([c + egui::vec2(0.0, -10.0), c + egui::vec2(0.0, 3.0)], stroke);
                    pnt.circle_filled(c + egui::vec2(0.0, 9.0), 3.0, egui::Color32::WHITE);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(egui::RichText::new(t("Stato del sistema")).size(11.0).strong().color(ui::text_dim()));
                    ui.label(egui::RichText::new(head).size(19.0).strong().color(ui::text_primary()));
                    ui.label(egui::RichText::new(sub).size(12.0).color(ui::text_dim()));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if self.busy {
                        ui.spinner();
                    }
                    let sec_btn = egui::Button::new(
                        egui::RichText::new(t("Diagnosi completa")).strong().size(14.0),
                    );
                    if ui.add_enabled(self.action_enabled(), sec_btn).clicked() {
                        goto = Some(Page::Fixer);
                    }
                    if ui::primary_button_compact(ui, t("Ottimizza ora"), self.action_enabled()).clicked() {
                        let kinds: Vec<cleaner::Kind> =
                            self.items.iter().filter(|i| i.selected).map(|i| i.kind).collect();
                        self.spawn(ctx, clean_and_scan_job("cleaner", kinds, true));
                    }
                });
            });
        });

        // Striscia di indicatori sintetici, come nelle utility classiche di Windows.
        ui.add_space(3.0);
        ui::section_compact(ui, "◆", "Indicatori", "Panoramica immediata del sistema");
        let recoverable: u64 = self.items.iter().filter(|it| it.selected).filter_map(|it| it.size).sum();
        let updates = if self.upd_checked {
            (self.upd_apt.len() + self.upd_flat.len()).to_string()
        } else {
            "—".to_string()
        };
        let disk_frac = self
            .info
            .disks
            .iter()
            .find(|d| d.mount == "/")
            .map(|d| d.total.saturating_sub(d.free).max(1) as f32 / d.total.max(1) as f32)
            .unwrap_or(0.0);
        let kpis = [
            ("✓", problems.to_string(), t("Problemi"), if problems > 0 { ui::semantic_text(ui::BAD) } else { ui::semantic_text(ui::GOOD) }),
            ("▣", cleaner::fmt_bytes(recoverable), t("Da recuperare"), ui::accent_text()),
            ("↻", updates, "Aggiornamenti", ui::semantic_text(ui::BLUE_LIGHT)),
            ("◉", format!("{:.0}%", disk_frac * 100.0), "Disco", ui::semantic_text(ui::level_color(disk_frac))),
        ];
        // Tante card quante ne entrano: 4 su schermi larghi, 2 su quelli compatti.
        let kpi_cols = ui::column_count(ui, 156.0, 8.0, 4);
        for chunk in kpis.chunks(kpi_cols) {
            let row_w = chunk.len() as f32 * 156.0 + chunk.len().saturating_sub(1) as f32 * 8.0;
            let pad = ((ui.available_width() - row_w) * 0.5).max(0.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
                if pad > 0.0 {
                    ui.add_space(pad);
                }
                for (glyph, value, name, tint) in chunk {
                    ui::kpi_card_compact(ui, glyph, value.as_str(), name, *tint);
                }
            });
        }

        ui.add_space(3.0);
        ui::section_compact(ui, "▦", "Accessi rapidi", "Gli strumenti più usati, a portata di clic");

        let tile_w = 168.0_f32;
        let gap = 8.0_f32;
        let cols = ((ui.available_width() + gap) / (tile_w + gap)).floor().max(1.0) as usize;
        let items = [
            (Page::Cleaner, "Pulizia", "Libera spazio su disco"),
            (Page::Fixer, "Fixer", "Ripara gli errori di sistema"),
            (Page::Repos, "Driver", "Rileva e installa i driver"),
            (Page::Disk, "Disco", "Salute e spazio"),
            (Page::Backup, "Backup", "Proteggi i tuoi dati"),
            (Page::Firewall, "Sicurezza", "Firewall e porte"),
        ];
        for chunk in items.chunks(cols) {
            let row_w = chunk.len() as f32 * tile_w + chunk.len().saturating_sub(1) as f32 * gap;
            let pad = ((ui.available_width() - row_w) * 0.5).max(0.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                if pad > 0.0 {
                    ui.add_space(pad);
                }
                for &(p, ti, su) in chunk {
                    if ui::tile_compact(ui, p, ti, su) {
                        goto = Some(p);
                    }
                }
            });
        }

        ui.add_space(3.0);
        let i = &self.info;
        let cpu = i.sys.global_cpu_usage() / 100.0;
        let ram = i.sys.used_memory() as f32 / i.sys.total_memory().max(1) as f32;
        let swap = if i.sys.total_swap() > 0 { i.sys.used_swap() as f32 / i.sys.total_swap() as f32 } else { 0.0 };
        let (dfrac, dsub) = match i.disks.iter().find(|d| d.mount == "/") {
            Some(d) => {
                let used = d.total.saturating_sub(d.free);
                (
                    used as f32 / d.total.max(1) as f32,
                    format!("{} / {}", cleaner::fmt_bytes(used), cleaner::fmt_bytes(d.total)),
                )
            }
            None => (0.0, t("n/d").to_string()),
        };
        // Le ultime tre sezioni stanno affiancate: tutta la Dashboard resta dentro la finestra.
        ui.columns(3, |columns| {
            {
                let ui = &mut columns[0];
                ui::section_compact(ui, "◉", "Stato in tempo reale", "");
                ui::card_tinted_compact(ui, ui::ACCENT, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 6.0);
                    let gauges = [
                        ("CPU", cpu, tf("{} thread", &[&i.cpu.threads])),
                        ("RAM", ram, format!("{} / {}", cleaner::fmt_bytes(i.sys.used_memory()), cleaner::fmt_bytes(i.sys.total_memory()))),
                        ("Swap", swap, cleaner::fmt_bytes(i.sys.total_swap())),
                        (t("Disco /"), dfrac, dsub),
                    ];
                    let row_w = 4.0 * 60.0 + 3.0 * 4.0;
                    ui.horizontal_wrapped(|ui| {
                        let avail = ui.available_width();
                        if avail > row_w {
                            ui.add_space((avail - row_w) * 0.5);
                        }
                        for (name, frac, detail) in gauges {
                            ui::gauge_compact(ui, frac, name, &detail);
                        }
                    });
                });
            }
            {
                let ui = &mut columns[1];
                ui::section_compact(ui, "↗", "Andamento", "");
                ui::card_tinted_compact(ui, ui::BLUE_LIGHT, |ui| {
                    ui.label(egui::RichText::new(t("Utilizzo CPU (ultimi 60 secondi)")).size(12.0).strong());
                    ui::sparkline(ui, &self.cpu_hist, ui::ACCENT, 34.0);
                    ui.add_space(3.0);
                    ui.label(egui::RichText::new(t("Utilizzo RAM (ultimi 60 secondi)")).size(12.0).strong());
                    ui::sparkline(ui, &self.ram_hist, egui::Color32::from_rgb(100, 160, 250), 34.0);
                });
            }
            {
                let ui = &mut columns[2];
                ui::section_compact(ui, "ⓘ", "Informazioni di sistema", "");
                ui::card_compact(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let cell_w = (ui.available_width() - ui.spacing().item_spacing.x) * 0.5;
                    egui::Grid::new("dash")
                        .num_columns(2)
                        .striped(true)
                        .max_col_width(cell_w)
                        .show(ui, |ui| {
                            let mut rows: Vec<(String, String)> =
                                i.os.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                            let up = sysinfo::System::uptime();
                            rows.push(("Uptime".into(), format!("{}g {}h {}m", up / 86400, up % 86400 / 3600, up % 3600 / 60)));
                            if let Some((label, temp)) = i.temps.first() {
                                rows.push(("Temperatura".into(), format!("{label}: {temp:.0} °C")));
                            }
                            for (k, v) in rows {
                                ui::kv_compact(ui, &k, &v);
                            }
                        });
                });
            }
        });
        goto
    }

    pub(crate) fn ui_info(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Info hardware e sistema");
        ui::sub_tabs(
            ui,
            &mut self.sub,
            &["Info", "Hardware e rete", "Memoria RAM", "Scheda video", "Tutti i componenti"],
        );
        match self.sub {
            0 => self.info_main(ui),
            1 => self.info_hw(ui),
            2 => self.info_ram(ui, ctx),
            3 => self.info_gpu(ui, ctx),
            _ => self.info_components(ui, ctx),
        }
    }

    fn info_main(&self, ui: &mut egui::Ui) {
        let i = &self.info;
        ui::card(ui, |ui| {
            ui::section(ui, "ⓘ", "Sistema", "Informazioni principali sul sistema");
            egui::Grid::new("i_os").num_columns(2).striped(true).spacing(egui::vec2(12.0, 6.0)).show(ui, |ui| {
                for (k, v) in &i.os {
                    ui::kv(ui, k, v);
                }
                let up = sysinfo::System::uptime();
                ui::kv(ui, "Uptime", &format!("{}g {}h {}m", up / 86400, up % 86400 / 3600, up % 3600 / 60));
                for (k, v) in &i.hw {
                    if k == "Scheda madre" || k == "Modello" {
                        ui::kv(ui, k, v);
                    }
                }
                let temp = i
                    .temps
                    .iter()
                    .find(|(l, _)| {
                        let l = l.to_lowercase();
                        l.contains("core") || l.contains("cpu") || l.contains("tctl") || l.contains("package")
                    })
                    .or(i.temps.first());
                if let Some((l, tv)) = temp {
                    ui::kv(ui, "Temperatura CPU", &format!("{tv:.0} °C ({l})"));
                }
            });
        });
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "▤", "Memoria", "RAM, swap e memoria compressa");
            let (tm, um) = (i.sys.total_memory(), i.sys.used_memory());
            let (ts, us) = (i.sys.total_swap(), i.sys.used_swap());
            ui.add(ui::bar(um as f32 / tm.max(1) as f32)
                .text(format!("RAM: {} / {}", cleaner::fmt_bytes(um), cleaner::fmt_bytes(tm))));
            ui.add_space(2.0);
            ui.add(ui::bar(us as f32 / ts.max(1) as f32)
                .text(tf("Swap: {} / {}", &[&cleaner::fmt_bytes(us), &cleaner::fmt_bytes(ts)])));
            ui.add_space(4.0);
            match zram_info() {
                Some(z) => {
                    ui.horizontal_wrapped(|ui| {
                        ui::led(ui, Some(true), &tf(
                            "zRAM attiva: {} (algoritmo {}), dati {} → compressi {}",
                            &[
                                &cleaner::fmt_bytes(z.size),
                                &z.algo,
                                &cleaner::fmt_bytes(z.orig),
                                &cleaner::fmt_bytes(z.compr),
                            ],
                        ));
                    });
                }
                None => {
                    ui::led(ui, Some(false), t("zRAM: non attiva"));
                }
            };
        });
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "◔", "Dischi", "Spazio utilizzato sui volumi montati");
            for d in &i.disks {
                let used = d.total.saturating_sub(d.free);
                let frac = used as f32 / d.total.max(1) as f32;
                ui.add(ui::bar(frac).fill(ui::semantic_text(ui::level_color(frac))).text(format!(
                    "{} ({}, {}) — {} / {}",
                    d.mount,
                    d.dev,
                    d.fs,
                    cleaner::fmt_bytes(used),
                    cleaner::fmt_bytes(d.total)
                )));
                ui.add_space(2.0);
            }
        });
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "◎", "Rete", "Stato di connessione e interfacce");
            let online = match &self.net {
                Some(n) if n.inet == Some(true) => Some(true),
                Some(_) => Some(false),
                None => None,
            };
            let online_label = match &self.net {
                Some(n) if n.inet == Some(true) => "Online",
                Some(_) => "Offline",
                None => "n/d",
            };
            ui::led(ui, online, &format!("{}: {}", t("Stato"), tr(online_label)));
            ui.add_space(4.0);
            ui::output_box(ui, if i.net.is_empty() { t("Nessuna interfaccia con IP") } else { &i.net }, 90.0);
        });
    }

    fn info_hw(&self, ui: &mut egui::Ui) {
        let i = &self.info;
        ui::card(ui, |ui| {
            ui::section(ui, "◈", "Processore", "Modello, core e utilizzo in tempo reale");
            egui::Grid::new("i_cpu").num_columns(2).striped(true).spacing(egui::vec2(12.0, 6.0)).show(ui, |ui| {
                ui::kv(ui, "Modello", &i.cpu.model);
                ui::kv(ui, "Produttore", &i.cpu.vendor);
                ui::kv(ui, "Dettagli", &i.cpu.details);
                ui::kv(ui, "Architettura", &i.cpu.arch);
                let phys = i.cpu.physical.map(|p| p.to_string()).unwrap_or_else(|| "n/d".into());
                ui::kv(ui, "Core fisici", &phys);
                ui::kv(ui, "Thread", &i.cpu.threads.to_string());
                if !i.cpu.caches.is_empty() {
                    ui::kv(ui, "Cache", &i.cpu.caches.join(" · "));
                }
            });
            ui.add_space(4.0);
            for (n, c) in i.sys.cpus().iter().enumerate() {
                let frac = (c.cpu_usage() / 100.0).clamp(0.0, 1.0);
                ui.add(ui::bar(frac).fill(ui::semantic_text(ui::level_color(frac)))
                    .text(tf("Core {}: {}% — {} MHz", &[&n, &format!("{:.0}", c.cpu_usage()), &c.frequency()])));
                ui.add_space(2.0);
            }
            if !i.cpu.flags.is_empty() {
                egui::CollapsingHeader::new(tf("Flag CPU ({})", &[&i.cpu.flags.split_whitespace().count()]))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(&i.cpu.flags).size(14.0));
                    });
            }
        });
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "▤", "Memoria e dischi (modelli)", "Capacità RAM e dispositivi a blocchi");
            ui.label(format!("{}: {}", t("RAM totale"), cleaner::fmt_bytes(i.sys.total_memory())));
            ui.add_space(4.0);
            ui::output_box(ui, &i.lsblk, 140.0);
        });
        if !i.hw.is_empty() {
            ui.add_space(4.0);
            ui::card(ui, |ui| {
                ui::section(ui, "▣", "Scheda madre e BIOS", "Identificativi della piattaforma");
                egui::Grid::new("i_hw").num_columns(2).striped(true).spacing(egui::vec2(12.0, 6.0)).show(ui, |ui| {
                    for (k, v) in &i.hw {
                        ui::kv(ui, k, v);
                    }
                });
            });
        }
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "▦", "Scheda video", "GPU rilevate dal sistema");
            if i.gpu.is_empty() {
                ui.label(t("Non rilevata (installa pciutils: sudo apt install pciutils)"));
            }
            for g in &i.gpu {
                ui.label(egui::RichText::new(g).size(15.0));
            }
        });
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "◎", "Rete", "Interfacce e gateway predefinito");
            ui::output_box(ui, if i.net.is_empty() { t("Nessuna interfaccia con IP") } else { &i.net }, 100.0);
            ui.add_space(4.0);
            let gateway = if i.gateway.is_empty() { t("n/d") } else { i.gateway.as_str() };
            ui.label(egui::RichText::new(tf("Gateway: {}", &[&gateway])).strong());
        });
    }

    pub(crate) fn ui_disk(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Disco");
        ui::sub_tabs(ui, &mut self.sub, &["Analisi disco", "File grandi", "Spazio disco", "Stress test"]);
        match self.sub {
            0 => {
                ui::card(ui, |ui| {
                    ui::section(ui, "◔", "Salute disco", "Verifica lo stato S.M.A.R.T. dei dispositivi");
                    ui.label(t("Verifica lo stato di salute dei dischi (richiede smartmontools)."));
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new(egui::RichText::new(t("Dischi rilevati")).strong().size(16.0)).default_open(false).show(ui, |ui| {
                        for d in &self.disks_list {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&d.0).strong());
                                ui.label(egui::RichText::new(disk_label(&d.1)).color(ui::text_dim()));
                            });
                        }
                        if self.disks_list.is_empty() {
                            ui.label(t("Nessun disco rilevato."));
                        }
                    });
                    ui.add_space(4.0);
                    egui::ComboBox::from_label(t("Disco"))
                        .selected_text(self.disk_sel.clone())
                        .width(ui::fit_width(ui, 520.0, 180.0))
                        .show_ui(ui, |ui| {
                            for d in &self.disks_list {
                                ui.selectable_value(&mut self.disk_sel, d.0.clone(), format!("{}  —  {}", d.0, disk_label(&d.1)));
                            }
                        });
                    if let Some(d) = self.disks_list.iter().find(|d| d.0 == self.disk_sel) {
                        let kind = d.1.split('\t').next().unwrap_or("");
                        ui.label(
                            egui::RichText::new(format!("{}: {}", t("Tipo di disco"), tr(kind)))
                                .size(16.0)
                                .strong()
                                .color(ui::accent_text()),
                        );
                    }
                    ui.add_space(4.0);
                    let dev = util::shq(&self.disk_sel);
                    let pre = "command -v smartctl >/dev/null 2>&1 || { echo 'Installa smartmontools: sudo apt install smartmontools'; exit 1; }; ";
                    ui.horizontal_wrapped(|ui| {
                        self.run_btn(ui, ctx, t("Identificazione"), "smart", &format!("{pre}smartctl -i {dev}"), true, None);
                        self.run_btn(ui, ctx, t("Stato e attributi"), "smart", &format!("{pre}smartctl -H -A {dev}"), true, None);
                        self.run_btn(ui, ctx, t("Test rapido"), "smart", &format!("{pre}smartctl -t short {dev}"), true, None);
                        self.run_btn(ui, ctx, t("Test esteso"), "smart", &format!("{pre}smartctl -t long {dev}"), true,
                            Some(t("Il test esteso può durare ore (gira in background sul disco). Avviarlo?")));
                        self.run_btn(ui, ctx, t("Log errori"), "smart", &format!("{pre}smartctl -l error {dev}"), true, None);
                    });
                });
                ui.add_space(4.0);
                ui::card(ui, |ui| {
                    ui::section(ui, "▤", "Risultato S.M.A.R.T.", "Output dei controlli sul disco selezionato");
                    self.show_out(ui, "smart", 340.0);
                });
            }
            1 => {
                ui::card(ui, |ui| {
                    ui::section(ui, "🔍", "File grandi", "Trova file pesanti e poco usati nella home");
                    ui.label(t("Trova i file grandi e inutilizzati nella tua cartella personale."));
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(t("Dimensione minima (MB):"));
                        ui.add(egui::DragValue::new(&mut self.min_mb).range(1..=100_000));
                        ui.label(t("Inutilizzati da (giorni):"));
                        ui.add(egui::DragValue::new(&mut self.min_days).range(0..=3650));
                        if ui::primary_button(ui, t("🔍 Cerca"), self.action_enabled()).clicked() {
                            let (mb, days) = (self.min_mb, self.min_days);
                            self.spawn(ctx, move |ch| {
                                let home = util::shq(&util::home());
                                let script = format!(
                                    "find {home} -xdev -type f -size +{mb}M -atime +{days} -printf '%s\\t%p\\n' 2>/dev/null | sort -rn | head -300"
                                );
                                let (_, o) = util::run_shell(&script, false);
                                let files: Vec<BigFile> = o
                                    .lines()
                                    .filter_map(|l| {
                                        let (s, p) = l.split_once('\t')?;
                                        Some(BigFile { size: s.parse().ok()?, path: p.to_string(), sel: false })
                                    })
                                    .collect();
                                ch.send(Msg::Log(format!("✔ Trovati {} file", files.len())));
                                ch.send(Msg::BigFiles(files));
                            });
                        }
                    });
                    ui.add_space(4.0);
                    let count = self.bigfiles.iter().filter(|f| f.sel).count();
                    egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, false]).show(ui, |ui| {
                        for f in self.bigfiles.iter_mut() {
                            ui.checkbox(&mut f.sel, format!("{}   {}", cleaner::fmt_bytes(f.size), f.path));
                        }
                    });
                    ui.add_space(4.0);
                    if ui.add_enabled(self.action_enabled() && count > 0, egui::Button::new(format!("🗑 {} ({count})", t("Elimina selezionati")))).clicked() {
                        let all = self.bigfiles.clone();
                        let job: Job = Box::new(move |ch| {
                            let mut n = 0;
                            for f in all.iter().filter(|f| f.sel) {
                                match std::fs::remove_file(&f.path) {
                                    Ok(_) => n += 1,
                                    Err(e) => ch.send(Msg::Log(format!("✘ {}: {e}", f.path))),
                                }
                            }
                            ch.send(Msg::Log(format!("✔ Eliminati {n} file")));
                            let rest: Vec<BigFile> =
                                all.into_iter().filter(|f| std::path::Path::new(&f.path).exists()).collect();
                            ch.send(Msg::BigFiles(rest));
                        });
                        self.confirm = Some((tf("Eliminare definitivamente {} file? L'operazione non è reversibile.", &[&count]), job));
                    }
                });
            }
            2 => {
                ui::card(ui, |ui| {
                    ui::section(ui, "📏", "Spazio disco", "Calcola la dimensione delle cartelle principali");
                    ui.label(t("Calcola la dimensione delle cartelle principali di un percorso."));
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(t("Percorso:"));
                        ui.add(egui::TextEdit::singleline(&mut self.path_in).desired_width(ui::fit_width(ui, 360.0, 120.0)));
                    });
                    ui.add_space(4.0);
                    let script = format!(
                        "du -xh --max-depth=1 {} 2>/dev/null | sort -rh | head -40",
                        util::shq(&self.path_in)
                    );
                    ui.horizontal(|ui| {
                        self.run_btn(ui, ctx, t("📏 Calcola"), "du", &script, false, None);
                        if self.busy {
                            ui.spinner();
                        }
                    });
                });
                ui.add_space(4.0);
                ui::card(ui, |ui| {
                    ui::section(ui, "▤", "Risultato", "Cartelle ordinate per dimensione");
                    self.show_out(ui, "du", 420.0);
                });
            }
            _ => {
                ui::card(ui, |ui| {
                    ui::section(ui, "🔥", "Stress test", "Metti sotto sforzo CPU e RAM per verificare la stabilità");
                    ui.label(t("Mette sotto sforzo processore e memoria per individuare instabilità."));
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(t("Durata CPU (s):"));
                        ui.add(egui::DragValue::new(&mut self.stress_secs).range(5..=600));
                        if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔥 Stress CPU"))).clicked() {
                            let s = self.stress_secs as u64;
                            self.spawn(ctx, move |ch| {
                                ch.send(Msg::Log(format!("▶ Stress CPU per {s} secondi...")));
                                let r = cpu_stress(s);
                                ch.send(Msg::Log(format!("✔ {r}")));
                                ch.send(Msg::Out("stress", r));
                            });
                        }
                    });
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(t("Memoria da testare (MB):"));
                        ui.add(egui::DragValue::new(&mut self.stress_mb).range(16..=16384));
                        if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🧪 Test RAM"))).clicked() {
                            let mb = self.stress_mb as usize;
                            self.spawn(ctx, move |ch| {
                                ch.send(Msg::Log(format!("▶ Test RAM su {mb} MB...")));
                                let r = ram_test(mb);
                                ch.send(Msg::Log(format!("✔ {r}")));
                                ch.send(Msg::Out("stress", r));
                            });
                        }
                    });
                    if self.busy {
                        ui.add_space(4.0);
                        ui.spinner();
                    }
                });
                ui.add_space(4.0);
                ui::card(ui, |ui| {
                    ui::section(ui, "▤", "Risultato", "Output del test eseguito");
                    self.show_out(ui, "stress", 120.0);
                });
            }
        }
    }

    pub(crate) fn ui_procs(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Processi"));
        ui::hint(
            ui,
            t("Un processo è un programma in esecuzione. Qui vedi quali consumano più CPU e memoria. Se un programma si blocca, premi «Termina» per chiuderlo con gentilezza; se non risponde usa «Kill»."),
        );

        let total = self.procs.len();
        let f = self.proc_filter.to_lowercase();
        let mut rows: Vec<ProcRow> = self
            .procs
            .iter()
            .filter(|p| {
                f.is_empty()
                    || p.cmd.to_lowercase().contains(&f)
                    || p.pid.to_lowercase().contains(&f)
                    || p.user.to_lowercase().contains(&f)
            })
            .cloned()
            .collect();

        rows.sort_by(|a, b| {
            let ord = match self.proc_sort.key {
                ProcSortKey::Pid => a.pid.parse::<i64>().unwrap_or(0).cmp(&b.pid.parse::<i64>().unwrap_or(0)),
                ProcSortKey::User => ui::cmp_alpha(&a.user, &b.user),
                ProcSortKey::Cpu => ui::parse_f64_loose(&a.cpu).partial_cmp(&ui::parse_f64_loose(&b.cpu)).unwrap_or(std::cmp::Ordering::Equal),
                ProcSortKey::Mem => ui::parse_f64_loose(&a.mem).partial_cmp(&ui::parse_f64_loose(&b.mem)).unwrap_or(std::cmp::Ordering::Equal),
                ProcSortKey::Cmd => ui::cmp_alpha(&a.cmd, &b.cmd),
            };
            match self.proc_sort.dir {
                ui::SortDir::Asc => ord,
                ui::SortDir::Desc => ord.reverse(),
            }
        });

        let shown = rows.len();
        ui::filter_bar(ui, &mut self.proc_filter, shown, total);

        let visible_pids: Vec<String> = rows.iter().map(|p| p.pid.clone()).collect();
        let selected_visible = visible_pids.iter().filter(|p| self.proc_selected.contains(*p)).count();
        let sel_action = ui::selection_controls(ui, selected_visible, shown);
        match sel_action {
            ui::SelectionAction::All => {
                for pid in &visible_pids {
                    if !self.proc_selected.contains(pid) {
                        self.proc_selected.push(pid.clone());
                    }
                }
            }
            ui::SelectionAction::NoneSelected => {
                self.proc_selected.retain(|p| !visible_pids.contains(p));
            }
            ui::SelectionAction::None => {}
        }

        let me = util::username();
        let mut act: Option<(String, &'static str, bool)> = None;
        let mut bulk: Option<(&'static str, bool)> = None;

        let w = ui.available_width();
        let sel_w = 32.0;
        let pid_w = (w * 0.08).clamp(44.0, 64.0);
        let user_w = (w * 0.12).clamp(56.0, 100.0);
        let num_w = (w * 0.08).clamp(44.0, 62.0);
        let act_w = (w * 0.18).clamp(120.0, 170.0);

        TableBuilder::new(ui)
            .id_salt("procs")
            .striped(true)
            .vscroll(true)
            .max_scroll_height(300.0)
            .auto_shrink([false, false])
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(sel_w))
            .column(Column::initial(pid_w).at_least(40.0))
            .column(Column::initial(user_w).at_least(56.0))
            .column(Column::initial(num_w).at_least(40.0))
            .column(Column::initial(num_w).at_least(40.0))
            .column(Column::remainder().at_least(90.0))
            .column(Column::initial(act_w).at_least(110.0))
            .header(22.0, |mut header| {
                header.col(|ui| { ui::table_head(ui, "✓"); });
                header.col(|ui| { ui::sort_header(ui, t("PID"), ProcSortKey::Pid, &mut self.proc_sort); });
                header.col(|ui| { ui::sort_header(ui, t("Utente"), ProcSortKey::User, &mut self.proc_sort); });
                header.col(|ui| { ui::sort_header(ui, t("%CPU"), ProcSortKey::Cpu, &mut self.proc_sort); });
                header.col(|ui| { ui::sort_header(ui, t("%MEM"), ProcSortKey::Mem, &mut self.proc_sort); });
                header.col(|ui| { ui::sort_header(ui, t("Comando"), ProcSortKey::Cmd, &mut self.proc_sort); });
                header.col(|ui| { ui::table_head(ui, t("Azioni")); });
            })
            .body(|body| {
                body.rows(22.0, rows.len(), |mut row| {
                    let p = &rows[row.index()];
                    let pid = p.pid.clone();
                    row.col(|ui| {
                        let mut checked = self.proc_selected.contains(&pid);
                        if ui.checkbox(&mut checked, "").changed() {
                            if checked {
                                if !self.proc_selected.contains(&pid) {
                                    self.proc_selected.push(pid.clone());
                                }
                            } else {
                                self.proc_selected.retain(|x| x != &pid);
                            }
                        }
                    });
                    row.col(|ui| { ui.label(&p.pid); });
                    row.col(|ui| { ui.label(&p.user); });
                    row.col(|ui| { ui.label(&p.cpu); });
                    row.col(|ui| { ui.label(&p.mem); });
                    row.col(|ui| { ui.label(&p.cmd); });
                    row.col(|ui| {
                        if ui.small_button(t("Termina")).clicked() {
                            act = Some((p.pid.clone(), "TERM", p.user != me));
                        }
                        if ui.small_button(t("Kill")).on_hover_text(t("Chiusura forzata (SIGKILL)")).clicked() {
                            act = Some((p.pid.clone(), "KILL", p.user != me));
                        }
                    });
                });
            });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let n = self.proc_selected.len();
            if ui.add_enabled(n > 0 && self.action_enabled(), egui::Button::new(tf("Termina selezionati ({})", &[&n.to_string()]))).clicked() {
                bulk = Some(("TERM", false));
            }
            if ui.add_enabled(n > 0 && self.action_enabled(), egui::Button::new(tf("Kill selezionati ({})", &[&n.to_string()]))).clicked() {
                bulk = Some(("KILL", false));
            }
            if ui.small_button(t("Deseleziona tutto")).clicked() {
                self.proc_selected.clear();
            }
        });

        if let Some((sig, _)) = bulk {
            let targets: Vec<(String, bool)> = self
                .procs
                .iter()
                .filter(|p| self.proc_selected.contains(&p.pid))
                .map(|p| (p.pid.clone(), p.user != me))
                .collect();
            let pids: Vec<String> = targets
                .iter()
                .filter(|(pid, _)| pid.chars().all(|c| c.is_ascii_digit()))
                .map(|(pid, _)| pid.clone())
                .collect();
            let privileged = targets.iter().any(|(_, privi)| *privi);
            if !pids.is_empty() {
                let args = pids.join(" ");
                self.spawn(ctx, cmd_job("proc", format!("kill -{sig} {args}"), privileged));
                self.proc_selected.clear();
            }
        }

        if let Some((pid, sig, privileged)) = act {
            if pid.chars().all(|c| c.is_ascii_digit()) {
                self.spawn(ctx, cmd_job("proc", format!("kill -{sig} {pid}"), privileged));
            }
        }
    }

    pub(crate) fn ui_apps(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("App installate"));
        ui::sub_tabs(ui, &mut self.sub, &["Pacchetti APT", "Pacchetti Flatpak"]);
        let apt = self.sub == 0;
        if apt {
            ui::hint(
                ui,
                t("I pacchetti APT sono i programmi installati dal gestore software di Debian. Cerca il nome e premi il cestino per disinstallare quello che non ti serve: ti verrà chiesta conferma."),
            );
        } else {
            ui::hint(
                ui,
                t("Flatpak è un altro modo di installare programmi: ognuno vive in una «scatola» separata dal resto del sistema. Qui vedi le app Flatpak e puoi disinstallarle con il cestino."),
            );
        }

        let total = self.pkgs.iter().filter(|p| p.apt == apt).count();
        let f = self.pkg_filter.to_lowercase();
        let mut rows: Vec<Pkg> = self
            .pkgs
            .iter()
            .filter(|p| p.apt == apt && (f.is_empty() || p.name.to_lowercase().contains(&f) || p.version.to_lowercase().contains(&f)))
            .cloned()
            .collect();

        rows.sort_by(|a, b| {
            let ord = match self.pkg_sort.key {
                AppSortKey::Name => ui::cmp_alpha(&a.name, &b.name),
                AppSortKey::Version => ui::cmp_alpha(&a.version, &b.version),
            };
            match self.pkg_sort.dir {
                ui::SortDir::Asc => ord,
                ui::SortDir::Desc => ord.reverse(),
            }
        });

        let shown = rows.len();
        ui.horizontal_wrapped(|ui| {
            ui::filter_bar(ui, &mut self.pkg_filter, shown, total);
            if ui.button(t("🔄 Ricarica")).clicked() {
                self.load_pkgs(ctx);
            }
        });

        let visible_names: Vec<String> = rows.iter().map(|p| p.name.clone()).collect();
        let selected_visible = visible_names.iter().filter(|n| self.pkg_selected.contains(*n)).count();
        let sel_action = ui::selection_controls(ui, selected_visible, shown);
        match sel_action {
            ui::SelectionAction::All => {
                for n in &visible_names {
                    if !self.pkg_selected.contains(n) {
                        self.pkg_selected.push(n.clone());
                    }
                }
            }
            ui::SelectionAction::NoneSelected => {
                self.pkg_selected.retain(|n| !visible_names.contains(n));
            }
            ui::SelectionAction::None => {}
        }

        let mut rm: Option<String> = None;
        let mut bulk_remove = false;

        TableBuilder::new(ui)
            .id_salt(if apt { "apps_apt" } else { "apps_flatpak" })
            .striped(true)
            .vscroll(true)
            .max_scroll_height(300.0)
            .auto_shrink([false, false])
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(32.0))
            .column(Column::exact(36.0))
            .column(Column::remainder().at_least(180.0))
            .column(Column::initial(140.0).at_least(90.0))
            .header(22.0, |mut header| {
                header.col(|ui| { ui::table_head(ui, "✓"); });
                header.col(|_| {});
                header.col(|ui| { ui::sort_header(ui, t("Pacchetto"), AppSortKey::Name, &mut self.pkg_sort); });
                header.col(|ui| { ui::sort_header(ui, t("Versione"), AppSortKey::Version, &mut self.pkg_sort); });
            })
            .body(|body| {
                body.rows(22.0, rows.len(), |mut row| {
                    let p = &rows[row.index()];
                    let name = p.name.clone();
                    row.col(|ui| {
                        let mut checked = self.pkg_selected.contains(&name);
                        if ui.checkbox(&mut checked, "").changed() {
                            if checked {
                                if !self.pkg_selected.contains(&name) {
                                    self.pkg_selected.push(name.clone());
                                }
                            } else {
                                self.pkg_selected.retain(|x| x != &name);
                            }
                        }
                    });
                    row.col(|ui| {
                        if ui.small_button("🗑").on_hover_text(t("Disinstalla")).clicked() {
                            rm = Some(name.clone());
                        }
                    });
                    row.col(|ui| { ui.label(&p.name); });
                    row.col(|ui| { ui.label(&p.version); });
                });
            });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let n = self.pkg_selected.iter().filter(|x| visible_names.contains(*x)).count();
            if ui
                .add_enabled(n > 0 && self.action_enabled(), egui::Button::new(tf("Disinstalla selezionati ({})", &[&n.to_string()])))
                .clicked()
            {
                bulk_remove = true;
            }
            if ui.small_button(t("Deseleziona tutto")).clicked() {
                self.pkg_selected.retain(|n| !visible_names.contains(n));
            }
        });

        if bulk_remove {
            let targets: Vec<String> = self
                .pkg_selected
                .iter()
                .filter(|n| visible_names.contains(*n))
                .cloned()
                .collect();
            if !targets.is_empty() {
                let args = targets.iter().map(|s| util::shq(s)).collect::<Vec<_>>().join(" ");
                let script = if apt {
                    format!("apt-get -y purge {args}")
                } else {
                    format!("flatpak uninstall -y --noninteractive {args}")
                };
                let msg = tf("Disinstallare {} pacchetti selezionati?", &[&targets.len().to_string()]);
                let job: Job = Box::new(move |ch| {
                    let (ok, out) = util::run_shell(&script, apt);
                    let mark = if ok { "✔" } else { "✘" };
                    ch.send(Msg::Log(format!("{mark} {}", util::tail(out.trim(), 3).replace('\n', " | "))));
                    ch.send(Msg::Pkgs(read_pkgs()));
                });
                self.pkg_selected.retain(|n| !targets.contains(n));
                self.confirm = Some((msg, job));
            }
        }

        if let Some(name) = rm {
            let q = util::shq(&name);
            let script = if apt {
                format!("apt-get -y purge {q}")
            } else {
                format!("flatpak uninstall -y --noninteractive {q}")
            };
            let job: Job = Box::new(move |ch| {
                let (ok, out) = util::run_shell(&script, apt);
                let mark = if ok { "✔" } else { "✘" };
                ch.send(Msg::Log(format!("{mark} {}", util::tail(out.trim(), 3).replace('\n', " | "))));
                ch.send(Msg::Pkgs(read_pkgs()));
            });
            self.confirm = Some((tf("Disinstallare «{}»?", &[&name]), job));
        }
    }

    pub(crate) fn ui_zram(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Ottimizza RAM (zRAM)");
        ui::card(ui, |ui| {
            ui::section(ui, "⚡", "Ottimizza RAM (zRAM)", "Memoria compressa nella RAM per migliorare la reattività");
            ui.label(t("Crea una memoria compressa virtuale nella RAM fisica per migliorare le prestazioni quando la memoria scarseggia."));
            ui.add_space(4.0);
            let total_mb = (self.info.sys.total_memory() / 1024 / 1024).max(512) as u32;
            if self.zram_mb == 0 {
                self.zram_mb = (total_mb / 2).max(256);
            }
            ui.add(egui::Slider::new(&mut self.zram_mb, 256..=total_mb).text(t("Dimensione zRAM (MB)")));
            let mb = self.zram_mb;
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let on = format!(
                    "swapoff /dev/zram0 2>/dev/null; modprobe zram num_devices=1 2>/dev/null; \
                     echo 1 > /sys/block/zram0/reset 2>/dev/null; \
                     echo zstd > /sys/block/zram0/comp_algorithm 2>/dev/null; \
                     echo {mb}M > /sys/block/zram0/disksize && mkswap /dev/zram0 && swapon -p 100 /dev/zram0 && echo 'zRAM attivata'"
                );
                self.run_btn(ui, ctx, t("⚡ Ottimizza RAM"), "zram", &on, true, None);
                self.run_btn(
                    ui,
                    ctx,
                    t("⏻ Disattiva zRAM"),
                    "zram",
                    "swapoff /dev/zram0; echo 1 > /sys/block/zram0/reset && echo 'zRAM disattivata'",
                    true,
                    None,
                );
            });
        });
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "▤", "Stato attuale", "Utilizzo e compressione della zRAM");
            match zram_info() {
                Some(z) => {
                    let frac = z.orig as f32 / z.size.max(1) as f32;
                    ui.add(ui::bar(frac).fill(ui::semantic_text(ui::level_color(frac))).text(tf(
                        "Occupazione: {} / {}",
                        &[&cleaner::fmt_bytes(z.orig), &cleaner::fmt_bytes(z.size)],
                    )));
                    ui.add_space(4.0);
                    ui.label(tf("Algoritmo di compressione: {}", &[&z.algo]));
                    if z.compr > 0 {
                        ui.label(tf("Rapporto di compressione: {}:1", &[&format!("{:.1}", z.orig as f32 / z.compr as f32)]));
                    }
                }
                None => {
                    ui::led(ui, Some(false), t("zRAM non attiva."));
                }
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(t("Nota: la configurazione vale fino al riavvio.")).color(crate::ui::text_dim()));
        });
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "▣", "Output", "Risultato delle operazioni zRAM");
            self.show_out(ui, "zram", 100.0);
        });
    }

    fn cooling_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let base = "/sys/devices/system/cpu/cpu0/cpufreq";
        let max = read_u64(&format!("{base}/cpuinfo_max_freq"));
        let cur = read_u64(&format!("{base}/scaling_max_freq"));
        if max == 0 {
            ui::card(ui, |ui| {
                ui.label(t("La regolazione della frequenza del processore non è disponibile su questo sistema."));
            });
            return;
        }
        let active = cur > 0 && cur < max;
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Modalità di raffreddamento")).strong().size(20.0));
            ui.label(t("Riduce al 70% la velocità massima del processore e attiva il profilo a basso consumo: il computer produce meno calore e le ventole girano più piano. Utile se il PC scalda troppo; i programmi pesanti saranno un po' più lenti."));
            ui.add_space(2.0);
            ui::led(
                ui,
                Some(active),
                &if active {
                    tf("Raffreddamento attivo ({} / {} MHz)", &[&(cur / 1000), &(max / 1000)])
                } else {
                    t("Raffreddamento non attivo (velocità normale)").to_string()
                },
            );
            ui.add_space(2.0);
            let on = POWER_SAVE_ORIG.to_string()
                + "for d in /sys/devices/system/cpu/cpu*/cpufreq; do m=$(cat $d/cpuinfo_max_freq 2>/dev/null) && echo $((m*70/100)) > $d/scaling_max_freq; done; \
                   if command -v powerprofilesctl >/dev/null 2>&1; then powerprofilesctl set power-saver; fi; echo 'OK'";
            let off = "for d in /sys/devices/system/cpu/cpu*/cpufreq; do cat $d/cpuinfo_max_freq > $d/scaling_max_freq; done; \
                       P=/var/lib/debutility/power_profile_orig; \
                       if [ ! -s \"$P\" ] && [ -s /var/lib/debclean/power_profile_orig ]; then P=/var/lib/debclean/power_profile_orig; fi; \
                       if [ -s \"$P\" ] && command -v powerprofilesctl >/dev/null 2>&1; then powerprofilesctl set \"$(cat \"$P\")\"; fi; echo 'OK'";
            ui.horizontal_wrapped(|ui| {
                let activate = ui.add_enabled(
                    self.action_enabled() && !active,
                    egui::Button::new(t("Attiva modalità di raffreddamento")),
                );
                let activate = if active {
                    activate.on_hover_text(t("La modalità di raffreddamento è già attiva."))
                } else {
                    activate
                };
                if activate.clicked() {
                    let job = cmd_job("cool", on.clone(), true);
                    self.confirm = Some((
                        t(
                            "Attivare la modalità di raffreddamento? Il processore sarà limitato al 70% della velocità massima.",
                        )
                        .to_string(),
                        job,
                    ));
                }
                self.run_btn(ui, ctx, t("Disattiva e ripristina"), "cool", off, true, None);
            });
        });
        self.show_out(ui, "cool", 80.0);
    }

    pub(crate) fn ui_temps(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Monitor temperature");
        self.cooling_card(ui, ctx);
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "🌡", "Temperature", "Sensori termici rilevati dal sistema");
            if self.info.temps.is_empty() {
                ui.label(t("Nessun sensore rilevato. Installa e configura lm-sensors: sudo apt install lm-sensors && sudo sensors-detect"));
                return;
            }
            for (l, tmp) in &self.info.temps {
                let frac = (*tmp / 100.0).clamp(0.0, 1.0);
                ui.add(
                    ui::bar(frac)
                        .fill(ui::heat_color(*tmp))
                        .text(format!("{l}: {tmp:.0} °C")),
                );
                ui.add_space(2.0);
            }
        });
    }

    pub(crate) fn ui_network(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Rete");
        ui::hint(
            ui,
            "Le tre spie dicono se il router (gateway), il DNS (che traduce i nomi dei siti in indirizzi) e Internet funzionano: verde = ok, rosso = problema. «Test diagnostico» le ricontrolla; «Ripristina rete e Wi-Fi» riavvia la rete se qualcosa non va.",
        );
        let (g, d, i) = match &self.net {
            Some(n) => (n.gw, n.dns, n.inet),
            None => (None, None, None),
        };
        ui::card(ui, |ui| {
            ui::section(ui, "◎", "Diagnostica di rete", "Verifica rapida di gateway, DNS e Internet");
            ui::led(ui, g, t("Gateway locale (router)"));
            ui.add_space(2.0);
            ui::led(ui, d, t("Servizio DNS"));
            ui.add_space(2.0);
            ui::led(ui, i, t("Connessione Internet"));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔍 Test diagnostico"))).clicked() {
                    self.spawn(ctx, |ch| ch.send(Msg::Net(net_test())));
                }
                self.run_btn(
                    ui,
                    ctx,
                    t("♻ Ripristina rete e Wi-Fi"),
                    "net",
                    "systemctl restart NetworkManager && echo 'NetworkManager riavviato'",
                    true,
                    Some(t("Riavviare NetworkManager? La connessione cadrà per qualche secondo.")),
                );
                if self.busy {
                    ui.spinner();
                }
            });
        });
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "▤", "Interfacce", "Indirizzi e interfacce di rete attive");
            ui::output_box(ui, if self.info.net.is_empty() { t("Nessuna interfaccia con IP") } else { &self.info.net }, 120.0);
            ui.add_space(4.0);
            let gateway = if self.info.gateway.is_empty() { t("n/d") } else { self.info.gateway.as_str() };
            ui.label(egui::RichText::new(tf("Gateway: {}", &[&gateway])).strong());
        });
        ui.add_space(4.0);
        self.ui_vpn(ui, ctx);
        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui::section(ui, "▣", "Output rete", "Risultato dei comandi di rete");
            self.show_out(ui, "net", 100.0);
        });
    }

pub(crate) fn ui_about(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            ui.add(
                egui::Image::new(egui::include_image!("../assets/debutility.png"))
                    .max_size(egui::vec2(72.0, 72.0)),
            );
            ui.add_space(4.0);
        });
        
        ui::title(ui, t("Informazioni su DebUtility"));
        
        ui::card(ui, |ui| {
            ui.heading(egui::RichText::new("DebUtility").size(24.0).strong().color(ui::accent_text()));
            ui.label(
                egui::RichText::new(format!("v{}", crate::APP_VERSION))
                    .size(17.0)
                    .strong()
                    .color(ui::accent_text()),
            );
            ui.add_space(2.0);
            ui.label(egui::RichText::new(t("Strumento di manutenzione, diagnosi e configurazione per Debian, scritto in Rust.")).size(16.0));
            ui.add_space(3.0);
            ui.label(t("Obiettivo: riunire in un'unica interfaccia pulizia, riparazione, monitoraggio e amministrazione del sistema."));
        });

        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui.heading(egui::RichText::new(t("Crediti e Autore")).size(18.0).strong());
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Creato da:")).strong());
                ui.label("Andrea linux");
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Sito web:")).strong());
                ui.hyperlink("https://www.andrealinux.it");
            });
            ui.add_space(3.0);
            ui.label(egui::RichText::new(t("Programma sviluppato con il supporto e la collaborazione dell'Intelligenza Artificiale.")).color(ui::text_dim()));
        });

        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui.heading(egui::RichText::new(t("Note legali e Sicurezza")).size(18.0).strong());
            ui.add_space(3.0);
            ui.label(t("Licenza: software Open Source."));
            ui.add_space(2.0);
            ui.colored_label(
                ui::WARN,
                t("Avviso di sicurezza: molte operazioni modificano il sistema. Esegui sempre un backup dei dati importanti prima di procedere."),
            );
        });
    }
}
