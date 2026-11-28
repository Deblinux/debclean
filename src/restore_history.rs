//! Cronologia unificata delle operazioni reversibili.
//!
//! Raccoglie in un solo elenco tre fonti già usate da DebUtility, senza
//! sostituirle né modificarne i percorsi:
//!  - i backup delle sorgenti APT creati con pkexec in
//!    `/etc/apt/debutility-backups/` (e il vecchio `debclean-backups`);
//!  - i file `.bak` generati dalle schede Grub e Gestore File di Sistema;
//!  - gli elementi ancora presenti nella Quarantena.
//!
//! La scansione ordinaria usa solo `std::fs`: se una cartella protetta non è
//! leggibile l'elenco segnala la limitazione e l'utente può richiedere una
//! ricarica con pkexec. Ripristino ed eliminazione dei file in `/etc` passano
//! sempre da uno script privilegiato con tutti i percorsi quotati e validati.

use crate::i18n::{t, tf, tr};
use crate::{cleaner, quarantine, ui, util, App, Chan, Job, Msg, ProgressReporter};
use eframe::egui;
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

/// Cartelle dei backup APT: la seconda è il percorso storico di DebClean.
const APT_ROOTS: [&str; 2] = ["/etc/apt/debutility-backups", "/etc/apt/debclean-backups"];

/// File `.bak` noti, creati dalle schede di DebUtility.
const EXPLICIT_BAKS: [&str; 8] = [
    "/etc/default/grub.debutility.bak",
    "/etc/default/grub.debclean.bak",
    "/etc/default/grub.bak",
    "/etc/fstab.bak",
    "/etc/hosts.bak",
    "/etc/environment.bak",
    "/etc/apt/sources.list.bak",
    "/etc/apt/sources.list.debutility.bak",
];

/// Radice delle configurazioni di sistema in cui cercare i `.bak` creati dal
/// Gestore File di Sistema (`cp -p -n -- "$file" "$file.bak"`).
const ETC_ROOT: &str = "/etc";

const STATUS_AVAILABLE: &str = "Disponibile";
const STATUS_EXPIRED: &str = "Scaduto";
const STATUS_EXPIRING: &str = "In scadenza";
const STATUS_VALID: &str = "Valido";

/// Radici di scansione. La produzione usa sempre `/etc`; i test passano
/// cartelle temporanee isolate, così nessuna lettura o scrittura finisce nei
/// percorsi di sistema.
#[derive(Clone, Copy)]
struct Roots<'a> {
    apt: &'a [&'a str],
    etc: &'a Path,
}

impl Roots<'static> {
    fn system() -> Self {
        Self { apt: &APT_ROOTS, etc: Path::new(ETC_ROOT) }
    }
}

impl<'a> Roots<'a> {
    fn in_apt(&self, path: &Path) -> bool {
        self.apt.iter().any(|root| path.starts_with(root))
    }

    fn in_etc(&self, path: &Path) -> bool {
        path.starts_with(self.etc) && path != self.etc
    }

    fn is_system(&self) -> bool {
        self.etc == Path::new(ETC_ROOT)
    }
}

/// Script di sola lettura usato dalla ricarica privilegiata: ogni riga è
/// `percorso<TAB>epoch`. Nessun nome di file viene costruito dalla UI.
const PRIV_SCAN: &str = r#"set -u
for d in /etc/apt/debutility-backups /etc/apt/debclean-backups; do
  [ -d "$d" ] || continue
  find "$d" -mindepth 2 -maxdepth 2 -type f -print 2>/dev/null | while IFS= read -r f; do
    e=$(stat -c %Y "$(dirname "$f")" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || echo 0)
    printf '%s\t%s\n' "$f" "$e"
  done
done
find /etc -xdev -type f -name '*.bak' -print 2>/dev/null | while IFS= read -r f; do
  e=$(stat -c %Y "$f" 2>/dev/null || echo 0)
  printf '%s\t%s\n' "$f" "$e"
done
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    AptBackup,
    SystemBak,
    Quarantine,
}

impl Kind {
    /// Chiave tradotta con `tr()` al momento della visualizzazione.
    pub fn label(self) -> &'static str {
        match self {
            Self::AptBackup => "Backup repository APT",
            Self::SystemBak => "File .bak di sistema",
            Self::Quarantine => "Quarantena",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Restore,
    Delete,
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Identificativo stabile: `apt:`/`bak:` + percorso, `q:` + id quarantena.
    pub id: String,
    pub kind: Kind,
    /// Data leggibile della copia o dello spostamento.
    pub date: String,
    /// Timestamp per ordinamento, indipendente dalla lingua.
    pub epoch: u64,
    /// File conservato (backup o file in quarantena).
    pub path: PathBuf,
    /// Percorso ripristinato dall'operazione.
    pub original: String,
    /// Modulo/fonte che ha creato la copia (chiave tradotta con `tr()`).
    pub origin: String,
    /// Chiave tradotta con `tr()` al momento della visualizzazione.
    pub status: String,
    pub size: u64,
    /// `true` quando serve pkexec (sempre per i file in /etc).
    pub privileged: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub entries: Vec<Entry>,
    /// True se una cartella di backup protetta non era leggibile.
    pub restricted: bool,
}

/// Nome di file senza separatori e senza traversal: usato per i backup APT,
/// i cui nomi sono generati dallo script come `NN-nome-file`.
fn valid_leaf(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 180
        && name
            .chars()
            .all(|c| !c.is_control() && c != '/' && c != '\\' && c != '\0')
}

/// Ricostruisce il percorso originale di un backup APT dal nome del file.
/// Lo script di pages_admin usa `NN-<nome originale>` e copia solo file da
/// `/etc/apt/sources.list` o `/etc/apt/sources.list.d/`.
fn apt_original(name: &str) -> Option<String> {
    let (index, suffix) = name.split_once('-')?;
    if index.len() != 2
        || !index.bytes().all(|b| b.is_ascii_digit())
        || suffix.ends_with(".bak")
        || !valid_leaf(suffix)
    {
        return None;
    }
    if suffix == "sources.list" {
        Some("/etc/apt/sources.list".to_string())
    } else {
        Some(format!("/etc/apt/sources.list.d/{suffix}"))
    }
}

/// Solo destinazioni sotto la radice di configurazione: un backup non può
/// indicare un percorso arbitrario fuori dalla configurazione di sistema.
fn allowed_original(path: &Path, roots: &Roots<'_>) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::CurDir | Component::Prefix(_)
        )
    }) {
        return false;
    }
    if !roots.in_etc(path) {
        return false;
    }
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(valid_leaf)
}

/// Toglie il suffisso di backup e accetta il risultato solo se è una delle
/// destinazioni ammesse (`fstab`, `hosts`, `grub`, sorgenti APT...).
fn system_original(path: &Path, roots: &Roots<'_>) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    for suffix in [".debutility.bak", ".debclean.bak", ".bak"] {
        if let Some(stem) = name.strip_suffix(suffix) {
            let candidate = path.with_file_name(stem);
            if allowed_original(&candidate, roots) {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn epoch_of(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn date_of(epoch: u64) -> String {
    if epoch == 0 {
        return String::new();
    }
    let script = format!("date -d @{} '+%Y-%m-%d %H:%M:%S' 2>/dev/null", epoch);
    let out = util::run_shell(&script, false).1;
    let text = out.trim();
    if text.is_empty() {
        epoch.to_string()
    } else {
        text.to_string()
    }
}

/// Verifica che il file conservato appartenga alle fonti riconosciute.
fn valid_backup_file(path: &Path, roots: &Roots<'_>) -> bool {
    if !path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        return false;
    }
    if roots.in_apt(path) {
        return path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| apt_original(n).is_some());
    }
    if !roots.in_etc(path) {
        return false;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let known = (roots.is_system() && EXPLICIT_BAKS.iter().copied().any(|p| Path::new(p) == path))
        || name.ends_with(".bak");
    known && system_original(path, roots).is_some()
}

fn push_unique(entries: &mut Vec<Entry>, seen: &mut HashSet<String>, entry: Entry) {
    if seen.insert(entry.id.clone()) {
        entries.push(entry);
    }
}

/// Costruisce una voce da un file di backup con data già risolta.
fn entry_from_path(path: &Path, epoch: u64, roots: &Roots<'_>) -> Option<Entry> {
    let name = path.file_name()?.to_str()?;
    let apt = apt_original(name);
    if roots.in_apt(path) != apt.is_some() {
        return None;
    }
    let (kind, original) = if let Some(original) = apt {
        (Kind::AptBackup, original)
    } else {
        let name = path.file_name()?.to_str()?;
        let known = (roots.is_system() && EXPLICIT_BAKS.iter().copied().any(|p| Path::new(p) == path))
            || (roots.in_etc(path) && name.ends_with(".bak"));
        if !known {
            return None;
        }
        (Kind::SystemBak, system_original(path, roots)?)
    };
    // Origine della copia: le schede APT/Grub/Gestore File di Sistema sono
    // chiavi già tradotte, così la colonna "Origine" resta multilingua.
    let origin = if kind == Kind::AptBackup {
        "Repository e driver"
    } else if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains("grub"))
    {
        "Kernel, Grub, Recovery"
    } else {
        "Gestore File di Sistema"
    };
    Some(Entry {
        id: format!(
            "{}:{}",
            if kind == Kind::AptBackup {
                "apt"
            } else {
                "bak"
            },
            path.display()
        ),
        kind,
        date: date_of(epoch),
        epoch,
        path: path.to_path_buf(),
        original,
        origin: origin.to_string(),
        status: STATUS_AVAILABLE.to_string(),
        size: fs::symlink_metadata(path).map(|m| m.len()).unwrap_or(0),
        privileged: true,
    })
}

/// Restituisce la chiave dello stato: la traduzione avviene nella UI.
fn quarantine_status(item: &quarantine::Item) -> &'static str {
    if quarantine::is_expired(item) {
        STATUS_EXPIRED
    } else if quarantine::days_left(item) <= 2 {
        STATUS_EXPIRING
    } else {
        STATUS_VALID
    }
}

fn add_quarantine(entries: &mut Vec<Entry>) -> Result<(), String> {
    let items = quarantine::load()?;
    let store = quarantine::dir()?;
    let mut seen = HashSet::new();
    for item in items {
        let safe_name = Path::new(&item.stored_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if safe_name.is_empty() || safe_name == "." || safe_name == ".." {
            continue;
        }
        push_unique(
            entries,
            &mut seen,
            Entry {
                id: format!("q:{}", item.id),
                kind: Kind::Quarantine,
                date: item.date.clone(),
                epoch: item.epoch,
                path: store.join(safe_name),
                original: item.original_path.clone(),
                origin: item.module.clone(),
                status: quarantine_status(&item).to_string(),
                size: item.size,
                privileged: item.privileged,
            },
        );
    }
    Ok(())
}

/// Scansiona in profondità le configurazioni di sistema cercando `.bak`.
/// I symlink non vengono seguiti e la profondità è limitata per evitare
/// attraversamenti accidentali di alberi molto grandi o montaggi esterni.
fn scan_etc_baks(
    dir: &Path,
    depth: usize,
    entries: &mut Vec<Entry>,
    seen: &mut HashSet<String>,
    roots: &Roots<'_>,
) {
    if depth > 8 {
        return;
    }
    let Ok(files) = fs::read_dir(dir) else { return };
    for file in files.flatten() {
        let path = file.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            scan_etc_baks(&path, depth + 1, entries, seen, roots);
            continue;
        }
        if meta.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".bak"))
        {
            if let Some(entry) = entry_from_path(&path, epoch_of(&path), roots) {
                push_unique(entries, seen, entry);
            }
        }
    }
}

/// Scansione senza privilegi: non chiede mai pkexec.
fn scan_fs(roots: &Roots<'_>) -> (Vec<Entry>, bool) {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut restricted = false;

    for root in roots.apt {
        let Ok(dirs) = fs::read_dir(*root) else {
            if matches!(
                fs::read_dir(*root).err().map(|e| e.kind()),
                Some(ErrorKind::PermissionDenied)
            ) {
                restricted = true;
            }
            continue;
        };
        for dir in dirs.flatten() {
            let dir_path = dir.path();
            if !fs::symlink_metadata(&dir_path)
                .map(|m| m.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            let files = match fs::read_dir(&dir_path) {
                Ok(v) => v,
                Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                    restricted = true;
                    continue;
                }
                Err(_) => continue,
            };
            for file in files.flatten() {
                let path = file.path();
                if !fs::symlink_metadata(&path)
                    .map(|m| m.is_file())
                    .unwrap_or(false)
                {
                    continue;
                }
                // `cp -p` conserva la data del file sorgente: per la data del
                // backup usa la cartella con timestamp creata dallo script.
                let epoch = epoch_of(&dir_path).max(epoch_of(&path));
                if let Some(entry) = entry_from_path(&path, epoch, roots) {
                    push_unique(&mut entries, &mut seen, entry);
                }
            }
        }
    }

    scan_etc_baks(roots.etc, 0, &mut entries, &mut seen, roots);

    // I percorsi espliciti sono utili solo nella scansione di sistema: con
    // radici temporanee non devono mai essere letti dalla /etc reale.
    for explicit in EXPLICIT_BAKS.iter().copied().filter(|_| roots.is_system()) {
        let path = Path::new(explicit);
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.is_file() => {
                if let Some(entry) = entry_from_path(path, epoch_of(path), roots) {
                    push_unique(&mut entries, &mut seen, entry);
                }
            }
            Err(e) if e.kind() == ErrorKind::PermissionDenied => restricted = true,
            _ => {}
        }
    }

    (entries, restricted)
}

/// Scansione con una sola richiesta di autorizzazione: restituisce direttamente
/// percorsi e timestamp letti come root.
fn scan_privileged() -> Result<(Vec<Entry>, bool), String> {
    let (ok, out) = util::run_shell(PRIV_SCAN, true);
    if !ok && permission_denied(&out) {
        return Err(
            t("Lettura annullata: autorizzazione di amministratore non concessa.").to_string(),
        );
    }
    if !ok {
        return Err(tf(
            "Impossibile leggere i backup protetti: {}",
            &[&util::tail(out.trim(), 2)],
        ));
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for line in out.lines() {
        let mut parts = line.splitn(2, '\t');
        let path = parts.next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let epoch = parts
            .next()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0);
        if let Some(entry) = entry_from_path(Path::new(path), epoch, &Roots::system()) {
            push_unique(&mut entries, &mut seen, entry);
        }
    }
    Ok((entries, false))
}

/// Scansione completa: `true` richiede pkexec, `false` resta in sola lettura.
pub fn scan(privileged: bool) -> Result<Scan, String> {
    scan_with(privileged, &Roots::system())
}

/// Variante sandboxed usata dai test: stessa logica di `scan`, ma con radici
/// temporanee. `privileged` resta sempre `false` nei test (nessun pkexec).
fn scan_with(privileged: bool, roots: &Roots<'_>) -> Result<Scan, String> {
    let (mut entries, restricted) = if privileged {
        scan_privileged()?
    } else {
        scan_fs(roots)
    };
    add_quarantine(&mut entries)?;
    entries.sort_by(|a, b| {
        b.epoch
            .cmp(&a.epoch)
            .then_with(|| ui::cmp_alpha(&a.original, &b.original))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(Scan {
        entries,
        restricted,
    })
}

/// Riconosce un rifiuto o un annullamento della finestra Polkit.
pub fn permission_denied(output: &str) -> bool {
    let low = output.to_lowercase();
    [
        "not authorized",
        "dismissed",
        "permission denied",
        "authentication failed",
        "authentication error",
    ]
    .iter()
    .any(|needle| low.contains(needle))
}

/// Script che ripristina un backup nella posizione originale. Il file attuale
/// può essere sovrascritto: la conferma viene chiesta prima di avviare il job.
pub fn restore_script(entry: &Entry) -> Result<String, String> {
    restore_script_with(entry, &Roots::system())
}

/// Versione con radici configurabili: usata dai test su cartelle temporanee,
/// senza pkexec e senza toccare `/etc`.
fn restore_script_with(entry: &Entry, roots: &Roots<'_>) -> Result<String, String> {
    if entry.kind == Kind::Quarantine {
        return Err(t("Usa il ripristino della Quarantena per questo elemento.").to_string());
    }
    if !valid_backup_file(&entry.path, roots) {
        return Err(t("Percorso del backup non valido.").to_string());
    }
    let original = PathBuf::from(&entry.original);
    if !allowed_original(&original, roots) {
        return Err(t("Percorso originale non valido.").to_string());
    }
    let backup = util::shq(&entry.path.to_string_lossy());
    let dest = util::shq(&entry.original);
    let parent = util::shq(&original.parent().unwrap_or(roots.etc).to_string_lossy());
    let mut script = String::from("set -eu\n");
    script.push_str(&format!(
        "[ -f {backup} ]\n[ ! -L {dest} ]\nmkdir -p -- {parent}\n"
    ));
    script.push_str(&format!("cp -p -f -- {backup} {dest}\n"));
    if original == Path::new("/etc/default/grub") {
        script.push_str("if command -v update-grub >/dev/null 2>&1; then update-grub; fi\n");
    }
    Ok(script)
}

/// Elimina un backup; la cartella con timestamp viene rimossa solo se resta vuota.
pub fn delete_script(entry: &Entry) -> Result<String, String> {
    delete_script_with(entry, &Roots::system())
}

fn delete_script_with(entry: &Entry, roots: &Roots<'_>) -> Result<String, String> {
    if entry.kind == Kind::Quarantine {
        return Err(t("Usa l'eliminazione della Quarantena per questo elemento.").to_string());
    }
    if !valid_backup_file(&entry.path, roots) {
        return Err(t("Percorso del backup non valido.").to_string());
    }
    let backup = util::shq(&entry.path.to_string_lossy());
    let mut script = String::from("set -eu\n");
    script.push_str(&format!("rm -f -- {backup}\n"));
    if let Some(parent) = entry.path.parent() {
        let dir = util::shq(&parent.to_string_lossy());
        script.push_str(&format!("rmdir -- {dir} 2>/dev/null || true\n"));
    }
    Ok(script)
}

/// Job unico per ripristino/eliminazione. Non riscansiona: la voce viene
/// rimossa dalla vista solo dopo esito positivo, evitando un secondo pkexec.
pub fn action_job(entry: Entry, action: Action) -> Job {
    Box::new(move |ch: &Chan| {
        let id = entry.id.clone();
        let is_quarantine = entry.kind == Kind::Quarantine;
        let privileged = entry.privileged;
        let original = entry.original.clone();
        let label = match action {
            Action::Restore => t("Ripristino in corso..."),
            Action::Delete => t("Eliminazione in corso..."),
        };
        let mut prog = ProgressReporter::start(ch, "restore_history", label);
        if privileged {
            prog.status(t("Attendo l'autorizzazione di amministratore..."));
        }

        let result: Result<String, String> = if is_quarantine {
            let qid = entry.id.strip_prefix("q:").unwrap_or(&entry.id);
            match action {
                Action::Restore => quarantine::restore(qid),
                Action::Delete => quarantine::delete_one(qid),
            }
        } else {
            let script = match action {
                Action::Restore => restore_script(&entry),
                Action::Delete => delete_script(&entry),
            };
            match script {
                Err(e) => Err(e),
                Ok(script) => {
                    let mut out = String::new();
                    let ok = util::run_stream(&script, true, |line, _commit| {
                        out.push_str(line);
                        out.push('\n');
                        prog.track_line(line);
                    });
                    if !ok {
                        let tail = util::tail(out.trim(), 2).replace('\n', " | ");
                        if permission_denied(&out) {
                            Err(t("Operazione annullata: autorizzazione di amministratore non concessa.").to_string())
                        } else if tail.is_empty() {
                            Err(t("Operazione terminata con errori").to_string())
                        } else {
                            Err(tail)
                        }
                    } else {
                        match action {
                            Action::Restore => Ok(tf("Ripristinato: {}", &[&original])),
                            Action::Delete => Ok(tf("Eliminato definitivamente: {}", &[&original])),
                        }
                    }
                }
            }
        };

        match result {
            Ok(msg) => {
                ch.send(Msg::RestoreHistoryMessage(msg, false));
                ch.send(Msg::RestoreHistoryRemoved(id.clone()));
                if is_quarantine {
                    match quarantine::load() {
                        Ok(items) => ch.send(Msg::QuarantineItems(items)),
                        Err(e) => ch.send(Msg::Log(format!("✘ {e}"))),
                    }
                }
            }
            Err(err) => {
                let msg = if permission_denied(&err) {
                    t("Operazione annullata: autorizzazione di amministratore non concessa.")
                        .to_string()
                } else {
                    match action {
                        Action::Restore => tf("Errore durante il ripristino: {}", &[&err]),
                        Action::Delete => tf("Errore durante l'eliminazione: {}", &[&err]),
                    }
                };
                ch.send(Msg::RestoreHistoryMessage(msg, true));
            }
        }
        prog.finish(if action == Action::Restore {
            t("Ripristino completato")
        } else {
            t("Eliminazione completata")
        });
    })
}

impl App {
    /// Ricarica senza privilegi. Con `privileged = true` avvia un job e usa una
    /// sola richiesta pkexec per leggere le cartelle protette.
    pub(crate) fn refresh_restore_history(&mut self, ctx: &egui::Context, privileged: bool) {
        if !privileged {
            match scan(false) {
                Ok(result) => {
                    let entries = result.entries;
                    self.restore_history_selected = self
                        .restore_history_selected
                        .take()
                        .filter(|id| entries.iter().any(|e| e.id == *id));
                    self.restore_history_restricted = result.restricted;
                    self.restore_history_entries = entries;
                }
                Err(e) => self.restore_history_msg = Some((e, true)),
            }
            return;
        }
        self.spawn(ctx, move |ch: &Chan| {
            let mut prog = ProgressReporter::start(
                ch,
                "restore_history",
                t("Lettura dei backup protetti in corso..."),
            );
            prog.status(t("Attendo l'autorizzazione di amministratore..."));
            match scan(true) {
                Ok(result) => {
                    ch.send(Msg::RestoreHistoryItems(result.entries, result.restricted));
                    ch.send(Msg::RestoreHistoryMessage(
                        t("Elenco aggiornato.").to_string(),
                        false,
                    ));
                }
                Err(e) => ch.send(Msg::RestoreHistoryMessage(e, true)),
            }
            prog.finish(t("Aggiornamento elenco completato"));
        });
    }

    pub(crate) fn ui_restore_history(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Cronologia e ripristino"));
        ui::hint(
            ui,
            t("Un unico elenco raccoglie i backup dei repository APT, i file .bak creati da DebUtility e gli elementi in Quarantena. Puoi ordinare, filtrare, ripristinare o eliminare una voce con conferma."),
        );

        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                let (badge, _) =
                    ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                ui::icon_badge(ui.painter(), badge, ui::WARN, "↺", 15.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(t("Operazioni reversibili registrate")).strong());
                    ui.label(
                        egui::RichText::new(t(
                            "Le voci lette in /etc richiedono pkexec per essere modificate.",
                        ))
                        .size(12.5)
                        .color(ui::text_dim()),
                    );
                });
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("Aggiorna elenco")).clicked() {
                    self.restore_history_msg = None;
                    self.refresh_restore_history(ctx, false);
                }
                let reload = ui.add_enabled(
                    self.action_enabled(),
                    egui::Button::new(t("Ricarica con privilegi")),
                );
                if reload
                    .on_hover_text(t("Richiede pkexec per leggere le cartelle protette"))
                    .clicked()
                {
                    self.refresh_restore_history(ctx, true);
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });

        if self.restore_history_restricted {
            ui.add_space(3.0);
            ui.colored_label(
                ui::semantic_text(ui::WARN),
                t("Alcuni backup in /etc non sono leggibili senza privilegi: usa «Ricarica con privilegi» per includerli."),
            );
        }
        if let Some((msg, is_error)) = &self.restore_history_msg {
            ui.add_space(3.0);
            ui.colored_label(
                if *is_error {
                    ui::semantic_text(ui::BAD)
                } else {
                    ui::semantic_text(ui::GOOD)
                },
                msg.as_str(),
            );
        }

        ui.add_space(4.0);
        ui::sub_tabs(
            ui,
            &mut self.restore_history_kind,
            &[
                "Tutti i tipi",
                "Backup repository APT",
                "File .bak di sistema",
                "Quarantena",
            ],
        );

        let total = self.restore_history_entries.len();
        let needle = self.restore_history_filter.to_lowercase();
        let kind_filter = self.restore_history_kind;
        let mut rows: Vec<Entry> = self
            .restore_history_entries
            .iter()
            .filter(|e| {
                if kind_filter == 1 && e.kind != Kind::AptBackup {
                    return false;
                }
                if kind_filter == 2 && e.kind != Kind::SystemBak {
                    return false;
                }
                if kind_filter == 3 && e.kind != Kind::Quarantine {
                    return false;
                }
                if needle.is_empty() {
                    return true;
                }
                e.original.to_lowercase().contains(&needle)
                    || tr(e.kind.label()).to_lowercase().contains(&needle)
                    || tr(&e.origin).to_lowercase().contains(&needle)
                    || tr(&e.status).to_lowercase().contains(&needle)
                    || e.date.to_lowercase().contains(&needle)
                    || e.path.to_string_lossy().to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();

        rows.sort_by(|a, b| {
            let ord = match self.restore_history_sort.key {
                HistorySortKey::Date => a.epoch.cmp(&b.epoch),
                HistorySortKey::Kind => {
                    let ak = tr(a.kind.label());
                    let bk = tr(b.kind.label());
                    ui::cmp_alpha(&ak, &bk)
                }
                HistorySortKey::Path => ui::cmp_alpha(&a.original, &b.original),
                HistorySortKey::Status => {
                    let as_ = tr(&a.status);
                    let bs = tr(&b.status);
                    ui::cmp_alpha(&as_, &bs)
                }
                HistorySortKey::Size => a.size.cmp(&b.size),
            };
            match self.restore_history_sort.dir {
                ui::SortDir::Asc => ord,
                ui::SortDir::Desc => ord.reverse(),
            }
        });

        ui::filter_bar(ui, &mut self.restore_history_filter, rows.len(), total);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(t("Ordina per:")).color(ui::text_dim()));
            ui::sort_header(
                ui,
                t("Data/Ora"),
                HistorySortKey::Date,
                &mut self.restore_history_sort,
            );
            ui::sort_header(
                ui,
                t("Tipo"),
                HistorySortKey::Kind,
                &mut self.restore_history_sort,
            );
            ui::sort_header(
                ui,
                t("Percorso originale"),
                HistorySortKey::Path,
                &mut self.restore_history_sort,
            );
            ui::sort_header(
                ui,
                t("Stato"),
                HistorySortKey::Status,
                &mut self.restore_history_sort,
            );
            ui::sort_header(
                ui,
                t("Dimensione"),
                HistorySortKey::Size,
                &mut self.restore_history_sort,
            );
        });

        let selected: Option<Entry> = self
            .restore_history_selected
            .as_ref()
            .and_then(|id| rows.iter().find(|e| e.id == *id))
            .cloned();

        ui.add_space(3.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.action_enabled() && selected.is_some(),
                    egui::Button::new(t("Ripristina selezionata")),
                )
                .clicked()
            {
                if let Some(entry) = &selected {
                    let text = if entry.kind == Kind::Quarantine {
                        tf("Ripristinare il file in {}?", &[&entry.original])
                    } else {
                        tf(
                            "Ripristinare il backup {} in {}? Il file attuale verrà sovrascritto.",
                            &[&tr(entry.kind.label()), &entry.original],
                        )
                    };
                    self.confirm = Some((text, action_job(entry.clone(), Action::Restore)));
                    ctx.request_repaint();
                }
            }
            if ui::danger_button(ui, t("Elimina selezionata"), self.action_enabled() && selected.is_some())
                .clicked()
            {
                if let Some(entry) = &selected {
                    let text = tf(
                        "Eliminare definitivamente {}? L'operazione non è reversibile.",
                        &[&entry.original],
                    );
                    self.confirm = Some((text, action_job(entry.clone(), Action::Delete)));
                    ctx.request_repaint();
                }
            }
        });

        ui.add_space(4.0);
        if rows.is_empty() {
            ui::card(ui, |ui| {
                ui.label(t("Nessuna operazione reversibile trovata."));
            });
            return;
        }

        ui.label(
            egui::RichText::new(tf("{} voci nella cronologia", &[&rows.len()]))
                .strong()
                .color(ui::accent_text()),
        );
        ui.add_space(3.0);

        egui::ScrollArea::vertical()
            .max_height(340.0)
            .show(ui, |ui| {
                for entry in &rows {
                    let is_selected =
                        self.restore_history_selected.as_deref() == Some(entry.id.as_str());
                    let head = format!("{} · {}", tr(entry.kind.label()), entry.date);
                    if ui.selectable_label(is_selected, head).clicked() {
                        self.restore_history_selected = Some(entry.id.clone());
                    }
                    ui::card(ui, |ui| {
                        // Sotto una certa larghezza le coppie chiave/valore vanno
                        // a capo: nessuna colonna fissa può tagliare i percorsi.
                        if ui.available_width() < 560.0 {
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&entry.original).strong());
                                ui.label(egui::RichText::new(&entry.date).color(ui::text_dim()));
                                ui.label(
                                    egui::RichText::new(tf("Stato: {}", &[&tr(&entry.status)]))
                                        .color(ui::semantic_text(
                                            if entry.status == STATUS_EXPIRED {
                                                ui::BAD
                                            } else {
                                                ui::GOOD
                                            },
                                        )),
                                );
                            });
                        } else {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(&entry.original).strong());
                                ui.label(egui::RichText::new(&entry.date).color(ui::text_dim()));
                                ui.label(
                                    egui::RichText::new(tf("Stato: {}", &[&tr(&entry.status)]))
                                        .color(ui::semantic_text(
                                            if entry.status == STATUS_EXPIRED {
                                                ui::BAD
                                            } else {
                                                ui::GOOD
                                            },
                                        )),
                                );
                            });
                        }
                        egui::Grid::new(format!("rh_{}", entry.id))
                            .num_columns(2)
                            .striped(true)
                            .show(ui, |ui| {
                                ui::kv(ui, "Tipo", &tr(entry.kind.label()));
                                ui::kv(ui, "Origine", &tr(&entry.origin));
                                ui::kv(ui, "Percorso originale", &entry.original);
                                ui::kv(ui, "Posizione del backup", &entry.path.to_string_lossy());
                                ui::kv(ui, "Stato", &tr(&entry.status));
                                ui::kv(ui, "Dimensione", &cleaner::fmt_bytes(entry.size));
                            });
                        if entry.privileged {
                            ui.label(
                                egui::RichText::new(t(
                                    "Richiede autorizzazione di amministratore (pkexec)",
                                ))
                                .size(12.0)
                                .color(ui::text_dim()),
                            );
                        }
                    });
                    ui.add_space(3.0);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quarantine;
    use crate::test_support::{EnvGuard, TempDir};
    use crate::util;
    use std::process::Command;

    #[test]
    fn backup_quarantena_cronologia_e_ripristino_in_sandbox() {
        // Tutto lo stato della quarantena viene rediretto in una cartella
        // univoca: né /etc né la home reale vengono mai toccati.
        let mut env = EnvGuard::new(&[
            "XDG_STATE_HOME",
            "PKEXEC_UID",
            "PKEXEC_GID",
            "SUDO_UID",
            "SUDO_GID",
        ]);
        let tmp = TempDir::new("history");
        let state = tmp.path.to_string_lossy().to_string();
        env.set("XDG_STATE_HOME", &state);
        for key in ["PKEXEC_UID", "PKEXEC_GID", "SUDO_UID", "SUDO_GID"] {
            env.remove(key);
        }

        let etc_dir = tmp.join("etc");
        let apt_root = tmp.join("apt_backups");
        let stamp_dir = apt_root.join("20260101000000");
        fs::create_dir_all(&etc_dir).expect("etc sandbox");
        fs::create_dir_all(&stamp_dir).expect("apt sandbox");

        let original = etc_dir.join("fstab");
        let original_text = "# fstab di test\nUUID=abc / ext4 defaults,noatime 0 1\n";
        fs::write(&original, original_text).expect("file originale");

        // 1. Copia `.bak` con la stessa helper usata in produzione.
        let backup = util::create_backup(&original).expect("creazione backup");
        assert_eq!(backup, etc_dir.join("fstab.bak"));
        assert_eq!(fs::read_to_string(&backup).expect("lettura backup"), original_text);

        // 2. Backup APT nella struttura usata dallo script (NN-nomefile).
        fs::write(
            stamp_dir.join("01-sources.list"),
            "deb http://example.invalid stable main\n",
        )
        .expect("backup APT");

        // 3. Quarantena del file originale, senza privilegi.
        let (moved, failed) = quarantine::quarantine_files(
            &[original.clone()],
            "Gestore File di Sistema",
            false,
        )
        .expect("spostamento in quarantena");
        assert_eq!((moved, failed), (1, 0));
        assert!(!original.exists(), "l'originale deve essere stato spostato");
        let qitems = quarantine::load().expect("manifesto quarantena");
        assert_eq!(qitems.len(), 1);
        assert_eq!(PathBuf::from(&qitems[0].original_path), original);

        // 4. Pannello unificato: backup sistema, backup APT e quarantena.
        let apt_root_s = apt_root.to_string_lossy().to_string();
        let apt_roots = [apt_root_s.as_str()];
        let roots = Roots { apt: &apt_roots, etc: &etc_dir };
        let scan = scan_with(false, &roots).expect("cronologia sandboxed");
        assert!(!scan.restricted);
        assert_eq!(scan.entries.len(), 3, "tre fonti devono comparire insieme");

        let apt = scan
            .entries
            .iter()
            .find(|e| e.kind == Kind::AptBackup)
            .expect("backup APT in cronologia");
        assert_eq!(apt.original, "/etc/apt/sources.list");

        let bak = scan
            .entries
            .iter()
            .find(|e| e.kind == Kind::SystemBak)
            .expect("backup .bak in cronologia");
        assert_eq!(bak.path, backup);
        assert_eq!(bak.original, original.to_string_lossy().to_string());
        assert_eq!(bak.size, original_text.len() as u64);

        let quarantined = scan
            .entries
            .iter()
            .find(|e| e.kind == Kind::Quarantine)
            .expect("quarantena in cronologia");
        assert_eq!(quarantined.id, format!("q:{}", qitems[0].id));
        assert_eq!(quarantined.original, original.to_string_lossy().to_string());

        // 5a. Ripristino della quarantena: contenuto e percorso identici.
        let msg = quarantine::restore(&qitems[0].id).expect("ripristino quarantena");
        assert!(msg.contains(&original.to_string_lossy().to_string()));
        assert_eq!(fs::read_to_string(&original).expect("file ripristinato"), original_text);
        assert!(quarantine::load().expect("manifesto dopo ripristino").is_empty());

        // 5b. Ripristino del .bak con lo script generato, eseguito direttamente
        // con `sh` su percorsi temporanei: nessun pkexec, nessun D-Bus.
        fs::write(&original, "contenuto danneggiato\n").expect("simula danno");
        let script = restore_script_with(bak, &roots).expect("script di ripristino");
        assert!(script.contains(&util::shq(&backup.to_string_lossy())));
        assert!(!script.contains("pkexec"));
        let out = Command::new("sh")
            .arg("-c")
            .arg(&script)
            .output()
            .expect("shell locale");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(fs::read_to_string(&original).expect("file ripristinato dal .bak"), original_text);
    }

    #[test]
    fn backup_non_puo_ripristinare_fuori_dalla_radice() {
        let tmp = TempDir::new("validation");
        let etc_dir = tmp.join("etc");
        fs::create_dir_all(&etc_dir).expect("etc sandbox");
        let backup = etc_dir.join("fstab.bak");
        fs::write(&backup, "contenuto\n").expect("backup");

        let roots = Roots { apt: &[], etc: &etc_dir };
        let base = Entry {
            id: "bak:test".into(),
            kind: Kind::SystemBak,
            date: String::new(),
            epoch: 0,
            path: backup.clone(),
            original: etc_dir.join("fstab").to_string_lossy().to_string(),
            origin: String::new(),
            status: STATUS_AVAILABLE.to_string(),
            size: 9,
            privileged: false,
        };

        let traversal = Entry { original: "../favicon.ico".into(), ..base.clone() };
        assert!(restore_script_with(&traversal, &roots).is_err());

        let outside = Entry {
            path: tmp.join("fuori.bak"),
            ..base.clone()
        };
        assert!(restore_script_with(&outside, &roots).is_err());

        // Percorso valido: lo script viene generato senza privilegi.
        assert!(restore_script_with(&base, &roots).is_ok());
        assert!(delete_script_with(&base, &roots).is_ok());
    }
}

/// Chiave di ordinamento della vista unificata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySortKey {
    Date,
    Kind,
    Path,
    Status,
    Size,
}
