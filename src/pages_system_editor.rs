use crate::i18n::{t, tf};
use eframe::egui;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub struct SystemFileEditor {
    pub selected_path: String,
    pub content: String,
    pub status_msg: String,
    pub saving: bool,
}

impl SystemFileEditor {
    pub fn new() -> Self {
        Self {
            selected_path: "/etc/fstab".into(),
            content: String::new(),
            status_msg: String::new(),
            saving: false,
        }
    }

    pub fn load_file(&mut self, path: &str) {
        self.selected_path = path.to_string();
        match fs::read_to_string(path) {
            Ok(c) => {
                self.content = c;
                self.status_msg = tf("Caricato con successo: {}", &[&path]);
            }
            Err(e) => {
                self.content = String::new();
                self.status_msg = tf("Errore di lettura: {}", &[&e]);
            }
        }
    }

    pub fn save_file(&mut self, path: &str) -> Result<String, String> {
        if path.is_empty() {
            return Err(t("Percorso non valido.").to_string());
        }

        // Controllo strutturale prima di qualsiasi scrittura: il parser è lo
        // stesso coperto dai test, così una riga fstab incompleta non arriva
        // nemmeno alla validazione con `mount --fake`.
        if path == "/etc/fstab" {
            let bad = crate::config_parsers::fstab_malformed(&self.content);
            if !bad.is_empty() {
                return Err(tf("Sintassi fstab non valida! {}", &[&format!("riga {}", bad[0])]));
            }
        }

        // File temporaneo con nome univoco (pid + istante) per non scrivere su nomi prevedibili in /tmp
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp_path = format!("/tmp/debutility-edit-{}-{}", std::process::id(), stamp);
        // create_new + mode 0600: mai seguire un symlink preesistente e mai
        // lasciare una copia leggibile di file sensibili (/etc/fstab, hosts...).
        let mut tmp = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp_path)
            .map_err(|e| tf("Errore creazione file temporaneo: {}", &[&e]))?;
        if let Err(e) = tmp.write_all(self.content.as_bytes()).and_then(|_| tmp.sync_all()) {
            let _ = fs::remove_file(&tmp_path);
            return Err(tf("Errore scrittura file temporaneo: {}", &[&e]));
        }
        drop(tmp);

        // 1. Validazione sintattica specifica per fstab, PRIMA di toccare qualsiasi file
        if path == "/etc/fstab" {
            let output = std::process::Command::new("mount")
                .args(["-a", "--fake", "-T", tmp_path.as_str()])
                .output();

            match output {
                Ok(res) if res.status.success() => {}
                Ok(res) => {
                    let err_out = String::from_utf8_lossy(&res.stderr).to_string();
                    let _ = fs::remove_file(&tmp_path);
                    return Err(tf("Sintassi fstab non valida! {}", &[&err_out]));
                }
                Err(e) => {
                    // `mount` assente: non possiamo garantire la validazione,
                    // quindi non scrivere un file che può rompere il boot.
                    let _ = fs::remove_file(&tmp_path);
                    return Err(tf("Impossibile validare fstab (mount non disponibile): {}", &[&e]));
                }
            }
        }

        // Con privilegi root la copia `.bak` può essere creata direttamente,
        // sempre con semantica `cp -n`; senza privilegi la crea lo script qui sotto.
        if crate::util::is_root()
            && fs::symlink_metadata(path).map(|m| m.is_file()).unwrap_or(false)
        {
            let _ = crate::util::create_backup(Path::new(path));
        }

        // 2. Backup + scrittura atomica come amministratore, in un'unica operazione privilegiata.
        //    Il backup (.bak) si fa con i permessi di root (come utente normale non si può scrivere in /etc)
        //    e il nuovo contenuto sostituisce il file con un mv, così un'interruzione a metà
        //    non lascia mai un fstab troncato. Permessi e proprietario originali vengono mantenuti.
        let script = r#"set -e
if [ -L "$1" ]; then echo 'Destinazione symlink: salvataggio annullato.' >&2; exit 1; fi
if [ -e "$1" ]; then cp -p -n -- "$1" "$1.bak" || true; fi
NEW="$(mktemp "$1.debutility-new.XXXXXX")"
trap 'rm -f -- "$NEW"' EXIT
cat -- "$2" > "$NEW"
chmod --reference="$1" "$NEW" 2>/dev/null || chmod 644 "$NEW"
chown --reference="$1" "$NEW" 2>/dev/null || true
mv -f -- "$NEW" "$1"
trap - EXIT"#;

        let mut cmd = if crate::util::is_root() {
            std::process::Command::new("sh")
        } else {
            let mut c = std::process::Command::new("pkexec");
            c.arg("sh");
            c
        };
        let res = cmd.args(["-c", script, "sh", path, tmp_path.as_str()]).output();
        let _ = fs::remove_file(&tmp_path);

        match res {
            Ok(o) if o.status.success() => Ok(t("File salvato correttamente (copia precedente in .bak).").to_string()),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if err.is_empty() {
                    Err(t("Salvataggio fallito o autorizzazione negata.").to_string())
                } else {
                    Err(tf("Salvataggio fallito: {}", &[&err]))
                }
            }
            Err(e) => Err(tf("Errore esecuzione pkexec: {}", &[&e])),
        }
    }

    pub fn save_file_async(
        &self,
        path: String,
        content: String,
        tx: std::sync::mpsc::Sender<crate::Msg>,
        ctx: egui::Context,
    ) {
        std::thread::spawn(move || {
            let mut editor = Self::new();
            editor.selected_path = path.clone();
            editor.content = content;
            let res = editor.save_file(&path);
            let _ = tx.send(crate::Msg::SystemEditorSaved(res));
            ctx.request_repaint();
        });
    }
}
