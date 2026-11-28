use crate::i18n::t;
use crate::{cleaner, ui, util, App, Chan, Job, Msg};
use eframe::egui;
use std::fs;
use std::path::Path;

/// Un gruppo di informazioni (titolo + righe etichetta/valore), già tradotto.
#[derive(Clone)]
pub struct Section {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

#[derive(Clone)]
pub struct RamModule {
    pub slot: String,
    pub size: String,
    pub kind: String,
    pub speed: String,
    pub maker: String,
    pub part: String,
    pub form: String,
    pub installed: bool,
}

struct PciDev {
    slot: String,
    class: String,
    name: String,
    ids: String,
    vendor: String,
    driver: String,
    modules: String,
}

fn sh(cmd: &str) -> String {
    util::run_shell(cmd, false).1
}

fn rd(p: &str) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn has(cmd: &str) -> bool {
    util::run_shell(&format!("command -v {cmd} >/dev/null 2>&1"), false).0
}

fn vendor_name(id: &str) -> &'static str {
    match id.to_lowercase().as_str() {
        "10de" => "NVIDIA",
        "1002" | "1022" => "AMD",
        "8086" | "8087" => "Intel",
        "1af4" => "Red Hat (VirtIO)",
        "15ad" => "VMware",
        "80ee" => "Oracle VirtualBox",
        "1b36" => "QEMU",
        "10ec" => "Realtek",
        "14e4" => "Broadcom",
        "168c" | "1969" => "Qualcomm Atheros",
        "14c3" => "MediaTek",
        "1814" => "Ralink",
        "1106" => "VIA",
        "1039" => "SiS",
        _ => "",
    }
}

/// Separa il nome dalla sigla `[xxxx:yyyy]` che lspci mette in fondo.
fn split_ids(s: &str) -> (String, String) {
    if let Some(i) = s.rfind('[') {
        if let Some(j) = s[i..].find(']') {
            let inner = &s[i + 1..i + j];
            let ok = inner.len() == 9
                && inner.as_bytes()[4] == b':'
                && inner.chars().all(|c| c.is_ascii_hexdigit() || c == ':');
            if ok {
                let name = format!("{}{}", &s[..i], &s[i + j + 1..]).trim().to_string();
                return (name, inner.to_string());
            }
        }
    }
    (s.trim().to_string(), String::new())
}

fn read_pci() -> Vec<PciDev> {
    let out = sh("lspci -nnk 2>/dev/null");
    let mut devs: Vec<PciDev> = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            if let Some(d) = devs.last_mut() {
                let l = line.trim();
                if let Some(v) = l.strip_prefix("Kernel driver in use:") {
                    d.driver = v.trim().to_string();
                } else if let Some(v) = l.strip_prefix("Kernel modules:") {
                    d.modules = v.trim().to_string();
                }
            }
            continue;
        }
        // "00:02.0 VGA compatible controller [0300]: Intel Corporation HD Graphics [8086:0046] (rev 02)"
        let Some((slot, rest)) = line.split_once(' ') else { continue };
        let Some((class_part, name_part)) = rest.split_once("]: ") else { continue };
        let class = class_part.rsplit_once(" [").map(|x| x.0).unwrap_or(class_part).trim().to_string();
        let (name, ids) = split_ids(name_part);
        let vendor = ids.split(':').next().map(|v| vendor_name(v).to_string()).unwrap_or_default();
        devs.push(PciDev {
            slot: slot.to_string(),
            class,
            name,
            ids,
            vendor,
            driver: String::new(),
            modules: String::new(),
        });
    }
    devs
}

fn is_gpu(d: &PciDev) -> bool {
    let c = d.class.to_lowercase();
    c.contains("vga") || c.contains("3d controller") || c.contains("display controller")
}

fn is_audio(d: &PciDev) -> bool {
    let c = d.class.to_lowercase();
    c.contains("audio") || c.contains("multimedia")
}

fn is_net(d: &PciDev) -> bool {
    let c = d.class.to_lowercase();
    c.contains("ethernet") || c.contains("network")
}

fn read_u64(p: &str) -> Option<u64> {
    rd(p).and_then(|s| s.parse().ok())
}

fn collect_monitors() -> Option<Section> {
    if !has("xrandr") {
        return None;
    }
    let o = sh("xrandr --query 2>/dev/null");
    let mut rows = Vec::new();
    for l in o.lines() {
        if l.contains(" connected") {
            let name = l.split_whitespace().next().unwrap_or("").to_string();
            let res = l
                .split_whitespace()
                .find(|tok| {
                    tok.contains('x') && tok.contains('+') && tok.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                })
                .map(|tok| tok.split('+').next().unwrap_or("").to_string())
                .unwrap_or_else(|| t("(spento o non attivo)").to_string());
            rows.push((name, res));
        }
    }
    if rows.is_empty() {
        None
    } else {
        Some(Section { title: t("Monitor").to_string(), rows })
    }
}

fn collect_gpus(devs: &[PciDev]) -> Vec<Section> {
    let mut out = Vec::new();
    let nvsmi = if has("nvidia-smi") {
        sh("nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader 2>/dev/null")
    } else {
        String::new()
    };
    let have_gl = has("glxinfo");
    let gl = if have_gl { sh("glxinfo -B 2>/dev/null") } else { String::new() };
    let gl_val = |key: &str| gl.lines().find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().to_string()));

    for (i, d) in devs.iter().filter(|d| is_gpu(d)).enumerate() {
        let mut rows: Vec<(String, String)> = Vec::new();
        rows.push((t("Modello").to_string(), d.name.clone()));
        let vend = if d.vendor.is_empty() {
            d.name.split_whitespace().next().unwrap_or("").to_string()
        } else {
            d.vendor.clone()
        };
        rows.push((t("Produttore").to_string(), vend));
        if !d.ids.is_empty() {
            rows.push((t("ID PCI").to_string(), d.ids.clone()));
        }
        rows.push((t("Posizione PCI").to_string(), d.slot.clone()));
        rows.push((
            t("Driver in uso").to_string(),
            if d.driver.is_empty() { t("nessuno (driver non caricato)").to_string() } else { d.driver.clone() },
        ));
        if !d.modules.is_empty() {
            rows.push((t("Moduli del kernel").to_string(), d.modules.clone()));
        }

        // memoria video
        if d.vendor == "NVIDIA" && !nvsmi.is_empty() {
            let parts: Vec<&str> = nvsmi.lines().next().unwrap_or("").split(',').map(|x| x.trim()).collect();
            if parts.len() >= 3 {
                rows.push((t("Memoria video").to_string(), parts[1].to_string()));
                rows.push((t("Versione driver NVIDIA").to_string(), parts[2].to_string()));
            }
        } else if let Some(b) = read_u64(&format!("/sys/bus/pci/devices/0000:{}/mem_info_vram_total", d.slot)) {
            if b > 0 {
                rows.push((t("Memoria video").to_string(), cleaner::fmt_bytes(b)));
            }
        } else if d.vendor == "Intel" {
            rows.push((t("Memoria video").to_string(), t("Condivisa con la RAM (scheda integrata)").to_string()));
        }

        if i == 0 {
            if have_gl {
                if let Some(v) = gl_val("OpenGL renderer string:") {
                    rows.push((t("Rendering OpenGL").to_string(), v));
                }
                if let Some(v) = gl_val("OpenGL version string:") {
                    rows.push((t("Versione OpenGL").to_string(), v));
                }
            } else {
                rows.push((t("OpenGL").to_string(), t("non disponibile (installa mesa-utils)").to_string()));
            }
        }
        out.push(Section { title: d.name.clone(), rows });
    }
    if !out.is_empty() {
        if let Some(m) = collect_monitors() {
            out.push(m);
        }
    }
    out
}

fn chassis(code: &str) -> Option<String> {
    let s = match code.trim().parse::<u32>().ok()? {
        3..=7 => t("Computer fisso (desktop/torre)"),
        8 | 9 | 10 | 14 => t("Portatile (notebook)"),
        13 => t("Tutto in uno"),
        30..=32 => t("Tablet / convertibile"),
        35 | 36 => t("Mini PC"),
        1 | 2 => t("Altro"),
        _ => return None,
    };
    Some(s.to_string())
}

fn join_opt(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        _ => None,
    }
}

fn collect_components(devs: &[PciDev], gpus: &[Section]) -> Vec<Section> {
    let mut v: Vec<Section> = Vec::new();
    let dmi = |n: &str| rd(&format!("/sys/class/dmi/id/{n}"));

    // --- sistema e scheda madre
    let mut rows: Vec<(String, String)> = Vec::new();
    {
        let mut push = |label: &'static str, val: Option<String>| {
            if let Some(x) = val {
                rows.push((t(label).to_string(), x));
            }
        };
        push("Produttore", dmi("sys_vendor"));
        push("Modello", dmi("product_name"));
        push("Versione", dmi("product_version"));
        push("Scheda madre", join_opt(dmi("board_vendor"), dmi("board_name")));
        push("BIOS/UEFI", join_opt(dmi("bios_vendor"), dmi("bios_version")));
        push("Data BIOS", dmi("bios_date"));
        push(t("Tipo di computer"), dmi("chassis_type").and_then(|c| chassis(&c)));
        let boot = if Path::new("/sys/firmware/efi").exists() { t("UEFI") } else { t("BIOS (legacy)") };
        push(t("Firmware di avvio"), Some(boot.to_string()));
    }
    v.push(Section { title: t("Sistema e scheda madre").to_string(), rows });

    // --- memoria
    let mem = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let total = mem
        .lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|x| x.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024;
    v.push(Section {
        title: t("Memoria RAM").to_string(),
        rows: vec![
            (t("RAM totale").to_string(), cleaner::fmt_bytes(total)),
            (t("Dettagli dei moduli").to_string(), t("vedi la scheda «Memoria RAM»").to_string()),
        ],
    });

    // --- schede video
    let vrows: Vec<(String, String)> = gpus
        .iter()
        .filter(|s| s.title != t("Monitor"))
        .map(|s| (t("Scheda video").to_string(), s.title.clone()))
        .collect();
    if !vrows.is_empty() {
        v.push(Section { title: t("Scheda video").to_string(), rows: vrows });
    }

    // --- audio
    let mut arows: Vec<(String, String)> = devs
        .iter()
        .filter(|d| is_audio(d))
        .map(|d| (t("Scheda audio").to_string(), d.name.clone()))
        .collect();
    if let Ok(cards) = fs::read_to_string("/proc/asound/cards") {
        for l in cards.lines() {
            if let Some((_, rest)) = l.split_once("]: ") {
                arows.push((t("Dispositivo audio").to_string(), rest.trim().to_string()));
            }
        }
    }
    if !arows.is_empty() {
        v.push(Section { title: t("Audio").to_string(), rows: arows });
    }

    // --- rete
    let mut nrows: Vec<(String, String)> = devs
        .iter()
        .filter(|d| is_net(d))
        .map(|d| (t("Scheda di rete").to_string(), d.name.clone()))
        .collect();
    for l in sh("ip -br link 2>/dev/null").lines() {
        let mut it = l.split_whitespace();
        if let (Some(name), Some(state)) = (it.next(), it.next()) {
            if name != "lo" {
                nrows.push((name.to_string(), state.to_string()));
            }
        }
    }
    if !nrows.is_empty() {
        v.push(Section { title: t("Rete").to_string(), rows: nrows });
    }

    // --- dischi
    let mut drows: Vec<(String, String)> = Vec::new();
    for dev in sh("lsblk -dnpo NAME -e 7,11 2>/dev/null").lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        let desc = crate::pages_sys::describe_disk(dev);
        if desc.split('\t').nth(1) == Some("0B") {
            continue;
        }
        drows.push((dev.to_string(), crate::pages_sys::disk_label(&desc)));
    }
    if !drows.is_empty() {
        v.push(Section { title: t("Dischi e memorie").to_string(), rows: drows });
    }

    // --- altri dispositivi PCI
    let prows: Vec<(String, String)> = devs
        .iter()
        .filter(|d| !is_gpu(d) && !is_audio(d) && !is_net(d))
        .filter(|d| {
            let c = d.class.to_lowercase();
            !c.contains("bridge") && !c.contains("smbus") && !c.contains("signal processing")
        })
        .map(|d| (d.class.clone(), d.name.clone()))
        .collect();
    if !prows.is_empty() {
        v.push(Section { title: t("Altri dispositivi PCI").to_string(), rows: prows });
    }

    // --- USB e Bluetooth
    let lsusb = sh("lsusb 2>/dev/null");
    let mut urows: Vec<(String, String)> = Vec::new();
    let mut bt: Option<String> = None;
    for l in lsusb.lines() {
        if let Some((_, after)) = l.split_once("ID ") {
            let desc = after.split_once(' ').map(|x| x.1).unwrap_or(after).trim().to_string();
            let low = desc.to_lowercase();
            if low.contains("root hub") {
                continue;
            }
            if low.contains("bluetooth") {
                bt = Some(desc.clone());
            }
            urows.push((t("Dispositivo USB").to_string(), desc));
        }
    }
    if !urows.is_empty() {
        v.push(Section { title: t("Dispositivi USB").to_string(), rows: urows });
    }
    if bt.is_some() || Path::new("/sys/class/bluetooth/hci0").exists() {
        v.push(Section {
            title: t("Bluetooth").to_string(),
            rows: vec![(t("Bluetooth").to_string(), bt.unwrap_or_else(|| t("Presente").to_string()))],
        });
    }

    // --- tastiera, mouse e altri input
    let mut irows: Vec<(String, String)> = Vec::new();
    if let Ok(inp) = fs::read_to_string("/proc/bus/input/devices") {
        for l in inp.lines() {
            if let Some(n) = l.strip_prefix("N: Name=\"") {
                let name = n.trim_end_matches('"').to_string();
                let low = name.to_lowercase();
                let skip = ["button", "video bus", "speaker", "lid switch", "hda", "jack", "sleep", "power", "hdmi", "pc speaker"];
                if skip.iter().any(|s| low.contains(s)) || irows.iter().any(|(_, x)| *x == name) {
                    continue;
                }
                irows.push((t("Dispositivo di input").to_string(), name));
            }
        }
    }
    if !irows.is_empty() {
        v.push(Section { title: t("Tastiera, mouse e altri input").to_string(), rows: irows });
    }

    // --- webcam
    let mut wrows: Vec<(String, String)> = Vec::new();
    if let Ok(rdir) = fs::read_dir("/sys/class/video4linux") {
        for e in rdir.flatten() {
            if let Some(n) = rd(&format!("{}/name", e.path().display())) {
                if !wrows.iter().any(|(_, x)| *x == n) && !n.to_lowercase().contains("metadata") {
                    wrows.push((t("Webcam").to_string(), n));
                }
            }
        }
    }
    if !wrows.is_empty() {
        v.push(Section { title: t("Webcam").to_string(), rows: wrows });
    }

    // --- monitor
    if let Some(m) = collect_monitors() {
        v.push(m);
    }

    // --- batteria
    if let Ok(rdir) = fs::read_dir("/sys/class/power_supply") {
        for e in rdir.flatten() {
            let p = e.path().display().to_string();
            if rd(&format!("{p}/type")).as_deref() != Some("Battery") || rd(&format!("{p}/scope")).as_deref() == Some("Device") {
                continue;
            }
            let mut brow: Vec<(String, String)> = Vec::new();
            if let Some(x) = rd(&format!("{p}/manufacturer")) {
                brow.push((t("Produttore").to_string(), x));
            }
            if let Some(x) = rd(&format!("{p}/model_name")) {
                brow.push((t("Modello").to_string(), x));
            }
            if let Some(x) = rd(&format!("{p}/technology")) {
                brow.push((t("Tecnologia").to_string(), x));
            }
            if let Some(x) = rd(&format!("{p}/cycle_count")) {
                brow.push((t("Cicli di carica").to_string(), x));
            }
            if let Some(x) = read_u64(&format!("{p}/energy_full_design")) {
                brow.push((t("Capacità di progetto").to_string(), format!("{:.1} Wh", x as f64 / 1_000_000.0)));
            }
            if !brow.is_empty() {
                v.push(Section { title: t("Batteria").to_string(), rows: brow });
            }
            break;
        }
    }
    v
}

// ---------------------------------------------------------------- memoria RAM (dmidecode)

fn field(block: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    block
        .lines()
        .find_map(|l| l.trim().strip_prefix(prefix.as_str()).map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty() && v != "Not Specified" && v != "Unknown" && v != "None" && v != "[Empty]")
}

pub fn parse_dmidecode(text: &str) -> (Vec<(String, String)>, Vec<RamModule>) {
    let mut arr = Vec::new();
    let mut mods = Vec::new();
    for block in text.split("\n\n") {
        let is_dev = block.lines().any(|l| l.trim() == "Memory Device");
        let is_arr = block.lines().any(|l| l.trim() == "Physical Memory Array");
        if is_arr {
            if let Some(v) = field(block, "Maximum Capacity") {
                arr.push((t("Capacità massima").to_string(), v));
            }
            if let Some(v) = field(block, "Number Of Devices") {
                arr.push((t("Numero di slot").to_string(), v));
            }
            if let Some(v) = field(block, "Error Correction Type") {
                arr.push((t("Correzione errori (ECC)").to_string(), v));
            }
        }
        if is_dev {
            let size = field(block, "Size").unwrap_or_default();
            let installed = !size.is_empty() && !size.to_lowercase().contains("no module");
            let slot = field(block, "Locator").unwrap_or_default();
            let speed = field(block, "Configured Memory Speed")
                .or_else(|| field(block, "Configured Clock Speed"))
                .or_else(|| field(block, "Speed"))
                .unwrap_or_default();
            mods.push(RamModule {
                slot,
                size,
                kind: field(block, "Type").unwrap_or_default(),
                speed,
                maker: field(block, "Manufacturer").unwrap_or_default(),
                part: field(block, "Part Number").unwrap_or_default().trim().to_string(),
                form: field(block, "Form Factor").unwrap_or_default(),
                installed,
            });
        }
    }
    (arr, mods)
}

fn ram_job() -> Job {
    Box::new(move |ch: &Chan| {
        ch.send(Msg::Log(t("▶ Leggo i moduli di memoria...").to_string()));
        let (_, out) = util::run_shell(
            "command -v dmidecode >/dev/null 2>&1 || { echo '::NODMIDECODE::'; exit 0; }; dmidecode -t memory",
            true,
        );
        if out.contains("::NODMIDECODE::") {
            ch.send(Msg::Ram(
                vec![],
                vec![],
                t("dmidecode non è installato: installalo con sudo apt install dmidecode").to_string(),
            ));
            return;
        }
        let (arr, mods) = parse_dmidecode(&out);
        let msg = if mods.is_empty() {
            t("Impossibile leggere i moduli di memoria (password annullata o dati non disponibili, per esempio in una macchina virtuale).").to_string()
        } else {
            String::new()
        };
        ch.send(Msg::Ram(arr, mods, msg));
    })
}

// ---------------------------------------------------------------- interfaccia

fn section_card(ui: &mut egui::Ui, id: usize, title: &str, rows: &[(String, String)]) {
    ui::card(ui, |ui| {
        ui::section_text(ui, "▣", title, "");
        egui::Grid::new(("hwsec", id)).num_columns(2).striped(true).spacing(egui::vec2(12.0, 6.0)).show(ui, |ui| {
            for (k, v) in rows {
                ui::kv(ui, k, v);
            }
        });
    });
    ui.add_space(4.0);
}

impl App {
    /// Rilegge dalla macchina schede video e componenti (senza privilegi).
    pub(crate) fn load_hw(&self, ctx: &egui::Context) {
        self.bg(ctx, |ch| {
            let devs = read_pci();
            let gpus = collect_gpus(&devs);
            let comps = collect_components(&devs, &gpus);
            ch.send(Msg::Gpus(gpus));
            ch.send(Msg::Comps(comps));
        });
    }

    pub(crate) fn info_ram(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::hint(
            ui,
            "Qui vedi quanta memoria RAM ha il computer e, premendo il pulsante, anche la marca, il tipo (DDR3, DDR4...) e la velocità di ogni modulo. Serve la password di amministratore perché questi dati si leggono dalla scheda madre.",
        );
        let (tm, um) = (self.info.sys.total_memory(), self.info.sys.used_memory());
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("RAM totale")).strong().size(19.0));
            ui.add(ui::bar(um as f32 / tm.max(1) as f32).text(format!(
                "{} / {}",
                cleaner::fmt_bytes(um),
                cleaner::fmt_bytes(tm)
            )));
        });
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui::primary_button(ui, t("Rileva marca e tipo dei moduli (richiede password)"), self.action_enabled()).clicked() {
                self.spawn(ctx, ram_job());
            }
            if self.busy {
                ui.spinner();
            }
        });
        if !self.ram_msg.is_empty() {
            ui.add_space(3.0);
            ui.colored_label(ui::semantic_text(ui::WARN), self.ram_msg.as_str());
        }
        ui.add_space(3.0);
        if !self.ram_arr.is_empty() {
            section_card(ui, 100, t("Slot e capacità"), &self.ram_arr);
        }
        for (i, m) in self.ram_modules.iter().enumerate() {
            ui::card(ui, |ui| {
                if !m.installed {
                    ui.label(
                        egui::RichText::new(format!("{} — {}", m.slot, t("Slot libero")))
                            .size(17.0)
                            .color(ui::text_dim()),
                    );
                    return;
                }
                ui.label(egui::RichText::new(format!("{}  ·  {} {}", m.slot, m.size, m.kind)).strong().size(19.0));
                ui.add_space(2.0);
                egui::Grid::new(("ram", i)).num_columns(2).striped(true).show(ui, |ui| {
                    if !m.maker.is_empty() {
                        ui::kv(ui, t("Marca"), &m.maker);
                    }
                    if !m.part.is_empty() {
                        ui::kv(ui, t("Codice modello"), &m.part);
                    }
                    if !m.kind.is_empty() {
                        ui::kv(ui, t("Tipo"), &m.kind);
                    }
                    if !m.speed.is_empty() {
                        ui::kv(ui, t("Velocità"), &m.speed);
                    }
                    if !m.form.is_empty() {
                        ui::kv(ui, t("Formato"), &m.form);
                    }
                });
            });
            ui.add_space(3.0);
        }
    }

    pub(crate) fn info_gpu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::hint(
            ui,
            "La scheda video (GPU) disegna tutto ciò che vedi sullo schermo. Qui trovi modello, produttore, driver in uso, memoria video e i monitor collegati.",
        );
        if ui.button(t("🔄 Ricarica")).clicked() {
            self.load_hw(ctx);
        }
        ui.add_space(3.0);
        if self.gpus.is_empty() {
            ui.label(t("Nessuna scheda video rilevata (installa pciutils: sudo apt install pciutils)"));
        }
        for (i, s) in self.gpus.iter().enumerate() {
            section_card(ui, 200 + i, &s.title, &s.rows);
        }
    }

    pub(crate) fn info_components(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::hint(
            ui,
            "Elenco di tutti i componenti del computer: scheda madre, processore, memoria, schede video e audio, rete, dischi, USB, tastiera, webcam, monitor e batteria. Utile per sapere esattamente cosa c'è dentro il tuo PC.",
        );
        if ui.button(t("🔄 Ricarica")).clicked() {
            self.load_hw(ctx);
        }
        ui.add_space(3.0);
        let c = &self.info.cpu;
        let mut cpu: Vec<(String, String)> = vec![
            (t("Modello").to_string(), c.model.clone()),
            (t("Produttore").to_string(), c.vendor.clone()),
            (t("Architettura").to_string(), c.arch.clone()),
            (
                t("Core fisici").to_string(),
                c.physical.map(|p| p.to_string()).unwrap_or_else(|| t("n/d").to_string()),
            ),
            (t("Thread").to_string(), c.threads.to_string()),
        ];
        if !c.caches.is_empty() {
            cpu.push((t("Cache").to_string(), c.caches.join(" · ")));
        }
        section_card(ui, 300, t("Processore"), &cpu);
        for (i, s) in self.comps.iter().enumerate() {
            section_card(ui, 310 + i, &s.title, &s.rows);
        }
    }
}
