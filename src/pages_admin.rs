use crate::config_parsers::{self, AptLineKind};
use crate::i18n::{t, tf, tr};
use crate::{fix_job, health, ui, util, App, Job, Msg, ProgressReporter};
use eframe::egui;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct StartEntry {
    pub file: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Clone)]
pub struct GroupRow {
    pub name: &'static str,
    pub label: &'static str,
    pub exists: bool,
    pub member: bool,
    pub want: bool,
}

const GROUPS: [(&str, &str); 4] = [
    ("sudo", "Amministratore (sudo)"),
    ("dialout", "Porte seriali Arduino/USB (dialout)"),
    ("vboxusers", "VirtualBox (vboxusers)"),
    ("docker", "Docker (docker)"),
];

#[derive(Clone)]
pub struct DriverProp {
    pub pkg: &'static str,
    pub desc: &'static str,
    pub hw: String,
    pub installed: bool,
    pub available: bool,
    pub recommended: bool,
    pub sel: bool,
}

/// Voce di una sorgente APT mostrata nella gestione repository.
/// `line = None` identifica un intero file in `/etc/apt/sources.list.d/`.
#[derive(Clone, Debug)]
pub struct RepoEntry {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub text: String,
    pub selected: bool,
}

/// File sorgente APT ammessi: solo file regolari, mai symlink.
fn apt_source_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let main = PathBuf::from("/etc/apt/sources.list");
    if fs::symlink_metadata(&main).map(|m| m.file_type().is_file()).unwrap_or(false) {
        files.push(main);
    }
    if let Ok(rd) = fs::read_dir("/etc/apt/sources.list.d") {
        let mut extra: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                let regular = fs::symlink_metadata(p).map(|m| m.file_type().is_file()).unwrap_or(false);
                let ext = p.extension().and_then(|x| x.to_str());
                regular && matches!(ext, Some("list") | Some("sources"))
            })
            .collect();
        extra.sort();
        files.extend(extra);
    }
    files
}

/// Elenco testuale delle sorgenti, uguale a quello mostrato prima ma letto
/// direttamente (e senza seguire symlink).
pub(crate) fn read_sources_text() -> String {
    let mut out = String::new();
    for p in apt_source_files() {
        match fs::read_to_string(&p) {
            Ok(txt) => out.push_str(&format!("### {}\n{}\n", p.display(), txt.trim_end())),
            Err(e) => out.push_str(&format!("### {}\n✘ {e}\n", p.display())),
        }
    }
    out
}

/// Ricostruisce l'elenco selezionabile da `/etc/apt/sources.list*`:
/// righe `deb`/`deb-src` per i file classici, una voce per file `.sources`.
pub(crate) fn collect_repo_entries() -> Vec<RepoEntry> {
    let mut out = Vec::new();
    for p in apt_source_files() {
        let in_list_d = p.parent() == Some(Path::new("/etc/apt/sources.list.d"));
        let is_deb822 = p.extension().and_then(|x| x.to_str()) == Some("sources");
        if is_deb822 {
            out.push(RepoEntry { path: p.clone(), line: None, text: String::new(), selected: false });
            continue;
        }
        if let Ok(txt) = fs::read_to_string(&p) {
            for parsed in config_parsers::parse_apt_lines(&txt) {
                if parsed.enabled
                    && matches!(parsed.kind, AptLineKind::Deb | AptLineKind::DebSrc)
                {
                    out.push(RepoEntry {
                        path: p.clone(),
                        line: Some(parsed.number),
                        text: parsed.text.to_string(),
                        selected: false,
                    });
                }
            }
        }
        // Per i file in sources.list.d offre anche l'azione sull'intero file.
        if in_list_d {
            out.push(RepoEntry { path: p.clone(), line: None, text: String::new(), selected: false });
        }
    }
    out
}

/// Valida un percorso prima di passarlo a uno script privilegiato: vieta
/// symlink e qualsiasi destinazione fuori dalle due posizioni ammesse.
fn safe_repo_path(p: &Path) -> Option<PathBuf> {
    let meta = fs::symlink_metadata(p).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let canon = fs::canonicalize(p).ok()?;
    if canon == Path::new("/etc/apt/sources.list") {
        return Some(canon);
    }
    let dir = fs::canonicalize("/etc/apt/sources.list.d").ok()?;
    let ext = canon.extension().and_then(|x| x.to_str());
    if canon.parent() == Some(dir.as_path()) && matches!(ext, Some("list") | Some("sources")) {
        Some(canon)
    } else {
        None
    }
}

fn find_line(text: &str, pred: impl Fn(&str) -> bool) -> Option<String> {
    text.lines()
        .find(|l| pred(&l.to_lowercase()))
        .map(|l| l.split_once(' ').map(|x| x.1).unwrap_or(l).trim().to_string())
}

fn push_prop(props: &mut Vec<DriverProp>, pkg: &'static str, desc: &'static str, hw: String, rec: bool) {
    if props.iter().any(|p| p.pkg == pkg) {
        return;
    }
    props.push(DriverProp { pkg, desc, hw, installed: false, available: true, recommended: rec, sel: false });
}

/// Rileva l'hardware (lspci, lsusb, /proc/cpuinfo) e propone i pacchetti driver/firmware adatti.
pub fn detect_drivers() -> (Vec<(String, String)>, Vec<DriverProp>) {
    let pci = util::run_shell("lspci -nn 2>/dev/null", false).1;
    let usb = util::run_shell("lsusb 2>/dev/null", false).1;
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let mut hw: Vec<(String, String)> = Vec::new();
    let mut props: Vec<DriverProp> = Vec::new();

    // Processore
    let cpu_model = cpuinfo
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split_once(':').map(|x| x.1.trim().to_string()))
        .unwrap_or_else(|| "n/d".into());
    hw.push(("Processore".into(), cpu_model.clone()));
    if cpuinfo.contains("GenuineIntel") {
        push_prop(&mut props, "intel-microcode", "Aggiornamenti microcode per processori Intel (sicurezza e stabilità)", cpu_model.clone(), true);
    } else if cpuinfo.contains("AuthenticAMD") {
        push_prop(&mut props, "amd64-microcode", "Aggiornamenti microcode per processori AMD (sicurezza e stabilità)", cpu_model.clone(), true);
    }

    // Schede video
    for l in pci.lines().filter(|l| {
        let l = l.to_lowercase();
        l.contains("vga compatible") || l.contains("3d controller") || l.contains("display controller")
    }) {
        let name = l.split_once(' ').map(|x| x.1).unwrap_or(l).trim().to_string();
        let low = name.to_lowercase();
        hw.push(("Scheda video".into(), name.clone()));
        if low.contains("nvidia") {
            push_prop(&mut props, "nvidia-driver", "Driver NVIDIA proprietario per la grafica 3D", name.clone(), true);
        } else if low.contains("advanced micro devices") || low.contains("amd/ati") || low.contains("ati technologies") {
            push_prop(&mut props, "firmware-amd-graphics", "Firmware per schede video AMD/ATI", name.clone(), true);
            push_prop(&mut props, "mesa-vulkan-drivers", "Driver Vulkan (giochi e applicazioni 3D)", name.clone(), false);
        } else if low.contains("intel") {
            push_prop(&mut props, "intel-media-va-driver", "Accelerazione video hardware Intel", name.clone(), false);
            push_prop(&mut props, "mesa-vulkan-drivers", "Driver Vulkan (giochi e applicazioni 3D)", name.clone(), false);
        }
    }

    // Rete (PCI e USB)
    let net_pci: Vec<&str> = pci
        .lines()
        .filter(|l| {
            let l = l.to_lowercase();
            l.contains("network controller") || l.contains("ethernet controller")
        })
        .collect();
    for l in &net_pci {
        let name = l.split_once(' ').map(|x| x.1).unwrap_or(l).trim().to_string();
        let low = name.to_lowercase();
        hw.push(("Rete".into(), name.clone()));
        if low.contains("realtek") {
            push_prop(&mut props, "firmware-realtek", "Firmware per schede di rete e Wi-Fi Realtek", name.clone(), true);
        } else if low.contains("atheros") || low.contains("qualcomm") {
            push_prop(&mut props, "firmware-atheros", "Firmware per Wi-Fi Atheros/Qualcomm", name.clone(), true);
        } else if low.contains("broadcom") {
            push_prop(&mut props, "firmware-brcm80211", "Firmware per Wi-Fi Broadcom", name.clone(), true);
        } else if low.contains("mediatek") || low.contains("ralink") {
            push_prop(&mut props, "firmware-misc-nonfree", "Firmware vari (Wi-Fi MediaTek/Ralink e altro)", name.clone(), true);
        } else if low.contains("intel") && (low.contains("wireless") || low.contains("wi-fi") || low.contains("network controller")) {
            push_prop(&mut props, "firmware-iwlwifi", "Firmware per Wi-Fi Intel", name.clone(), true);
        }
    }
    for l in usb.lines() {
        let low = l.to_lowercase();
        let name = l.split_once(" ID ").map(|x| x.1).unwrap_or(l).trim().to_string();
        let wifi = low.contains("wireless") || low.contains("802.11") || low.contains("wlan") || low.contains("wifi");
        if wifi {
            hw.push(("Rete (USB)".into(), name.clone()));
            if low.contains("realtek") {
                push_prop(&mut props, "firmware-realtek", "Firmware per adattatori Wi-Fi Realtek", name.clone(), true);
            } else if low.contains("ralink") || low.contains("mediatek") {
                push_prop(&mut props, "firmware-misc-nonfree", "Firmware vari (Wi-Fi MediaTek/Ralink e altro)", name.clone(), true);
            } else if low.contains("atheros") {
                push_prop(&mut props, "firmware-atheros", "Firmware per adattatori Wi-Fi Atheros", name.clone(), true);
            }
        }
        if low.contains("bluetooth") {
            hw.push(("Bluetooth".into(), name.clone()));
            push_prop(&mut props, "bluez", "Supporto Bluetooth", name.clone(), true);
            if low.contains("realtek") {
                push_prop(&mut props, "firmware-realtek", "Firmware per Bluetooth Realtek", name.clone(), true);
            }
        }
    }

    // Audio
    if let Some(a) = find_line(&pci, |l| l.contains("audio device") || l.contains("multimedia audio")) {
        hw.push(("Audio".into(), a.clone()));
        let low = a.to_lowercase();
        let sof = ["sunrise", "cannon", "comet", "tiger", "alder", "raptor", "meteor", "ice lake", "jasper", "elkhart", "gemini", "apollo", "smart sound", "cavs"];
        if low.contains("intel") && sof.iter().any(|k| low.contains(k)) {
            push_prop(&mut props, "firmware-sof-signed", "Firmware audio Intel Sound Open Firmware", a, true);
        }
    }

    // Firmware generico consigliato
    push_prop(&mut props, "firmware-linux", "Firmware non liberi generici (consigliato per quasi tutti i computer)", "Sistema".into(), true);

    // Stato dei pacchetti: installato / disponibile nei repository
    for p in props.iter_mut() {
        let st = util::run_shell(&format!("dpkg-query -W -f='${{db:Status-Abbrev}}' {} 2>/dev/null", p.pkg), false).1;
        p.installed = st.trim().starts_with("ii");
        let cand = util::run_shell(&format!("apt-cache policy {} 2>/dev/null | awk '/Candidate:/{{print $2}}'", p.pkg), false).1;
        let cand = cand.trim();
        p.available = !cand.is_empty() && cand != "(none)";
        p.sel = p.recommended && p.available && !p.installed;
    }
    props.sort_by_key(|p| (p.installed, !p.recommended));
    (hw, props)
}

fn grub_preview_text(new_line: &str) -> String {
    let old = read_grub_line();
    let new = new_line.trim();
    let oldv: Vec<&str> = old.split_whitespace().collect();
    let newv: Vec<&str> = new.split_whitespace().collect();
    let added: Vec<&str> = newv.iter().copied().filter(|x| !oldv.contains(x)).collect();
    let removed: Vec<&str> = oldv.iter().copied().filter(|x| !newv.contains(x)).collect();
    let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let mut after: Vec<&str> = cmdline.split_whitespace().filter(|x| !removed.contains(x)).collect();
    for a in &added {
        if !after.contains(a) {
            after.push(*a);
        }
    }
    let list = |v: &Vec<&str>| if v.is_empty() { "-".to_string() } else { v.join(" ") };
    format!(
        "{}:\n  GRUB_CMDLINE_LINUX_DEFAULT=\"{}\"\n\n{}:\n  GRUB_CMDLINE_LINUX_DEFAULT=\"{}\"\n\n{}: {}\n{}: {}\n\n{}:\n  {}\n\n{}:\n  {}\n",
        t("Prima"),
        old,
        t("Dopo"),
        new,
        t("Parametri aggiunti"),
        list(&added),
        t("Parametri rimossi"),
        list(&removed),
        t("Riga di comando del kernel attuale"),
        cmdline.trim(),
        t("Riga di comando dopo il riavvio"),
        after.join(" ")
    )
}

/// Traduce i dettagli dei controlli, comprese le frasi con parti variabili.
fn tr_detail(s: &str) -> String {
    let direct = tr(s);
    if direct != s {
        return direct;
    }
    if let Some(r) = s.strip_prefix("In hold: ") {
        return format!("{}{}", tr("In hold: "), r);
    }
    const RC: &str = " pacchetti rimossi hanno ancora file di configurazione";
    if let Some(n) = s.strip_suffix(RC) {
        return format!("{}{}", n, tr(RC));
    }
    const HINT: &str = " — usa la scheda Pulizia per liberare spazio";
    if let Some(r) = s.strip_prefix("Occupato ") {
        return match r.strip_suffix(HINT) {
            Some(p) => format!("{} {}{}", tr("Occupato"), p, tr(HINT)),
            None => format!("{} {}", tr("Occupato"), r),
        };
    }
    s.to_string()
}

fn valid_pkg(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-_:".contains(c))
}

fn codename() -> String {
    fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VERSION_CODENAME="))
                .map(|l| l.trim_start_matches("VERSION_CODENAME=").trim_matches('"').to_string())
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------- dati

pub fn read_kernels() -> Vec<(String, bool, bool)> {
    let running = util::run_shell("uname -r", false).1.trim().to_string();
    let (_, o) = util::run_shell("dpkg -l 'linux-image-[0-9]*' 2>/dev/null | awk '/^ii/{print $2}'", false);
    o.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .map(|n| {
            let r = !running.is_empty() && n.contains(&running);
            (n, r, false)
        })
        .collect()
}

fn read_gov_avail() -> Vec<String> {
    fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors")
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect()
}

pub fn read_grub_line() -> String {
    fs::read_to_string("/etc/default/grub")
        .map(|s| config_parsers::grub_value(&s, "GRUB_CMDLINE_LINUX_DEFAULT"))
        .unwrap_or_default()
}

pub fn read_groups() -> Vec<GroupRow> {
    let user = util::username();
    let ids = util::run_shell(&format!("id -nG {}", util::shq(&user)), false).1;
    let members: Vec<&str> = ids.split_whitespace().collect();
    GROUPS
        .iter()
        .map(|(n, l)| {
            let exists = util::run_shell(&format!("getent group {n} >/dev/null"), false).0;
            let member = members.contains(n);
            GroupRow { name: *n, label: *l, exists, member, want: member }
        })
        .collect()
}

fn startup_dirs() -> (PathBuf, PathBuf) {
    (PathBuf::from("/etc/xdg/autostart"), PathBuf::from(util::home()).join(".config/autostart"))
}

fn is_hidden(c: &str) -> bool {
    c.lines().any(|l| {
        let l = l.trim();
        l == "Hidden=true" || l == "X-GNOME-Autostart-enabled=false"
    })
}

pub fn read_startup() -> Vec<StartEntry> {
    let (sys, usr) = startup_dirs();
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for d in [&sys, &usr] {
        if let Ok(rd) = fs::read_dir(d) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".desktop") {
                    if let Ok(c) = fs::read_to_string(e.path()) {
                        map.insert(n, c);
                    }
                }
            }
        }
    }
    map.into_iter()
        .map(|(file, c)| {
            let name = c
                .lines()
                .find(|l| l.starts_with("Name="))
                .map(|l| l[5..].to_string())
                .unwrap_or_else(|| file.clone());
            StartEntry { enabled: !is_hidden(&c), file, name }
        })
        .collect()
}

fn set_startup(file: &str, enable: bool) -> Result<(), String> {
    if file.contains('/') {
        return Err("nome file non valido".into());
    }
    let (sys, usr) = startup_dirs();
    let content = fs::read_to_string(usr.join(file))
        .or_else(|_| fs::read_to_string(sys.join(file)))
        .map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = content
        .lines()
        .filter(|l| {
            let l = l.trim();
            l != "Hidden=true" && l != "X-GNOME-Autostart-enabled=false"
        })
        .map(String::from)
        .collect();
    if !enable {
        let pos = lines.iter().position(|l| l.trim() == "[Desktop Entry]").map(|p| p + 1).unwrap_or(lines.len());
        lines.insert(pos, "Hidden=true".into());
    }
    fs::create_dir_all(&usr).map_err(|e| e.to_string())?;
    fs::write(usr.join(file), lines.join("\n") + "\n").map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- interfaccia

impl App {
    pub(crate) fn load_kernels(&self, ctx: &egui::Context) {
        self.bg(ctx, |ch| {
            ch.send(Msg::Kernels(read_kernels()));
            let gov = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
                .unwrap_or_default()
                .trim()
                .to_string();
            let swap = fs::read_to_string("/proc/sys/vm/swappiness")
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(60);
            ch.send(Msg::Gov(gov, swap));
        });
    }

    pub(crate) fn load_groups(&self, ctx: &egui::Context) {
        self.bg(ctx, |ch| ch.send(Msg::Groups(read_groups())));
    }

    pub(crate) fn load_sources(&self, ctx: &egui::Context) {
        // Lettura rapida: niente barra di progresso (che non avrebbe `busy`
        // a proteggerla) e nessun output parziale da aggiornare.
        self.bg(ctx, |ch| {
            ch.send(Msg::Out("sources", read_sources_text()));
            ch.send(Msg::RepoEntries(collect_repo_entries()));
        });
    }

    /// Crea il job che modifica le sorgenti APT selezionate.
    ///
    /// Tutti i percorsi sono validati in Rust prima di entrare nello script:
    /// vengono accettati solo `/etc/apt/sources.list` e file regolari in
    /// `/etc/apt/sources.list.d/`. Lo script gira in background con pkexec,
    /// crea un backup, applica la modifica e aggiorna gli elenchi APT.
    pub(crate) fn repo_change_job(entries: Vec<RepoEntry>, disable: bool) -> Job {
        Box::new(move |ch| {
            let mut prog = ProgressReporter::start(ch, "repo_change", t("Modifica repository in corso..."));
            prog.status(t("Creazione backup e applicazione delle modifiche..."));

            let mut lines: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
            let mut files: Vec<PathBuf> = Vec::new();
            let mut invalid: Vec<String> = Vec::new();
            for e in entries.into_iter().filter(|e| e.selected) {
                let Some(p) = safe_repo_path(&e.path) else {
                    invalid.push(e.path.display().to_string());
                    continue;
                };
                match e.line {
                    Some(n) if n > 0 => lines.entry(p).or_default().push(n),
                    Some(_) => invalid.push(e.path.display().to_string()),
                    None if p == Path::new("/etc/apt/sources.list") => {
                        invalid.push(t("Il file principale /etc/apt/sources.list può essere modificato solo per righe.").to_string());
                    }
                    None => {
                        if !files.contains(&p) {
                            files.push(p);
                        }
                    }
                }
            }

            if !invalid.is_empty() {
                let joined = invalid.join(", ");
                let msg = tf("Percorso non valido o non consentito: {}", &[&joined]);
                ch.send(Msg::Log(format!("✘ {msg}")));
                ch.send(Msg::Out("repo", msg.clone()));
                prog.finish(msg);
                return;
            }
            if lines.is_empty() && files.is_empty() {
                let msg = t("Nessuna selezione valida: nessuna sorgente è stata modificata.").to_string();
                ch.send(Msg::Log(format!("✘ {msg}")));
                ch.send(Msg::Out("repo", msg.clone()));
                prog.finish(msg);
                return;
            }

            let mut unique: Vec<PathBuf> = lines.keys().cloned().chain(files.iter().cloned()).collect();
            unique.sort();
            unique.dedup();

            let mut script = String::from("set -eu\n");
            script.push_str("STAMP=$(date +%Y%m%d-%H%M%S)\n");
            script.push_str("BACKUP=/etc/apt/debutility-backups/$STAMP\n");
            script.push_str("mkdir -p \"$BACKUP\"\nchmod 700 \"$BACKUP\"\necho \"Backup: $BACKUP\"\n");
            for (i, p) in unique.iter().enumerate() {
                let fv = format!("F{i}");
                let bv = format!("B{i}");
                let backup_name = format!("{:02}-{}", i + 1, p.file_name().unwrap_or_default().to_string_lossy());
                script.push_str(&format!("{fv}={}\n", util::shq(&p.to_string_lossy())));
                script.push_str(&format!("[ -f \"${fv}\" ] || {{ echo \"ERROR missing: ${fv}\" >&2; exit 1; }}\n"));
                script.push_str(&format!("[ ! -L \"${fv}\" ] || {{ echo \"ERROR symlink: ${fv}\" >&2; exit 1; }}\n"));
                script.push_str(&format!("{bv}={}\n", util::shq(&backup_name)));
                script.push_str(&format!("cp -p -- \"${fv}\" \"$BACKUP/${bv}\" || exit 1\n"));
            }

            for (i, (p, mut nums)) in lines.into_iter().enumerate() {
                let f_idx = unique.iter().position(|x| x == &p).unwrap_or(0);
                let fv = format!("F{f_idx}");
                let tv = format!("T{i}");
                // In caso di rimozione, partire dal fondo evita di sfalsare i numeri di riga.
                nums.sort_unstable_by(|a, b| b.cmp(a));
                nums.dedup();
                script.push_str(&format!("{tv}=\"/etc/apt/.debutility-repo-{i}.tmp\"\n"));
                if disable {
                    let cond = nums.iter().map(|n| format!("NR=={n}")).collect::<Vec<_>>().join(" || ");
                    script.push_str(&format!(
                        "awk '{} {{ if ($0 ~ /^[[:space:]]*#/) print; else print \"# \" $0; next }} {{ print }}' \"${fv}\" > \"${tv}\" || exit 1\n",
                        cond
                    ));
                } else {
                    let cond = nums.iter().map(|n| format!("NR!={n}")).collect::<Vec<_>>().join(" && ");
                    script.push_str(&format!("awk '{} {{ print }}' \"${fv}\" > \"${tv}\" || exit 1\n", cond));
                }
                script.push_str(&format!("cat \"${tv}\" > \"${fv}\" || {{ rm -f \"${tv}\"; exit 1; }}\n"));
                script.push_str(&format!("rm -f \"${tv}\"\n"));
            }

            for (i, p) in files.iter().enumerate() {
                let f_idx = unique.iter().position(|x| x == p).unwrap_or(0);
                let fv = format!("F{f_idx}");
                if disable {
                    script.push_str(&format!("D{i}=\"${fv}.disabled-$STAMP\"\nmv -- \"${fv}\" \"$D{i}\" || exit 1\necho \"disabled: $D{i}\"\n"));
                } else {
                    script.push_str(&format!("rm -f -- \"${fv}\" || exit 1\necho \"removed: ${fv}\"\n"));
                }
            }
            script.push_str("echo 'update'; apt-get update || { echo 'ERROR apt-get update failed' >&2; exit 1; }\necho 'done'\n");

            let mut output = String::new();
            let ok = util::run_stream(&script, true, |line, _commit| {
                output.push_str(line);
                output.push('\n');
                prog.track_line(line);
            });

            // Output e elenco vengono aggiornati sempre, anche in caso di errore:
            // l'utente deve poter verificare cosa è cambiato e dove sta il backup.
            ch.send(Msg::RepoEntries(collect_repo_entries()));
            ch.send(Msg::Out("sources", read_sources_text()));
            ch.send(Msg::Out("repo", output));
            let msg = if ok {
                t("Modifica repository completata").to_string()
            } else {
                t("Modifica repository terminata con errori").to_string()
            };
            ch.send(Msg::Log(format!("{} {msg}", if ok { "✔" } else { "✘" })));
            prog.finish(msg);
        })
    }

    // ----- Fixer
    pub(crate) fn ui_fixer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Fixer — diagnosi e riparazione");
        ui::hint(
            ui,
            "Controlla se il sistema ha problemi (pacchetti rotti, servizi fermi, disco quasi pieno...). Il segno ✔ verde significa che va tutto bene; se vedi ⚠ o ✘ premi «Correggi» accanto al problema, oppure «Correggi tutto».",
        );
        self.snapshot_toggle(ui);
        
        let fix_list: Vec<&str> = self
            .checks
            .iter()
            .filter(|c| c.status != health::Status::Ok)
            .filter_map(|c| c.fix)
            .collect();
        let all_fixes: String = fix_list.join(" && ");
        
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🔄 Esegui controlli"))).clicked() {
                self.spawn(ctx, |ch| ch.send(Msg::Checks(health::run_checks())));
            }
            if ui
                .add_enabled(self.action_enabled() && !all_fixes.is_empty(), egui::Button::new(t("🛠 Correggi tutto")))
                .clicked()
            {
                self.spawn_snapshot(ctx, fix_job(all_fixes.clone()));
            }
            if self.busy {
                ui.spinner();
            }
        });
        ui.separator();

        let mut fix_one: Option<String> = None;
        ui::card(ui, |ui| {
            if self.checks.is_empty() {
                ui.label(t("Nessun controllo eseguito: premi «Esegui controlli»."));
            }
            for c in &self.checks {
                ui.horizontal(|ui| {
                    let (icon, col) = match c.status {
                        health::Status::Ok => ("✔", ui::GOOD),
                        health::Status::Warn => ("⚠", ui::WARN),
                        health::Status::Problem => ("✘", ui::BAD),
                    };
                    ui.label(egui::RichText::new(icon).size(20.0).strong().color(col));
                    ui.label(egui::RichText::new(t(c.name)).strong().size(18.0));
                    if let Some(f) = c.fix {
                        if c.status != health::Status::Ok
                            && ui.add_enabled(self.action_enabled(), egui::Button::new(t("Correggi"))).clicked()
                        {
                            fix_one = Some(f.to_string());
                        }
                    }
                });
                ui.label(egui::RichText::new(tr_detail(&c.detail)).color(ui::text_dim()));
                ui.add_space(3.0);
            }
        });
        if let Some(f) = fix_one {
            self.spawn_snapshot(ctx, fix_job(f));
        }
ui.add_space(3.0);
    ui::card(ui, |ui| {
        ui.label(egui::RichText::new(t("Risoluzione Errori APT e Chiavi GPG")).strong().color(ui::accent_text()).size(16.0));
        ui.add_space(2.0);
        ui.label(t("Rileva automaticamente le firme digitali mancanti (NO_PUBKEY) dai repository PPA di terze parti e le scarica dai server ufficiali per sbloccare gli aggiornamenti di sistema."));
        ui.add_space(3.0);

        if ui::primary_button(ui, t("Correggi chiavi GPG mancanti"), self.action_enabled()).clicked() {
            let script = crate::solutions::fix_missing_gpg_keys_script(); // <-- Modifica qui
            let job = crate::cmd_job("fixer", script, true);
            self.spawn_snapshot(ctx, job);
        }
    });
        ui.separator();
        ui.heading(t("Strumenti di riparazione manuali"));
        ui.horizontal_wrapped(|ui| {
            self.run_btn_snapshot(
                ui, ctx, t("Ripara database APT"), "fixer",
                "if command -v fuser >/dev/null 2>&1 && fuser /var/lib/dpkg/lock-frontend /var/lib/dpkg/lock /var/lib/apt/lists/lock /var/cache/apt/archives/lock >/dev/null 2>&1; then echo 'apt/dpkg è in esecuzione: attendi che finisca prima di riparare.'; exit 1; fi; rm -f /var/lib/apt/lists/lock /var/cache/apt/archives/lock; dpkg --configure -a; apt-get -y -f install; apt-get update",
                true, None,
            );
            self.run_btn_snapshot(ui, ctx, t("Correggi dipendenze rotte"), "fixer", "apt-get -y -f install", true, None);
            self.run_btn_snapshot(ui, ctx, t("Riconfigura pacchetti"), "fixer", "dpkg --configure -a", true, None);
            self.run_btn_snapshot(ui, ctx, t("Pulisci pacchetti orfani"), "fixer", "apt-get -y autoremove --purge", true, None);
            self.run_btn_snapshot(
                ui, ctx, t("Ripara chiavi APT mancanti"), "fixer",
                "for k in $(apt-get update 2>&1 | grep -o 'NO_PUBKEY [0-9A-F]*' | awk '{print $2}' | sort -u); do \
                 gpg --batch --keyserver hkps://keyserver.ubuntu.com --recv-keys $k && \
                 gpg --batch --export $k > /etc/apt/trusted.gpg.d/debutility-$k.gpg; done; apt-get update",
                true, None,
            );
            self.run_btn(
                ui, ctx, t("Verifica integrità pacchetti"), "fixer",
                "if command -v debsums >/dev/null 2>&1; then debsums -s 2>&1 | head -60; else dpkg --verify 2>&1 | head -60; fi; echo '(fine verifica)'",
                false, None,
            );
            self.run_btn(
                ui, ctx, t("Controlla residui dpkg/rc"), "fixer",
                "echo '=== Pacchetti in stato rc ==='; dpkg -l | awk '/^rc/{print $2, $3}' || true; \
                 echo '=== File .dpkg-old / .dpkg-dist / .ucf-dist in /etc ==='; \
                 find /etc \\( -name '*.dpkg-old' -o -name '*.dpkg-dist' -o -name '*.ucf-dist' \\) 2>/dev/null | head -100",
                false, None,
            );
            self.run_btn_snapshot(
                ui, ctx, t("Pulisci residui dpkg/rc"), "fixer",
                "RC_PKGS=$(dpkg -l | awk '/^rc/{print $2}'); \
                 if [ -n \"$RC_PKGS\" ]; then dpkg --purge $RC_PKGS; else echo 'Nessun pacchetto in stato rc.'; fi; \
                 find /etc \\( -name '*.dpkg-old' -o -name '*.dpkg-dist' -o -name '*.ucf-dist' \\) -print -delete 2>/dev/null || true",
                true,
                Some(t("Eliminare definitivamente i file di configurazione residui (.dpkg-old, .dpkg-dist, .ucf-dist) e purgare i pacchetti in stato rc?")),
            );
            self.run_btn(
                ui, ctx, t("Mostra servizi falliti"), "fixer",
                "systemctl --failed --no-pager --plain || true",
                false, None,
            );
            self.run_btn_snapshot(
                ui, ctx, t("Riavvia servizi falliti"), "fixer",
                "UNITS=$(systemctl --failed --no-legend --plain | awk '{print $1}'); \
                 if [ -z \"$UNITS\" ]; then echo 'Nessuna unità systemd in stato failed.'; \
                 else echo \"Riavvio unità: $UNITS\"; systemctl restart $UNITS; systemctl --failed --no-pager --plain || true; fi",
                true,
                Some(t("Riavviare tutte le unità systemd attualmente in stato failed?")),
            );
        });
        self.show_out(ui, "fixer", 260.0);
    }

    // ----- Kernel / Grub / Recovery
    pub(crate) fn ui_kernel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Kernel, Grub e Recovery"));
        ui::sub_tabs(ui, &mut self.sub, &["Kernel manager", "Kernel e prestazioni", "Grub", "Recovery"]);
        match self.sub {
            0 => self.kernel_mgr(ui, ctx),
            1 => self.kernel_perf(ui, ctx),
            2 => self.grub_ui(ui, ctx),
            _ => self.recovery_ui(ui, ctx),
        }
    }

    fn kernel_mgr(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(t("Rimuovi i kernel obsoleti per liberare spazio. Il kernel in uso è protetto."));
        for k in self.kernels.iter_mut() {
            let label = if k.1 { format!("{} ({})", k.0, t("in uso")) } else { k.0.clone() };
            ui.add_enabled_ui(!k.1, |ui| {
                ui.checkbox(&mut k.2, label);
            });
        }
        let sel: Vec<String> =
            self.kernels.iter().filter(|k| k.2 && !k.1 && valid_pkg(&k.0)).map(|k| k.0.clone()).collect();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.action_enabled() && !sel.is_empty(), egui::Button::new(t("🗑 Rimuovi selezionati")))
                .clicked()
            {
                let pk = sel.iter().map(|s| util::shq(s)).collect::<Vec<_>>().join(" ");
                let script = format!("apt-get -y purge {pk}; update-grub");
                let job: Job = Box::new(move |ch| {
                    let (ok, out) = util::run_shell(&script, true);
                    let mark = if ok { "✔" } else { "✘" };
                    ch.send(Msg::Log(format!("{mark} {}", util::tail(out.trim(), 3).replace('\n', " | "))));
                    ch.send(Msg::Kernels(read_kernels()));
                });
                self.confirm = Some((tf("Rimuovere {} kernel e aggiornare GRUB?", &[&sel.len()]), job));
            }
            if ui.button(t("🔄 Ricarica")).clicked() {
                self.load_kernels(ctx);
            }
        });
    }

    fn kernel_perf(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::hint(
            ui,
            t("Il kernel è il cuore del sistema. Qui scegli come il processore bilancia velocità e consumi (governor) e quanto il sistema usa il disco al posto della RAM (swappiness). Se non sei sicuro, lascia i valori attuali."),
        );
        let avail = read_gov_avail();
        ui::section(ui, "⚙", "Governor CPU", "");
        if avail.is_empty() {
            ui.label(t("Gestione della frequenza non disponibile su questo sistema."));
        } else {
            egui::ComboBox::from_label(t("Governor"))
                .selected_text(self.governor.clone())
                .show_ui(ui, |ui| {
                    for g in &avail {
                        ui.selectable_value(&mut self.governor, g.clone(), g.as_str());
                    }
                });
            let script = format!(
                "for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo {} > \"$f\"; done; echo 'Governor impostato'",
                self.governor
            );
            let valid = avail.contains(&self.governor);
            ui.add_enabled_ui(valid, |ui| {
                self.run_btn(ui, ctx, t("Applica governor"), "perf", &script, true, None);
            });
        }
        ui.add_space(4.0);
        ui::section(ui, "◈", "Memoria virtuale (swappiness)", "");
        ui.add(egui::Slider::new(&mut self.swappiness, 0..=100).text(t("vm.swappiness")));
        let n = self.swappiness;
        let script = format!(
            "sysctl -w vm.swappiness={n} && echo vm.swappiness={n} > /etc/sysctl.d/99-debutility.conf && echo 'Swappiness impostato (permanente)'"
        );
        self.run_btn(ui, ctx, t("Applica swappiness"), "perf", &script, true, None);
        self.show_out(ui, "perf", 100.0);
    }

    fn grub_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::hint(
            ui,
            t("GRUB è il programma che parte per primo e avvia Debian. I «parametri di avvio» sono opzioni passate al sistema quando si accende. Usa «Anteprima» per vedere cosa cambierebbe prima di salvare; se non sei sicuro, non modificare nulla."),
        );
        ui.label(t("Parametri di avvio (GRUB_CMDLINE_LINUX_DEFAULT):"));
        ui.add(egui::TextEdit::singleline(&mut self.grub_line).desired_width(ui::fit_width(ui, 520.0, 180.0)));
        let hw = format!("{} {}", self.info.gpu.join(" "), self.info.cpu.vendor).to_lowercase();
        let mut sugg: Vec<(&'static str, &'static str)> = vec![
            ("quiet", "Avvio silenzioso"),
            ("splash", "Schermata grafica di avvio"),
            ("nomodeset", "Utile solo se hai problemi con la grafica"),
            ("zswap.enabled=1", "Swap compresso in RAM"),
        ];
        if hw.contains("nvidia") {
            sugg.push(("nvidia-drm.modeset=1", "Consigliato con driver NVIDIA proprietario"));
        }
        ui.label(t("Suggerimenti in base all'hardware:"));
        ui.horizontal_wrapped(|ui| {
            for (tok, desc) in sugg {
                if ui.small_button(format!("+ {tok}")).on_hover_text(t(desc)).clicked()
                    && !self.grub_line.split_whitespace().any(|t| t == tok)
                {
                    if !self.grub_line.trim().is_empty() {
                        self.grub_line.push(' ');
                    }
                    self.grub_line.push_str(tok);
                }
            }
        });
        let valid = self.grub_line.chars().all(|c| c.is_ascii_alphanumeric() || " _=.,:-".contains(c));
        if !valid {
            ui.colored_label(ui::semantic_text(ui::BAD), t("Caratteri non ammessi nei parametri."));
        }

        let clean_grub = self.grub_line.trim();
        let script = format!(
            "cp -n /etc/default/grub /etc/default/grub.debutility.bak; \
             VAL={}; \
             awk -v val=\"$VAL\" 'BEGIN{{done=0}} /^GRUB_CMDLINE_LINUX_DEFAULT=/{{\
               print \"GRUB_CMDLINE_LINUX_DEFAULT=\\\"\" val \"\\\"\"; done=1; next}} \
               {{print}} END{{if(!done) print \"GRUB_CMDLINE_LINUX_DEFAULT=\\\"\" val \"\\\"\"}}' \
               /etc/default/grub > /etc/default/grub.tmp && mv /etc/default/grub.tmp /etc/default/grub && update-grub",
            util::shq(clean_grub)
        );

        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(valid, |ui| {
                self.run_btn(
                    ui, ctx, t("💾 Salva e aggiorna GRUB"), "grub", &script, true,
                    Some(t("Salvare i parametri e rigenerare GRUB? Verrà creato un backup in /etc/default/grub.debutility.bak.")),
                );
            });
            if ui.button(t("Anteprima")).clicked() {
                self.grub_preview = grub_preview_text(&self.grub_line);
            }
            self.run_btn(ui, ctx, t("Aggiorna GRUB"), "grub", "update-grub", true, None);
            let restore_script = "if [ -f /etc/default/grub.debutility.bak ]; then cp /etc/default/grub.debutility.bak /etc/default/grub && update-grub; elif [ -f /etc/default/grub.debclean.bak ]; then cp /etc/default/grub.debclean.bak /etc/default/grub && update-grub; else echo 'Nessun backup grub.debutility.bak o grub.debclean.bak trovato'; exit 1; fi";
            self.run_btn(
                ui,
                ctx,
                t("↩ Ripristina backup"),
                "grub",
                restore_script,
                true,
                Some(t("Ripristinare /etc/default/grub dal backup debutility.bak e aggiornare GRUB?")),
            );
        });
        if !self.grub_preview.is_empty() {
            ui::section_text(ui, "👁", t("Anteprima delle modifiche (non è stato cambiato nulla)"), "");
            ui::output_box(ui, &self.grub_preview, 280.0);
        }
        self.show_out(ui, "grub", 200.0);
    }

    fn recovery_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::hint(
            ui,
            t("Strumenti di emergenza per quando qualcosa smette di funzionare: riavviano audio e rete, ricostruiscono il menu di avvio (GRUB) o ripristinano Flathub. Usali solo se hai un problema preciso."),
        );
        ui.horizontal_wrapped(|ui| {
            self.run_btn(
                ui, ctx, t("🔊 Riavvia audio"), "recovery",
                "systemctl --user restart pipewire pipewire-pulse wireplumber 2>/dev/null || pulseaudio -k; echo 'Audio riavviato'",
                false, None,
            );
            self.run_btn(
                ui, ctx, t("🌐 Riavvia rete"), "recovery",
                "systemctl restart NetworkManager && echo 'Rete riavviata'", true,
                Some(t("Riavviare i servizi di rete?")),
            );
            self.run_btn(
                ui, ctx, t("🧱 Ricostruisci GRUB"), "recovery", "update-grub", true,
                Some(t("Rigenerare la configurazione di GRUB?")),
            );
            self.run_btn(
                ui, ctx, t("♻ Reinstalla GRUB"), "recovery",
                "if [ -d /sys/firmware/efi ]; then grub-install; else grub-install \"/dev/$(lsblk -no pkname \"$(findmnt -no SOURCE /)\" | head -1)\"; fi; update-grub",
                true,
                Some(t("Reinstallare il bootloader GRUB? Operazione delicata: assicurati di avere un backup.")),
            );
            self.run_btn(
                ui, ctx, t("📦 Ripristina Flathub"), "recovery",
                "flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo && echo 'Flathub ripristinato'",
                false, None,
            );
        });
        self.show_out(ui, "recovery", 260.0);
    }

    // ----- Gestione sicura delle sorgenti APT

    fn repo_action(&mut self, disable: bool) {
        let selected: Vec<RepoEntry> =
            self.repo_entries.iter().filter(|e| e.selected).cloned().collect();
        if selected.is_empty() {
            self.log.push_str(t("Seleziona almeno una riga o un file prima di procedere."));
            self.log.push('\n');
            return;
        }
        let n = selected.len();
        let text = if disable {
            tf("Disabilitare {} sorgenti selezionate? Le righe verranno commentate e i file rinominati con .disabled; prima viene creato un backup.", &[&n])
        } else {
            tf("Rimuovere {} sorgenti selezionate? Verrà creato un backup e poi aggiornato l'elenco dei pacchetti.", &[&n])
        };
        self.confirm = Some((text, Self::repo_change_job(selected, disable)));
    }

    pub(crate) fn ui_repo_management(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::card_compact(ui, |ui| {
            ui.label(
                egui::RichText::new(t("Rimozione e disabilitazione repository"))
                    .strong()
                    .size(15.0)
                    .color(ui::accent_text()),
            );
            ui.label(
                egui::RichText::new(t("⚠ Le modifiche alle sorgenti APT richiedono pkexec. Viene creato un backup in /etc/apt/debutility-backups/ prima di toccare qualsiasi file; i symlink e i percorsi non ammessi vengono ignorati."))
                    .size(12.5)
                    .color(ui::semantic_text(ui::WARN)),
            );
            ui.add_space(3.0);

            ui.label(egui::RichText::new(t("Righe di repository configurate:")).strong().size(13.5));
            egui::ScrollArea::vertical().max_height(96.0).show(ui, |ui| {
                let mut shown = 0;
                for e in self.repo_entries.iter_mut().filter(|e| e.line.is_some()) {
                    shown += 1;
                    let label = format!(
                        "{}:{} — {}",
                        e.path.display(),
                        e.line.unwrap_or(0),
                        e.text
                    );
                    ui.checkbox(&mut e.selected, egui::RichText::new(label).size(12.5));
                }
                if shown == 0 {
                    ui.label(egui::RichText::new(t("Nessuna riga di repository trovata.")).size(12.5).color(ui::text_dim()));
                }
            });

            ui.add_space(2.0);
            ui.label(egui::RichText::new(t("File sorgente in /etc/apt/sources.list.d/:")).strong().size(13.5));
            egui::ScrollArea::vertical().max_height(72.0).show(ui, |ui| {
                let mut shown = 0;
                for e in self.repo_entries.iter_mut().filter(|e| e.line.is_none()) {
                    shown += 1;
                    let label = format!("{}", e.path.display());
                    ui.checkbox(&mut e.selected, egui::RichText::new(label).size(12.5));
                }
                if shown == 0 {
                    ui.label(egui::RichText::new(t("Nessun file sorgente trovato in /etc/apt/sources.list.d/.")).size(12.5).color(ui::text_dim()));
                }
            });

            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("🗑 Rimuovi selezionati"))).clicked() {
                    self.repo_action(false);
                }
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("⛔ Disabilita selezionati"))).clicked() {
                    self.repo_action(true);
                }
                if ui.button(t("🔄 Rileggi sorgenti")).clicked() {
                    self.load_sources(ctx);
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });
    }

    // ----- Repository e driver
    pub(crate) fn ui_repos(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Repository e driver");
        ui::sub_tabs(ui, &mut self.sub, &["Driver manager", "App repository", "Repository Manager"]);
        match self.sub {
            0 => self.drivers_ui(ui, ctx),
            1 => {
                ui.label(t("Sorgenti software configurate:"));
                self.show_out(ui, "sources", 180.0);
                self.ui_repo_management(ui, ctx);
                ui.add_space(3.0);
                ui.label(t("Aggiungi un repository (riga completa, es. \"deb http://... suite componenti\"):"));
                ui.add(egui::TextEdit::singleline(&mut self.repo_line).desired_width(ui::fit_width(ui, 520.0, 180.0)));
                let l = self.repo_line.trim().to_string();
                let valid = (l.starts_with("deb ") || l.starts_with("deb-src ")) && !l.contains('\n');
                let script = format!(
                    "printf '%s\\n' {} >> /etc/apt/sources.list.d/debutility.list && apt-get update",
                    util::shq(&l)
                );
                ui.add_enabled_ui(valid, |ui| {
                    self.run_btn(ui, ctx, t("Aggiungi repository"), "repo", &script, true,
                        Some(t("Aggiungere questo repository alle sorgenti APT?")));
                });
                self.show_out(ui, "repo", 120.0);
            }
            _ => {
                let cn = codename();
                let cn = if cn.is_empty() { t("n/d") } else { cn.as_str() };
                ui.label(tf("Release rilevata: {}", &[&cn]));
                let present = std::path::Path::new("/etc/apt/sources.list.d/debutility-backports.list").exists()
                    || std::path::Path::new("/etc/apt/sources.list.d/debclean-backports.list").exists();
                let backports = if present { t("abilitati da DebUtility") } else { t("non abilitati da DebUtility") };
                ui.label(tf("Backports: {}", &[&backports]));
                if cn.is_empty() || cn == "sid" || cn == "testing" || !cn.chars().all(|c| c.is_ascii_alphanumeric()) {
                    ui.label(t("I Backports non sono disponibili per questa release."));
                } else {
                    let comps = if cn == "buster" || cn == "bullseye" {
                        "main contrib non-free"
                    } else {
                        "main contrib non-free non-free-firmware"
                    };
                    let script = format!(
                        "echo 'deb http://deb.debian.org/debian {cn}-backports {comps}' > /etc/apt/sources.list.d/debutility-backports.list && apt-get update"
                    );
                    self.run_btn(ui, ctx, t("Abilita Debian Backports"), "repo", &script, true,
                        Some(t("Abilitare i Debian Backports?")));
                }
                ui.horizontal(|ui| {
                    self.run_btn(ui, ctx, t("Aggiorna elenchi pacchetti"), "repo", "apt-get update", true, None);
                });
                self.show_out(ui, "repo", 160.0);
                ui.label(t("Sorgenti avanzate:"));
                self.show_out(ui, "sources", 180.0);
                self.ui_repo_management(ui, ctx);
            }
        }
    }

    fn drivers_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(egui::RichText::new(t("Rileva l'hardware del computer e propone i driver e i firmware da installare.")).size(18.0));
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui::primary_button(ui, t("Rileva hardware e driver"), self.action_enabled()).clicked() {
                self.spawn(ctx, |ch| {
                    ch.send(Msg::Log("▶ Rilevo l'hardware...".into()));
                    let (hw, props) = detect_drivers();
                    ch.send(Msg::Log(format!("✔ Rilevati {} driver proponibili", props.len())));
                    ch.send(Msg::Drivers(hw, props));
                });
            }
            if self.busy {
                ui.spinner();
            }
        });
        ui.add_space(3.0);

        if self.drv.is_empty() {
            ui.label(egui::RichText::new(t("Premi «Rileva hardware e driver» per analizzare il computer.")).color(ui::text_dim()));
            return;
        }

        if !self.drv_hw.is_empty() {
            ui::card(ui, |ui| {
                ui.label(egui::RichText::new(t("Hardware rilevato")).strong().size(20.0));
                egui::Grid::new("drv_hw").num_columns(2).striped(true).show(ui, |ui| {
                    for (k, v) in &self.drv_hw {
                        ui::kv(ui, k, v);
                    }
                });
            });
            ui.add_space(4.0);
        }

        ui.label(egui::RichText::new(t("Driver proposti")).strong().size(20.0));
        ui.add_space(2.0);
        for d in self.drv.iter_mut() {
            ui::card(ui, |ui| {
                ui.horizontal(|ui| {
                    let enabled = d.available && !d.installed;
                    ui.add_enabled(
                        enabled,
                        egui::Checkbox::new(&mut d.sel, egui::RichText::new(d.pkg).strong().size(18.0)),
                    );
                    if d.installed {
                        ui.colored_label(ui::semantic_text(ui::GOOD), t("già installato"));
                    } else if !d.available {
                        ui.colored_label(ui::semantic_text(ui::WARN), t("non disponibile nei repository"));
                    } else if d.recommended {
                        ui.colored_label(ui::accent_text(), t("consigliato"));
                    }
                });
                ui.label(t(d.desc));
                ui.label(egui::RichText::new(format!("{}: {}", t("Hardware"), d.hw)).color(ui::text_dim()));
            });
            ui.add_space(3.0);
        }

        let missing = self.drv.iter().any(|d| !d.available && !d.installed);
        if missing {
            let cn = codename();
            ui::card(ui, |ui| {
                ui.colored_label(
                    ui::WARN,
                    t("Alcuni pacchetti non sono disponibili: i componenti contrib/non-free potrebbero non essere abilitati."),
                );
            });
            if !cn.is_empty()
                && cn.chars().all(|c| c.is_ascii_alphanumeric())
                && ui.add_enabled(self.action_enabled(), egui::Button::new(t("Abilita contrib e non-free"))).clicked()
            {
                let comps = if cn == "buster" || cn == "bullseye" {
                    "main contrib non-free"
                } else {
                    "main contrib non-free non-free-firmware"
                };
                let script = format!(
                    "echo 'deb http://deb.debian.org/debian {cn} {comps}' > /etc/apt/sources.list.d/debutility-nonfree.list && apt-get update"
                );
                let job: Job = Box::new(move |ch| {
                    let (ok, out) = util::run_shell(&script, true);
                    let mark = if ok { "✔" } else { "✘" };
                    ch.send(Msg::Log(format!("{mark} Componenti non-free: {}", util::tail(out.trim(), 2).replace('\n', " | "))));
                    let (hw, props) = detect_drivers();
                    ch.send(Msg::Drivers(hw, props));
                });
                self.confirm = Some((t("Abilitare i componenti contrib e non-free di Debian?").into(), job));
            }
            ui.add_space(3.0);
        }

        let sel: Vec<&'static str> =
            self.drv.iter().filter(|d| d.sel && d.available && !d.installed).map(|d| d.pkg).collect();
        ui.horizontal(|ui| {
            if ui::primary_button(ui, &format!("{} ({})", t("Installa selezionati"), sel.len()), self.action_enabled() && !sel.is_empty()).clicked() {
                let script = format!("apt-get -y install {}", sel.join(" "));
                let n = sel.len();
                let job: Job = Box::new(move |ch| {
                    let (ok, out) = util::run_shell(&script, true);
                    let mark = if ok { "✔" } else { "✘" };
                    ch.send(Msg::Log(format!("{mark} Installazione driver: {}", util::tail(out.trim(), 3).replace('\n', " | "))));
                    let (hw, props) = detect_drivers();
                    ch.send(Msg::Drivers(hw, props));
                });
                self.confirm = Some((tf("Installare {} pacchetti driver/firmware?", &[&n]), job));
            }
            if ui
                .add_enabled(
                    self.action_enabled() && self.drv.iter().any(|d| d.recommended && d.available && !d.installed),
                    egui::Button::new(t("⭐ Seleziona consigliati")),
                )
                .clicked()
            {
                for d in self.drv.iter_mut() {
                    if d.available && !d.installed {
                        d.sel = d.recommended;
                    }
                }
            }
        });
    }

    // ----- Startup

pub(crate) fn ui_startup(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui::title(ui, "Avvio e Servizi");
        
        // Aggiungiamo le sub-tab per dividere i programmi in avvio e le estensioni desktop
        ui::sub_tabs(
            ui,
            &mut self.sub,
            &["Programmi all'avvio", "Estensioni desktop"],
        );

        match self.sub {
            0 => {
                ui.label(t("Disattiva le applicazioni avviate automaticamente al login per ridurre i tempi di avvio."));
                let filter_id = egui::Id::new("debutility_startup_filter");
                let mut filter_text = ui.data_mut(|d| {
                    d.get_temp::<String>(filter_id).unwrap_or_default()
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Filtra:"));
                    if ui.add(egui::TextEdit::singleline(&mut filter_text).desired_width(ui::fit_width(ui, 240.0, 100.0))).changed() {
                        ui.data_mut(|d| d.insert_temp(filter_id, filter_text.clone()));
                    }
                    if !filter_text.is_empty() && ui.button(t("Pulisci")).clicked() {
                        filter_text.clear();
                        ui.data_mut(|d| d.insert_temp(filter_id, filter_text.clone()));
                    }
                });
                let q = filter_text.trim().to_lowercase();
                let mut toggle: Option<(String, bool)> = None;
                let mut shown = 0usize;
                for e in self.startup.iter() {
                    if !q.is_empty()
                        && !e.name.to_lowercase().contains(&q)
                        && !e.file.to_lowercase().contains(&q)
                    {
                        continue;
                    }
                    shown += 1;
                    let mut en = e.enabled;
                    if ui.checkbox(&mut en, format!("{}    ({})", e.name, e.file)).changed() {
                        toggle = Some((e.file.clone(), en));
                    }
                }
                if self.startup.is_empty() {
                    ui.label(t("Nessuna voce di avvio automatico trovata."));
                } else if shown == 0 {
                    ui.label(t("Nessuna voce corrisponde al filtro."));
                }
                ui.add_space(3.0);
                if ui.button(t("🔄 Ricarica elenco")).clicked() {
                    self.startup = read_startup();
                }
                // Se c'è un toggle da salvare, gestiscilo qui se era presente nel resto della funzione originale
                
                if let Some((file, val)) = toggle {
                    match set_startup(&file, val) {
                        Ok(()) => {
                            self.startup = read_startup();
                            self.log.push_str(&format!(
                                "✔ {}: {} ({})\n",
                                if val { t("Abilitato") } else { t("Disabilitato") },
                                file,
                                if val { "attivo" } else { "disattivo" }
                            ));
                        }
                        Err(e) => {
                            self.log.push_str(&format!("✘ {}: {e}\n", t("Errore avvio automatico")));
                        }
                    }
                }
                
            }
            1 => {
                // Chiamata richiesta da Google AI Studio
                self.ui_desktop_ext(ui);
            }
            _ => {}
        }
    }

    // ----- Permessi utente
    pub(crate) fn ui_perms(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Gestione permessi utente");
        ui::hint(
            ui,
            "I gruppi decidono cosa può fare il tuo utente: sudo = diventare amministratore, dialout = usare le porte seriali (Arduino), vboxusers = VirtualBox, docker = Docker. Spunta quelli che ti servono, premi «Applica modifiche» e poi riavvia la sessione.",
        );
        let user = util::username();
        ui.label(format!("{}: {}", t("Utente"), user));
        for g in self.groups.iter_mut() {
            let label = if g.exists { t(g.label).to_string() } else { format!("{} ({})", t(g.label), t("gruppo non presente")) };
            ui.add_enabled_ui(g.exists, |ui| {
                ui.checkbox(&mut g.want, label);
            });
        }
        let mut cmds: Vec<String> = Vec::new();
        for g in &self.groups {
            if g.want && !g.member {
                cmds.push(format!("usermod -aG {} {}", g.name, util::shq(&user)));
            } else if !g.want && g.member {
                cmds.push(format!("gpasswd -d {} {}", util::shq(&user), g.name));
            }
        }
        let changing_sudo = self.groups.iter().any(|g| g.name == "sudo" && g.want != g.member);
        if changing_sudo {
            ui.add_space(2.0);
            ui.colored_label(
                ui::WARN,
                t("⚠ Attenzione: stai modificando il gruppo sudo (privilegi di amministratore). Controlla bene prima di confermare."),
            );
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled() && !cmds.is_empty(), egui::Button::new(t("✔ Applica modifiche"))).clicked() {
                let script = cmds.join("; ");
                let job: Job = Box::new(move |ch| {
                    let (ok, out) = util::run_shell(&script, true);
                    let mark = if ok { "✔" } else { "✘" };
                    ch.send(Msg::Log(format!("{mark} Permessi aggiornati {}", util::tail(out.trim(), 2))));
                    ch.send(Msg::Groups(read_groups()));
                });
                self.confirm = Some((
                    t("Modificare i gruppi dell'utente? Serve un logout o un riavvio per renderle attive.").into(),
                    job,
                ));
            }
            if ui.button(t("🔄 Ricarica")).clicked() {
                self.load_groups(ctx);
            }
        });
        ui.label(egui::RichText::new(t("Le modifiche diventano attive dopo logout o riavvio.")).color(crate::ui::text_dim()));
    }
}
