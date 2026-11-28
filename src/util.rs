use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|u| u.parse().ok())
        })
        // Valore sentinella: se non riusciamo a leggere l'UID non dobbiamo
        // considerarci root e avviare operazioni privilegiate.
        .unwrap_or(u32::MAX)
}

pub fn is_root() -> bool {
    uid() == 0
}

/// Esegue uno script di shell. Se `privileged` e non siamo root usa pkexec
/// (finestra grafica di richiesta password).
pub fn run_shell(script: &str, privileged: bool) -> (bool, String) {
    let full = format!("export DEBIAN_FRONTEND=noninteractive LC_ALL=C; {script}");
    let mut cmd = if privileged && !is_root() {
        let mut c = Command::new("pkexec");
        c.args(["sh", "-c", &full]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", &full]);
        c
    };
    match cmd.output() {
        Ok(o) => {
            let mut t = String::from_utf8_lossy(&o.stdout).to_string();
            t.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.success(), t)
        }
        Err(e) => (false, format!("Impossibile eseguire il comando: {e}")),
    }
}

/// Espande il prefisso `~` nei percorsi digitati dall'utente nei campi di testo.
/// La shell non espande la tilde quando il percorso arriva come argomento.
pub fn expand_path(s: &str) -> String {
    let s = s.trim();
    if s == "~" {
        return home();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        let h = home();
        if !h.is_empty() {
            return format!("{h}/{rest}");
        }
    }
    s.to_string()
}

pub fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Racchiude una stringa tra apici singoli per la shell.
pub fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Home dell'utente che ha avviato DebUtility.
///
/// Se il programma gira come root (pkexec/sudo), `$HOME` sarebbe `/root` e
/// tutte le operazioni sulla home finirebbero nel posto sbagliato: in quel
/// caso risolviamo la home dell'utente reale tramite `PKEXEC_UID`/`SUDO_USER`.
pub fn home() -> String {
    if is_root() {
        let accounts = [
            std::env::var("PKEXEC_UID").ok(),
            std::env::var("SUDO_USER").ok(),
        ];
        for account in accounts.into_iter().flatten() {
            if let Some(h) = home_for_account(&account) {
                return h;
            }
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| {
            if is_root() {
                "/root".to_string()
            } else {
                // Senza HOME, risolvere la home reale evita percorsi relativi
                // alla cartella corrente (pericolosi nelle operazioni di pulizia).
                home_for_account(&uid().to_string()).unwrap_or_default()
            }
        })
}

/// Campo di `getent passwd` per un uid numerico o un nome utente.
fn passwd_field(account: &str, index: usize) -> Option<String> {
    let account = account.trim();
    if account.is_empty() {
        return None;
    }
    let out = Command::new("getent")
        .args(["passwd", account])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .next()
        .and_then(|line| line.split(':').nth(index))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Ricava la home di un utente (uid numerico o nome) da `getent passwd`.
fn home_for_account(account: &str) -> Option<String> {
    passwd_field(account, 5)
}

pub fn username() -> String {
    // Con pkexec/sudo `$USER` è root: risolvere l'account reale evita di
    // leggere/modificare i gruppi (o generare chiavi) dell'utente sbagliato.
    if is_root() {
        for account in [
            std::env::var("PKEXEC_UID").ok(),
            std::env::var("SUDO_USER").ok(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(name) = passwd_field(&account, 0) {
                return name;
            }
        }
    }
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| run_shell("id -un", false).1.trim().to_string())
}

/// Apre il selettore di file del sistema (zenity, yad o kdialog).
/// Restituisce None se nessuno è installato, un elenco vuoto se l'utente annulla.
pub fn pick_paths(dirs: bool) -> Option<Vec<String>> {
    let script = if dirs {
        "if command -v zenity >/dev/null 2>&1; then zenity --file-selection --directory --multiple --separator='|' --filename=\"$HOME/\" 2>/dev/null; \
         elif command -v yad >/dev/null 2>&1; then yad --file --directory --multiple --separator='|' --filename=\"$HOME/\" 2>/dev/null; \
         elif command -v kdialog >/dev/null 2>&1; then kdialog --getexistingdirectory \"$HOME\" 2>/dev/null; \
         else echo '::NODIALOG::'; fi"
    } else {
        "if command -v zenity >/dev/null 2>&1; then zenity --file-selection --multiple --separator='|' --filename=\"$HOME/\" 2>/dev/null; \
         elif command -v yad >/dev/null 2>&1; then yad --file --multiple --separator='|' --filename=\"$HOME/\" 2>/dev/null; \
         elif command -v kdialog >/dev/null 2>&1; then kdialog --getopenfilename \"$HOME\" --multiple --separate-output 2>/dev/null; \
         else echo '::NODIALOG::'; fi"
    };
    let (_, out) = run_shell(script, false);
    if out.contains("::NODIALOG::") {
        return None;
    }
    Some(
        out.split(|c| c == '|' || c == '\n')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
    )
}

/// Esegue uno script e comunica ogni riga di output appena compare.
/// `emit(testo, definitiva)`: definitiva = true se la riga è terminata da un a capo;
/// false per righe parziali o barre di avanzamento che verranno sostituite.
pub fn run_stream(script: &str, privileged: bool, mut emit: impl FnMut(&str, bool)) -> bool {
    use std::io::Read;
    use std::process::Stdio;
    let full = format!("export DEBIAN_FRONTEND=noninteractive LC_ALL=C; exec 2>&1; {script}");
    let mut cmd = if privileged && !is_root() {
        let mut c = Command::new("pkexec");
        c.args(["sh", "-c", &full]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", &full]);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit(&format!("Impossibile eseguire il comando: {e}"), true);
            return false;
        }
    };
    let mut buf = [0u8; 4096];
    let mut cur: Vec<u8> = Vec::new();
    let mut last_partial: Option<String> = None;
    if let Some(mut out) = child.stdout.take() {
        loop {
            let n = match out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for &b in &buf[..n] {
                match b {
                    b'\n' => {
                        if cur.is_empty() {
                            if let Some(p) = last_partial.take() {
                                emit(&p, true);
                                continue;
                            }
                        }
                        emit(&String::from_utf8_lossy(&cur), true);
                        cur.clear();
                        last_partial = None;
                    }
                    b'\r' => {
                        if !cur.is_empty() {
                            let p = String::from_utf8_lossy(&cur).to_string();
                            emit(&p, false);
                            last_partial = Some(p);
                        }
                        cur.clear();
                    }
                    _ => cur.push(b),
                }
            }
            if !cur.is_empty() {
                emit(&String::from_utf8_lossy(&cur), false);
            }
        }
    }
    if !cur.is_empty() {
        emit(&String::from_utf8_lossy(&cur), true);
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// Se DebUtility gira elevato, restituisce la cartella di stato all'utente
/// che ha richiesto l'elevazione: altrimenti la CLI `--user` non potrebbe
/// più leggere/scrivere lo stato.
fn chown_to_invoking_user(path: &Path) {
    if !is_root() {
        return;
    }
    // pkexec e sudo espongono variabili diverse: usare il primo pair valido.
    for (uid_var, gid_var) in [("PKEXEC_UID", "PKEXEC_GID"), ("SUDO_UID", "SUDO_GID")] {
        if let (Ok(uid), Ok(gid)) = (
            std::env::var(uid_var),
            std::env::var(gid_var),
        ) {
            if !uid.is_empty()
                && !gid.is_empty()
                && uid.bytes().all(|b| b.is_ascii_digit())
                && gid.bytes().all(|b| b.is_ascii_digit())
            {
                let _ = Command::new("chown")
                    .arg(format!("{uid}:{gid}"))
                    .arg(path)
                    .status();
                return;
            }
        }
    }
}

/// Cartella di stato per i dati scritti dalla modalità headless.
///
/// Usa `XDG_STATE_HOME/debutility` o `~/.local/state/debutility`; se la home non
/// è scrivibile ripiega su `~/.config/debutility` e infine su una cartella
/// privata in `/tmp`. Restituisce un errore esplicito se nessuna è utilizzabile.
pub fn state_dir() -> Result<PathBuf, String> {
    let home = home();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if xdg.starts_with('/') {
            candidates.push(PathBuf::from(xdg).join("debutility"));
        }
    }
    if !home.is_empty() {
        candidates.push(PathBuf::from(&home).join(".local/state/debutility"));
        candidates.push(PathBuf::from(&home).join(".config/debutility"));
    }
    candidates.push(PathBuf::from("/tmp").join(format!("debutility-state-{}", uid())));

    let mut last_error = String::new();
    for dir in candidates {
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {
                // La cartella contiene lo stato della pulizia: solo l'utente
                // che la crea deve poterla leggere/scrivere.
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| format!("Impossibile proteggere {}: {e}", dir.display()))?;
                // Se l'app gira con pkexec la cartella è di root: restituirla
                // all'utente reale, altrimenti la CLI non può leggerla.
                chown_to_invoking_user(&dir);
                return Ok(dir);
            }
            Err(e) => last_error = format!("{}: {e}", dir.display()),
        }
    }
    Err(format!(
        "Impossibile creare una cartella di stato per DebUtility ({last_error})"
    ))
}

/// Cartelle di stato usate dalle versioni precedenti, lette in fallback.
///
/// Non vengono create né modificate: servono solo a recuperare `last-run.json`
/// e `last-run.log` scritti quando l'applicazione si chiamava debclean.
pub fn legacy_state_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if xdg.starts_with('/') {
            dirs.push(PathBuf::from(xdg).join("debclean"));
        }
    }
    let home = home();
    if !home.is_empty() {
        dirs.push(PathBuf::from(&home).join(".local/state/debclean"));
        dirs.push(PathBuf::from(&home).join(".config/debclean"));
    }
    dirs.push(PathBuf::from("/tmp").join(format!("debclean-state-{}", uid())));
    dirs
}

/// Percorso della copia `.bak` di un file: il suffisso viene aggiunto al nome
/// completo, come fa `cp -p -n -- "$file" "$file.bak"` nella UI.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

/// Crea una copia di sicurezza `.bak` senza sovrascrivere quella esistente
/// (stessa semantica di `cp -n`). Usata quando DebUtility gira già come root;
/// negli altri casi la copia avviene dentro il job privilegiato.
pub fn create_backup(path: &Path) -> Result<PathBuf, String> {
    let backup = backup_path(path);
    if std::fs::symlink_metadata(&backup).is_ok() {
        return Ok(backup);
    }
    std::fs::copy(path, &backup)
        .map_err(|e| format!("Impossibile creare il backup {}: {e}", backup.display()))?;
    Ok(backup)
}

/// Scrive un file in modo atomico (scrittura temporanea + rename).
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Impossibile creare {}: {e}", dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".debutility-{}-{stamp}.tmp", std::process::id()));
    // create_new evita di seguire un link simbolico preesistente; mode(0o600)
    // impedisce che un file di stato privato diventi leggibile da tutti dopo
    // la rename.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("Impossibile creare {}: {e}", tmp.display()))?;
    if let Err(e) = file.write_all(contents.as_bytes()).and_then(|_| file.sync_all()) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Impossibile scrivere {}: {e}", tmp.display()));
    }
    drop(file);
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let cleanup = std::fs::remove_file(&tmp)
                .err()
                .map(|c| format!("; impossibile rimuovere {}: {c}", tmp.display()))
                .unwrap_or_default();
            Err(format!(
                "Impossibile sostituire {}: {e}{cleanup}",
                path.display()
            ))
        }
    }
}

/// Data/ora leggibile per i file di stato. `date` è presente su ogni Debian;
/// se manca viene usato il timestamp epoch, sempre non ambiguo.
pub fn timestamp() -> String {
    Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S %z")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("epoch {secs}")
        })
}

fn executable_file(p: &Path) -> bool {
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Eseguibile da mettere nell'unità systemd.
///
/// Usa il percorso del processo in esecuzione; se non è disponibile o non è
/// eseguibile cerca `debutility` nel PATH. Come ultima risorsa accetta anche il
/// vecchio eseguibile `debclean`, così le unità systemd precedenti restano
/// utilizzabili finché non vengono migrate.
pub fn debutility_executable() -> String {
    if let Ok(p) = std::env::current_exe() {
        if executable_file(&p) {
            return p.to_string_lossy().to_string();
        }
    }
    let mut legacy = None;
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':').filter(|d| !d.is_empty()) {
            let candidate = Path::new(dir).join("debutility");
            if executable_file(&candidate) {
                return candidate.to_string_lossy().to_string();
            }
            if legacy.is_none() {
                let old = Path::new(dir).join("debclean");
                if executable_file(&old) {
                    legacy = Some(old.to_string_lossy().to_string());
                }
            }
        }
    }
    legacy.unwrap_or_else(|| "debutility".to_string())
}

/// Racchiude un argomento per `ExecStart=` secondo la sintassi systemd:
/// virgolette doppie, escape C-style e raddoppio di `%` (specifier).
pub fn systemd_arg(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
