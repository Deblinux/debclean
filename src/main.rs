#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(deprecated)]
mod cleaner;
mod health;
mod pages_mounts;
mod i18n;
mod fonts;
mod profile;
mod telemetry;
mod info;
mod pages_admin;
mod pages_clean;
mod hwinfo;
mod migration;
mod pages_guide;
mod pages_update;
mod pages_sec;
mod pages_sys;
mod ui;
mod util;
mod safe_optimize;
mod pages_appimage;
mod pages_containers;
mod pages_devcontainers;
mod pages_desktop_ext;
mod pages_sec_audit;
mod pages_timers;
mod timers_state;
mod pages_vpn;
mod pages_synaptic;
mod history;
mod pages_duplicates;
mod quarantine;
mod pages_services;
mod snap;
mod pages_report;
mod pages_disk_analyzer;
mod pages_disk_treemap;
mod pages_boot;
mod pages_kernel_boot;
mod solutions;
mod pages_menu_editor;
mod security_hardening;
mod hardware_health;
mod app_lifecycle;
mod update_check;
mod pages_themes;
mod pages_system_editor;
mod progress;
mod restore_points;
mod restore_history;
// Alcune funzioni del modulo sono entry point già coperte dai test e saranno
// collegate a nuove azioni della UI: evita i warning di dead_code nella build
// normale senza nasconderli nella suite di test.
#[cfg_attr(not(test), allow(dead_code))]
mod config_parsers;
#[cfg(test)]
mod test_support;
use eframe::egui;
use i18n::{t, tf, tr};
use hwinfo::{RamModule, Section};
use pages_admin::{DriverProp, GroupRow, StartEntry};
use pages_clean::Found;
use pages_sys::{BigFile, NetStatus, Pkg, ProcRow};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use progress::{Progress, ProgressReporter};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcSortKey {
    Pid,
    User,
    Cpu,
    Mem,
    Cmd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppSortKey {
    Name,
    Version,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceSortKey {
    Name,
    State,
    Description,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySortKey {
    Date,
    Command,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Page {
    Dashboard,
    Cleaner,
    SafeOptimize,
    Fixer,
    Info,
    Disk,
    DiskAnalyzer,
    DiskTreemap,
    Procs,
    Apps,
    AppImages,
    Containers,
    Zram,
    Temps,
    Network,
    Kernel,
    Repos,
    Startup,
    Perms,
    Firewall,
    Backup,
    Log,
    Shredder,
    Settings,
    Guide,
    About,
    Updates,
    Timers,
    Synaptic,
    History,
    AdvancedCleaner,
    Duplicates,
    Quarantine,
    RestoreHistory,
    Services,
    Snaps,
    Report,
    Boot,
    MenuEditor,
    SecurityHardening,
    HardwareHealth,
    AppLifecycle,
    Themes,
    SystemEditor,
    TelemetryGuard,
    Mounts,
    DevContainers,
    KernelBoot,
}

/// Versione dell'app: fonte unica per GUI, pagine informazioni e CLI.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

const MENU: &[(&str, &[(Page, &str)])] = &[
    ("", &[(Page::Dashboard, "Dashboard")]),
    (
        "MANUTENZIONE",
        &[
            (Page::Cleaner, "Pulizia"),
            (Page::SafeOptimize, safe_optimize::TAB_TITLE),
            (Page::AdvancedCleaner, "Residui di sistema"),
            (Page::Duplicates, "File duplicati"),
            (Page::Quarantine, "Quarantena / Ripristino"),
            (Page::RestoreHistory, "Cronologia e ripristino"),
            (Page::Fixer, "Fixer"),
            (Page::Updates, "Aggiornamenti"),
            (Page::Timers, "Pianificatore"),
            (Page::History, "Storico APT"),
            (Page::Snaps, "Pacchetti Snap"),
            (Page::DevContainers, "Sviluppo & Container"),
        ],
    ),
    (
        "SISTEMA",
        &[
            (Page::Info, "Info hardware"),
            (Page::Disk, "Disco"),
            (Page::DiskAnalyzer, "Analizzatore spazio"),
            (Page::DiskTreemap, "Analizzatore Disco"),
            (Page::Procs, "Processi"),
            (Page::Apps, "App installate"),
            (Page::AppImages, "AppImage"),
            (Page::Containers, "Container"),
            (Page::Zram, "Ottimizza RAM"),
            (Page::Temps, "Temperature"),
            (Page::Network, "Rete"),
            (Page::Mounts, "Dischi e condivisioni"),
        ],
    ),
    (
        "AMMINISTRATORE",
        &[
            (Page::Kernel, "Kernel, Grub, Recovery"),
            (Page::KernelBoot, "Kernel & Boot"),
            (Page::Repos, "Repository e driver"),
            (Page::Startup, "Startup"),
            (Page::Boot, "Tempi di avvio"),
            (Page::Perms, "Permessi utente"),
            (Page::Services, "Servizi di sistema"),
            (Page::Synaptic, "Gestore Pacchetti"),
            (Page::Themes, "Temi e Font Utente"),
            (Page::MenuEditor, "Editor Menu"),
            (Page::SystemEditor, "Gestore File di Sistema"),
        ],
    ),
    (
        "SICUREZZA E DATI",
        &[
            (Page::Firewall, "Sicurezza e firewall"),
            (Page::SecurityHardening, "Hardening di sicurezza"),
            (Page::HardwareHealth, "Guardiano Predittivo"),
            (Page::AppLifecycle, "Pulizia Chirurgica"),
            (Page::Backup, "Backup"),
            (Page::Shredder, "File shredder"),
            (Page::Log, "Log debug"),
            (Page::Report, "Report diagnostica"),
            (Page::TelemetryGuard, "Privacy & Telemetria"),
            (Page::Settings, "Impostazioni"),
        ],
    ),
    ("AIUTO", &[
        (Page::Guide, "Guida"),
        (Page::About, "Informazioni"),
    ]),
];

pub enum Msg {
    Log(String),
    Busy(bool),
    Sizes(Vec<Option<u64>>),
    Checks(Vec<health::Check>),
    Out(&'static str, String),
    /// Elenco aggiornato delle sorgenti APT selezionabili nella gestione repository.
    RepoEntries(Vec<pages_admin::RepoEntry>),
    Net(NetStatus),
    Procs(Vec<ProcRow>),
    Pkgs(Vec<Pkg>),
    BigFiles(Vec<BigFile>),
    Kernels(Vec<(String, bool, bool)>),
    Startup(Vec<StartEntry>),
    Groups(Vec<GroupRow>),
    Gov(String, u32),
    Disks(Vec<(String, String)>),
    AdvDirs(Vec<Found>),
    AdvFiles(Vec<Found>),
    SystemEditorSaved(Result<String, String>),
    Drivers(Vec<(String, String)>, Vec<DriverProp>),
    Picked(Option<Vec<String>>),
    /// File scelti per l'installazione di font/temi: canale separato da
    /// `Picked`, che alimenta il File shredder.
    ThemesPicked(Option<Vec<String>>),
    /// Profili VPN riletti in background (mai a ogni frame della UI).
    Vpns(Vec<pages_vpn::VpnProfile>),
    UpdApt(Vec<String>, bool),
    UpdFlat(Vec<String>),
    FlatLoading,
    Live(&'static str, String, bool),
    Ram(Vec<(String, String)>, Vec<RamModule>, String),
    Gpus(Vec<Section>),
    Comps(Vec<Section>),
    SynapticPkgs(Vec<pages_synaptic::SynapticPkg>),
    RcPkgs(Vec<(String, bool)>),
    FlatpakUnused(Vec<(String, bool)>),
    OrphanConfigs(Vec<(std::path::PathBuf, bool)>),
    ConfigBackups(Vec<(std::path::PathBuf, bool)>),
    DisabledSnaps(Vec<(String, bool)>),
    TelemetryItems(Vec<telemetry::TelemetryItem>),
    Duplicates(Vec<pages_duplicates::DuplicateGroup>),
    Services(Vec<pages_services::SystemdService>),
    DiskAnalyzerItems(Vec<pages_disk_analyzer::DiskItem>),
    DiskAnalyzerPath(String),
    BootSummary(String),
    BootItems(Vec<pages_boot::BootService>),
    Themes(Vec<pages_themes::CustomItem>),
    OrphanKernels(Vec<(String, u64, bool)>),
    NetSockets(Vec<(String, String, String, String, String)>),
    GpgKeys(Vec<(String, String, String, String, String)>),
    Mounts(Vec<pages_mounts::MountEntry>),
    SlowDiag(Vec<SlowDiagFinding>),
    SafeOptimizeReport(safe_optimize::OptimizeReport),
    DiskDeepScan(pages_disk_analyzer::DeepDiskAnalysis),
    DiskSpaceComparison(pages_disk_analyzer::SpaceComparison),
    /// Scansione treemap completata (generation evita risultati di scansioni annullate).
    TreemapScanDone {
        path: PathBuf,
        scan: pages_disk_treemap::DirScan,
        generation: u64,
    },
    /// Un file è stato spostato/eliminato: invalida le scansioni che lo contenevano.
    TreemapInvalidate(PathBuf),
    TreemapPath(String),
    /// Snapshot dello stato di avanzamento inviato da un job in background.
    Progress(Progress),
    /// Report della scansione Sviluppo & Container.
    DevScan(pages_devcontainers::Report),
    /// Avanzamento locale della pagina Sviluppo & Container.
    DevProgress { fraction: f32, detail: String },
    /// Report della pagina Kernel & Boot.
    KernelBootScan(pages_kernel_boot::Report),
    /// Avanzamento locale della pagina Kernel & Boot.
    KernelBootProgress { fraction: f32, detail: String },
    /// Stato reale del timer utente (systemd + ultima esecuzione CLI).
    TimerStatus(timers_state::TimerStatus),
    /// Notifica della scheda Pianificatore (`true` = errore).
    TimerMessage(String, bool),
    /// Tag della release più recente trovata all'avvio della GUI (`None` = nessuna novità).
    UpdateAvailable(Option<String>),
    /// Elenco aggiornato dei file in quarantena.
    QuarantineItems(Vec<quarantine::Item>),
    /// Esito di un'operazione sulla quarantena (`true` = errore).
    QuarantineMessage(String, bool),
    /// Elenco aggiornato dei punti di ripristino (Timeshift + cartelle).
    RestorePoints(Vec<restore_points::Point>),
    /// Esito della scansione/rimozione dei punti di ripristino (`true` = errore).
    RestoreMessage(String, bool),
    /// Elenco unificato di backup APT, `.bak` e quarantena.
    RestoreHistoryItems(Vec<restore_history::Entry>, bool),
    /// Una voce è stata ripristinata o eliminata con successo: va rimossa dalla vista.
    RestoreHistoryRemoved(String),
    /// Esito di un'azione sulla cronologia unificata (`true` = errore).
    RestoreHistoryMessage(String, bool),
}

#[derive(Clone)]
pub struct Chan {
    tx: Sender<Msg>,
    ctx: egui::Context,
    /// Distingue i job `bg()` (indipendenti da `busy`) da quelli `spawn()`:
    /// le loro barre non vanno chiuse dal `Busy(false)` di altri job.
    background: bool,
}

impl Chan {
    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }
}

pub type Job = Box<dyn FnOnce(&Chan) + Send + 'static>;

/// Operazione in attesa nella coda FIFO. I job sono `FnOnce`, quindi vengono
/// spostati (non clonati) dalla coda al thread quando arriva il loro turno.
pub(crate) struct QueuedJob {
    label: String,
    job: Job,
}

/// Numero massimo di operazioni accodabili per evitare code infinite.
const MAX_QUEUED_JOBS: usize = 12;

/// Etichetta di una pagina, usata per etichettare i job accodati senza dover
/// modificare le decine di chiamanti di `spawn`.
fn page_title(page: Page) -> &'static str {
    for (_, entries) in MENU {
        for (p, name) in entries.iter() {
            if *p == page {
                return *name;
            }
        }
    }
    t("Operazione in corso...")
}

/// Etichetta della barra per i comandi eseguiti tramite `cmd_job`.
fn job_label(key: &'static str) -> &'static str {
    match key {
        "upd_apt" => t("Aggiornamento pacchetti APT in corso..."),
        "upd_flat" => t("Aggiornamento app Flatpak in corso..."),
        "fixer" => t("Applico la correzione..."),
        "adv_clean" => t("Pulizia dei residui in corso..."),
        "history" => t("Operazione APT in corso..."),
        "quarantine" => t("Operazioni sulla quarantena in corso..."),
        "restore_scan" => t("Ricerca dei punti di ripristino in corso..."),
        "restore_delete" => t("Rimozione dei punti di ripristino in corso..."),
        "services" => t("Operazione sui servizi in corso..."),
        "boot" => t("Operazione su GRUB e avvio in corso..."),
        "kernel_boot" => t("Rimozione kernel selezionati in corso..."),
        "snapshot" => t("Creazione snapshot di sicurezza..."),
        "mounts" => t("Operazione sui dischi in corso..."),
        "timers" => t("Configurazione della pianificazione in corso..."),
        "vpn" => t("Configurazione VPN in corso..."),
        _ => t("Operazione in corso..."),
    }
}

pub fn cmd_job(key: &'static str, script: String, privileged: bool) -> Job {
    Box::new(move |ch: &Chan| {
        let head: String = script.chars().take(90).collect::<String>().replace('\n', " ");
        ch.send(Msg::Log(format!("▶ {head}")));
        ch.send(Msg::Out(key, String::new()));
        let mut prog = ProgressReporter::start(ch, key, job_label(key));
        if privileged {
            let wait = t("Attendo l'autorizzazione di amministratore...").to_string();
            ch.send(Msg::Live(key, wait.clone(), false));
            prog.status(wait);
        }
        let mut all = String::new();
        let ok = util::run_stream(&script, privileged, |line, commit| {
            if commit {
                all.push_str(line);
                all.push('\n');
            }
            prog.track_line(line);
            ch.send(Msg::Live(key, line.to_string(), commit));
        });
        let mark = if ok { "✔" } else { "✘" };
        ch.send(Msg::Log(format!("{mark} {}", util::tail(all.trim(), 3).replace('\n', " | "))));
        prog.finish(if ok { t("Completato") } else { t("Operazione terminata con errori") });
    })
}

pub fn fix_job(cmd: String) -> Job {
    Box::new(move |ch: &Chan| {
        ch.send(Msg::Log("▶ Applico la correzione...".into()));
        let mut prog = ProgressReporter::start(ch, "fixer", t("Applico la correzione..."));
        let (mut ok, mut out) = util::run_shell(&cmd, true);
        let low_first = out.to_lowercase();
        if !ok && (low_first.contains("grub-pc") || low_first.contains("install_devices")) {
            ch.send(Msg::Log("▶ Rilevo il disco per configurare grub-pc senza terminale...".into()));
            prog.status(t("Rilevamento del disco per GRUB..."));
            let grub_fix = "export DEBIAN_FRONTEND=noninteractive; \
                ROOT_SRC=\"$(findmnt -no SOURCE / 2>/dev/null | head -n1)\"; \
                PK=\"$(lsblk -no pkname \"$ROOT_SRC\" 2>/dev/null | head -n1)\"; \
                DEV=\"\"; \
                if [ -n \"$PK\" ] && [ -b \"/dev/$PK\" ]; then DEV=\"/dev/$PK\"; fi; \
                if [ -z \"$DEV\" ]; then DEV=\"$(lsblk -dpno NAME,TYPE 2>/dev/null | awk '$2==\"disk\"{print $1; exit}')\"; fi; \
                if [ -n \"$DEV\" ]; then \
                  printf 'grub-pc grub-pc/install_devices multiselect %s\\ngrub-pc grub-pc/install_devices_empty boolean false\\n' \"$DEV\" | debconf-set-selections; \
                fi; \
                dpkg --configure -a; \
                apt-get -y -f install";
            let (ok2, out2) = util::run_shell(grub_fix, true);
            ok = ok2;
            out = format!(
                "{out}\n{}\n{out2}",
                tf("--- Riparazione automatica GRUB ({}) ---", &[&if ok2 { t("OK") } else { t("errore") }])
            );
        }
        let low = out.to_lowercase();
        let head = if ok {
            t("✔ Completato")
        } else if low.contains("grub-pc") || low.contains("install_devices") {
            t("✘ Errore configurazione GRUB: usa la riparazione GRUB dedicata o verifica il disco di avvio")
        } else if low.contains("not authorized") || low.contains("dismissed") || low.contains("permission denied") {
            t("✘ Privilegi di amministratore non concessi o annullati")
        } else {
            t("✘ Errore durante la correzione")
        };
        ch.send(Msg::Out("fixer", out.clone()));
        ch.send(Msg::Log(format!("{head}: {}", util::tail(out.trim(), 4).replace('\n', " | "))));
        prog.status(t("Verifica dello stato del sistema..."));
        ch.send(Msg::Checks(health::run_checks()));
        prog.finish(head);
    })
}

/// Job di pulizia con avanzamento passo per passo, riutilizzato dalle schede.
pub fn clean_and_scan_job(id: &'static str, kinds: Vec<cleaner::Kind>, with_checks: bool) -> Job {
    Box::new(move |ch: &Chan| {
        let mut prog = ProgressReporter::start(ch, id, t("Pulizia in corso..."));
        for line in cleaner::clean_all_with_progress(&kinds, |cur, total, label| {
            prog.phase(0.0, 0.6, cur, total, label)
        }) {
            ch.send(Msg::Log(line));
        }
        ch.send(Msg::Sizes(cleaner::scan_with(|cur, total, label| {
            prog.phase(0.6, 0.35, cur, total, label)
        })));
        if with_checks {
            prog.status(t("Verifica dello stato del sistema..."));
            ch.send(Msg::Checks(health::run_checks()));
        }
        prog.finish(t("Pulizia completata"));
    })
}

/// Job di sola analisi spazio con avanzamento visibile.
pub fn scan_sizes_job(id: &'static str, label: &'static str) -> Job {
    Box::new(move |ch: &Chan| {
        let mut prog = ProgressReporter::start(ch, id, label);
        ch.send(Msg::Sizes(cleaner::scan_with(|cur, total, name| {
            prog.step(cur, total, crate::i18n::tr(name))
        })));
        prog.finish(t("Analisi completata"));
    })
}

/// Scansione dei punti di ripristino con barra di avanzamento.
pub fn restore_scan_job() -> Job {
    Box::new(move |ch: &Chan| {
        ch.send(Msg::Out("restore", String::new()));
        ch.send(Msg::RestoreMessage(
            t("Scansione dei punti di ripristino...").to_string(),
            false,
        ));
        let mut prog = ProgressReporter::start(
            ch,
            "restore_scan",
            t("Ricerca dei punti di ripristino in corso..."),
        );
        let scan = restore_points::scan(&mut |fraction, detail| prog.set(fraction, detail));
        let count = scan.points.len();
        ch.send(Msg::Log(tf("Scansione punti di ripristino: {} trovati.", &[&count])));
        ch.send(Msg::RestorePoints(scan.points));
        let msg = if count == 0 {
            t("Nessun punto di ripristino trovato.").to_string()
        } else {
            tf("Trovati {} punti di ripristino.", &[&count])
        };
        ch.send(Msg::RestoreMessage(msg, false));
        prog.finish(t("Scansione completata"));
    })
}

/// Rimozione (confermata) dei punti di ripristino selezionati o di tutti.
/// Usa un unico `pkexec` per tutta l'operazione e aggiorna l'elenco a fine job.
pub fn restore_delete_job(points: Vec<restore_points::Point>) -> Job {
    Box::new(move |ch: &Chan| {
        let total = points.len();
        ch.send(Msg::Out("restore", String::new()));
        let mut prog = ProgressReporter::start(
            ch,
            "restore_delete",
            t("Rimozione dei punti di ripristino in corso..."),
        );
        prog.status(t("Attendo l'autorizzazione di amministratore..."));
        ch.send(Msg::Log("▶ timeshift --delete / rimozione cartelle snapshot".into()));
        let script = restore_points::delete_script(&points);
        let mut all = String::new();
        let ok = util::run_stream(&script, true, |line, commit| {
            if commit {
                all.push_str(line);
                all.push('\n');
            }
            let trimmed = line.trim();
            if let Some(index) = trimmed.strip_prefix("DU_RP_").and_then(|s| s.parse::<usize>().ok()) {
                if index < total {
                    let name = points[index].name.clone();
                    prog.step(index + 1, total, tf("Rimozione: {}", &[&name]));
                }
            } else if trimmed != "DU_RP_DONE" {
                prog.track_line(line);
            }
            // I marcatori interni servono solo all'avanzamento: non sporcano
            // il pannello di output visibile all'utente.
            if !trimmed.starts_with("DU_RP_") {
                ch.send(Msg::Live("restore", line.to_string(), commit));
            }
        });
        let mark = if ok { "✔" } else { "✘" };
        ch.send(Msg::Log(format!(
            "{mark} {}",
            util::tail(all.trim(), 3).replace('\n', " | ")
        )));
        ch.send(Msg::Out("restore", all));

        prog.set(0.8, t("Aggiornamento dell'elenco..."));
        let scan = restore_points::scan(&mut |fraction, detail| {
            prog.set(0.8 + 0.2 * fraction, detail)
        });
        ch.send(Msg::RestorePoints(scan.points));
        let msg = if ok {
            t("Punti di ripristino rimossi. Elenco aggiornato.")
        } else {
            t("Rimozione terminata con errori: controlla l'output.")
        };
        ch.send(Msg::RestoreMessage(msg.to_string(), !ok));
        prog.finish(if ok { t("Rimozione completata") } else { t("Operazione terminata con errori") });
    })
}

pub struct App {
    page: Page,
    sub: usize,
    items: Vec<cleaner::CleanItem>,
    safe_preview: Vec<safe_optimize::PreviewStep>,
    safe_log: Vec<String>,
    checks: Vec<health::Check>,
    info: info::SysInfo,
    log: String,
    /// Inizio dei messaggi della scheda corrente: la barra di stato ignorava
    /// i messaggi vecchi, così cambiando pagina non restano scritte come
    /// "Treemap aggiornata." di un'altra sezione.
    status_from: usize,
    log_pause: bool,
    log_frozen: String,
    busy: bool,
    /// Avanzamento dei job in corso, indicizzato dall'id usato dal job.
    /// Più operazioni parallele (es. APT e Flatpak) hanno una barra ciascuna.
    progress: HashMap<&'static str, Progress>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    last_refresh: Instant,
    cpu_hist: VecDeque<f32>,
    ram_hist: VecDeque<f32>,
    out: HashMap<&'static str, String>,
    confirm: Option<(String, Job)>,
    /// Coda FIFO delle operazioni lunghe: una alla volta, nell'ordine di richiesta.
    job_queue: VecDeque<QueuedJob>,
    net: Option<NetStatus>,
    net_sockets: Vec<(String, String, String, String, String)>,
    gpg_keys: Vec<(String, String, String, String, String)>,
    net_socket_filter: String,
    procs: Vec<ProcRow>,
    proc_filter: String,
    proc_sort: ui::SortState<ProcSortKey>,
    proc_selected: Vec<String>,
    proc_pending: bool,
    last_proc: Instant,
    pkgs: Vec<Pkg>,
    pkg_filter: String,
    pkg_sort: ui::SortState<AppSortKey>,
    pkg_selected: Vec<String>,
    bigfiles: Vec<BigFile>,
    min_mb: u32,
    min_days: u32,
    disks_list: Vec<(String, String)>,
    adv_hidden: bool,
    adv_dirs: Vec<Found>,
    adv_files: Vec<Found>,
    svc_name: String,
    log_root: bool,
    log_filter: String,
    journal_max_mb: u32,
    grub_preview: String,
    disk_sel: String,
    path_in: String,
    stress_secs: u32,
    stress_mb: u32,
    zram_mb: u32,
    kernels: Vec<(String, bool, bool)>,
    orphan_kernels: Vec<(String, u64, bool)>,
    governor: String,
    swappiness: u32,
    grub_line: String,
    repo_line: String,
    repo_entries: Vec<pages_admin::RepoEntry>,
    drv_hw: Vec<(String, String)>,
    drv: Vec<DriverProp>,
    startup: Vec<StartEntry>,
    groups: Vec<GroupRow>,
    backup_src: String,
    backup_dst: String,
    snap_name: String,
    /// Punti di ripristino rilevati nella sottoscheda Backup.
    restore_points: Vec<restore_points::Point>,
    /// Id dei punti selezionati nella tabella (selezione singola o multipla).
    restore_selected: Vec<String>,
    restore_msg: Option<(String, bool)>,
    shred_path: String,
    shred_passes: u32,
    cfg: pages_sec::Config,
    guide_filter: String,
    upd_apt: Vec<String>,
    upd_flat: Vec<String>,
    upd_checked: bool,
    upd_flat_loading: bool,
    live: HashMap<&'static str, String>,
    ram_modules: Vec<RamModule>,
    ram_arr: Vec<(String, String)>,
    ram_msg: String,
    gpus: Vec<Section>,
    comps: Vec<Section>,
    synaptic_pkgs: Vec<pages_synaptic::SynapticPkg>,
    synaptic_selected: Option<usize>,
    synaptic_filter_mode: usize,
    synaptic_details: String,
    show_quit: bool,
    allow_close: bool,
    rc_pkgs: Vec<(String, bool)>,
    flatpak_unused: Vec<(String, bool)>,
    orphan_configs: Vec<(std::path::PathBuf, bool)>,
    config_backups: Vec<(std::path::PathBuf, bool)>,
    disabled_snaps: Vec<(String, bool)>,
    telemetry_items: Vec<telemetry::TelemetryItem>,
    dup_dir: String,
    dup_min_kb: u64,
    dup_skip_hidden: bool,
    dup_groups: Vec<pages_duplicates::DuplicateGroup>,
    quarantine_items: Vec<quarantine::Item>,
    quarantine_msg: Option<(String, bool)>,
    /// Stato della cronologia unificata (backup APT, `.bak`, quarantena).
    restore_history_entries: Vec<restore_history::Entry>,
    restore_history_selected: Option<String>,
    restore_history_filter: String,
    restore_history_kind: usize,
    restore_history_sort: ui::SortState<restore_history::HistorySortKey>,
    restore_history_msg: Option<(String, bool)>,
    restore_history_restricted: bool,
    apt_history: Vec<history::AptTransaction>,
    history_filter: String,
    history_sort: ui::SortState<HistorySortKey>,
    services_list: Vec<pages_services::SystemdService>,
    svc_filter: String,
    svc_sort: ui::SortState<ServiceSortKey>,
    svc_selected: Vec<String>,
    report_opts: pages_report::ReportOptions,
    report_path: String,
    disk_analyzer_path: String,
    disk_analyzer_items: Vec<pages_disk_analyzer::DiskItem>,
    pub boot_summary: String,
    pub boot_items: Vec<pages_boot::BootService>,
    pub boot_tab: usize,
    pub autostart_items: Vec<pages_boot::AutostartItem>,
    pub new_autostart_name: String,
    pub new_autostart_exec: String,
    pub new_autostart_delay: String,
    custom_menu_items: Vec<pages_menu_editor::MenuItemCustom>,
    menu_category: String,
    menu_filter: String,
    menu_editing_id: Option<String>,
    pub new_menu_name: String,
    pub new_menu_exec: String,
    pub new_menu_icon: String,
    pub new_menu_category: String,
    pub app_lifecycle_query: String,
    pub app_lifecycle_residuals: Vec<app_lifecycle::ResidualArtifact>,
    pub theme_items: Vec<pages_themes::CustomItem>,
    vpn_list: Vec<pages_vpn::VpnProfile>,
    pub system_editor: pages_system_editor::SystemFileEditor,
    pub(crate) apt_audit_file: String,
    pub(crate) apt_audit_pattern: String,
    pub mounts_state: pages_mounts::MountsState,
    slow_diag: Vec<SlowDiagFinding>,
    slow_diag_ran: bool,
    safe_report: Option<safe_optimize::OptimizeReport>,
    disk_deep: pages_disk_analyzer::DeepDiskAnalysis,
    disk_before_free: Option<u64>,
    /// Percorso esatto usato per la misura «prima», per confrontare sempre lo stesso disco.
    disk_before_path: Option<String>,
    disk_comparison: Option<pages_disk_analyzer::SpaceComparison>,
    // Stato della treemap: una scansione per cartella, con navigazione breadcrumb.
    treemap_input: String,
    treemap_current: PathBuf,
    treemap_scans: HashMap<PathBuf, pages_disk_treemap::DirScan>,
    treemap_selected: Option<PathBuf>,
    treemap_cancel: Arc<AtomicBool>,
    treemap_generation: u64,
    treemap_pending_scan: Option<PathBuf>,
    /// Stato della sezione Sviluppo & Container (scansione e selezioni).
    dev_report: pages_devcontainers::Report,
    dev_progress: Option<(f32, String)>,
    /// Stato della sezione Kernel & Boot.
    kb_report: pages_kernel_boot::Report,
    kb_progress: Option<(f32, String)>,
    /// Strumenti di snapshot rilevati e preferenza dell'utente.
    snapshot_timeshift: bool,
    snapshot_snapper: bool,
    snapshot_enabled: bool,
    /// Stato della pagina Pianificatore.
    timer_freq: usize,
    timer_includes: Vec<String>,
    timer_status: Option<timers_state::TimerStatus>,
    timer_msg: Option<(String, bool)>,
    timer_loaded: bool,
    /// Versione più recente segnalata dal controllo all'avvio (solo GUI).
    update_available: Option<String>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = pages_sec::Config::load();
        fonts::install(&cc.egui_ctx);
        ui::apply_theme(&cc.egui_ctx, cfg.dark_mode);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let (tx, rx) = channel();
        let home = util::home();
        i18n::set_lang(cfg.lang);
        let quick = cfg.quick_clean;
        let check_updates = cfg.check_updates;
        let timer_schedule = timers_state::Schedule::load().unwrap_or_default();
        // Rilevamento degli strumenti di snapshot: nessun errore se assenti.
        let snapshot_timeshift = util::run_shell("command -v timeshift >/dev/null 2>&1", false).0;
        let snapshot_snapper = util::run_shell("command -v snapper >/dev/null 2>&1", false).0;
        let mut app = App {
            page: Page::Dashboard,
            sub: 0,
            items: cleaner::items(),
            safe_preview: safe_optimize::preview_steps(),
            safe_log: Vec::new(),
            checks: Vec::new(),
            info: info::SysInfo::new(),
            log: String::new(),
            status_from: 0,
            log_pause: false,
            log_frozen: String::new(),
            busy: false,
            progress: HashMap::new(),
            tx,
            rx,
            last_refresh: Instant::now(),
            cpu_hist: VecDeque::new(),
            ram_hist: VecDeque::new(),
            out: HashMap::new(),
            confirm: None,
            job_queue: VecDeque::new(),
            net: None,
            net_sockets: Vec::new(),
            gpg_keys: Vec::new(),
            net_socket_filter: String::new(),
            procs: Vec::new(),
            proc_filter: String::new(),
            proc_sort: ui::SortState::new(ProcSortKey::Cpu, ui::SortDir::Desc),
            proc_selected: Vec::new(),
            proc_pending: false,
            last_proc: Instant::now().checked_sub(Duration::from_secs(10)).unwrap_or_else(Instant::now),
            pkgs: Vec::new(),
            pkg_filter: String::new(),
            pkg_sort: ui::SortState::new(AppSortKey::Name, ui::SortDir::Asc),
            pkg_selected: Vec::new(),
            bigfiles: Vec::new(),
            min_mb: 100,
            min_days: 90,
            disks_list: Vec::new(),
            adv_hidden: false,
            adv_dirs: Vec::new(),
            adv_files: Vec::new(),
            svc_name: "NetworkManager".into(),
            log_root: true,
            log_filter: String::new(),
            journal_max_mb: 500,
            grub_preview: String::new(),
            disk_sel: String::new(),
            path_in: "/".into(),
            stress_secs: 30,
            stress_mb: 256,
            zram_mb: 0,
            kernels: Vec::new(),
            orphan_kernels: Vec::new(),
            governor: String::new(),
            swappiness: 60,
            grub_line: String::new(),
            repo_line: String::new(),
            repo_entries: Vec::new(),
            drv_hw: Vec::new(),
            drv: Vec::new(),
            startup: Vec::new(),
            groups: Vec::new(),
            backup_src: format!("{home}/Documenti"),
            backup_dst: format!("{home}/Backup-DebUtility"),
            snap_name: String::new(),
            restore_points: Vec::new(),
            restore_selected: Vec::new(),
            restore_msg: None,
            shred_path: String::new(),
            shred_passes: 3,
            cfg,
            guide_filter: String::new(),
            upd_apt: Vec::new(),
            upd_flat: Vec::new(),
            upd_checked: false,
            upd_flat_loading: false,
            live: HashMap::new(),
            ram_modules: Vec::new(),
            ram_arr: Vec::new(),
            ram_msg: String::new(),
            gpus: Vec::new(),
            comps: Vec::new(),
            show_quit: false,
            allow_close: false,
            synaptic_pkgs: Vec::new(),
            synaptic_selected: None,
            synaptic_filter_mode: 0,
            synaptic_details: String::new(),
            rc_pkgs: Vec::new(),
            telemetry_items: Vec::new(),
            flatpak_unused: Vec::new(),
            orphan_configs: Vec::new(),
            config_backups: Vec::new(),
            disabled_snaps: Vec::new(),
            dup_dir: home.clone(),
            dup_min_kb: 100,
            dup_skip_hidden: true,
            dup_groups: Vec::new(),
            quarantine_items: quarantine::load().unwrap_or_default(),
            quarantine_msg: None,
            restore_history_entries: Vec::new(),
            restore_history_selected: None,
            restore_history_filter: String::new(),
            restore_history_kind: 0,
            restore_history_sort: ui::SortState::new(
                restore_history::HistorySortKey::Date,
                ui::SortDir::Desc,
            ),
            restore_history_msg: None,
            restore_history_restricted: false,
            apt_history: Vec::new(),
            history_filter: String::new(),
            history_sort: ui::SortState::new(HistorySortKey::Date, ui::SortDir::Desc),
            services_list: Vec::new(),
            svc_filter: String::new(),
            svc_sort: ui::SortState::new(ServiceSortKey::Name, ui::SortDir::Asc),
            svc_selected: Vec::new(),
            report_opts: pages_report::ReportOptions::default(),
            report_path: format!("{home}/debutility-report.html"),
            disk_analyzer_path: home.clone(),
            disk_analyzer_items: Vec::new(),
            boot_summary: String::new(),
            boot_items: Vec::new(),
            boot_tab: 0,
            autostart_items: Vec::new(),
            new_autostart_name: String::new(),
            new_autostart_exec: String::new(),
            new_autostart_delay: String::new(),
            custom_menu_items: Vec::new(),
            menu_category: "Tutti".to_string(),
            menu_filter: String::new(),
            menu_editing_id: None,
            new_menu_name: String::new(),
            new_menu_exec: String::new(),
            new_menu_icon: String::new(),
            new_menu_category: String::new(),
            app_lifecycle_query: String::new(),
            app_lifecycle_residuals: Vec::new(),
            theme_items: Vec::new(),
            vpn_list: Vec::new(),
            system_editor: pages_system_editor::SystemFileEditor::new(),  
            apt_audit_file: String::new(),
            apt_audit_pattern: String::new(),
            mounts_state: pages_mounts::MountsState::default(),  
            slow_diag: Vec::new(),
            slow_diag_ran: false,   
            safe_report: None, 
            disk_deep: pages_disk_analyzer::DeepDiskAnalysis::default(),
            disk_before_free: None,
            disk_before_path: None,
            disk_comparison: None,
            treemap_input: home.clone(),
            treemap_current: PathBuf::from(&home),
            treemap_scans: HashMap::new(),
            treemap_selected: None,
            treemap_cancel: Arc::new(AtomicBool::new(false)),
            treemap_generation: 0,
            treemap_pending_scan: None,
            dev_report: pages_devcontainers::Report::default(),
            dev_progress: None,
            kb_report: pages_kernel_boot::Report::default(),
            kb_progress: None,
            snapshot_timeshift,
            snapshot_snapper,
            snapshot_enabled: false,
            timer_freq: timer_schedule.frequency_index(),
            timer_includes: timer_schedule.include,
            timer_status: None,
            timer_msg: None,
            timer_loaded: false,
            update_available: None,
        };
        app.system_editor.load_file("/etc/fstab");
        let ctx = cc.egui_ctx.clone();
        app.spawn(&ctx, move |ch| {
            let mut prog = ProgressReporter::start(&ch, "startup", t("Avvio e controllo iniziale..."));
            // Scadenza automatica: vale anche se l'utente non apre mai la scheda.
            match quarantine::purge_expired() {
                Ok(n) if n > 0 => {
                    ch.send(Msg::QuarantineItems(quarantine::load().unwrap_or_default()));
                    ch.send(Msg::Log(tf("Eliminati {} file scaduti dalla quarantena.", &[&n])));
                }
                Ok(_) => {}
                Err(e) => ch.send(Msg::Log(tf("Errore nella scadenza della quarantena: {}", &[&e]))),
            }
            if quick {
                for l in cleaner::clean_all_with_progress(&[cleaner::Kind::UserCache], |c, t, l| {
                    prog.phase(0.0, 0.6, c, t, l)
                }) {
                    ch.send(Msg::Log(l));
                }
            }
            ch.send(Msg::Sizes(cleaner::scan_with(|c, t, l| {
                prog.phase(0.6, 0.35, c, t, crate::i18n::tr(l))
            })));
            prog.status(t("Verifica dello stato del sistema..."));
            ch.send(Msg::Checks(health::run_checks()));
            prog.status(t("Test della connessione di rete..."));
            ch.send(Msg::Net(pages_sys::net_test()));
            prog.finish(t("Controlli iniziali completati"));
        });
        app.load_icewm_menu();
        // Controllo aggiornamenti: solo qui (costruzione della GUI), mai nella
        // modalità CLI/headless gestita da `cli::handle` in `main`.
        if check_updates {
            let check_ctx = ctx.clone();
            app.bg(&check_ctx, |ch| {
                // Nessun log in caso di errore: rete assente o API non valida
                // vengono ignorati in silenzio.
                ch.send(Msg::UpdateAvailable(update_check::check_available_update()));
            });
        }
        app
    }
    
    fn ui_telemetry_guard(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Privacy & Telemetry Guard"));
        ui::hint(
            ui,
            t("Scansiona il sistema alla ricerca di agenti di telemetria, tracker o demoni di diagnostica invisibili, permettendoti di disattivarli o bloccarli con un clic."),
        );

        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui::primary_button(ui, t("Pulisci selezionati"), self.action_enabled()).clicked() {
                let kinds: Vec<cleaner::Kind> =
                    self.items.iter().filter(|i| i.selected).map(|i| i.kind).collect();
                let has_browser_data = kinds.contains(&cleaner::Kind::BrowserData);
                let job = clean_and_scan_job("cleaner", kinds, false);
                if has_browser_data {
                    self.confirm = Some((
                        t("Attenzione: hai selezionato 'Cronologia e cookie dei browser'. Verranno rimossi cronologia, cookie/sessioni e, su Firefox, anche i preferiti/segnalibri (le password salvate non vengono toccate). Vuoi continuare?")
                            .to_string(),
                        job,
                    ));
                } else {
                    self.spawn(ctx, job);
                }
            }
            
            if self.busy {
                ui.spinner();
            }
        });

        ui.add_space(5.0);

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Demoni di telemetria e diagnostica rilevati")).strong().size(16.0).color(ui::accent_text()));
            ui.add_space(3.0);

         if self.telemetry_items.is_empty() {
                ui.label(t("Nessun demone di telemetria noto attivo rilevato. (Premi 'Scansiona' per verificare)"));
            } else {
                let items_to_show = self.telemetry_items.clone(); // <--- Clona la lista per evitare conflitti di prestito
                for item in items_to_show {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(tr(&item.name)).strong().size(14.0));
                            ui.label(egui::RichText::new(tr(&item.description)).color(ui::text_dim()));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Disattiva e Blocca"))).clicked() {
                                let target = item.target.clone();
                                self.spawn(ctx, move |ch| {
                                    ch.send(Msg::Log(format!("▶ Disattivo il demone {}...", target)));
                                    let ok = telemetry::disable_service(&target);
                                    ch.send(Msg::TelemetryItems(telemetry::scan_telemetry_daemons()));
                                    if ok {
                                        ch.send(Msg::Log(format!("✔ Demone {} disattivato.", target)));
                                    } else {
                                        ch.send(Msg::Log(t("Operazione terminata con errori").to_string()));
                                    }
                                });
                            }
                        });
                    });
                    ui.add_space(3.0);
                }
            }
        });

        ui.add_space(5.0);

        ui::card(ui, |ui| {
            ui.label(
                egui::RichText::new(t("Blocco Rapido Domini di Telemetria (Hosts)"))
                    .strong()
                    .size(16.0)
                    .color(ui::accent_text()),
            );
            ui.label(t(
                "Impedisci al sistema di contattare server noti di telemetria di grandi produttori bloccandoli a livello locale.",
            ));
            ui.add_space(3.0);

            let telemetry_domains = vec![
                (t("Telemetry Microsoft (vortex.data.microsoft.com)"), "vortex.data.microsoft.com"),
                (t("Telemetria Errori Ubuntu (collector.ubuntu.com)"), "collector.ubuntu.com"),
                (t("Telemetria Nvidia (gfwsl.geforce.com)"), "gfwsl.geforce.com"),
            ];

            for (label_text, domain) in telemetry_domains {
                ui.horizontal(|ui| {
                    ui.label(label_text);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t("Blocca dominio")).clicked() {
                            let dom = domain.to_string();
                            self.spawn(ctx, move |ch| {
                                ch.send(Msg::Log(format!("▶ Blocco il dominio {}...", dom)));
                                if telemetry::block_domain(&dom) {
                                    ch.send(Msg::Log(format!("✔ Dominio {} bloccato con successo.", dom)));
                                } else {
                                    ch.send(Msg::Log(t("Operazione terminata con errori").to_string()));
                                }
                            });
                        }
                    });
                });
                ui.add_space(2.0);
            }
        });
    }

    pub fn spawn<F>(&mut self, ctx: &egui::Context, f: F)
    where
        F: FnOnce(&Chan) + Send + 'static,
    {
        if self.busy {
            // Invece di scartare l'azione, la mettiamo in coda: verrà avviata
            // da `drain` quando l'operazione corrente invierà `Busy(false)`.
            if self.job_queue.len() < MAX_QUEUED_JOBS {
                let label = t(page_title(self.page)).to_string();
                self.job_queue.push_back(QueuedJob { label, job: Box::new(f) });
                self.log.push_str(&tf("Operazione accodata: {}", &[&t(page_title(self.page))]));
                self.log.push('\n');
                ctx.request_repaint();
            } else {
                self.log.push_str(t("Coda operazioni piena: attendi il completamento di quelle in corso."));
                self.log.push('\n');
            }
            return;
        }
        self.busy = true;
        // Una nuova operazione sostituisce le barre chiuse della precedente;
        // i job `bg()` non usano `busy` e possono ancora essere in corso.
        self.progress.retain(|_, p| !p.done);
        let ch = Chan { tx: self.tx.clone(), ctx: ctx.clone(), background: false };
        std::thread::spawn(move || {
            // Se il job va in panic l'interfaccia non deve restare bloccata su "occupato"
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&ch)));
            if res.is_err() {
                ch.send(Msg::Log("✘ Errore interno: l'operazione si è interrotta in modo anomalo.".into()));
            }
            ch.send(Msg::Busy(false));
        });
    }

    /// I pulsanti delle operazioni restano azionabili mentre un altro job gira:
    /// l'azione viene semplicemente accodata e serializzata da `spawn`.
    pub(crate) fn action_enabled(&self) -> bool {
        self.job_queue.len() < MAX_QUEUED_JOBS
    }

    /// Annulla soltanto i job in attesa: quello in esecuzione continua fino
    /// alla fine, senza interruzioni brusche di processi di sistema.
    pub(crate) fn cancel_queued(&mut self) {
        let n = self.job_queue.len();
        self.job_queue.clear();
        if n > 0 {
            self.log.push_str(&tf("Operazioni in attesa annullate: {}", &[&n]));
            self.log.push('\n');
        }
    }

    pub fn bg<F>(&self, ctx: &egui::Context, f: F)
    where
        F: FnOnce(&Chan) + Send + 'static,
    {
        let ch = Chan { tx: self.tx.clone(), ctx: ctx.clone(), background: true };
        std::thread::spawn(move || f(&ch));
    }

    pub fn run_btn(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        label: &str,
        key: &'static str,
        script: &str,
        privileged: bool,
        confirm: Option<&str>,
    ) {
        if ui.add_enabled(self.action_enabled(), egui::Button::new(label)).clicked() {
            let job = cmd_job(key, script.to_string(), privileged);
            match confirm {
                Some(t) => self.confirm = Some((t.to_string(), job)),
                None => self.spawn(ctx, job),
            }
        }
    }

    /// Toggle condiviso dalle schermate di pulizia profonda.
    /// È attivo solo se `timeshift` o `snapper` è installato.
    pub(crate) fn snapshot_toggle(&mut self, ui: &mut egui::Ui) {
        let timeshift = self.snapshot_timeshift;
        let snapper = self.snapshot_snapper;
        let available = timeshift || snapper;
        let mut enabled = self.snapshot_enabled;
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                ui::icon_badge(ui.painter(), rect, ui::GOOD, "⛨", 15.0);
                ui.vertical(|ui| {
                    ui.add_enabled(
                        available,
                        egui::Checkbox::new(
                            &mut enabled,
                            egui::RichText::new(t("Crea snapshot di sicurezza prima di procedere")).strong(),
                        ),
                    );
                    let tool = if timeshift {
                        "Timeshift"
                    } else if snapper {
                        "Snapper"
                    } else {
                        ""
                    };
                    let detail = if available {
                        t("Verrà creato uno snapshot prima di ogni rimozione; se la creazione fallisce la pulizia viene annullata.")
                    } else {
                        t("Nessuno strumento di snapshot rilevato: installa Timeshift o Snapper per usare questa opzione.")
                    };
                    ui.label(
                        egui::RichText::new(if available {
                            format!("{tool} · {detail}")
                        } else {
                            detail.to_string()
                        })
                        .size(12.5)
                        .color(ui::text_dim()),
                    );
                });
            });
        });
        self.snapshot_enabled = enabled;
        ui.add_space(3.0);
    }

    /// Esegue lo snapshot (se richiesto) e solo in caso di successo avvia il job.
    pub(crate) fn with_snapshot(&self, follow: Job) -> Job {
        let enabled = self.snapshot_enabled && (self.snapshot_timeshift || self.snapshot_snapper);
        let timeshift = self.snapshot_timeshift;
        Box::new(move |ch: &Chan| {
            if !enabled {
                follow(ch);
                return;
            }
            let (tool, script) = if timeshift {
                (
                    "Timeshift",
                    format!("timeshift --create --comments {}", util::shq(t("DebUtility pre-pulizia"))),
                )
            } else {
                ("Snapper", format!("snapper create -d {}", util::shq(t("DebUtility pre-pulizia"))))
            };
            ch.send(Msg::Log(tf(
                "▶ {}: creazione snapshot di sicurezza...",
                &[&tool],
            )));
            let mut prog = ProgressReporter::start(
                ch,
                "snapshot",
                tf("Snapshot di sicurezza ({})...", &[&tool]),
            );
            prog.status(t("Attendo l'autorizzazione di amministratore..."));
            let mut out = String::new();
            let ok = util::run_stream(&script, true, |line, commit| {
                if commit {
                    out.push_str(line);
                    out.push('\n');
                    if !line.trim().is_empty() {
                        ch.send(Msg::Log(line.to_string()));
                    }
                }
                prog.track_line(line);
            });
            if !ok {
                let tail = util::tail(out.trim(), 3).replace('\n', " | ");
                ch.send(Msg::Log(tf(
                    "✘ Snapshot non riuscito ({}): pulizia annullata.",
                    &[&tail],
                )));
                prog.finish(t("Snapshot non riuscito: operazione annullata"));
                return;
            }
            ch.send(Msg::Log(tf(
                "✔ Snapshot di sicurezza creato con {}.",
                &[&tool],
            )));
            prog.finish(t("Snapshot creato"));
            follow(ch);
        })
    }

    /// Avvia un job di pulizia applicando lo snapshot se selezionato.
    pub(crate) fn spawn_snapshot(&mut self, ctx: &egui::Context, follow: Job) {
        let job = self.with_snapshot(follow);
        self.spawn(ctx, job);
    }

    /// Mette in coda un job di pulizia con conferma e snapshot opzionale.
    pub(crate) fn confirm_snapshot(&mut self, text: String, follow: Job) {
        let job = self.with_snapshot(follow);
        self.confirm = Some((text, job));
    }

    /// Variante di [`App::run_btn`] che applica lo snapshot opzionale.
    pub(crate) fn run_btn_snapshot(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        label: &str,
        key: &'static str,
        script: &str,
        privileged: bool,
        confirm: Option<&str>,
    ) {
        let job = self.with_snapshot(cmd_job(key, script.to_string(), privileged));
        if ui.add_enabled(self.action_enabled(), egui::Button::new(label)).clicked() {
            match confirm {
                Some(t) => self.confirm = Some((t.to_string(), job)),
                None => self.spawn(ctx, job),
            }
        }
    }

    pub fn show_out(&self, ui: &mut egui::Ui, key: &str, height: f32) {
        let mut text = self.out.get(key).cloned().unwrap_or_default();
        if let Some(cur) = self.live.get(key) {
            text.push_str(cur);
        }
        ui::output_box(ui, &text, height);
    }

    pub fn show_out_follow(&self, ui: &mut egui::Ui, key: &str, height: f32) {
        let mut text = self.out.get(key).cloned().unwrap_or_default();
        if let Some(cur) = self.live.get(key) {
            text.push_str(cur);
        }
        ui::output_box_follow(ui, &text, height);
    }

    /// Barre di avanzamento complete (una per ogni job parallelo attivo).
    /// Viene chiamata in cima al pannello centrale: resta visibile da qualsiasi
    /// scheda mentre gira un'operazione lunga.
    pub(crate) fn progress_ui(&self, ui: &mut egui::Ui) {
        let mut active: Vec<&Progress> = self.progress.values().filter(|p| p.visible()).collect();
        if active.is_empty() {
            return;
        }
        active.sort_by_key(|p| p.started);
        for p in active {
            ui::card_tinted(ui, ui::ACCENT, |ui| {
                ui.horizontal(|ui| {
                    let (badge, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                    ui::icon_badge(ui.painter(), badge, ui::ACCENT, "⟳", 16.0);
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(tr(&p.label)).strong().size(16.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(pct) = p.percent() {
                            ui.label(egui::RichText::new(format!("{pct}%")).strong().size(17.0).color(ui::accent_text()));
                        }
                        let secs = p.elapsed().as_secs();
                        ui.label(egui::RichText::new(format!("{}s", secs)).color(ui::text_dim()).size(12.0));
                    });
                });
                ui.add_space(2.0);
                let width = (ui.available_width() - 4.0).max(120.0);
                let bar = ui::progress_bar(p.fraction.unwrap_or(0.0))
                    .desired_width(width)
                    .animate(true);
                let bar = if p.fraction.is_some() {
                    bar.show_percentage()
                } else {
                    bar.text(t("In corso..."))
                };
                ui.add(bar);
                if !p.detail.is_empty() {
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(tr(&p.detail)).color(ui::text_dim()).size(12.5));
                }
            });
            ui.add_space(3.0);
        }
    }

    /// Versione compatta nella barra di stato in basso.
    /// Restituisce true se almeno un job è ancora attivo: in quel caso lo
    /// spinner non serve, anche se una barra completata è ancora visibile.
    pub(crate) fn progress_status_ui(&self, ui: &mut egui::Ui) -> bool {
        let mut active: Vec<&Progress> = self.progress.values().filter(|p| p.visible()).collect();
        if active.is_empty() {
            return false;
        }
        active.sort_by_key(|p| p.started);
        let labels = active
            .iter()
            .map(|p| p.label.as_str())
            .collect::<Vec<_>>()
            .join(" + ");
        // La barra compatta segue un job ancora attivo; solo se non ce ne sono
        // mostra l'ultima barra completata ancora visibile.
        let fraction = active
            .iter()
            .find(|p| !p.done)
            .and_then(|p| p.fraction)
            .or_else(|| active.first().and_then(|p| p.fraction));
        let width = (ui.available_width() * 0.22).clamp(80.0, 300.0).min(ui.available_width().max(60.0));
        let bar = ui::progress_bar(fraction.unwrap_or(0.0))
            .desired_width(width)
            .desired_height(12.0)
            .animate(true);
        let bar = if fraction.is_some() {
            bar.show_percentage()
        } else {
            bar.text(t("In corso..."))
        };
        ui.add(bar);
        ui.label(egui::RichText::new(tr(&labels)).color(ui::text_dim()).size(12.0));
        active.iter().any(|p| !p.done)
    }

    /// Pannello discreto della coda: compare solo con operazioni attive o in
    /// attesa. I pending possono essere annullati senza toccare il job in corso.
    fn queue_ui(&mut self, ctx: &egui::Context) {
        if !self.busy && self.job_queue.is_empty() {
            return;
        }
        let running: Vec<String> = self
            .progress
            .values()
            .filter(|p| p.visible() && !p.done)
            .map(|p| tr(&p.label))
            .collect();
        let pending = self.job_queue.len();
        if running.is_empty() && pending == 0 {
            return;
        }
        let preview: Vec<String> = self.job_queue.iter().take(2).map(|q| q.label.clone()).collect();
        let mut cancel = false;
        let bg = ui::sidebar_bg(ctx);
        egui::TopBottomPanel::bottom("queue")
            .frame(
                egui::Frame::default()
                    .fill(bg)
                    .inner_margin(egui::Margin::symmetric(10.0, 4.0))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        if ui::is_dark() { egui::Color32::from_white_alpha(10) } else { egui::Color32::from_rgb(216, 224, 233) },
                    )),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.label(
                        egui::RichText::new(t("Coda operazioni"))
                            .strong()
                            .size(12.5)
                            .color(ui::accent_text()),
                    );
                    let current = if running.is_empty() { t("In corso").to_string() } else { running.join(" + ") };
                    ui.label(egui::RichText::new(tf("In corso: {}", &[&current])).size(12.0));
                    if pending > 0 {
                        ui.label(
                            egui::RichText::new(tf("In attesa: {}", &[&pending]))
                                .size(12.0)
                                .color(ui::text_dim()),
                        );
                        if !preview.is_empty() {
                            ui.label(egui::RichText::new(preview.join(", ")).size(11.5).color(ui::text_dim()));
                        }
                        if ui.button(t("Annulla le operazioni in attesa")).clicked() {
                            cancel = true;
                        }
                    }
                });
            });
        if cancel {
            self.cancel_queued();
        }
    }

    /// `true` mentre gira un'operazione con avanzamento (anche in background).
    /// Usato per bloccare il menu laterale: cambiare scheda a metà job può
    /// nascondere lo stato o avviare un'azione sovrapposta.
    pub(crate) fn operation_in_progress(&self) -> bool {
        self.busy
            || self.progress.values().any(|p| !p.done && p.visible())
            || self.dev_progress.is_some()
            || self.kb_progress.is_some()
    }

    fn enter(&mut self, ctx: &egui::Context, p: Page) {
        self.page = p;
        self.sub = 0;
        // La barra di stato riparte da "Pronto" a ogni cambio di scheda.
        self.status_from = self.log.len();
        match p {
            Page::Apps => self.load_pkgs(ctx),
            Page::Kernel => {
                self.load_kernels(ctx);
                self.grub_line = pages_admin::read_grub_line();
            }
            Page::Startup => self.startup = pages_admin::read_startup(),
            Page::Perms => self.load_groups(ctx),
            Page::Network => {
                self.info.reload_static();
                self.bg(ctx, |ch| ch.send(Msg::Net(pages_sys::net_test())));
                self.load_vpn(ctx);
            }
            Page::Info => {
                self.info.reload_static();
                self.load_hw(ctx);
            }
            Page::Cleaner => {
                if let Some(v) = std::fs::read_to_string("/proc/sys/vm/swappiness").ok().and_then(|x| x.trim().parse().ok()) {
                    self.swappiness = v;
                }
            }
            Page::Disk => self.load_disks(ctx),
            Page::DiskAnalyzer => {
                if self.disk_analyzer_items.is_empty() {
                    let p = self.disk_analyzer_path.clone();
                    self.spawn(ctx, move |ch| {
                        let mut prog = ProgressReporter::start(
                            &ch,
                            "disk_analyzer",
                            t("Scansione spazio disco in corso..."),
                        );
                        ch.send(Msg::Log("▶ Scansione spazio disco in corso...".into()));
                        let items = pages_disk_analyzer::scan_directory_with(&p, |i, name| {
                            prog.status(tf("{} — {} elementi", &[&name, &i]))
                        });
                        ch.send(Msg::DiskAnalyzerItems(items));
                        prog.finish(t("Scansione completata"));
                    });
                }
            }
            Page::DiskTreemap => {
                let p = self
                    .treemap_pending_scan
                    .take()
                    .unwrap_or_else(|| self.treemap_current.clone());
                if !self.treemap_scans.contains_key(&p) {
                    self.treemap_scan(ctx, p);
                }
            }
            Page::Updates => self.load_updates_cache(ctx),
            Page::Repos => self.load_sources(ctx),
            Page::History => {
                match history::load_apt_history() {
                    Ok(h) => self.apt_history = h,
                    Err(e) => self.log.push_str(&format!("Errore caricamento history APT: {e}\n")),
                }
            }
            Page::Quarantine => {
                // La purga può invocare pkexec: mai in modo sincrono sul thread
                // della UI e mai ignorando gli errori come prima.
                self.refresh_quarantine();
                self.spawn(ctx, |ch: &Chan| {
                    let prog = ProgressReporter::start(
                        ch,
                        "quarantine",
                        t("Pulizia della quarantena in corso..."),
                    );
                    match quarantine::purge_expired() {
                        Ok(n) => ch.send(Msg::QuarantineMessage(
                            tf("Eliminati {} elementi scaduti.", &[&n]),
                            false,
                        )),
                        Err(e) => ch.send(Msg::QuarantineMessage(e, true)),
                    }
                    match quarantine::load() {
                        Ok(items) => ch.send(Msg::QuarantineItems(items)),
                        Err(e) => ch.send(Msg::Log(format!("✘ {e}"))),
                    }
                    prog.finish(t("Pulizia della quarantena completata"));
                });
            }
            Page::RestoreHistory => self.refresh_restore_history(ctx, false),
            Page::SystemEditor => {
                if self.system_editor.content.is_empty() {
                    self.system_editor.load_file("/etc/fstab");
                }
            }  
            Page::Mounts => self.load_mounts(ctx),
            Page::DevContainers => {
                if self.dev_report.categories.is_empty() {
                    self.dev_scan(ctx);
                }
            }
            Page::KernelBoot => {
                if self.kb_report.kernels.is_empty() && self.kb_report.total == 0 {
                    self.kb_scan(ctx);
                }
            }
            Page::Timers => {
                // Ricarica la configurazione dal disco e lo stato da systemd.
                match timers_state::Schedule::load() {
                    Ok(s) => {
                        self.timer_freq = s.frequency_index();
                        self.timer_includes = s.include;
                    }
                    Err(e) => self.timer_msg = Some((e, true)),
                }
                self.timer_loaded = false;
            }
            _ => {}
        }
    }

    fn clean_basic(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::hint(
            ui,
            "Qui elimini i file che non servono più (cache, file temporanei, cestino...) per liberare spazio. Spunta le voci che vuoi pulire, premi «Analizza» per vedere quanto spazio recuperi e poi «Pulisci selezionati».",
        );
        let total: u64 = self.items.iter().filter(|i| i.selected).filter_map(|i| i.size).sum();
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Spazio recuperabile stimato:")).size(18.0));
                ui.label(egui::RichText::new(cleaner::fmt_bytes(total)).size(22.0).strong().color(ui::accent_text()));
            });
        });
        ui.add_space(3.0);
        ui::card(ui, |ui| {
            for it in self.items.iter_mut() {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut it.selected, egui::RichText::new(t(it.name)).strong().size(18.0));
                    if let Some(s) = it.size {
                        ui.label(egui::RichText::new(cleaner::fmt_bytes(s)).strong().color(ui::accent_text()));
                    }
                });
                ui.label(egui::RichText::new(t(it.desc)).color(ui::text_dim()));
                ui.add_space(3.0);
            }
        });
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui::primary_button(ui, t("Pulisci selezionati"), self.action_enabled()).clicked() {
                let kinds: Vec<cleaner::Kind> =
                    self.items.iter().filter(|i| i.selected).map(|i| i.kind).collect();
                let job = clean_and_scan_job("cleaner", kinds, false);
                self.spawn_snapshot(ctx, job);
            }
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Analizza"))).clicked() {
                self.spawn(ctx, scan_sizes_job("cleaner", t("Analisi spazio in corso...")));
            }
            
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Ottimizza database SQLite browser"))).clicked() {
                self.spawn(ctx, |ch| {
                    let mut prog = ProgressReporter::start(
                        &ch,
                        "cleaner",
                        t("Ottimizzazione database browser in corso..."),
                    );
                    ch.send(Msg::Log("▶ Avvio ottimizzazione database SQLite dei browser...".into()));
                    for line in cleaner::optimize_browser_databases() {
                        prog.status(line.clone());
                        ch.send(Msg::Log(line));
                    }
                    prog.finish(t("Ottimizzazione completata"));
                });
            }
            
            if self.busy {
                ui.spinner();
            }
        });
    }

    fn drain(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Log(s) => {
                    self.log.push_str(&s);
                    self.log.push('\n');
                    
                    if s.starts_with("▶ Avvio ottimizzazione sicura") {
                        self.safe_log.clear();
                    } else if s.starts_with("✔ ") || s.starts_with("⚠ ") || s.starts_with("✘ ") {
                        self.safe_log.push(s.clone());
                    }
                    
                    if self.log.len() > 100_000 {
                        let mut cut = 50_000;
                        while !self.log.is_char_boundary(cut) {
                            cut += 1;
                        }
                        self.log.drain(..cut);
                        self.status_from = 0;
                    }
                }
                Msg::Busy(b) => {
                    self.busy = b;
                    if !b {
                        // Anche i job che non chiamano `finish` devono chiudere
                        // la barra: la UI la mostra per pochi secondi e la nasconde.
                        if self.page == Page::Timers {
                            self.timer_loaded = false;
                        }
                        for p in self.progress.values_mut() {
                            // I job `bg()` (es. treemap) possono ancora girare:
                            // il loro `Busy(false)` non arriva, quindi vanno lasciati
                            // al loro `finish`. Non riaprire nemmeno barre già chiuse.
                            if p.background || p.done {
                                continue;
                            }
                            p.done = true;
                            p.fraction = Some(1.0);
                            p.finished = Some(Instant::now());
                        }
                        self.dev_progress = None;
                        self.kb_progress = None;
                        // Turno successivo della coda FIFO: il job in corso è
                        // terminato (anche in caso di panic), si può partire
                        // con il primo in attesa senza sovrapporre processi.
                        if let Some(next) = self.job_queue.pop_front() {
                            self.log.push_str(&tf("Avvio: {}", &[&next.label]));
                            self.log.push('\n');
                            self.spawn(ctx, next.job);
                        }
                    }
                }
                Msg::Progress(p) => {
                    // Una scansione annullata può ancora inviare il suo `finish`
                    // dopo l'avvio di quella nuova: non deve sovrascriverla.
                    if p.id == "disk_treemap"
                        && matches!(p.generation, Some(g) if g != self.treemap_generation)
                    {
                        continue;
                    }
                    self.progress.insert(p.id, p);
                }
                Msg::DevScan(report) => self.dev_report = report,
                Msg::DevProgress { fraction, detail } => {
                    self.dev_progress = Some((fraction, detail));
                }
                Msg::KernelBootScan(report) => self.kb_report = report,
                Msg::KernelBootProgress { fraction, detail } => {
                    self.kb_progress = Some((fraction, detail));
                }
                Msg::Sizes(s) => {
                    for (it, sz) in self.items.iter_mut().zip(s) {
                        it.size = sz;
                    }
                }
                Msg::Checks(c) => self.checks = c,
                Msg::Out(k, text) => {
                    self.out.insert(k, text);
                    self.live.remove(k);
                }
                Msg::RepoEntries(entries) => self.repo_entries = entries,
                Msg::Live(k, text, commit) => {
                    if commit {
                        let o = self.out.entry(k).or_default();
                        o.push_str(&text);
                        o.push('\n');
                        if o.len() > 200_000 {
                            let mut cut = o.len() - 100_000;
                            while !o.is_char_boundary(cut) {
                                cut += 1;
                            }
                            o.drain(..cut);
                        }
                        self.live.remove(k);
                    } else {
                        self.live.insert(k, text);
                    }
                }
                Msg::Net(n) => self.net = Some(n),
                Msg::UpdateAvailable(version) => self.update_available = version,
                Msg::NetSockets(list) => {
                 self.net_sockets = list;
                }
                Msg::GpgKeys(list) => {
                 self.gpg_keys = list;
                }
                Msg::Procs(p) => {
                    self.proc_selected.retain(|pid| p.iter().any(|row| row.pid == *pid));
                    self.procs = p;
                    self.proc_pending = false;
                }
                Msg::Pkgs(p) => {
                    self.pkg_selected.retain(|n| p.iter().any(|pkg| pkg.name == *n));
                    self.pkgs = p;
                }
                Msg::BigFiles(b) => self.bigfiles = b,
                Msg::Kernels(k) => self.kernels = k,
                Msg::Startup(s) => self.startup = s,
                Msg::BootSummary(s) => self.boot_summary = s,
                Msg::BootItems(items) => self.boot_items = items,
                Msg::Groups(g) => self.groups = g,
                Msg::Gov(g, s) => {
                    self.governor = g;
                    self.swappiness = s;
                }
                Msg::Disks(d) => {
                    if self.disk_sel.is_empty() {
                        if let Some(f) = d.first() {
                            self.disk_sel = f.0.clone();
                        }
                    }
                    self.disks_list = d;
                }
                Msg::AdvDirs(l) => self.adv_dirs = l,
                Msg::AdvFiles(l) => self.adv_files = l,
                Msg::Drivers(h, d) => {
                    self.drv_hw = h;
                    self.drv = d;
                }
                Msg::Picked(Some(v)) => {
                    for p in v {
                        if !self.shred_path.lines().any(|l| l.trim() == p) {
                            if !self.shred_path.trim().is_empty() && !self.shred_path.ends_with('\n') {
                                self.shred_path.push('\n');
                            }
                            self.shred_path.push_str(&p);
                            self.shred_path.push('\n');
                        }
                    }
                }
                Msg::TelemetryItems(list) => self.telemetry_items = list,
                Msg::UpdApt(a, checked) => {
                    self.upd_apt = a;
                    if checked {
                        self.upd_checked = true;
                    }
                }
                Msg::UpdFlat(f) => {
                    self.upd_flat = f;
                    self.upd_flat_loading = false;
                }
                Msg::FlatLoading => self.upd_flat_loading = true,
                Msg::Ram(arr, mods, msg) => {
                    self.ram_arr = arr;
                    self.ram_modules = mods;
                    self.ram_msg = msg;
                }
                Msg::Gpus(g) => self.gpus = g,
                Msg::Comps(c) => self.comps = c,
                Msg::SynapticPkgs(list) => {
                    self.synaptic_pkgs = list;
                    self.synaptic_selected = None;
                    self.synaptic_details.clear();
                }
                Msg::RcPkgs(list) => self.rc_pkgs = list,
                Msg::FlatpakUnused(list) => self.flatpak_unused = list, 
                Msg::OrphanConfigs(list) => self.orphan_configs = list, 
                Msg::ConfigBackups(list) => self.config_backups = list,
                Msg::DisabledSnaps(list) => self.disabled_snaps = list,
                Msg::Duplicates(list) => self.dup_groups = list,
                Msg::Services(list) => {
                    self.svc_selected.retain(|n| list.iter().any(|s| s.name == *n));
                    self.services_list = list;
                }
                Msg::DiskAnalyzerItems(items) => self.disk_analyzer_items = items, 
                Msg::DiskAnalyzerPath(path) => self.disk_analyzer_path = path,
                Msg::Picked(None) => {
                    self.log.push_str(t("Nessun selettore di file trovato: installa zenity (sudo apt install zenity)"));
                    self.log.push('\n');
                }
                Msg::ThemesPicked(Some(files)) => {
                    let job: Job = Box::new(move |ch: &Chan| {
                        let prog = ProgressReporter::start(
                            ch,
                            "theme_install",
                            t("Installazione di font/temi in corso..."),
                        );
                        match crate::pages_themes::install_files(&files) {
                            Ok(lines) => {
                                for line in lines {
                                    ch.send(Msg::Log(line));
                                }
                                prog.finish(t("Completato"));
                            }
                            Err(e) => {
                                ch.send(Msg::Log(format!("✘ {e}")));
                                prog.finish(t("Operazione terminata con errori"));
                            }
                        }
                        ch.send(Msg::Themes(crate::pages_themes::scan_user_themes()));
                    });
                    self.spawn(ctx, job);
                }
                Msg::ThemesPicked(None) => {
                    self.log.push_str(t("Nessun selettore di file trovato: installa zenity (sudo apt install zenity)"));
                    self.log.push('\n');
                }
                Msg::Themes(items) => {
                    self.theme_items = items;
                }
                Msg::Vpns(list) => self.vpn_list = list,
                Msg::OrphanKernels(list) => {
                     self.orphan_kernels = list;
                }
                Msg::Mounts(list) => self.mounts_state.entries = list,
                Msg::SlowDiag(findings) => {
                    self.slow_diag = findings;
                    self.slow_diag_ran = true;
                }
                Msg::SafeOptimizeReport(rep) => self.safe_report = Some(rep),
                Msg::DiskDeepScan(deep) => self.disk_deep = deep,
                Msg::DiskSpaceComparison(comp) => self.disk_comparison = Some(comp),
                Msg::TreemapPath(path) => self.treemap_input = path,
                Msg::TreemapScanDone { path, scan, generation } => {
                    if generation == self.treemap_generation {
                        self.treemap_scans.insert(path.clone(), scan);
                        self.treemap_current = path;
                        self.treemap_selected = None;
                    }
                }
                Msg::TreemapInvalidate(path) => {
                    // La cartella modificata e tutti i suoi antenati hanno
                    // dimensioni obsolete: vengono ricalcolate alla prossima visita.
                    self.treemap_scans
                        .retain(|p, _| !p.starts_with(&path) && !path.starts_with(p));
                    self.treemap_selected = None;
                    let cur = self.treemap_current.clone();
                    if cur.starts_with(&path) || path.starts_with(&cur) {
                        // `drain` non ha il contesto egui: la scansione viene
                        // avviata al prossimo frame, dove `ctx` è disponibile.
                        self.treemap_pending_scan = Some(cur);
                    }
                }

        
        Msg::SystemEditorSaved(res) => {
                    self.system_editor.saving = false;
                    match res {
                        Ok(msg) => {
                            self.log.push_str(&format!("✔ {}\n", msg));
                            self.system_editor.status_msg = msg;
                        }
                        Err(e) => {
                            self.log.push_str(&format!("✘ {}\n", e));
                            self.system_editor.status_msg = e;
                        }
                    }
                }
                Msg::TimerStatus(status) => {
                    self.timer_status = Some(status);
                    self.timer_loaded = true;
                }
                Msg::TimerMessage(msg, is_error) => {
                    self.timer_msg = Some((msg.clone(), is_error));
                    self.log.push_str(&format!("{} {}\n", if is_error { "✘" } else { "✔" }, msg));
                }
                Msg::QuarantineItems(items) => self.quarantine_items = items,
                Msg::QuarantineMessage(msg, is_error) => {
                    self.quarantine_msg = Some((msg.clone(), is_error));
                    self.log.push_str(&format!("{} {}\n", if is_error { "✘" } else { "✔" }, msg));
                }
                Msg::RestorePoints(points) => {
                    // La selezione può contenere id rimossi: mantieni solo quelli
                    // ancora presenti nell'elenco aggiornato.
                    self.restore_selected.retain(|id| points.iter().any(|p| p.id == *id));
                    self.restore_points = points;
                }
                Msg::RestoreMessage(msg, is_error) => {
                    self.restore_msg = Some((msg.clone(), is_error));
                    self.log.push_str(&format!("{} {}\n", if is_error { "✘" } else { "✔" }, msg));
                }
                Msg::RestoreHistoryItems(entries, restricted) => {
                    self.restore_history_selected = self
                        .restore_history_selected
                        .take()
                        .filter(|id| entries.iter().any(|e| e.id == *id));
                    self.restore_history_restricted = restricted;
                    self.restore_history_entries = entries;
                }
                Msg::RestoreHistoryRemoved(id) => {
                    self.restore_history_entries.retain(|e| e.id != id);
                    if self.restore_history_selected.as_deref() == Some(id.as_str()) {
                        self.restore_history_selected = None;
                    }
                }
                Msg::RestoreHistoryMessage(msg, is_error) => {
                    self.restore_history_msg = Some((msg.clone(), is_error));
                    self.log.push_str(&format!("{} {}\n", if is_error { "✘" } else { "✔" }, msg));
                }
           }
        }
        
    }
    
    fn ui_history(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Storico APT"));
        ui::hint(
            ui,
            t("Qui puoi consultare il registro completo delle operazioni eseguite da APT sul sistema (installazioni, aggiornamenti e rimozioni di pacchetti)."),
        );

        ui.add_space(3.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Ricarica Storico"))).clicked() {
                match history::load_apt_history() {
                    Ok(h) => self.apt_history = h,
                    Err(e) => self.log.push_str(&format!("Errore: {e}\n")),
                }
            }
            if self.busy {
                ui.spinner();
            }
        });

        if self.out.contains_key("history") || self.live.contains_key("history") {
            ui.add_space(3.0);
            self.show_out_follow(ui, "history", 140.0);
        }

        ui.add_space(5.0);

        if self.apt_history.is_empty() {
            ui::card(ui, |ui| {
                ui.label(t("Nessuna transazione APT trovata o file di log non accessibile (/var/log/apt/history.log)."));
            });
            return;
        }

        let total = self.apt_history.len();
        let f = self.history_filter.to_lowercase();
        let mut rows: Vec<history::AptTransaction> = self
            .apt_history
            .iter()
            .filter(|tx| {
                if f.is_empty() {
                    return true;
                }
                tx.start_date.to_lowercase().contains(&f)
                    || tx.end_date.to_lowercase().contains(&f)
                    || tx.command_line.to_lowercase().contains(&f)
                    || tx.installed.join(" ").to_lowercase().contains(&f)
                    || tx.upgraded.join(" ").to_lowercase().contains(&f)
                    || tx.removed.join(" ").to_lowercase().contains(&f)
            })
            .cloned()
            .collect();

        rows.sort_by(|a, b| {
            let ord = match self.history_sort.key {
                HistorySortKey::Date => a.start_date.cmp(&b.start_date),
                HistorySortKey::Command => ui::cmp_alpha(&a.command_line, &b.command_line),
            };
            match self.history_sort.dir {
                ui::SortDir::Asc => ord,
                ui::SortDir::Desc => ord.reverse(),
            }
        });

        ui::filter_bar(ui, &mut self.history_filter, rows.len(), total);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(t("Ordina per:")).color(ui::text_dim()));
            ui::sort_header(ui, t("Data/Ora"), HistorySortKey::Date, &mut self.history_sort);
            ui::sort_header(ui, t("Comando"), HistorySortKey::Command, &mut self.history_sort);
        });

        let mut rollback_cmd: Option<(String, String)> = None;

        egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
            for tx in &rows {
                ui::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(&tx.start_date).strong().color(ui::accent_text()).size(16.0));
                        if !tx.end_date.is_empty() {
                            ui.label(
                                egui::RichText::new(format!("({}: {})", t("Completato"), tx.end_date))
                                    .color(ui::text_dim()),
                            );
                        }

                        if let Some(cmd) = history::build_rollback_command(tx) {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add_enabled(
                                        self.action_enabled(),
                                        egui::Button::new(
                                            egui::RichText::new(t("Annulla operazione")).color(ui::semantic_text(ui::WARN)),
                                        ),
                                    )
                                    .clicked()
                                {
                                    rollback_cmd = Some((tx.start_date.clone(), cmd));
                                }
                            });
                        }
                    });

                    ui.add_space(2.0);
                    let command = tx.command_line.trim();
                    if !command.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("{} {}", t("Comando:"), command))
                                .monospace()
                                .color(ui::text_primary()),
                        );
                        ui.add_space(3.0);
                    }

                    if !tx.installed.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(ui::semantic_text(ui::GOOD), t("Installati:"));
                            ui.label(tx.installed.join(", "));
                        });
                    }
                    if !tx.upgraded.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(ui::semantic_text(ui::WARN), t("Aggiornati:"));
                            ui.label(tx.upgraded.join(", "));
                        });
                    }
                    if !tx.removed.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(ui::semantic_text(ui::BAD), t("Rimossi:"));
                            ui.label(tx.removed.join(", "));
                        });
                    }
                });
                ui.add_space(3.0);
            }
        });

        if let Some((date, cmd)) = rollback_cmd {
            let msg = format!(
                "{} ({})\n\n{}:\n{}",
                t("Vuoi davvero annullare questa operazione APT?"),
                date,
                t("Verrà eseguito il comando"),
                cmd
            );
            let job = cmd_job("history", cmd, true);
            self.confirm = Some((msg, job));
            ctx.request_repaint();
        }
    }

    fn ui_advanced_cleaner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Pulizia Avanzata e Residui"));
        ui::hint(
            ui,
            t("Rimuovi configurazioni residue (pacchetti RC), backup dimenticati (.dpkg-bak, .pacsave), runtime Flatpak inutilizzati, vecchie revisioni Snap disabilitate e cartelle orfane in ~/.config."),
        );
        self.snapshot_toggle(ui);

        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Scansiona tutto"))).clicked() {
                self.spawn(ctx, move |ch| {
                    let mut prog = ProgressReporter::start(
                        &ch,
                        "adv_clean",
                        t("Scansione approfondita dei residui in corso..."),
                    );
                    ch.send(Msg::Log("▶ Scansione approfondita residui e file orfani in corso...".into()));
                    prog.step(1, 5, t("Pacchetti residui (rc)..."));
                    ch.send(Msg::RcPkgs(cleaner::scan_rc_pkgs()));
                    prog.step(2, 5, t("Runtime Flatpak inutilizzati..."));
                    ch.send(Msg::FlatpakUnused(cleaner::scan_flatpak_unused()));
                    prog.step(3, 5, t("Backup di configurazione dimenticati..."));
                    ch.send(Msg::ConfigBackups(cleaner::scan_config_backups()));
                    prog.step(4, 5, t("Revisioni Snap disabilitate..."));
                    ch.send(Msg::DisabledSnaps(cleaner::scan_disabled_snaps()));
                    prog.step(5, 5, t("Configurazioni orfane in ~/.config..."));
                    ch.send(Msg::OrphanConfigs(cleaner::scan_orphan_configs()));
                    ch.send(Msg::Log("✔ Scansione residui completata.".into()));
                    prog.finish(t("Scansione residui completata"));
                });
            }
            if self.busy {
                ui.spinner();
            }
        });

        ui.add_space(5.0);

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Configurazioni residue (Pacchetti RC)")).strong().size(16.0).color(ui::accent_text()));
            ui.add_space(2.0);
            if self.rc_pkgs.is_empty() {
                ui.label(t("Nessun pacchetto residuo trovato. (Premi 'Scansiona tutto' per avviare)"));
            } else {
                for (pkg, selected) in &mut self.rc_pkgs {
                    ui.checkbox(selected, pkg.as_str());
                }
                ui.add_space(3.0);
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Pulisci pacchetti RC selezionati"))).clicked() {
                    let targets: Vec<String> = self.rc_pkgs.iter().filter(|(_, sel)| *sel).map(|(p, _)| p.clone()).collect();
                    let args_str = targets.iter().map(|s| util::shq(s)).collect::<Vec<_>>().join(" ");
                    let script = format!("apt-get purge -y {args_str}");
                    let job = cmd_job("adv_clean", script, true);
                    self.spawn_snapshot(ctx, job);
                }
            }
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Backup di configurazione dimenticati (/etc)")).strong().size(16.0).color(ui::accent_text()));
            ui.add_space(2.0);
            if self.config_backups.is_empty() {
                ui.label(t("Nessun file .dpkg-bak, .dpkg-old o .pacsave trovato."));
            } else {
                egui::ScrollArea::vertical().id_salt("cfg_bak_scroll").max_height(130.0).show(ui, |ui| {
                    for (path, selected) in &mut self.config_backups {
                        ui.checkbox(selected, path.to_string_lossy().as_ref());
                    }
                });
                ui.add_space(3.0);
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Sposta in quarantena i backup selezionati"))).clicked() {
                    let targets: Vec<std::path::PathBuf> = self.config_backups
                        .iter()
                        .filter(|(_, sel)| *sel)
                        .map(|(p, _)| p.clone())
                        .collect();
                    if !targets.is_empty() {
                        let count = targets.len();
                        let text = tf(
                            "Spostare {} backup di configurazione in quarantena? Potrai ripristinarli entro {} giorni.",
                            &[&count, &quarantine::RETENTION_DAYS],
                        );
                        let job: Job = Box::new(move |ch: &Chan| {
                            let prog = ProgressReporter::start(
                                &ch,
                                "quarantine",
                                t("Spostamento backup in quarantena in corso..."),
                            );
                            match quarantine::quarantine_files(&targets, "Residui di sistema", true) {
                                Ok((moved, failed)) => {
                                    ch.send(Msg::QuarantineMessage(
                                        tf("Spostati {} file in quarantena, {} non spostati.", &[&moved, &failed]),
                                        failed > 0,
                                    ));
                                }
                                Err(e) => ch.send(Msg::QuarantineMessage(e, true)),
                            }
                            ch.send(Msg::QuarantineItems(quarantine::load().unwrap_or_default()));
                            ch.send(Msg::ConfigBackups(cleaner::scan_config_backups()));
                            prog.finish(t("Spostamento completato"));
                        });
                        self.confirm_snapshot(text, job);
                    }
                }
            }
        });

        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Kit di Migrazione (1-Click)")).strong().size(18.0));
            ui.label(t("Esporta l'intero profilo della macchina in un unico archivio compresso (.tar.gz) contenente pacchetti APT manuali, app Flatpak installate, repository aggiuntivi, lanciatori utente e configurazioni dei Window Manager, includendo uno script automatico per ripristinare tutto sul nuovo sistema."));
            ui.add_space(3.0);
            if ui::primary_button(ui, t("Crea Kit di Migrazione"), self.action_enabled()).clicked() {
                let mut dest = std::path::PathBuf::from(crate::util::home()).join("Desktop");
                if let Ok(out) = std::process::Command::new("xdg-user-dir").arg("DESKTOP").output() {
                    if out.status.success() {
                        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        if p.starts_with('/') && std::path::PathBuf::from(&p).is_dir() {
                            dest = std::path::PathBuf::from(p);
                        }
                    }
                }
                self.spawn(ctx, move |ch| {
                    let mut prog = ProgressReporter::start(
                        &ch,
                        "adv_clean",
                        t("Creazione del Kit di Migrazione in corso..."),
                    );
                    ch.send(Msg::Log("▶ Creazione Kit di Migrazione in corso...".into()));
                    let (lines, ok) = crate::migration::create_migration_kit(&dest);
                    for line in lines {
                        prog.status(line.clone());
                        ch.send(Msg::Log(line));
                    }
                    prog.finish(if ok {
                        t("Kit di Migrazione creato")
                    } else {
                        t("Operazione terminata con errori")
                    });
                });
            }
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Runtime e App Flatpak Inutilizzati")).strong().size(16.0).color(ui::accent_text()));
            ui.add_space(2.0);
            if self.flatpak_unused.is_empty() {
                ui.label(t("Nessun componente Flatpak orfano trovato."));
            } else {
                for (item, selected) in &mut self.flatpak_unused {
                    ui.checkbox(selected, item.as_str());
                }
                ui.add_space(3.0);
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Rimuovi Flatpak inutilizzati"))).clicked() {
                    let targets: Vec<String> = self
                        .flatpak_unused
                        .iter()
                        .filter(|(_, sel)| *sel)
                        .map(|(item, _)| item.clone())
                        .collect();
                    let job: Job = Box::new(move |ch: &Chan| {
                        ch.send(Msg::Log("▶ Rimuovo i runtime Flatpak selezionati...".into()));
                        let ok = cleaner::remove_unused_flatpaks(&targets);
                        ch.send(Msg::FlatpakUnused(cleaner::scan_flatpak_unused()));
                        if ok {
                            ch.send(Msg::Log("✔ Pulizia Flatpak completata.".into()));
                        } else {
                            ch.send(Msg::Log("✘ Rimozione Flatpak non riuscita o annullata.".into()));
                        }
                    });
                    self.spawn_snapshot(ctx, job);
                }
            }
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Vecchie revisioni Snap disabilitate")).strong().size(16.0).color(ui::accent_text()));
            ui.add_space(2.0);
            if self.disabled_snaps.is_empty() {
                ui.label(t("Nessuna revisione Snap disabilitata trovata."));
            } else {
                for (item, selected) in &mut self.disabled_snaps {
                    ui.checkbox(selected, item.as_str());
                }
                ui.add_space(3.0);
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Rimuovi revisioni Snap selezionate"))).clicked() {
                    let targets: Vec<String> = self.disabled_snaps.iter().filter(|(_, sel)| *sel).map(|(s, _)| s.clone()).collect();
                    let job: Job = Box::new(move |ch: &Chan| {
                        ch.send(Msg::Log("▶ Rimozione vecchie revisioni Snap...".into()));
                        let ok = cleaner::remove_disabled_snaps(&targets);
                        ch.send(Msg::DisabledSnaps(cleaner::scan_disabled_snaps()));
                        if ok {
                            ch.send(Msg::Log("✔ Pulizia revisioni Snap completata.".into()));
                        } else {
                            ch.send(Msg::Log("✘ Pulizia revisioni Snap non riuscita (autorizzazione negata o errore di snap).".into()));
                        }
                    });
                    self.spawn_snapshot(ctx, job);
                }
            }
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Cartelle di configurazione utente orfane (~/.config)")).strong().size(16.0).color(ui::accent_text()));
            ui.add_space(2.0);
            if self.orphan_configs.is_empty() {
                ui.label(t("Nessuna cartella orfana rilevata in ~/.config."));
            } else {
                egui::ScrollArea::vertical().id_salt("orphan_cfg_scroll").max_height(130.0).show(ui, |ui| {
                    for (path, selected) in &mut self.orphan_configs {
                        ui.checkbox(selected, path.to_string_lossy().as_ref());
                    }
                });
                ui.add_space(3.0);
                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Elimina configurazioni orfane selezionate"))).clicked() {
                    let targets: Vec<std::path::PathBuf> = self.orphan_configs.iter().filter(|(_, sel)| *sel).map(|(p, _)| p.clone()).collect();
                    if !targets.is_empty() {
                        let n_targets = targets.len();
                        let job: Job = Box::new(move |ch: &Chan| {
                            ch.send(Msg::Log("▶ Rimozione cartelle orfane in ~/.config...".into()));
                            let ok = cleaner::remove_orphan_configs(&targets);
                            ch.send(Msg::OrphanConfigs(cleaner::scan_orphan_configs()));
                            let mark = if ok { "✔" } else { "⚠" };
                            ch.send(Msg::Log(format!("{mark} Rimozione configurazioni orfane terminata{}.", if ok { "" } else { " (alcune non rimosse)" })));
                        });
                        self.confirm_snapshot(
                            tf("Eliminare definitivamente {} cartelle di configurazione in ~/.config? Le impostazioni dei programmi andranno perse.", &[&n_targets]),
                            job,
                        );
                    }
                }
            }
        });
    }
     
    fn ui_snaps(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Pacchetti Snap"));
        ui::hint(
            ui,
            t("Gestisci i pacchetti Snap installati sul sistema. Puoi aggiornarli o rimuoverli singolarmente."),
        );

        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Aggiorna tutti gli Snap"))).clicked() {
                let job: Job = Box::new(move |ch: &Chan| {
                    ch.send(Msg::Log("▶ Aggiorno tutti i pacchetti Snap...".into()));
                    match snap::refresh_snap(None) {
                        Ok(_) => ch.send(Msg::Log("✔ Aggiornamento Snap completato.".into())),
                        Err(e) => ch.send(Msg::Log(format!("✘ Errore aggiornamento Snap: {e}"))),
                    }
                });
                self.spawn(ctx, job);
            }
            if self.busy {
                ui.spinner();
            }
        });

        ui.add_space(5.0);

        ui::card(ui, |ui| {
            match snap::get_installed_snaps() {
                Ok(apps) => {
                    if apps.is_empty() {
                        ui.label(t("Nessun pacchetto Snap installato."));
                    } else {
                        for app in apps {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{} (v{})", app.name, app.version))
                                        .strong()
                                        .size(15.0),
                                );
                                ui.label(
                                    egui::RichText::new(format!("[{}]", app.publisher))
                                        .color(ui::text_dim()),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui
                                        .add_enabled(
                                            self.action_enabled(),
                                            egui::Button::new(
                                                egui::RichText::new(t("Rimuovi")).color(ui::semantic_text(ui::BAD)),
                                            ),
                                        )
                                        .clicked()
                                    {
                                        let app_name = app.name.clone();
                                        let job: Job = Box::new(move |ch: &Chan| {
                                            ch.send(Msg::Log(format!("▶ Rimuovo lo snap '{}'...", app_name)));
                                            match snap::remove_snap(&app_name) {
                                                Ok(_) => ch.send(Msg::Log(format!("✔ Snap '{}' rimosso con successo.", app_name))),
                                                Err(e) => ch.send(Msg::Log(format!("✘ Errore rimozione snap: {e}"))),
                                            }
                                        });
                                        self.spawn(ctx, job);
                                    }
                                });
                            });
                            ui.add_space(3.0);
                        }
                    }
                }
                Err(e) => {
                    ui.colored_label(ui::semantic_text(ui::BAD), t(e));
                    ui.label(t("Assicurati che snapd sia installato e attivo sul sistema."));
                }
            }
        });
    }
}
#[derive(Clone)]
pub enum SlowSeverity {
    Ok,
    Warn,
    Bad,
}

#[derive(Clone)]
pub struct SlowDiagFinding {
    pub area: &'static str,
    pub severity: SlowSeverity,
    pub summary: String,
    pub detail: String,
    pub action_label: &'static str,
    pub target_page: Page,
}

fn run_slow_diag(info: &info::SysInfo) -> Vec<SlowDiagFinding> {
    let mut out = Vec::new();

    // 1. CPU & Load average
    let cpu_pct = info.sys.global_cpu_usage();
    let cores = info.sys.cpus().len().max(1) as f32;
    let load1 = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f32>().ok())
        .unwrap_or(0.0);
    let load_per_core = load1 / cores;
    let cpu = format!("{cpu_pct:.0}");
    let load = format!("{load1:.2}");

    if cpu_pct >= 80.0 || load_per_core >= 1.5 {
        out.push(SlowDiagFinding {
            area: "CPU",
            severity: SlowSeverity::Bad,
            summary: tf("CPU sotto carico elevato ({}% · load 1m: {})", &[&cpu, &load]),
            detail: t("Uno o più processi stanno impegnando il processore o la coda di esecuzione è satura.").to_string(),
            action_label: "Apri Processi",
            target_page: Page::Procs,
        });
    } else if cpu_pct >= 50.0 || load_per_core >= 0.9 {
        out.push(SlowDiagFinding {
            area: "CPU",
            severity: SlowSeverity::Warn,
            summary: tf("Attività CPU moderata/alta ({}% · load 1m: {})", &[&cpu, &load]),
            detail: t("Controlla i processi attivi se noti micro-blocchi nell'interfaccia.").to_string(),
            action_label: "Apri Processi",
            target_page: Page::Procs,
        });
    } else {
        out.push(SlowDiagFinding {
            area: "CPU",
            severity: SlowSeverity::Ok,
            summary: tf("CPU regolare ({}% · load 1m: {})", &[&cpu, &load]),
            detail: t("Nessuna saturazione del processore rilevata.").to_string(),
            action_label: "Dettagli Processi",
            target_page: Page::Procs,
        });
    }

    // 2. RAM & Swap
    let total_ram = info.sys.total_memory().max(1);
    let used_ram = info.sys.used_memory();
    let ram_frac = used_ram as f32 / total_ram as f32;
    let total_swap = info.sys.total_swap();
    let used_swap = info.sys.used_swap();
    let swap_frac = if total_swap > 0 {
        used_swap as f32 / total_swap as f32
    } else {
        0.0
    };
    let ram = format!("{:.0}", ram_frac * 100.0);
    let swap = format!("{:.0}", swap_frac * 100.0);

    if ram_frac >= 0.88 || swap_frac >= 0.65 {
        out.push(SlowDiagFinding {
            area: "Memoria",
            severity: SlowSeverity::Bad,
            summary: tf("Memoria quasi satura (RAM {}% · Swap {}%)", &[&ram, &swap]),
            detail: t("L'uso intenso della swap rallenta sensibilmente l'apertura delle app e il cambio finestra.").to_string(),
            action_label: "Ottimizza RAM / Zram",
            target_page: Page::Zram,
        });
    } else if ram_frac >= 0.72 || used_swap > 512 * 1024 * 1024 {
        out.push(SlowDiagFinding {
            area: "Memoria",
            severity: SlowSeverity::Warn,
            summary: tf("Uso memoria sostenuto (RAM {}% · Swap {}%)", &[&ram, &swap]),
            detail: t("Valuta di chiudere programmi pesanti o configurare Zram per ridurre lo swap su disco.").to_string(),
            action_label: "Apri Ottimizza RAM",
            target_page: Page::Zram,
        });
    } else {
        out.push(SlowDiagFinding {
            area: "Memoria",
            severity: SlowSeverity::Ok,
            summary: tf("RAM disponibile ({}% in uso)", &[&ram]),
            detail: t("Margine di memoria adeguato per il funzionamento fluido.").to_string(),
            action_label: "Dettagli RAM",
            target_page: Page::Zram,
        });
    }

    // 3. Disco e spazio libero
    let mut low_disk: Option<(String, u8)> = None;
    if let Ok(out_df) = std::process::Command::new("df")
        .args(["-P", "--output=target,pcent"])
        .output()
    {
        if out_df.status.success() {
            let txt = String::from_utf8_lossy(&out_df.stdout);
            for line in txt.lines().skip(1) {
                let mut parts = line.split_whitespace();
                let (Some(mnt), Some(pct_s)) = (parts.next(), parts.next()) else { continue };
                if mnt.starts_with("/snap")
                    || mnt.starts_with("/proc")
                    || mnt.starts_with("/sys")
                    || mnt.starts_with("/run")
                {
                    continue;
                }
                let pct: u8 = pct_s.trim_end_matches('%').parse().unwrap_or(0);
                if pct >= 85 && low_disk.as_ref().map_or(true, |(_, p)| pct > *p) {
                    low_disk = Some((mnt.to_string(), pct));
                }
            }
        }
    }

    if let Some((mnt, pct)) = low_disk {
        out.push(SlowDiagFinding {
            area: "Disco",
            severity: if pct >= 94 { SlowSeverity::Bad } else { SlowSeverity::Warn },
            summary: tf("Spazio su disco quasi esaurito su {} ({})", &[&mnt, &pct]),
            detail: t("Con poco spazio libero il file system e la cache di sistema possono subire rallentamenti.").to_string(),
            action_label: "Analizza Spazio",
            target_page: Page::DiskAnalyzer,
        });
    } else {
        out.push(SlowDiagFinding {
            area: "Disco",
            severity: SlowSeverity::Ok,
            summary: t("Spazio su disco nella norma").to_string(),
            detail: t("Nessuna partizione critica vicina alla saturazione.").to_string(),
            action_label: "Apri Disco",
            target_page: Page::Disk,
        });
    }
    // 4. Processi top e servizi systemd in errore
    let mut procs = pages_sys::read_procs();
procs.sort_by(|a, b| {
        let ac = a.cpu.parse::<f32>().unwrap_or(0.0);
        let bc = b.cpu.parse::<f32>().unwrap_or(0.0);
        bc.partial_cmp(&ac)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.mem
                    .parse::<f32>()
                    .unwrap_or(0.0)
                    .partial_cmp(&a.mem.parse::<f32>().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    let top_info = procs
        .first()
        .map(|p| tf("{} (CPU {}%, RAM {}%)", &[&p.cmd, &p.cpu, &p.mem]))
        .unwrap_or_else(|| t("nessun dato").to_string());

    let failed_units: Vec<String> = std::process::Command::new("systemctl")
        .args(["--failed", "--no-legend", "--plain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.split_whitespace().next().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    if !failed_units.is_empty() {
        out.push(SlowDiagFinding {
            area: "Servizi",
            severity: SlowSeverity::Warn,
            summary: tf("{} servizio/i systemd in stato failed", &[&failed_units.len()]),
            detail: tf(
                "Unità: {}. Processo più attivo: {}",
                &[&failed_units.join(", "), &top_info]
            ),
            action_label: "Apri Servizi",
            target_page: Page::Services,
        });
    } else {
        out.push(SlowDiagFinding {
            area: "Processi",
            severity: SlowSeverity::Ok,
            summary: tf("Processo principale: {}", &[&top_info]),
            detail: t("Nessun servizio systemd in errore rilevato.").to_string(),
            action_label: "Apri Processi",
            target_page: Page::Procs,
        });
    }

    out
}

impl App {
    pub fn ui_slow_diag(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, goto: &mut Option<Page>) {
        ui::card_compact(ui, |ui| {
            let avail_w = ui.available_width();
            let text_w = (avail_w * 0.62).max(180.0).min(avail_w);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.vertical(|ui| {
                    ui.set_max_width(text_w);
                    let title = ui.label(
                        egui::RichText::new(t("Perché è lento adesso?"))
                            .size(18.0)
                            .strong()
                            .color(ui::accent_text()),
                    );
                    let _ = title.on_hover_text(t("Diagnostica in 1 clic su CPU, memoria/swap, spazio disco, processi e servizi."));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if self.busy {
                        ui.spinner();
                    }
                    if ui::primary_button_compact(ui, t("Diagnostica ora"), self.action_enabled()).clicked() {
                        self.info.refresh();
                        self.spawn(ctx, move |ch| {
                            let mut prog = ProgressReporter::start(
                                &ch,
                                "slow_diag",
                                t("Diagnostica rallentamenti in corso..."),
                            );
                            ch.send(Msg::Log("▶ Eseguo diagnostica rallentamenti in corso...".into()));
                            let mut snap = info::SysInfo::new();
                            prog.status(t("Lettura dei contatori di sistema..."));
                            snap.refresh();
                            prog.status(t("Analisi dei processi e delle risorse..."));
                            let findings = run_slow_diag(&snap);
                            ch.send(Msg::SlowDiag(findings));
                            ch.send(Msg::Log("✔ Diagnostica rallentamenti completata.".into()));
                            prog.finish(t("Diagnostica completata"));
                        });
                    }
                });
            });

            if !self.slow_diag_ran {
                ui.label(
                    egui::RichText::new(t("Premi «Diagnostica ora» per analizzare cosa sta impegnando il sistema in questo momento."))
                        .size(12.0)
                        .color(ui::text_dim()),
                );
                return;
            }

            ui.add_space(2.0);
            for f in self.slow_diag.clone() {
                let (badge, col) = match f.severity {
                    SlowSeverity::Ok => (t("OK"), ui::GOOD),
                    SlowSeverity::Warn => (t("ATTENZIONE"), ui::WARN),
                    SlowSeverity::Bad => (t("CRITICO"), ui::BAD),
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.colored_label(col, egui::RichText::new(format!("[{}]", tr(badge))).strong().size(12.0));
                    ui.label(egui::RichText::new(tr(f.area)).strong().size(12.0));
                    let summary = ui.label(egui::RichText::new(tr(&f.summary)).size(12.0));
                    let _ = summary.on_hover_text(tr(&f.detail));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn = egui::Button::new(egui::RichText::new(t(f.action_label)).size(12.0));
                        if ui.add_enabled(self.action_enabled(), btn).clicked() {
                            *goto = Some(f.target_page);
                        }
                    });
                });
            }
        });
    }

    fn ui_report(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Report diagnostica"));
        ui::hint(
            ui,
            t("Genera un report completo dello stato del sistema in formato HTML (pronto anche per la stampa in PDF), utile per l'assistenza tecnica."),
        );

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Sezioni da includere nel report:")).strong().size(16.0));
            ui.add_space(3.0);
            ui.checkbox(&mut self.report_opts.hardware, t("Informazioni Hardware, CPU, RAM e Kernel"));
            ui.checkbox(&mut self.report_opts.disk, t("Spazio disco e dispositivi a blocchi"));
            ui.checkbox(&mut self.report_opts.packages, t("Pacchetti APT bloccati (hold) e integrità"));
            ui.checkbox(&mut self.report_opts.logs, t("Log di errore recenti (journalctl e kernel)"));
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Percorso file di destinazione:")).strong());
            ui.add_space(2.0);
            ui.add(egui::TextEdit::singleline(&mut self.report_path).desired_width(ui::fit_width(ui, 420.0, 160.0)));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let any_selected = self.report_opts.hardware
                    || self.report_opts.disk
                    || self.report_opts.packages
                    || self.report_opts.logs;

                if ui::primary_button(ui, t("Genera Report HTML"), self.action_enabled() && any_selected).clicked() {
                    let opts = self.report_opts.clone();
                    let path = util::expand_path(&self.report_path);
                    self.spawn(ctx, move |ch| {
                        let prog = ProgressReporter::start(
                            &ch,
                            "report",
                            t("Generazione del report diagnostico in corso..."),
                        );
                        ch.send(Msg::Log("▶ Generazione del report diagnostico in corso...".into()));
                        match pages_report::generate_html_report(&opts, &path) {
                            Ok(saved) => {
                                ch.send(Msg::Log(format!("✔ Report salvato in: {saved}")));
                                if let Err(e) = std::process::Command::new("xdg-open").arg(&saved).spawn() {
                                    ch.send(Msg::Log(format!("✘ {}: {e}", t("Impossibile aprire il file manager"))));
                                }
                                prog.finish(t("Report completato"));
                            }
                            Err(e) => {
                                ch.send(Msg::Log(format!("✘ {}", t(e))));
                                prog.finish(t("Generazione report non riuscita"));
                            }
                        }
                    });
                }

                if self.busy {
                    ui.spinner();
                }
            });
        });
    }

    // ---------- Analizzatore Disco (treemap) -------------------------------------------------

    /// Avvia una nuova scansione in background; la precedente viene annullata.
    fn treemap_scan(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.treemap_cancel.store(true, AtomicOrdering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.treemap_cancel = cancel.clone();
        self.treemap_generation = self.treemap_generation.wrapping_add(1);
        let generation = self.treemap_generation;
        self.treemap_current = path.clone();
        self.treemap_selected = None;

        self.bg(ctx, move |ch| {
            let mut prog = ProgressReporter::start(
                &ch,
                "disk_treemap",
                t("Scansione Treemap in corso..."),
            )
            .with_generation(generation);
            ch.send(Msg::Log(tf(
                "▶ {} {}",
                &[&t("Scansione treemap di"), &path.display()],
            )));
            let scan = pages_disk_treemap::scan_directory_recursive(&path, cancel.as_ref(), &mut |n| {
                prog.status(tf("{} elementi esaminati", &[&n]))
            });
            if cancel.load(AtomicOrdering::Relaxed) {
                prog.finish(t("Scansione annullata"));
                return;
            }
            ch.send(Msg::TreemapScanDone {
                path: path.clone(),
                scan,
                generation,
            });
            ch.send(Msg::Log(format!("✔ {}", t("Treemap aggiornata."))));
            prog.finish(t("Treemap pronta"));
        });
    }

    /// Naviga dentro una cartella: usa la cache se già scansionata.
    fn treemap_navigate(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.treemap_scans.contains_key(&path) {
            self.treemap_cancel.store(true, AtomicOrdering::Relaxed);
            self.treemap_generation = self.treemap_generation.wrapping_add(1);
            // Nessun nuovo worker: il vecchio `finish` verrebbe scartato dal
            // filtro sulla generation, quindi chiudere qui la barra.
            self.progress.remove("disk_treemap");
            self.treemap_current = path;
            self.treemap_selected = None;
        } else {
            self.treemap_scan(ctx, path);
        }
    }

    fn treemap_parent(&self) -> Option<PathBuf> {
        self.treemap_current.parent().map(|p| p.to_path_buf())
    }

    fn treemap_crumbs(&self) -> Vec<(String, PathBuf)> {
        let home = PathBuf::from(util::home());
        pages_disk_treemap::breadcrumbs(&self.treemap_current)
            .into_iter()
            .map(|p| (pages_disk_treemap::short_name(&p, &home), p))
            .collect()
    }

    fn treemap_open_in_manager(&self, ctx: &egui::Context, path: PathBuf, is_dir: bool) {
        let target = if is_dir {
            path.clone()
        } else {
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| path.clone())
        };
        self.bg(ctx, move |ch| {
            match std::process::Command::new("xdg-open").arg(&target).spawn() {
                Ok(_) => ch.send(Msg::Log(format!("✔ {}", t("Cartella aperta nel file manager.")))),
                Err(e) => ch.send(Msg::Log(format!("✘ {}: {e}", t("Impossibile aprire il file manager")))),
            }
        });
    }

    fn treemap_ask_trash(&mut self, path: PathBuf, is_dir: bool) {
        let parent = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/"));
        let what = if is_dir { t("cartella") } else { t("file") };
        let msg = format!(
            "{}\n{}: {}\n{}",
            t("Spostare nel cestino?"),
            what,
            path.display(),
            t("Potrai ripristinarlo dal cestino del file manager.")
        );
        self.confirm = Some((
            msg,
            Box::new(move |ch| {
                let mut res = std::process::Command::new("gio")
                    .arg("trash")
                    .arg("--")
                    .arg(&path)
                    .output();
                if res.as_ref().map(|o| !o.status.success()).unwrap_or(true) {
                    res = std::process::Command::new("trash-put").arg("--").arg(&path).output();
                }
                match res {
                    Ok(o) if o.status.success() => {
                        ch.send(Msg::Log(format!("✔ {} {}", t("Spostato nel cestino:"), path.display())));
                        ch.send(Msg::TreemapInvalidate(parent));
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                        ch.send(Msg::Log(format!(
                            "✘ {}: {}",
                            t("Spostamento nel cestino non riuscito"),
                            if err.is_empty() { t("errore sconosciuto") } else { err.as_str() }
                        )));
                    }
                    Err(e) => ch.send(Msg::Log(format!(
                        "✘ {}: {e}",
                        t("Spostamento nel cestino non riuscito")
                    ))),
                }
            }),
        ));
    }

    fn treemap_ask_delete(&mut self, path: PathBuf, is_dir: bool) {
        let parent = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/"));
        let what = if is_dir { t("cartella") } else { t("file") };
        let msg = format!(
            "{}\n{}: {}\n{}",
            t("Eliminare definitivamente?"),
            what,
            path.display(),
            t("ATTENZIONE: l'operazione non è reversibile.")
        );
        self.confirm = Some((
            msg,
            Box::new(move |ch| {
                let mut cmd = std::process::Command::new("rm");
                if is_dir {
                    cmd.arg("-rf");
                } else {
                    cmd.arg("-f");
                }
                match cmd.arg("--").arg(&path).output() {
                    Ok(o) if o.status.success() => {
                        ch.send(Msg::Log(format!("✔ {} {}", t("Eliminato:"), path.display())));
                        ch.send(Msg::TreemapInvalidate(parent));
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                        ch.send(Msg::Log(format!(
                            "✘ {}: {}",
                            t("Eliminazione non riuscita"),
                            if err.is_empty() { t("errore sconosciuto") } else { err.as_str() }
                        )));
                    }
                    Err(e) => ch.send(Msg::Log(format!("✘ {}: {e}", t("Eliminazione non riuscita")))),
                }
            }),
        ));
    }

    fn ui_disk_analyzer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Analizzatore Spazio Disco"));
        ui::hint(
            ui,
            t("Esplora le cartelle del file system per individuare a colpo d'occhio quali file o directory occupano più spazio."),
        );

        ui::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t("Percorso da analizzare:")).strong());
                ui.add(egui::TextEdit::singleline(&mut self.disk_analyzer_path).desired_width(ui::fit_width(ui, 400.0, 140.0)));

                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Sfoglia..."))).clicked() {
                    let ctx_clone = ctx.clone();
                    let tx_clone = self.tx.clone();
                    std::thread::spawn(move || {
                        let out = std::process::Command::new("zenity")
                            .args(["--file-selection", "--directory"])
                            .output();
                        match out {
                            Ok(res) if res.status.success() => {
                                if let Ok(dir) = String::from_utf8(res.stdout) {
                                    let dir = dir.trim().to_string();
                                    if !dir.is_empty() {
                                        let _ = tx_clone.send(Msg::DiskAnalyzerPath(dir));
                                        ctx_clone.request_repaint();
                                    }
                                }
                            }
                            Ok(_) => {} // annulla: nessun cambiamento
                            Err(_) => {
                                let _ = tx_clone.send(Msg::Log(t("Nessun selettore di file trovato: installa zenity (sudo apt install zenity)").to_string()));
                                ctx_clone.request_repaint();
                            }
                        }
                    });
                }

                if ui::primary_button(ui, t("Scansiona"), self.action_enabled()).clicked() {
                    let path = util::expand_path(&self.disk_analyzer_path);
                    self.spawn(ctx, move |ch| {
                        let mut prog = ProgressReporter::start(
                            &ch,
                            "disk_analyzer",
                            t("Scansione disco in corso..."),
                        );
                        ch.send(Msg::Log(format!("▶ Scansione di {path} in corso...")));
                        let items = pages_disk_analyzer::scan_directory_with(&path, |i, name| {
                            prog.status(tf("{} — {} elementi", &[&name, &i]))
                        });
                        ch.send(Msg::DiskAnalyzerItems(items));
                        ch.send(Msg::Log("✔ Scansione completata.".into()));
                        prog.finish(t("Scansione completata"));
                    });
                }

                if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Analisi approfondita"))).clicked() {
                    let path = util::expand_path(&self.disk_analyzer_path);
                    self.disk_before_free = pages_disk_analyzer::available_bytes_for_path(&path);
                    self.disk_before_path = self.disk_before_free.map(|_| path.clone());
                    self.spawn(ctx, move |ch| {
                        let mut prog = ProgressReporter::start(
                            &ch,
                            "disk_analyzer",
                            t("Analisi approfondita del disco in corso..."),
                        );
                        ch.send(Msg::Log(format!("▶ Analisi cache e file grandi recenti in {path}...")));
                        let deep = pages_disk_analyzer::scan_deep_analysis_with(&path, 100, 14, |name| {
                            prog.status(tf("Analisi di {}", &[&name]))
                        });
                        ch.send(Msg::DiskDeepScan(deep));
                        ch.send(Msg::Log("✔ Analisi approfondita completata.".into()));
                        prog.finish(t("Analisi approfondita completata"));
                    });
                }

                if self.busy {
                    ui.spinner();
                }
            });
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            if self.disk_analyzer_items.is_empty() {
                ui.label(t("Nessun risultato. Inserisci un percorso e premi 'Scansiona'."));
            } else {
                let total_size: u64 = self.disk_analyzer_items.iter().map(|i| i.size).sum();
                ui.label(egui::RichText::new(tf("Spazio totale rilevato: {}", &[&pages_disk_analyzer::fmt_bytes(total_size)])).strong().color(ui::accent_text()));
                ui.add_space(3.0);
                let items = self.disk_analyzer_items.clone();
                egui::ScrollArea::vertical().max_height(310.0).show(ui, |ui| {
                    for item in items {
                        ui.horizontal(|ui| {
                            let icon = if item.is_dir { "📁" } else { "📄" };
                            ui.label(icon);
                            if ui.link(&item.name).clicked() && item.is_dir {
                                self.disk_analyzer_path = item.path.clone();
                                let path = item.path.clone();
                                self.spawn(ctx, move |ch| {
                                    let mut prog = ProgressReporter::start(
                                        &ch,
                                        "disk_analyzer",
                                        t("Scansione cartella in corso..."),
                                    );
                                    let items = pages_disk_analyzer::scan_directory_with(&path, |i, name| {
                                        prog.status(tf("{} — {} elementi", &[&name, &i]))
                                    });
                                    ch.send(Msg::DiskAnalyzerItems(items));
                                    prog.finish(t("Scansione completata"));
                                });
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    egui::RichText::new(pages_disk_analyzer::fmt_bytes(item.size))
                                        .strong()
                                        .color(ui::text_dim()),
                                );
                            });
                        });
                        ui.add_space(2.0);
                    }
                });
            }
        });
        
ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Cartelle di cache più pesanti")).strong().size(16.0).color(ui::accent_text()));
            ui.add_space(3.0);
            if self.disk_deep.heavy_caches.is_empty() {
                ui.label(t("Nessuna cartella di cache analizzata. Premi 'Analisi approfondita'."));
            } else {
                let max_sz = self.disk_deep.heavy_caches.iter().map(|c| c.size).max().unwrap_or(1).max(1) as f32;
                for c in &self.disk_deep.heavy_caches {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(tr(&c.label)).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(pages_disk_analyzer::fmt_bytes(c.size)).strong().color(ui::accent_text()));
                        });
                    });
                    {
                        let fill = ui::ACCENT;
                        let text_color = ui::progress_text_color(fill);
                        ui.scope(|ui| {
                            ui.visuals_mut().override_text_color = Some(text_color);
                            ui.visuals_mut().selection.stroke.color = text_color;
                            ui.add(egui::ProgressBar::new((c.size as f32 / max_sz).clamp(0.0, 1.0)).fill(fill));
                        });
                    }
                    ui.label(egui::RichText::new(&c.path).color(ui::text_dim()));
                    ui.add_space(3.0);
                }
            }
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("File grandi recenti")).strong().size(16.0).color(ui::accent_text()));
            ui.add_space(3.0);
            if self.disk_deep.recent_big_files.is_empty() {
                ui.label(t("Nessun file grande recente trovato (ultimi 14 giorni, >= 100 MB)."));
            } else {
                for f in &self.disk_deep.recent_big_files {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&f.name).strong());
                        ui.label(egui::RichText::new(format!("({}d)", f.days_ago)).color(ui::text_dim()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(pages_disk_analyzer::fmt_bytes(f.size)).strong().color(ui::semantic_text(ui::WARN)));
                        });
                    });
                    ui.label(egui::RichText::new(&f.path).color(ui::text_dim()));
                    ui.add_space(2.0);
                }
            }
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Confronto spazio Prima / Dopo")).strong().size(16.0).color(ui::accent_text()));
                if let Some(before) = self.disk_before_free {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Misura spazio Dopo"))).clicked() {
                            let path = self.disk_before_path.clone().unwrap_or_else(|| util::expand_path(&self.disk_analyzer_path));
                            if let Some(comp) = pages_disk_analyzer::build_space_comparison(&path, before) {
                                self.disk_comparison = Some(comp);
                            }
                        }
                    });
                }
            });
            ui.add_space(3.0);
            if let Some(comp) = &self.disk_comparison {
                ui.label(format!("{} {}", t("Spazio libero prima:"), pages_disk_analyzer::fmt_bytes(comp.before_free)));
                ui.label(format!("{} {}", t("Spazio libero dopo:"), pages_disk_analyzer::fmt_bytes(comp.after_free)));
                let diff_txt = if comp.net_freed >= 0 {
                    format!("+{}", pages_disk_analyzer::fmt_bytes(comp.net_freed as u64))
                } else {
                    format!("-{}", pages_disk_analyzer::fmt_bytes(comp.net_freed.unsigned_abs() as u64))
                };
                ui.colored_label(
                    if comp.net_freed >= 0 { ui::GOOD } else { ui::WARN },
                    format!("{} {diff_txt}", t("Differenza netta:")),
                );
            } else if let Some(before) = self.disk_before_free {
                ui.label(format!("{} {}", t("Spazio libero prima:"), pages_disk_analyzer::fmt_bytes(before)));
                ui.label(t("Premi 'Misura spazio Dopo' dopo aver liberato o rimosso file."));
            } else {
                ui.label(t("Avvia 'Analisi approfondita' per registrare lo spazio iniziale."));
            }
        });        
        
    }

    fn ui_disk_treemap(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use pages_disk_treemap as tm;

        ui::title(ui, t("Analizzatore Disco"));
        ui::hint(
            ui,
            t("Scansiona una cartella e esplora lo spazio con una mappa di rettangoli proporzionali: clic su una cartella per entrare, tasto destro per aprire o rimuovere elementi."),
        );

        if let Some(path) = self.treemap_pending_scan.take() {
            self.treemap_scan(ctx, path);
        }

        let scanning = self
            .progress
            .get("disk_treemap")
            .map(|p| !p.done && p.visible())
            .unwrap_or(false);
        let scan = self.treemap_scans.get(&self.treemap_current).cloned();

        ui::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t("Cartella:")).strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.treemap_input).desired_width(ui::fit_width(ui, 420.0, 140.0)),
                );
                if ui.button(t("Sfoglia...")).clicked() {
                    let ctx2 = ctx.clone();
                    let tx2 = self.tx.clone();
                    std::thread::spawn(move || {
                        match std::process::Command::new("zenity")
                            .args(["--file-selection", "--directory"])
                            .output()
                        {
                            Ok(out) if out.status.success() => {
                                if let Ok(dir) = String::from_utf8(out.stdout) {
                                    let dir = dir.trim().to_string();
                                    if !dir.is_empty() {
                                        let _ = tx2.send(Msg::TreemapPath(dir));
                                        ctx2.request_repaint();
                                    }
                                }
                            }
                            Ok(_) => {} // annulla: nessun cambiamento
                            Err(_) => {
                                let _ = tx2.send(Msg::Log(t("Nessun selettore di file trovato: installa zenity (sudo apt install zenity)").to_string()));
                                ctx2.request_repaint();
                            }
                        }
                    });
                }
                if ui::primary_button(ui, t("Scansiona"), !scanning).clicked() {
                    let path = PathBuf::from(util::expand_path(&self.treemap_input));
                    self.treemap_scan(ctx, path);
                }
                if ui
                    .add_enabled(!scanning, egui::Button::new(t("Aggiorna")))
                    .clicked()
                {
                    let path = self.treemap_current.clone();
                    self.treemap_scan(ctx, path);
                }
                let can_go_up = self.treemap_parent().is_some();
                if ui
                    .add_enabled(can_go_up, egui::Button::new(t("Torna su")))
                    .clicked()
                {
                    if let Some(parent) = self.treemap_parent() {
                        self.treemap_navigate(ctx, parent);
                    }
                }
                if scanning {
                    ui.spinner();
                }
            });

            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t("Percorso:")).color(ui::text_dim()));
                for (i, (label, path)) in self.treemap_crumbs().into_iter().enumerate() {
                    if i > 0 {
                        ui.label(egui::RichText::new("›").color(ui::text_dim()));
                    }
                    if ui.link(&label).clicked() {
                        self.treemap_navigate(ctx, path);
                    }
                }
            });

            if scanning {
                if let Some(p) = self.progress.get("disk_treemap").cloned() {
                    ui.add_space(3.0);
                    ui.add(
                        ui::progress_bar(p.fraction.unwrap_or(0.0))
                            .animate(true)
                            .text(tr(&p.detail)),
                    );
                }
            }
        });

        ui.add_space(4.0);

        ui::card(ui, |ui| {
            if let Some(s) = &scan {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            t("Totale:"),
                            tm::format_size(s.total_bytes)
                        ))
                        .strong()
                        .color(ui::accent_text()),
                    );
                    ui.label(
                        egui::RichText::new(tf(
                            "{} file · {} cartelle · {} errori",
                            &[&s.files, &s.dirs, &s.errors]
                        ))
                        .color(ui::text_dim()),
                    );
                });
            } else if scanning {
                ui.label(t("Scansione in corso..."));
            } else {
                ui.label(t("Nessuna scansione: scegli una cartella e premi 'Scansiona'."));
            }
            ui.add_space(3.0);

            ui.horizontal_wrapped(|ui| {
                for cat in tm::DiskCategory::ALL {
                    let (square, _) =
                        ui.allocate_exact_size(egui::vec2(13.0, 13.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(square, 3.0, ui::treemap_fill(*cat, false));
                    ui.label(
                        egui::RichText::new(t(cat.label())).size(13.0).color(ui::text_dim()),
                    );
                    ui.add_space(2.0);
                }
            });
            ui.add_space(3.0);

            let map_h = ui.available_height().min(430.0).max(180.0);
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), map_h),
                egui::Sense::hover(),
            );
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 10.0, ui::card_bg(ctx));
            painter.rect_stroke(rect, 10.0, egui::Stroke::new(1.0_f32, ui::card_border(ctx)));

            if let Some(s) = &scan {
                let area = egui::Rect::from_min_max(
                    egui::Pos2::new(rect.left() + 10.0, rect.top() + 10.0),
                    egui::Pos2::new(rect.right() - 10.0, rect.bottom() - 10.0),
                );
                let sizes: Vec<u64> = s.entries.iter().map(|e| e.size).collect();
                let cells = tm::squarify(
                    &sizes,
                    (area.min.x, area.min.y, area.width(), area.height()),
                    3.0,
                );
                let total = s.total_bytes.max(1);

                for cell in cells {
                    let Some(entry) = s.entries.get(cell.index) else { continue };
                    let r = egui::Rect::from_min_size(
                        egui::Pos2::new(cell.x, cell.y),
                        egui::vec2(cell.w, cell.h),
                    );
                    if r.width() < 3.0 || r.height() < 3.0 {
                        continue;
                    }
                    let id = ui.id().with(("disk_treemap_item", entry.path.clone()));
                    let resp = ui.interact(r, id, egui::Sense::click());
                    let selected = self.treemap_selected.as_ref() == Some(&entry.path);
                    let hovered = resp.hovered();

                    painter.rect_filled(
                        r,
                        6.0,
                        ui::treemap_fill(entry.category, hovered || selected),
                    );
                    painter.rect_stroke(r, 6.0, ui::treemap_stroke(entry.category, selected));

                    if r.width() > 76.0 && r.height() > 38.0 {
                        let max_chars = ((r.width() - 14.0) / 7.6).max(4.0) as usize;
                        let name: String = entry.name.chars().take(max_chars).collect();
                        let text_col = ui::text_primary();
                        let mut pos = egui::Pos2::new(r.left() + 7.0, r.top() + 6.0);
                        painter.text(
                            pos,
                            egui::Align2::LEFT_TOP,
                            name,
                            egui::FontId::proportional(13.0),
                            text_col,
                        );
                        pos.y += 17.0;
                        painter.text(
                            pos,
                            egui::Align2::LEFT_TOP,
                            tm::format_size(entry.size),
                            egui::FontId::proportional(11.5),
                            ui::alpha(text_col, 205),
                        );
                    }

                    let pct = entry.size as f64 * 100.0 / total as f64;
                    resp.clone().on_hover_ui_at_pointer(|tip| {
                        tip.set_max_width(ui::fit_width(ui, 420.0, 220.0));
                        tip.label(egui::RichText::new(&entry.name).strong());
                        tip.label(
                            egui::RichText::new(entry.path.display().to_string())
                                .size(12.0)
                                .color(ui::text_dim()),
                        );
                        tip.label(format!("{} {}", t("Dimensione:"), tm::format_exact(entry.size)));
                        tip.label(format!("{pct:.2}% {}", t("del totale")));
                        tip.label(if entry.is_dir {
                            t("Cartella — clic per entrare")
                        } else {
                            t("File — clic per selezionare")
                        });
                    });

                    if resp.clicked() {
                        if entry.is_dir {
                            self.treemap_navigate(ctx, entry.path.clone());
                        } else {
                            self.treemap_selected = Some(entry.path.clone());
                        }
                    }
                    if resp.secondary_clicked() {
                        self.treemap_selected = Some(entry.path.clone());
                    }

                    let path = entry.path.clone();
                    let is_dir = entry.is_dir;
                    resp.context_menu(|menu| {
                        if menu.button(t("Apri nel file manager")).clicked() {
                            menu.close_menu();
                            self.treemap_open_in_manager(ctx, path.clone(), is_dir);
                        }
                        if menu.button(t("Sposta nel cestino")).clicked() {
                            menu.close_menu();
                            self.treemap_ask_trash(path.clone(), is_dir);
                        }
                        if menu.button(t("Elimina definitivamente")).clicked() {
                            menu.close_menu();
                            self.treemap_ask_delete(path.clone(), is_dir);
                        }
                    });
                }
            } else {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if scanning {
                        t("Scansione in corso...")
                    } else {
                        t("Nessun dato da mostrare")
                    },
                    egui::FontId::proportional(18.0),
                    ui::text_dim(),
                );
            }
        });

        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if let Some(sel) = self.treemap_selected.clone() {
                    ui.label(egui::RichText::new(t("Selezionato:")).strong());
                    ui.label(
                        egui::RichText::new(sel.display().to_string()).color(ui::text_dim()),
                    );
                    let is_dir = sel.is_dir();
                    if ui.button(t("Apri nel file manager")).clicked() {
                        self.treemap_open_in_manager(ctx, sel.clone(), is_dir);
                    }
                    if ui.button(t("Sposta nel cestino")).clicked() {
                        self.treemap_ask_trash(sel.clone(), is_dir);
                    }
                    if ui.button(t("Elimina definitivamente")).clicked() {
                        self.treemap_ask_delete(sel.clone(), is_dir);
                    }
                } else {
                    ui.label(
                        egui::RichText::new(t("Nessun elemento selezionato: clic su un rettangolo."))
                            .color(ui::text_dim()),
                    );
                }
            });
        });

        ui.add_space(4.0);
        ui::card(ui, |ui| {
            ui.label(
                egui::RichText::new(t("Top 20 file più pesanti"))
                    .strong()
                    .size(18.0)
                    .color(ui::accent_text()),
            );
            ui.label(
                egui::RichText::new(t("File dell'intero sottoalbero, ordinati per dimensione."))
                    .size(13.0)
                    .color(ui::text_dim()),
            );
            ui.add_space(3.0);
            if let Some(s) = &scan {
                if s.top_files.is_empty() {
                    ui.label(t("Nessun file trovato in questa cartella."));
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(210.0)
                        .show(ui, |ui| {
                            for (i, f) in s.top_files.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{:>2}.", i + 1));
                                    let (square, _) = ui.allocate_exact_size(
                                        egui::vec2(10.0, 10.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter()
                                        .rect_filled(square, 2.0, ui::treemap_fill(f.category, false));
                                    if ui.link(&f.name).clicked() {
                                        self.treemap_selected = Some(f.path.clone());
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(tm::format_size(f.size))
                                                    .strong()
                                                    .color(ui::accent_text()),
                                            );
                                        },
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(f.path.display().to_string())
                                        .size(12.0)
                                        .color(ui::text_dim()),
                                );
                                ui.add_space(2.0);
                            }
                        });
                }
            } else {
                ui.label(t("In attesa della scansione..."));
            }
        });
    }

    fn ui_system_editor(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Gestore File di Sistema"));
        ui::hint(
            ui,
            t("Modifica file di configurazione critici di sistema (/etc/fstab, /etc/hosts, /etc/environment). Viene creato automaticamente un backup (.bak) e validata la sintassi prima di salvare per evitare blocchi del boot."),
        );

        ui.add_space(3.0);
        ui::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("/etc/fstab").clicked() {
                    self.system_editor.load_file("/etc/fstab");
                }
                if ui.button("/etc/hosts").clicked() {
                    self.system_editor.load_file("/etc/hosts");
                }
                if ui.button("/etc/environment").clicked() {
                    self.system_editor.load_file("/etc/environment");
                }
            });

            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tf("File in modifica: {}", &[&self.system_editor.selected_path])).strong().color(ui::accent_text()));
            });

            ui.add_space(3.0);
            egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.system_editor.content)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui::fit_width(ui, 640.0, 240.0))
                        .desired_rows(18), // <-- Corretto da .rows(18) a .desired_rows(18)
                );
            });

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
            let can_save = self.action_enabled() && !self.system_editor.saving;
                if ui::primary_button(ui, t("Salva con Backup e Validazione"), can_save).clicked() {
                    let path = self.system_editor.selected_path.clone();
                    let content = self.system_editor.content.clone();
                    let tx = self.tx.clone();
                    let ctx_clone = ctx.clone();
                    self.system_editor.saving = true;
                    self.system_editor.status_msg = t("Salvataggio in corso...").to_string();
                    self.system_editor.save_file_async(path, content, tx, ctx_clone);
                }

                if ui.button(t("Ricarica originale")).clicked() {
                    let path = self.system_editor.selected_path.clone();
                    self.system_editor.load_file(&path);
                }
            });

            if !self.system_editor.status_msg.is_empty() {
                ui.add_space(3.0);
                ui.label(egui::RichText::new(&self.system_editor.status_msg).color(ui::text_dim()));
            }
        });
    }


fn ui_safe_optimize(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t(safe_optimize::TAB_TITLE));
        ui::hint(
            ui,
            t("Percorso guidato per principianti: prima ti mostra cosa farà senza fare danni, poi ti dice cosa ha fatto. Non tocca password, preferiti, cronologia, cookie o Cestino."),
        );

        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("1. Cosa farà (anteprima sicura)")).strong().size(16.0).color(ui::accent_text()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Aggiorna anteprima"))).clicked() {
                        self.safe_preview = safe_optimize::preview_steps();
                        self.safe_report = None;
                    }
                });
            });

            ui.add_space(3.0);
            for step in &self.safe_preview {
                ui.horizontal(|ui| {
                    let status_col = if step.will_run { ui::GOOD } else { ui::WARN };
                    let status_txt = if step.will_run { "✔" } else { "⚠" };
                    ui.colored_label(status_col, status_txt);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(t(step.title)).strong());
                            if let Some(sz) = step.size {
                                ui.label(egui::RichText::new(cleaner::fmt_bytes(sz)).color(ui::accent_text()));
                            }
                        });
                        ui.label(egui::RichText::new(t(step.detail)).color(ui::text_dim()));
                        if let Some(note) = step.note {
                            ui.colored_label(ui::semantic_text(ui::WARN), t(note));
                        }
                    });
                });
                ui.add_space(3.0);
            }

            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(format!(
                    "{}: {}",
                    t("Spazio stimato recuperabile ora"),
                    cleaner::fmt_bytes(safe_optimize::estimated_bytes())
                ))
                .strong()
                .color(ui::accent_text()),
            );
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui::primary_button(ui, t("Ottimizza ora in sicurezza"), self.action_enabled()).clicked() {
                self.safe_preview = safe_optimize::preview_steps();
                self.safe_report = None;
                self.spawn(ctx, |ch| {
                    let mut prog = ProgressReporter::start(
                        &ch,
                        "safe_optimize",
                        t("Ottimizzazione sicura in corso..."),
                    );
                    ch.send(Msg::Log("▶ Avvio ottimizzazione sicura per principianti...".into()));
                    let rep = safe_optimize::run_safe_optimize_report_with(|cur, total, phase| {
                        prog.step(cur, total, phase)
                    });
                    for line in rep.clone().into_log_lines() {
                        ch.send(Msg::Log(line));
                    }
                    ch.send(Msg::SafeOptimizeReport(rep));
                    prog.status(t("Verifica dello stato del sistema..."));
                    ch.send(Msg::Sizes(cleaner::scan_with(|c, t, l| prog.step(c, t, crate::i18n::tr(l)))));
                    ch.send(Msg::Checks(health::run_checks()));
                    prog.finish(t("Ottimizzazione completata"));
                });
            }
            if self.busy {
                ui.spinner();
            }
        });

        ui.add_space(4.0);
self.draw_safe_optimize_summary(ui);
    }
fn draw_safe_optimize_summary(&self, ui: &mut egui::Ui) {
        ui::card(ui, |ui| {
            ui.label(
                egui::RichText::new(t("2. Cosa ha fatto"))
                    .strong()
                    .size(16.0)
                    .color(ui::accent_text()),
            );
            ui.add_space(3.0);

            let Some(report) = &self.safe_report else {
                if self.safe_log.is_empty() {
                    ui.label(
                        egui::RichText::new(t("Nessuna ottimizzazione avviata ancora in questa sessione."))
                            .color(ui::text_dim()),
                    );
                } else {
                    for line in &self.safe_log {
                        ui.label(line);
                    }
                }
                return;
            };

            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{}: {}",
                        t("Spazio liberato"),
                        cleaner::fmt_bytes(report.total_freed)
                    ))
                    .strong()
                    .size(18.0)
                    .color(ui::semantic_text(ui::GOOD)),
                );
                ui.add_space(5.0);
                ui.colored_label(
                    ui::GOOD,
                    format!("✔ {}: {}", t("Completate"), report.done_count()),
                );
                ui.colored_label(
                    ui::WARN,
                    format!("⚠ {}: {}", t("Saltate"), report.skipped_count()),
                );
                if report.warning_count() > 0 {
                    ui.colored_label(
                        ui::BAD,
                        format!("! {}: {}", t("Avvisi"), report.warning_count()),
                    );
                }
            });

            ui.add_space(3.0);
            ui.separator();
            ui.add_space(3.0);

            for entry in &report.entries {
                let (icon, col) = match entry.status {
                    safe_optimize::StepStatus::Done => ("✔", ui::GOOD),
                    safe_optimize::StepStatus::Skipped => ("⚠", ui::WARN),
                    safe_optimize::StepStatus::Warning => ("⚠", ui::BAD),
                };
                ui.horizontal(|ui| {
                    ui.colored_label(col, egui::RichText::new(icon).strong());
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(t(entry.title)).strong());
                            if let Some(bytes) = entry.freed_bytes {
                                if bytes > 0 {
                                    ui.label(
                                        egui::RichText::new(cleaner::fmt_bytes(bytes))
                                            .color(ui::accent_text()),
                                    );
                                }
                            }
                        });
                        if !entry.detail.is_empty() {
                            ui.label(egui::RichText::new(&entry.detail).color(ui::text_dim()));
                        }
                    });
                });
                ui.add_space(3.0);
            }
        });
    }

}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain(ctx);
        // Il badge dell'intestazione segue sempre la pagina attiva, anche nelle sottoschede.
        ui::set_current_page(self.page);
        // Applica il tema solo quando cambia: in precedenza il set_visuals di
        // ogni frame annullava gradienti, rounding e colori di apply_theme.
        if ctx.style().visuals.dark_mode != self.cfg.dark_mode {
            ui::apply_theme(ctx, self.cfg.dark_mode);
        }
        let sidebar_bg = ui::sidebar_bg(ctx);
        let content_bg = ui::content_bg(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_quit = true;
        }

        if self.last_refresh.elapsed() > Duration::from_secs(1) {
            self.info.refresh();
            self.last_refresh = Instant::now();
            let cpu = (self.info.sys.global_cpu_usage() / 100.0).clamp(0.0, 1.0);
            let ram = (self.info.sys.used_memory() as f32 / self.info.sys.total_memory().max(1) as f32).min(1.0);
            for (h, v) in [(&mut self.cpu_hist, cpu), (&mut self.ram_hist, ram)] {
                h.push_back(v);
                while h.len() > ui::HIST_LEN {
                    h.pop_front();
                }
            }
        }
        let live = matches!(self.page, Page::Dashboard | Page::Info | Page::Temps | Page::Procs | Page::Zram);
        if live {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if self.page == Page::Procs && !self.proc_pending && self.last_proc.elapsed() > Duration::from_secs(2) {
            self.proc_pending = true;
            self.last_proc = Instant::now();
            self.bg(ctx, |ch| ch.send(Msg::Procs(pages_sys::read_procs())));
        }

        let mut next: Option<Page> = None;
        let sidebar_locked = self.operation_in_progress();
        // Sotto ~1040 px la sidebar diventa una barra di icone: il contenuto
        // resta leggibile anche a 800×600 e affiancato a metà schermo.
        let compact_sidebar = ctx.screen_rect().width() < 1040.0;
        let sidebar_w = if compact_sidebar { 72.0 } else { 260.0 };
        let sidebar_inner = if compact_sidebar { 6.0 } else { 10.0 };
        egui::SidePanel::left("side")
            .exact_width(sidebar_w)
            .resizable(false)
            .frame(egui::Frame::default().fill(sidebar_bg).inner_margin(egui::Margin::same(sidebar_inner)))
            .show(ctx, |ui| {
                // Testata: logo sempre, testo solo con sidebar estesa.
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let (logo, _) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::hover());
                    ui.put(
                        logo,
                        egui::Image::new(egui::include_image!("../assets/debutility.png"))
                            .max_size(egui::vec2(34.0, 34.0)),
                    );
                    if !compact_sidebar {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                ui.label(egui::RichText::new(t("Deb")).size(27.0).strong().color(ui::text_primary()));
                                ui.label(egui::RichText::new(t("Utility")).size(27.0).strong().color(ui::accent_text()));
                            });
                            ui.label(
                                egui::RichText::new(format!("v{}", APP_VERSION))
                                    .size(12.5)
                                    .strong()
                                    .color(ui::text_dim()),
                            );
                            ui.label(egui::RichText::new(t("Ottimizzatore di sistema")).size(12.0).color(ui::text_dim()));
                        });
                    }
                });
                let (line, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 2.0), egui::Sense::hover());
                ui::accent_strip(ui.painter(), line, ui::alpha(ui::ACCENT, 130), egui::Color32::TRANSPARENT);
                ui.add_space(9.0);
                if sidebar_locked {
                    let warning = t("⚠ Operazione in corso: attendi il completamento prima di cambiare scheda.");
                    if compact_sidebar {
                        ui.label(egui::RichText::new("⚠").size(18.0).color(ui::semantic_text(ui::WARN)))
                            .on_hover_text(warning);
                    } else {
                        ui.add(egui::Label::new(
                            egui::RichText::new(warning).size(12.5).color(ui::semantic_text(ui::WARN)),
                        ).wrap());
                    }
                    ui.add_space(4.0);
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for (group, entries) in MENU {
                        if !group.is_empty() {
                            if compact_sidebar {
                                ui.add_space(8.0);
                            } else {
                                ui.add_space(12.0);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 7.0;
                                    let (dot, _) = ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
                                    ui.painter().circle_filled(dot.center(), 3.0, ui::ACCENT);
                                    ui.label(egui::RichText::new(t(*group)).size(12.0).strong().color(ui::text_dim()));
                                });
                                ui.add_space(2.0);
                            }
                        }
                        for (p, name) in entries.iter() {
                            let label = t(*name);
                            // La sidebar usa un'etichetta breve; il tooltip mostra il nome completo.
                            let full_label = if *p == Page::TelemetryGuard {
                                t("Privacy & Telemetry Guard")
                            } else {
                                label
                            };
                            if ui::sidebar_item(ui, *p, label, full_label, self.page == *p, !sidebar_locked) {
                                next = Some(*p);
                            }
                        }
                    }
                });
            });

        if let Some(p) = next {
            self.enter(ctx, p);
        }

        self.queue_ui(ctx);

        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::default()
                    .fill(sidebar_bg)
                    .inner_margin(egui::Margin::symmetric(12.0, 7.0))
                    .stroke(egui::Stroke::new(1.0_f32, if ui::is_dark() { egui::Color32::from_white_alpha(12) } else { egui::Color32::from_rgb(203, 213, 225) })),
            )
            .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if !self.progress_status_ui(ui) && self.busy {
                    ui.spinner();
                }
                if !self.job_queue.is_empty() {
                    ui.label(
                        egui::RichText::new(tf("In attesa: {}", &[&self.job_queue.len()]))
                            .size(12.0)
                            .color(ui::text_dim()),
                    );
                }
                if util::is_root() {
                    let admin_text = format!(" [⚡ {}] ", t("MODALITÀ AMMINISTRATORE"));
                    ui.colored_label(crate::ui::semantic_text(crate::ui::WARN), admin_text);
                }
                let tail = self.log.get(self.status_from..).unwrap_or("");
                let last = tail.trim_end().rsplit('\n').next().unwrap_or("");
                let last = if last.is_empty() { t("Pronto") } else { last };
                ui.add(
                    egui::Label::new(egui::RichText::new(last).color(crate::ui::text_dim())).wrap(),
                );
            });
        });

        let mut decision: Option<bool> = None;
        if let Some((text, _)) = &self.confirm {
            egui::Window::new(t("Conferma"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(text);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(t("Conferma")).clicked() {
                            decision = Some(true);
                        }
                        if ui.button(t("Annulla")).clicked() {
                            decision = Some(false);
                        }
                    });
                });
        }
        match decision {
            Some(true) => {
                // `spawn()` avvia subito il job se la slot è libera, altrimenti
                // lo accoda: la conferma data durante un'operazione non va persa.
                if let Some((_, job)) = self.confirm.take() {
                    self.spawn(ctx, job);
                }
            }
            Some(false) => {
                // Feedback esplicito: annullare una conferma non applica alcuna modifica.
                self.log.push_str(t("Operazione annullata: nessuna modifica è stata applicata."));
                self.log.push('\n');
                if self.page == Page::RestoreHistory {
                    self.restore_history_msg = Some((
                        t("Operazione annullata: nessuna modifica è stata applicata.").to_string(),
                        false,
                    ));
                }
                self.confirm = None;
            }
            None => {}
        }

        if self.show_quit {
            let (mut close, mut stay) = (false, false);
            egui::Window::new(t("Chiudere DebUtility?"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_max_width(460.0);
                    if self.busy {
                        ui.colored_label(ui::semantic_text(ui::WARN), t("Un'operazione è ancora in corso (pulizia, aggiornamento o altro). Se chiudi ora potrebbe interrompersi a metà e lasciare il sistema in uno stato incompleto."));
                        ui.label(t("Ti consiglio di aspettare che finisca."));
                    } else {
                        ui.label(t("Vuoi davvero chiudere DebUtility?"));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new(egui::RichText::new(t("Chiudi")).strong().color(ui::on_fill_text(ui::BAD))).fill(ui::BAD))
                            .clicked()
                        {
                            close = true;
                        }
                        if ui.button(t("Continua a usare DebUtility")).clicked() {
                            stay = true;
                        }
                    });
                });
            if close {
                self.allow_close = true;
                self.show_quit = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if stay {
                self.show_quit = false;
            }
        }

        let mut goto: Option<Page> = None;
        let content_margin = if ui::compact_pages() { 10.0 } else { 14.0 };
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(content_bg).inner_margin(egui::Margin::same(content_margin)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui::begin_page(ui);
                self.progress_ui(ui);
                match self.page {
                    Page::Dashboard => goto = self.ui_dashboard(ui, ctx),
                    Page::Cleaner => self.ui_cleaner(ui, ctx),
                    Page::SafeOptimize => self.ui_safe_optimize(ui, ctx),
                    Page::Fixer => self.ui_fixer(ui, ctx),
                    Page::Info => self.ui_info(ui, ctx),
                    Page::Disk => self.ui_disk(ui, ctx),
                    Page::Procs => self.ui_procs(ui, ctx),
                    Page::Apps => self.ui_apps(ui, ctx),
                    Page::AppImages => self.ui_appimages(ui, ctx),
                    Page::Zram => self.ui_zram(ui, ctx),
                    Page::Temps => self.ui_temps(ui, ctx),
                    Page::Network => {
                       self.ui_network(ui, ctx);
                       ui.add_space(6.0);
                       self.ui_net_sockets(ui, ctx);
                    }
                    Page::Kernel => {
                        self.ui_kernel(ui, ctx);
                        ui.add_space(6.0);
                        self.ui_orphan_kernels(ui, ctx);
                    }
                    Page::Repos => {
                       self.ui_repos(ui, ctx);
                    ui.add_space(6.0);
                       self.ui_gpg_keys(ui, ctx);
                       ui.add_space(6.0);
                       self.ui_apt_sources_audit(ui, ctx);
                    }
                    Page::Startup => self.ui_startup(ui, ctx),
                    Page::Boot => self.ui_boot(ui, ctx),
                    Page::Perms => self.ui_perms(ui, ctx),
                    Page::Firewall => self.ui_firewall(ui, ctx),
                    Page::Backup => self.ui_backup(ui, ctx),
                    Page::Log => self.ui_log(ui, ctx),
                    Page::Shredder => self.ui_shredder(ui, ctx),
                    Page::Settings => self.ui_settings(ui),
                    Page::Guide => self.ui_guide(ui),
                    Page::About => self.ui_about(ui, ctx),
                    Page::Duplicates => self.ui_duplicates(ui, ctx),
                    Page::Quarantine => self.ui_quarantine(ui, ctx),
                    Page::RestoreHistory => self.ui_restore_history(ui, ctx),
                    Page::Services => self.ui_services(ui, ctx),
                    Page::Updates => self.ui_updates(ui, ctx),
                    Page::Snaps => self.ui_snaps(ui, ctx),
                    Page::Timers => self.ui_timers(ui, ctx),
                    Page::Containers => self.ui_containers(ui, ctx),
                    Page::Synaptic => self.ui_synaptic(ui, ctx),
                    Page::History => self.ui_history(ui, ctx),
                    Page::AdvancedCleaner => self.ui_advanced_cleaner(ui, ctx),
                    Page::Report => self.ui_report(ui, ctx),
                    Page::DiskAnalyzer => self.ui_disk_analyzer(ui, ctx),
                    Page::DiskTreemap => self.ui_disk_treemap(ui, ctx),
                    Page::MenuEditor => self.ui_menu_editor(ui),
                    Page::SecurityHardening => self.ui_security_hardening(ui, ctx),
                    Page::HardwareHealth => self.ui_hardware_health(ui, ctx),
                    Page::AppLifecycle => self.ui_app_lifecycle(ui, ctx),
                    Page::Themes => self.ui_themes(ui, ctx),
                    Page::SystemEditor => self.ui_system_editor(ui, ctx),
                    Page::TelemetryGuard => self.ui_telemetry_guard(ui, ctx),
                    Page::Mounts => self.ui_mounts(ui, ctx),
                    Page::DevContainers => self.ui_dev_containers(ui, ctx),
                    Page::KernelBoot => self.ui_kernel_boot(ui, ctx),
                }
                });
            });
        if let Some(p) = goto {
            self.enter(ctx, p);
        }
    }
}

/// Percorso della socket di istanza singola, fuori da `/tmp`.
///
/// In `/tmp` il nome era prevedibile e sostituibile da qualsiasi utente:
/// una socket "fantasma" poteva impedire l'avvio, e un link simbolico poteva
/// indurre a rimuovere un file non appartenente all'applicazione. Usiamo
/// `XDG_RUNTIME_DIR`, altrimenti una cartella privata nella home dell'utente
/// (risolta con `util::home()` anche quando DebUtility è avviato tramite pkexec).
fn instance_socket_path() -> std::path::PathBuf {
    let xdg_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir());
    let dir = match xdg_dir {
        Some(d) => d,
        None => {
            let home = util::home();
            let d = if home.is_empty() {
                std::path::PathBuf::from(format!("/run/user/{}", util::uid()))
            } else {
                std::path::PathBuf::from(home).join(".cache/debutility")
            };
            let _ = std::fs::create_dir_all(&d);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
            }
            // Se è stato avviato con pkexec/sudo, la cartella deve restare
            // accessibile all'utente che ha richiesto l'elevazione.
            if let (Ok(uid), Ok(gid)) = (std::env::var("PKEXEC_UID"), std::env::var("PKEXEC_GID")) {
                // Valori non numerici non sono identificatori validi: chown
                // fallirebbe comunque, ma non deve essere invocato con dati
                // arbitrari presi dall'ambiente.
                if !uid.is_empty()
                    && !gid.is_empty()
                    && uid.bytes().all(|b| b.is_ascii_digit())
                    && gid.bytes().all(|b| b.is_ascii_digit())
                {
                    let _ = std::process::Command::new("chown")
                        .arg(format!("{uid}:{gid}"))
                        .arg(&d)
                        .status();
                }
            }
            d
        }
    };
    dir.join(format!("debutility-single-instance-{}.sock", util::uid()))
}

/// Modalità CLI/headless. Viene gestita all'inizio di `main`, prima di creare
/// il lock di istanza singola e prima di `eframe::run_native`: così funziona
/// anche su server senza display X11/Wayland.
mod cli {
    use crate::i18n::{t, tf};
    use crate::timers_state;
    use crate::{cleaner, pages_clean, util};
    use crate::APP_VERSION;

    const EXIT_OK: i32 = 0;
    const EXIT_ERROR: i32 = 1;
    const EXIT_USAGE: i32 = 2;

    #[derive(Default)]
    struct Args {
        scan: bool,
        clean: bool,
        dry_run: bool,
        json: bool,
        yes: bool,
        include_set: bool,
        include: Vec<String>,
        exclude: Vec<String>,
    }

    impl Args {
        fn mode(&self) -> &'static str {
            if self.dry_run {
                "dry-run"
            } else if self.clean {
                "clean"
            } else {
                "scan"
            }
        }
    }

    /// Solo `clean` può scrivere: `scan` e `dry-run` restano in sola lettura.
    fn needs_write(mode: &str) -> bool {
        mode == "clean"
    }

    /// Categorie effettivamente selezionate per la pulizia headless.
    #[derive(Clone, Debug)]
    struct Selection {
        ids: Vec<String>,
    }

    impl Selection {
        fn has(&self, id: &str) -> bool {
            self.ids.iter().any(|v| v == id)
        }

        fn kinds(&self) -> Vec<cleaner::Kind> {
            cleaner::items()
                .into_iter()
                .filter(|item| self.has(kind_id(item.kind)))
                .map(|item| item.kind)
                .collect()
        }

    }

    #[derive(Default)]
    struct Entry {
        id: &'static str,
        label: String,
        bytes: Option<u64>,
        count: Option<usize>,
        paths: Vec<String>,
    }

    #[derive(Default)]
    struct Report {
        entries: Vec<Entry>,
        logs: Vec<String>,
        warnings: Vec<String>,
        freed_bytes: Option<u64>,
    }

    impl Report {
        fn total_bytes(&self) -> u64 {
            self.entries.iter().filter_map(|e| e.bytes).sum()
        }
    }

    /// Gestisce gli argomenti: ritorna solo se deve essere avviata la GUI.
    pub fn handle() {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        if raw.is_empty() {
            return;
        }
        crate::i18n::set_lang(crate::i18n::detect());

        if raw.iter().any(|a| matches!(a.as_str(), "-h" | "--help")) {
            println!("{}", help_text());
            std::process::exit(EXIT_OK);
        }
        if raw.iter().any(|a| matches!(a.as_str(), "-V" | "--version")) {
            println!("DebUtility {}", APP_VERSION);
            std::process::exit(EXIT_OK);
        }

        let args = match parse(&raw) {
            Ok(args) => args,
            Err(msg) => usage_error(msg),
        };
        let selection = build_selection(&args);
        let mode = args.mode();
        let mut report = collect(mode != "scan", &selection);
        let ok = if needs_write(mode) {
            execute_clean(&mut report, &selection)
        } else {
            true
        };
        print_report(mode, &report, args.json, ok, &selection);
        if args.json && mode == "clean" {
            let run = timers_state::LastRun {
                timestamp: util::timestamp(),
                status: if ok { "ok" } else { "error" }.to_string(),
                exit_code: if ok { EXIT_OK } else { EXIT_ERROR },
                freed_bytes: report.freed_bytes.unwrap_or_else(|| report.total_bytes()),
                freed_human: cleaner::fmt_bytes(report.freed_bytes.unwrap_or_else(|| report.total_bytes())),
                selected: selection.ids.clone(),
                warnings: report.warnings.clone(),
            };
            if let Err(e) = timers_state::save_last_run(&run, &report.logs) {
                eprintln!("{}", tf("Impossibile salvare lo stato della pulizia: {}", &[&e]));
            }
        }
        std::process::exit(if ok { EXIT_OK } else { EXIT_ERROR });
    }

    /// Interpreta e valida gli argomenti senza effetti collaterali: `handle`
    /// si limita a stampare l'eventuale errore e a uscire. I test possono così
    /// verificare i flag (soprattutto `--dry-run`) senza avviare pulizie.
    fn parse(raw: &[String]) -> Result<Args, String> {
        let mut args = Args::default();
        let mut i = 0;
        while i < raw.len() {
            let arg = raw[i].as_str();
            match arg {
                "--scan" => args.scan = true,
                "--clean" => args.clean = true,
                "--dry-run" => args.dry_run = true,
                "--json" => args.json = true,
                "--yes" => args.yes = true,
                "--include" | "--exclude" => {
                    i += 1;
                    let value = raw.get(i).cloned().unwrap_or_default();
                    let ids = parse_ids(&value)?;
                    if arg == "--include" {
                        args.include_set = true;
                        args.include = ids;
                    } else {
                        args.exclude.extend(ids);
                    }
                }
                other if other.starts_with("--include=") => {
                    args.include_set = true;
                    args.include = parse_ids(&other["--include=".len()..])?;
                }
                other if other.starts_with("--exclude=") => {
                    args.exclude
                        .extend(parse_ids(&other["--exclude=".len()..])?);
                }
                other => {
                    return Err(tf("Argomento non valido: {}", &[&other as &dyn std::fmt::Display]));
                }
            }
            i += 1;
        }

        if !args.scan && !args.clean && !args.dry_run {
            return Err(t("Nessun comando specificato: usa --scan, --clean o --dry-run (oppure avvia DebUtility senza argomenti per la GUI).").to_string());
        }
        if args.scan && (args.clean || args.dry_run) {
            return Err(t("--scan non può essere combinato con --clean o --dry-run.").to_string());
        }
        if args.json && !(args.scan || args.clean || args.dry_run) {
            return Err(t("--json richiede --scan, --clean o --dry-run.").to_string());
        }
        if args.yes && !(args.clean || args.dry_run) {
            return Err(t("--yes richiede --clean o --dry-run.").to_string());
        }
        if args.clean && !args.dry_run && !args.yes {
            return Err(t("--clean richiede --yes: nessun file è stato rimosso.").to_string());
        }

        if build_selection(&args).ids.is_empty() {
            return Err(t("Nessuna categoria valida selezionata.").to_string());
        }
        Ok(args)
    }

    fn parse_ids(value: &str) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for raw in value.split(',') {
            let id = raw.trim();
            if id.is_empty() || id == "-" {
                continue;
            }
            if !timers_state::CLEAN_OPTIONS
                .iter()
                .any(|(known, _, _)| *known == id)
            {
                return Err(tf("Categoria non valida in --include/--exclude: {}", &[&id]));
            }
            if !out.iter().any(|v| v == id) {
                out.push(id.to_string());
            }
        }
        Ok(out)
    }

    fn build_selection(args: &Args) -> Selection {
        let requested: Vec<String> = if args.include_set {
            args.include.clone()
        } else {
            timers_state::DEFAULT_INCLUDES.iter().map(|s| s.to_string()).collect()
        };
        let ids = timers_state::CLEAN_OPTIONS
            .iter()
            .filter(|(id, _, _)| requested.iter().any(|v| v == id) && !args.exclude.iter().any(|v| v == id))
            .map(|(id, _, _)| id.to_string())
            .collect();
        Selection { ids }
    }

    fn usage_error(msg: String) -> ! {
        eprintln!("{msg}");
        eprintln!("{}", help_text());
        std::process::exit(EXIT_USAGE)
    }

    fn help_text() -> String {
        let lines = [
            format!("DebUtility {} — {}", APP_VERSION, t("Modalità CLI / headless (nessuna GUI richiesta)")),
            String::new(),
            t("Uso:").to_string(),
            t("  debutility                     Avvia la GUI").to_string(),
            t("  debutility --scan [--json]     Scansione sicura e riepilogo").to_string(),
            t("  debutility --clean --yes       Pulizia sicura non interattiva").to_string(),
            t("  debutility --dry-run           Anteprima senza eliminare nulla").to_string(),
            String::new(),
            t("Opzioni:").to_string(),
            t("  -h, --help     Mostra questo aiuto ed esce").to_string(),
            t("  -V, --version  Mostra la versione ed esce").to_string(),
            t("  --json         Output JSON strutturato per script").to_string(),
            t("  --yes          Conferma esplicita richiesta da --clean").to_string(),
            t("  --include=ID1,ID2   Limita la pulizia alle categorie indicate").to_string(),
            t("  --exclude=ID1,ID2   Esclude categorie dalla selezione predefinita").to_string(),
            String::new(),
            tf(
                "Categorie disponibili: {}",
                &[&timers_state::CLEAN_OPTIONS
                    .iter()
                    .map(|(id, _, _)| *id)
                    .collect::<Vec<_>>()
                    .join(", ")],
            ),
            String::new(),
            t("Codici di uscita:").to_string(),
            t("  0 successo, 1 errore operativo, 2 argomenti non validi o --yes mancante").to_string(),
        ];
        lines.join("\n")
    }

    /// Identificatore CLI di ogni categoria pulita da `cleaner`.
    fn kind_id(kind: cleaner::Kind) -> &'static str {
        match kind {
            cleaner::Kind::Temp => "temp",
            cleaner::Kind::AptCache => "apt_cache",
            cleaner::Kind::BrowserCache => "browser_cache",
            cleaner::Kind::OldLogs => "old_logs",
            cleaner::Kind::Journal => "journal",
            cleaner::Kind::UserCache => "user_cache",
            cleaner::Kind::Thumbnails => "thumbnails",
            _ => "other",
        }
    }

    /// Percorsi concreti che verrebbero rimossi. Per i comandi privilegiati
    /// riutilizza esattamente gli stessi filtri di `cleaner`, in sola lettura.
    fn preview_paths(kind: cleaner::Kind) -> Vec<String> {
        if matches!(kind, cleaner::Kind::Temp | cleaner::Kind::OldLogs) {
            if let Some(cmd) = cleaner::priv_cmd(kind) {
                let (_, out) = util::run_shell(&cmd.replace("-delete", "-print"), false);
                return out
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(str::to_string)
                    .collect();
            }
            return Vec::new();
        }
        let mut list = Vec::new();
        for base in cleaner::paths(kind) {
            match std::fs::read_dir(&base) {
                Ok(rd) => {
                    for entry in rd.flatten() {
                        list.push(entry.path().display().to_string());
                    }
                }
                Err(_) => {
                    if base.exists() {
                        list.push(base.display().to_string());
                    }
                }
            }
        }
        list
    }

    fn collect(with_paths: bool, selection: &Selection) -> Report {
        let mut report = Report::default();
        let items = cleaner::items();
        let sizes = cleaner::scan();
        for (item, size) in items.iter().zip(sizes.into_iter()) {
            if !selection.has(kind_id(item.kind)) {
                continue;
            }
            report.entries.push(Entry {
                id: kind_id(item.kind),
                label: t(item.name).to_string(),
                bytes: size,
                count: None,
                paths: if with_paths { preview_paths(item.kind) } else { Vec::new() },
            });
        }

        if selection.has("rc_packages") {
            let rc: Vec<String> = cleaner::scan_rc_pkgs().into_iter().map(|(n, _)| n).collect();
            report.entries.push(Entry {
                id: "rc_packages",
                label: t("Residui APT (pacchetti rc)").to_string(),
                bytes: None,
                count: Some(rc.len()),
                paths: if with_paths { rc } else { Vec::new() },
            });
        }

        if selection.has("flatpak_unused") {
            let flatpak: Vec<String> = cleaner::scan_flatpak_unused().into_iter().map(|(n, _)| n).collect();
            report.entries.push(Entry {
                id: "flatpak_unused",
                label: t("Runtime Flatpak inutilizzati").to_string(),
                bytes: None,
                count: Some(flatpak.len()),
                paths: if with_paths { flatpak } else { Vec::new() },
            });
        }

        if selection.has("empty_dirs") {
            let dirs: Vec<String> = pages_clean::scan_dirs(false).into_iter().map(|f| f.path).collect();
            report.entries.push(Entry {
                id: "empty_dirs",
                label: t("Cartelle vuote").to_string(),
                bytes: Some(0),
                count: Some(dirs.len()),
                paths: if with_paths { dirs } else { Vec::new() },
            });
        }

        if selection.has("empty_files") {
            let files: Vec<String> = pages_clean::scan_files(false).into_iter().map(|f| f.path).collect();
            report.entries.push(Entry {
                id: "empty_files",
                label: t("File vuoti (0 byte)").to_string(),
                bytes: Some(0),
                count: Some(files.len()),
                paths: if with_paths { files } else { Vec::new() },
            });
        }
        report
    }

    /// Esegue solo le categorie selezionate, riutilizzando le funzioni di
    /// pulizia della GUI. Lo spazio liberato è misurato prima/dopo.
    fn execute_clean(report: &mut Report, selection: &Selection) -> bool {
        let kinds = selection.kinds();
        let nested_in_user_cache = kinds.contains(&cleaner::Kind::UserCache);
        let measured: Vec<cleaner::Kind> = kinds
            .iter()
            .copied()
            .filter(|k| {
                !nested_in_user_cache
                    || !matches!(k, cleaner::Kind::BrowserCache | cleaner::Kind::Thumbnails)
            })
            .collect();
        let before: Vec<(cleaner::Kind, Option<u64>)> = measured
            .iter()
            .copied()
            .map(|k| (k, cleaner::size(k)))
            .collect();

        let mut ok = true;
        let logs = cleaner::clean_all_with_progress(&kinds, |_, _, _| {});
        for line in logs {
            if line.starts_with('✘') {
                ok = false;
                report.warnings.push(line.clone());
            }
            report.logs.push(line);
        }
        let mut freed = 0u64;
        for (kind, before_size) in before {
            if let (Some(b), Some(a)) = (before_size, cleaner::size(kind)) {
                freed += b.saturating_sub(a);
            }
        }

        if selection.has("rc_packages") {
            let rc: Vec<String> = cleaner::scan_rc_pkgs().into_iter().map(|(n, _)| n).collect();
            if !rc.is_empty() {
                let names = rc.iter().map(|p| util::shq(p)).collect::<Vec<_>>().join(" ");
                let (res, out) = util::run_shell(&format!("apt-get purge -y {names}"), true);
                report.logs.push(format!("{} {}: {}", if res { "✔" } else { "✘" }, t("Residui APT (pacchetti rc)"), util::tail(out.trim(), 3).replace('\n', " | ")));
                if !res {
                    ok = false;
                }
            }
        }

        if selection.has("flatpak_unused") {
            let flatpak: Vec<String> = cleaner::scan_flatpak_unused().into_iter().map(|(n, _)| n).collect();
            if !flatpak.is_empty() {
                let res = cleaner::remove_unused_flatpaks(&flatpak);
                report.logs.push(format!("{} {}", if res { "✔" } else { "✘" }, t("Runtime Flatpak inutilizzati")));
                if !res {
                    ok = false;
                }
            }
        }

        if selection.has("empty_dirs") {
            let dirs: Vec<String> = pages_clean::scan_dirs(false).into_iter().map(|f| f.path).collect();
            let (_, err) = pages_clean::delete_empty_items(true, dirs, |_, _, _| {});
            report.logs.push(format!("{} {}", if err == 0 { "✔" } else { "✘" }, t("Cartelle vuote")));
            ok &= err == 0;
        }

        if selection.has("empty_files") {
            let files: Vec<String> = pages_clean::scan_files(false).into_iter().map(|f| f.path).collect();
            let (_, err) = pages_clean::delete_empty_items(false, files, |_, _, _| {});
            report.logs.push(format!("{} {}", if err == 0 { "✔" } else { "✘" }, t("File vuoti (0 byte)")));
            ok &= err == 0;
        }

        report.freed_bytes = Some(freed);
        ok
    }

    fn print_report(mode: &str, report: &Report, json: bool, ok: bool, selection: &Selection) {
        if json {
            println!("{}", report_json(mode, report, ok, selection));
            return;
        }
        let heading = match mode {
            "scan" => t("Scansione sicura completata"),
            "dry-run" => t("Elementi da rimuovere:"),
            _ => t("Pulizia completata."),
        };
        println!("{heading}");
        for entry in &report.entries {
            let amount = match (entry.bytes, entry.count) {
                (Some(b), _) => cleaner::fmt_bytes(b),
                (None, Some(c)) => c.to_string(),
                (None, None) => "-".to_string(),
            };
            println!("- {}: {}", entry.label, amount);
            if mode == "dry-run" {
                for path in &entry.paths {
                    println!("    {path}");
                }
            }
        }
        if mode == "scan" {
            println!(
                "{} {}",
                t("Spazio totale recuperabile:"),
                cleaner::fmt_bytes(report.total_bytes())
            );
        }
        if mode == "clean"
            && report
                .entries
                .iter()
                .all(|e| e.bytes.unwrap_or(0) == 0 && e.count.unwrap_or(0) == 0)
        {
            println!("{}", t("Nessun elemento sicuro da rimuovere."));
        }
        if mode == "clean" {
            for line in &report.logs {
                println!("{line}");
            }
        }
        for warning in &report.warnings {
            eprintln!("{}: {}", t("Attenzione"), warning);
        }
        if !ok {
            eprintln!("{}", t("Errore operativo durante la pulizia."));
        }
    }

    fn jstr(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn jarr(items: &[String]) -> String {
        format!(
            "[{}]",
            items.iter().map(|s| jstr(s)).collect::<Vec<_>>().join(",")
        )
    }

    fn report_json(mode: &str, report: &Report, ok: bool, selection: &Selection) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        out.push_str(&format!("  \"mode\": {},\n", jstr(mode)));
        out.push_str(&format!("  \"timestamp\": {},\n", jstr(&util::timestamp())));
        out.push_str(&format!("  \"status\": {},\n", jstr(if ok { "ok" } else { "error" })));
        out.push_str(&format!("  \"exit_code\": {},\n", if ok { EXIT_OK } else { EXIT_ERROR }));
        out.push_str(&format!("  \"selected\": {},\n", jarr(&selection.ids)));
        out.push_str(&format!("  \"total_recoverable_bytes\": {},\n", report.total_bytes()));
        out.push_str(&format!(
            "  \"total_recoverable_human\": {},\n",
            jstr(&cleaner::fmt_bytes(report.total_bytes()))
        ));
        out.push_str(&format!(
            "  \"freed_bytes\": {},\n",
            match report.freed_bytes {
                Some(b) => b.to_string(),
                None => "null".to_string(),
            }
        ));
        out.push_str(&format!(
            "  \"freed_human\": {},\n",
            jstr(&cleaner::fmt_bytes(report.freed_bytes.unwrap_or(0)))
        ));
        out.push_str("  \"entries\": [\n");
        for (i, e) in report.entries.iter().enumerate() {
            out.push_str("    {\n");
            out.push_str(&format!("      \"id\": {},\n", jstr(e.id)));
            out.push_str(&format!("      \"label\": {},\n", jstr(&e.label)));
            out.push_str(&format!(
                "      \"bytes\": {},\n",
                match e.bytes {
                    Some(b) => b.to_string(),
                    None => "null".to_string(),
                }
            ));
            out.push_str(&format!(
                "      \"human\": {},\n",
                match e.bytes {
                    Some(b) => jstr(&cleaner::fmt_bytes(b)),
                    None => "null".to_string(),
                }
            ));
            out.push_str(&format!(
                "      \"count\": {},\n",
                match e.count {
                    Some(c) => c.to_string(),
                    None => "null".to_string(),
                }
            ));
            out.push_str(&format!("      \"paths\": {}\n", jarr(&e.paths)));
            out.push_str("    }");
            if i + 1 < report.entries.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ],\n");
        out.push_str(&format!("  \"logs\": {},\n", jarr(&report.logs)));
        out.push_str(&format!("  \"warnings\": {}\n", jarr(&report.warnings)));
        out.push_str("}\n");
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn parse_args(raw: &[&str]) -> Result<Args, String> {
            parse(&raw.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        }

        #[test]
        fn dry_run_e_in_sola_lettura() {
            let dry = parse_args(&["--dry-run"]).expect("--dry-run valido");
            assert_eq!(dry.mode(), "dry-run");
            assert!(!dry.clean, "dry-run non deve attivare la pulizia");
            assert!(!needs_write(dry.mode()), "dry-run non scrive nulla");

            let dry_json = parse_args(&["--dry-run", "--json"]).expect("dry-run + json");
            assert_eq!(dry_json.mode(), "dry-run");
            assert!(dry_json.json);
            assert!(!needs_write(dry_json.mode()));

            let dry_yes = parse_args(&["--dry-run", "--yes"]).expect("dry-run + yes");
            assert_eq!(dry_yes.mode(), "dry-run");
            assert!(!needs_write(dry_yes.mode()));

            let scan = parse_args(&["--scan"]).expect("--scan valido");
            assert_eq!(scan.mode(), "scan");
            assert!(!needs_write(scan.mode()));

            let clean = parse_args(&["--clean", "--yes"]).expect("--clean --yes valido");
            assert_eq!(clean.mode(), "clean");
            assert!(needs_write(clean.mode()), "solo clean può rimuovere file");
        }

        #[test]
        fn combinazioni_non_valide_vengono_rifiutate() {
            assert!(parse_args(&["--clean"]).is_err(), "--clean richiede --yes");
            assert!(parse_args(&["--scan", "--dry-run"]).is_err());
            assert!(parse_args(&["--json"]).is_err());
            assert!(parse_args(&["--yes"]).is_err());
            assert!(parse_args(&["--sconosciuto"]).is_err());
            assert!(parse_args(&["--dry-run", "--include=categoria_inesistente"]).is_err());
            assert!(
                parse_args(&["--dry-run", "--include=temp", "--exclude=temp"]).is_err(),
                "selezione vuota"
            );
        }

        #[test]
        fn output_json_e_valido_con_caratteri_speciali() {
            assert_eq!(jstr("a\"b\\c\nd\te\r\u{1}"), "\"a\\\"b\\\\c\\nd\\te\\r\\u0001\"");

            let report = Report {
                entries: vec![Entry {
                    id: "temp",
                    label: "cartella \"speciale\"\ncon tab\t\\".to_string(),
                    bytes: Some(2048),
                    count: None,
                    paths: vec!["/tmp/a\tb".into(), "/tmp/quote\"".into()],
                }],
                logs: vec!["log\nriga".into()],
                warnings: vec!["attenzione".into()],
                freed_bytes: None,
            };
            let selection = Selection { ids: vec!["temp".into(), "user_cache".into()] };
            let json = report_json("dry-run", &report, true, &selection);

            assert!(json.starts_with("{\n"));
            assert!(json.ends_with("}\n"));
            assert!(json.contains("\"mode\": \"dry-run\""));
            assert!(json.contains("\"status\": \"ok\""));
            assert!(json.contains("\"exit_code\": 0"));
            assert!(json.contains("\"freed_bytes\": null"));
            assert!(json.contains("\"id\": \"temp\""));
            JsonParser::new(&json).parse().expect("report_json deve produrre JSON valido");
        }

        /// Validatore JSON minimale e senza dipendenze: verifica l'intero output
        /// di `--json` (struttura, stringhe escape, numeri, valori null).
        struct JsonParser<'a> {
            s: &'a [u8],
            i: usize,
        }

        impl<'a> JsonParser<'a> {
            fn new(s: &'a str) -> Self {
                Self { s: s.as_bytes(), i: 0 }
            }

            fn peek(&self) -> Option<u8> {
                self.s.get(self.i).copied()
            }

            fn ws(&mut self) {
                while self.peek().is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t')) {
                    self.i += 1;
                }
            }

            fn parse(&mut self) -> Result<(), String> {
                self.value()?;
                self.ws();
                if self.i != self.s.len() {
                    return Err(format!("dati extra in posizione {}", self.i));
                }
                Ok(())
            }

            fn value(&mut self) -> Result<(), String> {
                self.ws();
                match self.peek().ok_or("fine inaspettata")? {
                    b'{' => self.object(),
                    b'[' => self.array(),
                    b'"' => self.string(),
                    b't' => self.literal("true"),
                    b'f' => self.literal("false"),
                    b'n' => self.literal("null"),
                    b'-' | b'0'..=b'9' => self.number(),
                    b => Err(format!("valore non valido {:?} in posizione {}", b as char, self.i)),
                }
            }

            fn object(&mut self) -> Result<(), String> {
                self.i += 1;
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(());
                }
                loop {
                    self.ws();
                    self.string()?;
                    self.ws();
                    if self.peek() != Some(b':') {
                        return Err("':' mancante in oggetto".into());
                    }
                    self.i += 1;
                    self.value()?;
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(());
                        }
                        _ => return Err("oggetto non chiuso".into()),
                    }
                }
            }

            fn array(&mut self) -> Result<(), String> {
                self.i += 1;
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(());
                }
                loop {
                    self.value()?;
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(());
                        }
                        _ => return Err("array non chiuso".into()),
                    }
                }
            }

            fn string(&mut self) -> Result<(), String> {
                if self.peek() != Some(b'"') {
                    return Err(format!("stringa attesa in posizione {}", self.i));
                }
                self.i += 1;
                while let Some(b) = self.peek() {
                    match b {
                        b'"' => {
                            self.i += 1;
                            return Ok(());
                        }
                        b'\\' => {
                            self.i += 1;
                            match self.peek() {
                                Some(c) if matches!(c, b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                                    self.i += 1;
                                }
                                Some(b'u') => {
                                    self.i += 1;
                                    for _ in 0..4 {
                                        if !self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                                            return Err("\\u con cifre errate".into());
                                        }
                                        self.i += 1;
                                    }
                                }
                                _ => return Err("escape non valido".into()),
                            }
                        }
                        c if c < 0x20 => return Err("controllo non escapato".into()),
                        _ => self.i += 1,
                    }
                }
                Err("stringa non chiusa".into())
            }

            fn literal(&mut self, word: &str) -> Result<(), String> {
                if self.s[self.i..].starts_with(word.as_bytes()) {
                    self.i += word.len();
                    Ok(())
                } else {
                    Err(format!("letterale {word} atteso in posizione {}", self.i))
                }
            }

            fn number(&mut self) -> Result<(), String> {
                if self.peek() == Some(b'-') {
                    self.i += 1;
                }
                let digits = self.digits();
                if digits == 0 {
                    return Err("numero senza cifre".into());
                }
                if self.peek() == Some(b'.') {
                    self.i += 1;
                    if self.digits() == 0 {
                        return Err("parte frazionaria vuota".into());
                    }
                }
                if self.peek().is_some_and(|c| matches!(c, b'e' | b'E')) {
                    self.i += 1;
                    if self.peek().is_some_and(|c| matches!(c, b'+' | b'-')) {
                        self.i += 1;
                    }
                    if self.digits() == 0 {
                        return Err("esponente vuoto".into());
                    }
                }
                Ok(())
            }

            fn digits(&mut self) -> usize {
                let start = self.i;
                while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    self.i += 1;
                }
                self.i - start
            }
        }
    }
}

/// Rileva, tramite `glxinfo -B`, se il sistema richiede il rendering software.
/// Ritorna `false` se `glxinfo` non è installato o non fornisce indicazioni.
fn glxinfo_needs_software_rendering() -> bool {
    let output = match std::process::Command::new("glxinfo").arg("-B").output() {
        Ok(output) => output,
        Err(_) => return false, // glxinfo assente: non imponiamo nulla.
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lower = stdout.to_ascii_lowercase();

    if lower.contains("direct rendering: no")
        || lower.contains("llvmpipe")
        || lower.contains("softpipe")
        || lower.contains("swrast")
    {
        return true;
    }

    // Versioni tipiche: "OpenGL version string: 2.1 Mesa ..." oppure
    // "OpenGL core profile version string: 3.0 Mesa ...".
    stdout.lines().any(|line| {
        let trimmed = line.trim().to_ascii_lowercase();
        let is_version_line = trimmed.starts_with("opengl version string:")
            || trimmed.starts_with("opengl core profile version string:")
            || trimmed.starts_with("opengl version:");
        if !is_version_line {
            return false;
        }

        let value = line.split_once(':').map(|(_, v)| v.trim()).unwrap_or("");
        let mut parts = value.split('.');
        let major = match parts.next().and_then(|v| v.trim_start().parse::<u32>().ok()) {
            Some(major) => major,
            None => return false,
        };
        let minor = parts
            .next()
            .map(|v| v.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);

        (major, minor) < (3, 2)
    })
}

/// Riavvia lo stesso eseguibile con `LIBGL_ALWAYS_SOFTWARE=1`.
/// `exec` sostituisce il processo corrente: non resta un padre in attesa.
fn exec_with_software_rendering() -> std::io::Error {
    use std::os::unix::process::CommandExt;

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => return err,
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(std::env::args_os().skip(1));
    cmd.env("LIBGL_ALWAYS_SOFTWARE", "1");
    cmd.exec()
}

fn main() -> eframe::Result<()> {
    // Va prima di qualsiasi finestra/contesto grafico: se il PC ha OpenGL
    // vecchio o un renderer software, il nuovo processo eredita già la
    // variabile e non ripete il controllo.
    if std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_none()
        && glxinfo_needs_software_rendering()
    {
        let err = exec_with_software_rendering();
        eprintln!("DebUtility: riavvio in rendering software non riuscito: {err}");
    }

    // Modalità CLI/headless: nessuna GUI, nessun lock di istanza e nessun
    // controllo su DISPLAY/WAYLAND_DISPLAY deve precedere questi comandi.
    cli::handle();

    // L'avvio automatico come root è stato rimosso: i privilegi sono richiesti
    // solo dalle singole operazioni tramite pkexec. Il cfg resta in App::new().

    let socket_path = instance_socket_path();

    if std::os::unix::net::UnixStream::connect(&socket_path).is_ok() {
        eprintln!("DebUtility è già in esecuzione!");
        let _ = std::process::Command::new("notify-send")
            .args(["DebUtility", "L'applicazione è già in esecuzione."])
            .spawn();
        return Ok(());
    }

    // Rimuove solo una socket rimasta da un arresto anomalo: mai symlink o
    // altri file, così anche in una cartella condivisa non si tocca nulla di
    // estraneo.
    {
        use std::os::unix::fs::FileTypeExt;
        if let Ok(md) = std::fs::symlink_metadata(&socket_path) {
            if md.file_type().is_socket() {
                let _ = std::fs::remove_file(&socket_path);
            }
        }
    }

    let _instance_lock = match std::os::unix::net::UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(_) => {
            eprintln!("Impossibile creare il lock di istanza singola");
            return Ok(());
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 820.0])
            .with_min_inner_size([800.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native("DebUtility", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
