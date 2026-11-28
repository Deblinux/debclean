use crate::i18n::t;
use crate::util::{home, run_shell, tail};
use std::path::{Path, PathBuf};
use std::fs;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Temp,
    AptCache,
    Autoremove,
    BrowserCache,
    BrowserData,
    Ram,
    Journal,
    OldLogs,
    UserCache,
    Trash,
    Thumbnails,
    DiscordCache,
    TelegramCache,
    SteamCache,
    SlackCache,
}

pub struct CleanItem {
    pub kind: Kind,
    pub name: &'static str,
    pub desc: &'static str,
    pub selected: bool,
    pub size: Option<u64>,
}

const BROWSER_CACHE: [&str; 4] = [
    ".cache/mozilla",
    ".cache/google-chrome",
    ".cache/chromium",
    ".cache/BraveSoftware",
];

const BROWSER_DATA_CMD: &str = "rm -f \
\"$HOME\"/.mozilla/firefox/*/cookies.sqlite* \
\"$HOME\"/.config/google-chrome/*/History* \
\"$HOME\"/.config/google-chrome/*/Cookies* \
\"$HOME\"/.config/chromium/*/History* \
\"$HOME\"/.config/chromium/*/Cookies* \
\"$HOME\"/.config/BraveSoftware/Brave-Browser/*/History* \
\"$HOME\"/.config/BraveSoftware/Brave-Browser/*/Cookies*";

pub fn items() -> Vec<CleanItem> {
    let mk = |kind, name, desc, selected| CleanItem { kind, name, desc, selected, size: None };
    vec![
        mk(Kind::Temp, "File temporanei", "File in /tmp e /var/tmp non modificati da più di 2 giorni (richiede password)", true),
        mk(Kind::AptCache, "Cache pacchetti APT", "Rimuove i .deb scaricati e non più necessari (richiede password)", true),
        mk(Kind::Autoremove, "Pacchetti orfani", "apt autoremove --purge: dipendenze non più usate (richiede password)", true),
        mk(Kind::BrowserCache, "Cache dei browser", "Firefox, Chrome, Chromium, Brave. Chiudi i browser prima di pulire", true),
        mk(Kind::BrowserData, "Cronologia e cookie dei browser", "Firefox, Chrome, Chromium: operazione irreversibile", false),
        mk(Kind::Ram, "Cache della RAM", "Svuota page cache e buffer temporanei (richiede password)", false),
        mk(Kind::Journal, "Log del journal", "Mantiene solo gli ultimi 7 giorni di log systemd (richiede password)", true),
        mk(Kind::OldLogs, "Log obsoleti in /var/log", "File ruotati e compressi (.gz, .1, .old) (richiede password)", true),
        mk(Kind::UserCache, "Cache utente", "Contenuto di ~/.cache (esclude cartelle sensibili)", false),
        mk(Kind::Trash, "Cestino", "File nel cestino dell'utente", true),
        mk(Kind::Thumbnails, "Miniature (Thumbnails)", "Cache delle miniature di immagini e video (Nautilus/Dolphin)", true),
        mk(Kind::DiscordCache, "Cache Discord", "File temporanei e cache accumulati da Discord", false),
        mk(Kind::TelegramCache, "Cache Telegram", "File temporanei e cache multimediali di Telegram Desktop", false),
        mk(Kind::SteamCache, "Cache Steam", "File di cache e depot temporanei di Steam", false),
        mk(Kind::SlackCache, "Cache Slack", "File temporanei e cache accumulati da Slack", false),
    ]
}

pub fn dir_size(p: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if let Ok(md) = path.symlink_metadata() {
                if md.is_dir() {
                    total += dir_size(&path);
                } else {
                    total += md.len();
                }
            }
        }
    }
    total
}

fn is_old_log(name: &str) -> bool {
    name.ends_with(".gz")
        || name.ends_with(".old")
        || name
            .rsplit('.')
            .next()
            .map(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false)
}

/// File di log ruotati/compressi presenti in una cartella. Condiviso con la
/// CLI per mostrare l'anteprima senza duplicare i criteri di pulizia.
pub(crate) fn old_log_paths(p: &Path) -> Vec<PathBuf> {
    let mut list = Vec::new();
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if let Ok(md) = path.symlink_metadata() {
                if md.is_dir() {
                    list.extend(old_log_paths(&path));
                } else if is_old_log(&e.file_name().to_string_lossy()) {
                    list.push(path);
                }
            }
        }
    }
    list
}

fn old_logs_size(p: &Path) -> u64 {
    old_log_paths(p)
        .iter()
        .filter_map(|f| std::fs::symlink_metadata(f).ok())
        .map(|m| m.len())
        .sum()
}

/// Percorsi su cui agisce ogni categoria. Usati sia dal calcolo delle
/// dimensioni sia dall'anteprima CLI, così le due liste non possono divergere.
pub(crate) fn paths(kind: Kind) -> Vec<PathBuf> {
    let h = PathBuf::from(home());
    match kind {
        Kind::AptCache => vec![PathBuf::from("/var/cache/apt/archives")],
        Kind::BrowserCache => BROWSER_CACHE.iter().map(|d| h.join(d)).collect(),
        Kind::Journal => vec![PathBuf::from("/var/log/journal")],
        Kind::OldLogs => vec![PathBuf::from("/var/log")],
        Kind::UserCache => vec![h.join(".cache")],
        Kind::Trash => vec![h.join(".local/share/Trash")],
        Kind::Thumbnails => vec![h.join(".cache/thumbnails")],
        Kind::DiscordCache => vec![
            h.join(".config/discord/Cache"),
            h.join(".config/discord/Code Cache"),
            h.join(".config/discord/GPUCache"),
        ],
        Kind::TelegramCache => vec![h.join(".local/share/TelegramDesktop/tdata/user_data/cache")],
        Kind::SteamCache => vec![
            h.join(".local/share/Steam/appcache"),
            h.join(".local/share/Steam/depotcache"),
        ],
        Kind::SlackCache => vec![
            h.join(".config/Slack/Cache"),
            h.join(".config/Slack/Code Cache"),
        ],
        Kind::Temp | Kind::Autoremove | Kind::BrowserData | Kind::Ram => Vec::new(),
    }
}

pub fn size(kind: Kind) -> Option<u64> {
    match kind {
        Kind::Temp | Kind::Autoremove | Kind::BrowserData | Kind::Ram => None,
        Kind::OldLogs => Some(old_logs_size(Path::new("/var/log"))),
        _ => Some(paths(kind).iter().map(|p| dir_size(p)).sum()),
    }
}

/// Come `scan`, ma notifica il progresso dopo ogni voce analizzata.
pub fn scan_with<F: FnMut(usize, usize, &str)>(mut on_progress: F) -> Vec<Option<u64>> {
    let list = items();
    let total = list.len();
    let mut sizes = Vec::with_capacity(total);
    for (i, item) in list.iter().enumerate() {
        on_progress(i + 1, total, item.name);
        sizes.push(size(item.kind));
    }
    sizes
}

pub fn scan() -> Vec<Option<u64>> {
    scan_with(|_, _, _| {})
}

fn remove_contents(p: &Path) -> usize {
    let mut errors = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            let res = match path.symlink_metadata() {
                Ok(md) if md.is_dir() => std::fs::remove_dir_all(&path),
                Ok(_) => std::fs::remove_file(&path),
                Err(e) => Err(e),
            };
            if res.is_err() {
                errors += 1;
            }
        }
    }
    errors
}

fn remove_user_cache_contents(p: &Path) -> usize {
    const SKIP: &[&str] = &["keyrings", "gnupg", "ssh", "dconf", "ibus", "thumbnails"];
    let mut errors = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            let path = e.path();
            let res = match path.symlink_metadata() {
                Ok(md) if md.is_dir() => std::fs::remove_dir_all(&path),
                Ok(_) => std::fs::remove_file(&path),
                Err(err) => Err(err),
            };
            if res.is_err() {
                errors += 1;
            }
        }
    }
    errors
}

/// Comando privilegiato associato a una categoria. La CLI lo riutilizza anche
/// in sola lettura (sostituendo `-delete` con `-print`) per l'anteprima.
pub(crate) fn priv_cmd(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Temp => Some(
            "find /tmp /var/tmp -xdev -mindepth 1 -atime +2 -mtime +2 ! -type s \
             ! -path '/tmp/.*-unix*' ! -path '/tmp/.X*-lock' ! -path '/tmp/ssh-*' \
             ! -path '/tmp/systemd-private-*' ! -path '/var/tmp/systemd-private-*' \
             -delete 2>/dev/null",
        ),
        Kind::AptCache => Some("apt-get clean; apt-get -y autoclean"),
        Kind::Autoremove => Some("apt-get -y autoremove --purge"),
        Kind::Ram => Some("sync; echo 3 > /proc/sys/vm/drop_caches"),
        Kind::Journal => Some("journalctl --vacuum-time=7d"),
        Kind::OldLogs => Some(
            "find /var/log -type f \\( -name '*.gz' -o -name '*.old' -o -regex '.*\\.[0-9]+' \\) -delete",
        ),
        _ => None,
    }
}

pub fn clean_all(kinds: &[Kind]) -> Vec<String> {
    clean_all_with_progress(kinds, |_, _, _| {})
}

/// Come `clean_all`, ma aggiorna un osservatore dopo ogni operazione completata.
pub fn clean_all_with_progress<F: FnMut(usize, usize, &str)>(
    kinds: &[Kind],
    mut on_progress: F,
) -> Vec<String> {
    let mut log = Vec::new();
    let total = kinds.len().max(1);
    let privs: Vec<&str> = kinds.iter().filter_map(|k| priv_cmd(*k)).collect();
    if !privs.is_empty() {
        on_progress(0, total, t("Pulizia di sistema in corso..."));
        // `&&`: un comando fallito non deve essere mascherato dall'ultimo della
        // catena (con `;` il risultato era sempre quello dell'ultimo comando).
        let (ok, out) = run_shell(&privs.join(" && "), true);
        let mark = if ok { "✔" } else { "✘" };
        log.push(format!("{mark} Pulizia di sistema: {}", tail(out.trim(), 3).replace('\n', " | ")));
        on_progress(1, total, log.last().map(String::as_str).unwrap_or(t("Pulizia di sistema")));
    }
    let h = PathBuf::from(home());
    for (idx, k) in kinds.iter().enumerate() {
        match k {
            Kind::BrowserCache => {
                let e: usize = BROWSER_CACHE.iter().map(|d| remove_contents(&h.join(d))).sum();
                log.push(format!("✔ Cache dei browser pulita ({e} elementi non rimovibili)"));
            }
            Kind::BrowserData => {
                let (ok, _) = run_shell(BROWSER_DATA_CMD, false);
                log.push(format!("{} Cronologia e cookie dei browser", if ok { "✔" } else { "✘" }));
            }
            Kind::UserCache => {
                let e = remove_user_cache_contents(&h.join(".cache"));
                log.push(format!("✔ Cache utente svuotata ({e} elementi non rimovibili)"));
            }
            Kind::Trash => {
                let t = h.join(".local/share/Trash");
                let e = remove_contents(&t.join("files")) + remove_contents(&t.join("info"));
                log.push(format!("✔ Cestino svuotato ({e} elementi non rimovibili)"));
            }
            Kind::Thumbnails => {
                let e = remove_contents(&h.join(".cache/thumbnails"));
                log.push(format!("✔ Miniature pulite ({e} elementi non rimovibili)"));
            }
            Kind::DiscordCache => {
                let base = h.join(".config/discord");
                let e = remove_contents(&base.join("Cache"))
                    + remove_contents(&base.join("Code Cache"))
                    + remove_contents(&base.join("GPUCache"));
                log.push(format!("✔ Cache Discord pulita ({e} elementi non rimovibili)"));
            }
            Kind::TelegramCache => {
                let e = remove_contents(&h.join(".local/share/TelegramDesktop/tdata/user_data/cache"));
                log.push(format!("✔ Cache Telegram pulita ({e} elementi non rimovibili)"));
            }
            Kind::SteamCache => {
                let base = h.join(".local/share/Steam");
                let e = remove_contents(&base.join("appcache")) + remove_contents(&base.join("depotcache"));
                log.push(format!("✔ Cache Steam pulita ({e} elementi non rimovibili)"));
            }
            Kind::SlackCache => {
                let base = h.join(".config/Slack");
                let e = remove_contents(&base.join("Cache")) + remove_contents(&base.join("Code Cache"));
                log.push(format!("✔ Cache Slack pulita ({e} elementi non rimovibili)"));
            }
            _ => {}
        }
        let label = log.last().map(String::as_str).unwrap_or(t("Pulizia in corso..."));
        on_progress(idx + 1, total, label);
    }
    log
}

pub fn fmt_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

pub fn scan_rc_pkgs() -> Vec<(String, bool)> {
    let mut pkgs = Vec::new();
    if let Ok(output) = std::process::Command::new("dpkg-query").args(["-W", "-f=${db:Status-Abbrev} ${Package}\n"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.starts_with("rc ") {
                if let Some(pkg) = line.split_whitespace().nth(1) {
                    pkgs.push((pkg.to_string(), true));
                }
            }
        }
    }
    pkgs
}

pub fn scan_flatpak_unused() -> Vec<(String, bool)> {
    let mut list = Vec::new();
    let mut cmd = std::process::Command::new("flatpak");
    cmd.args(["list", "--unused", "--columns=application,ref"]);
    let Ok(output) = cmd.output() else {
        return list;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).filter(|s| !s.is_empty()).collect();
        let ref_id = cols
            .iter()
            .copied()
            .find(|c| c.contains('/'))
            .or_else(|| cols.first().copied())
            .unwrap_or(line);
        if !ref_id.is_empty() && !list.iter().any(|(existing, _)| existing == ref_id) {
            list.push((ref_id.to_string(), false));
        }
    }
    list
}

// --- NUOVE FUNZIONI DI PULIZIA AVANZATA ---

/// Scansiona file di backup di configurazione dimenticati (.dpkg-bak, .dpkg-old, .pacsave, ecc.) in /etc
pub fn scan_config_backups() -> Vec<(PathBuf, bool)> {
    let mut list = Vec::new();
    let target_dir = Path::new("/etc");
    if let Ok(entries) = walkdir_simple(target_dir, 3) {
        for path in entries {
            if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                if matches!(ext, "dpkg-bak" | "dpkg-old" | "dpkg-dist" | "ucf-old" | "pacsave" | "pacnew") {
                    list.push((path, false));
                }
            }
        }
    }
    list
}

/// Scansiona versioni disabilitate di pacchetti Snap (snap list --all)
pub fn scan_disabled_snaps() -> Vec<(String, bool)> {
    let mut list = Vec::new();
    if let Ok(output) = std::process::Command::new("snap").args(["list", "--all"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Colonne: Name Version Rev Tracking Publisher Notes. Le revisioni vecchie
            // hanno "disabled" nelle Notes (ultima colonna).
            if parts.len() >= 4 && parts.last().map_or(false, |n| n.contains("disabled")) {
                let name = format!("{} (rev {})", parts[0], parts[2]);
                list.push((name, false));
            }
        }
    }
    list
}

/// Scansiona cartelle ~/.config orfane di programmi non più installati.
/// Euristica prudente: nessun elemento è preselezionato e le cartelle condivise
/// dall'ambiente desktop o dal sistema audio non vengono mai proposte.
pub fn scan_orphan_configs() -> Vec<(PathBuf, bool)> {
    const KEEP: &[&str] = &[
        "autostart", "dconf", "systemd", "environment.d", "menus", "fontconfig", "ibus",
        "goa-1.0", "evolution", "procps", "session", "xdg-desktop-portal", "gtk-2.0",
        "gtk-3.0", "gtk-4.0", "fcitx", "fcitx5", "enchant", "wireplumber",
    ];
    const KEEP_PREFIX: &[&str] = &[
        "kde", "plasma", "xfce", "gnome", "lxqt", "lxsession", "mate", "cinnamon", "openbox",
        "pulse", "pipewire",
    ];
    let mut list = Vec::new();
    let h = PathBuf::from(home());
    let config_dir = h.join(".config");

    if let Ok(rd) = fs::read_dir(config_dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                let low = name.to_lowercase();
                if KEEP.contains(&low.as_str()) || KEEP_PREFIX.iter().any(|p| low.starts_with(*p)) {
                    continue;
                }
                // Controlla se esiste il rispettivo comando o pacchetto installato
                // (prova anche il nome in minuscolo: "Code" -> "code")
                let check = |n: &str| {
                    let which_ok = std::process::Command::new("which")
                        .arg(n)
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false);
                    let dpkg_ok = std::process::Command::new("dpkg-query")
                        .args(["-s", n])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false);
                    which_ok || dpkg_ok
                };
                if !(check(name) || check(&low)) {
                    list.push((path, false));
                }
            }
        }
    }
    list
}

// Funzione di supporto per scansionare ricorsivamente fino a una certa profondità
fn walkdir_simple(dir: &Path, max_depth: usize) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut results = Vec::new();
    if max_depth == 0 {
        return Ok(results);
    }
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(sub) = walkdir_simple(&path, max_depth - 1) {
                    results.extend(sub);
                }
            } else {
                results.push(path);
            }
        }
    }
    Ok(results)
}

pub fn remove_unused_flatpaks(selected: &[String]) -> bool {
    if selected.is_empty() {
        return true;
    }
    let mut args = vec!["uninstall", "-y"];
    args.extend(selected.iter().map(|s| s.as_str()));
    let status = std::process::Command::new("flatpak").args(&args).status();
    status.map(|s| s.success()).unwrap_or(false)
}

pub fn remove_config_backups(files: &[PathBuf]) -> bool {
    files.iter().all(|file| fs::remove_file(file).is_ok())
}

pub fn remove_disabled_snaps(snaps: &[String]) -> bool {
    snaps.iter().all(|item| {
        if let Some(idx) = item.find(" (rev ") {
            let name = &item[..idx];
            let rev_part = &item[idx + 6..];
            if let Some(end_idx) = rev_part.find(')') {
                let rev = &rev_part[..end_idx];
                return std::process::Command::new("pkexec")
                    .args(["snap", "remove", name, "--revision", rev])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            }
        }
        false
    })
}

pub fn remove_orphan_configs(dirs: &[PathBuf]) -> bool {
    let mut success = true;
    for dir in dirs {
        if fs::remove_dir_all(dir).is_err() {
            success = false;
        }
    }
    success
}

// --- OTTIMIZZAZIONE DATABASE SQLITE BROWSER ---

/// Controlla se un processo browser è attualmente in esecuzione
fn is_browser_running(process_names: &[&str]) -> bool {
    for name in process_names {
        match std::process::Command::new("pgrep").arg("-x").arg(name).output() {
            Ok(out) if out.status.success() => return true,
            // Se pgrep non è installato non possiamo garantire che il browser
            // sia chiuso: in modalità cautelativa consideriamo il processo attivo.
            Err(_) => return true,
            _ => {}
        }
    }
    false
}

/// Raccoglie ricorsivamente tutti i file SQLite rilevanti per i browser
fn find_browser_sqlite_dbs(dir: &Path, dbs: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        // file_type() non segue i symlink: non usciamo dal profilo del browser.
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            // Evitiamo ricorsioni superflue saltando cartelle pesanti non pertinenti
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name != "Cache" && name != "Code Cache" && name != "GPUCache" {
                    find_browser_sqlite_dbs(&path, dbs);
                }
            }
        } else if ft.is_file() {
            if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
                let is_firefox_db = fname.ends_with(".sqlite");
                let is_chromium_db = matches!(
                    fname,
                    "History" | "Web Data" | "Cookies" | "Favicons" | "Top Sites" | "Shortcuts"
                );
                if is_firefox_db || is_chromium_db {
                    dbs.push(path);
                }
            }
        }
    }
}

/// Esegue VACUUM e REINDEX sui database dei browser chiusi
pub fn optimize_browser_databases() -> Vec<String> {
    let mut logs = Vec::new();

    // Verifica la presenza dell'eseguibile sqlite3 sul sistema
    if !std::process::Command::new("sqlite3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        logs.push("✘ Comando 'sqlite3' non trovato sul sistema. Installalo con: sudo apt install sqlite3".into());
        return logs;
    }

    let h = PathBuf::from(home());
    // Definizione dei browser supportati: (Nome, processi da controllare, percorsi relativi dei profili)
    let browsers: &[(&str, &[&str], &[&str])] = &[
        ("Firefox", &["firefox", "firefox-bin", "firefox-esr"], &[".mozilla/firefox"]),
        ("Google Chrome", &["chrome", "google-chrome"], &[".config/google-chrome"]),
        ("Chromium", &["chromium", "chromium-browser"], &[".config/chromium"]),
        ("Brave", &["brave", "brave-browser"], &[".config/BraveSoftware/Brave-Browser"]),
    ];

    let mut total_saved: u64 = 0;
    let mut optimized_count: usize = 0;

    for (name, procs, paths) in browsers {
        // Controllo di sicurezza: se il browser è attivo, saltiamo l'ottimizzazione per evitare blocchi o corruzione
        if is_browser_running(procs) {
            logs.push(format!("⚠ {name} è in esecuzione: chiudilo prima di ottimizzare i database SQLite."));
            continue;
        }

        let mut dbs = Vec::new();
        for rel_path in *paths {
            let full_path = h.join(rel_path);
            if full_path.exists() {
                find_browser_sqlite_dbs(&full_path, &mut dbs);
            }
        }

        if dbs.is_empty() {
            continue;
        }

        let mut browser_optimized = 0;
        for db in dbs {
            let before = fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
            let res = std::process::Command::new("sqlite3")
                .arg(&db)
                .arg("VACUUM; REINDEX;")
                .output();

            match res {
                Ok(out) if out.status.success() => {
                    let after = fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
                    if before > after {
                        total_saved += before - after;
                    }
                    browser_optimized += 1;
                    optimized_count += 1;
                }
                Ok(out) => logs.push(format!(
                    "✘ sqlite3: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                )),
                Err(e) => logs.push(format!("✘ sqlite3: {e}")),
            }
        }

        if browser_optimized > 0 {
            logs.push(format!("✔ {name}: ottimizzati {browser_optimized} database SQLite (VACUUM & REINDEX completati)."));
        }
    }

    if optimized_count > 0 {
        logs.push(format!(
            "✔ Pulizia profonda browser completata: compattati {optimized_count} database (spazio recuperato: {}).",
            fmt_bytes(total_saved)
        ));
    } else if logs.is_empty() {
        logs.push("ℹ Nessun database SQLite di browser trovato o idoneo all'ottimizzazione.".into());
    }

    logs
}
