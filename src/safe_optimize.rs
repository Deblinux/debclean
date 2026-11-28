use crate::cleaner::{self, Kind};
use crate::i18n::{t, tf};
use crate::util::{home, run_shell, tail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const TAB_TITLE: &str = "Ottimizza in sicurezza";

pub const SAFE_KINDS: [Kind; 4] = [
    Kind::BrowserCache,
    Kind::AptCache,
    Kind::OldLogs,
    Kind::Thumbnails,
];

const BROWSER_CACHE_DIRS: [&str; 4] = [
    ".cache/mozilla",
    ".cache/google-chrome",
    ".cache/chromium",
    ".cache/BraveSoftware",
];

const BROWSER_PROCS: &[&str] = &[
    "firefox",
    "firefox-bin",
    "firefox-esr",
    "chrome",
    "google-chrome",
    "chromium",
    "chromium-browser",
    "brave",
    "brave-browser",
];

pub struct PreviewStep {
    pub title: &'static str,
    pub detail: &'static str,
    pub size: Option<u64>,
    pub will_run: bool,
    pub note: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Done,
    Skipped,
    Warning,
}

#[derive(Clone)]
pub struct StepResult {
    pub status: StepStatus,
    pub title: &'static str,
    pub detail: String,
    pub freed_bytes: Option<u64>,
}

#[derive(Clone, Default)]
pub struct OptimizeReport {
    pub entries: Vec<StepResult>,
    pub total_freed: u64,
}

impl OptimizeReport {
    pub fn done_count(&self) -> usize {
        self.entries.iter().filter(|e| e.status == StepStatus::Done).count()
    }

    pub fn skipped_count(&self) -> usize {
        self.entries.iter().filter(|e| e.status == StepStatus::Skipped).count()
    }

    pub fn warning_count(&self) -> usize {
        self.entries.iter().filter(|e| e.status == StepStatus::Warning).count()
    }

    pub fn into_log_lines(self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.entries.len() + 1);
        for e in &self.entries {
            let icon = match e.status {
                StepStatus::Done => "✔",
                StepStatus::Skipped => "⚠",
                StepStatus::Warning => "⚠",
            };
            if e.detail.is_empty() {
                lines.push(format!("{icon} {}", t(e.title)));
            } else {
                lines.push(format!("{icon} {}: {}", t(e.title), e.detail));
            }
        }
        lines.push(tf(
            "Riepilogo: {} completate, {} saltate, spazio liberato {}.",
            &[
                &self.done_count(),
                &self.skipped_count(),
                &cleaner::fmt_bytes(self.total_freed),
            ],
        ));
        lines
    }
}

fn any_browser_running() -> bool {
    for name in BROWSER_PROCS {
        match Command::new("pgrep").arg("-x").arg(name).output() {
            Ok(out) if out.status.success() => return true,
            // pgrep assente: non possiamo garantire che i browser siano chiusi.
            Err(_) => return true,
            _ => {}
        }
    }
    false
}

fn remove_dir_children(path: &Path) -> usize {
    let mut errors = 0;
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.flatten() {
            let p = entry.path();
            let res = match p.symlink_metadata() {
                Ok(md) if md.is_dir() => std::fs::remove_dir_all(&p),
                Ok(_) => std::fs::remove_file(&p),
                Err(e) => Err(e),
            };
            if res.is_err() {
                errors += 1;
            }
        }
    }
    errors
}

pub fn preview_steps() -> Vec<PreviewStep> {
    let browser_open = any_browser_running();
    let browser_note = if browser_open {
        Some(t("Saltato ora perché un browser è aperto"))
    } else {
        None
    };

    vec![
        PreviewStep {
            title: t("Riparazione pacchetti e dipendenze"),
            detail: t("Completa configurazioni APT/dpkg interrotte e sistema dipendenze rotte senza forzare rimozioni."),
            size: None,
            will_run: true,
            note: None,
        },
        PreviewStep {
            title: t("Cache dei browser"),
            detail: t("Svuota solo i file temporanei della cache. Non tocca password, preferiti, cronologia o cookie."),
            size: cleaner::size(Kind::BrowserCache),
            will_run: !browser_open,
            note: browser_note,
        },
        PreviewStep {
            title: t("Cache pacchetti APT"),
            detail: t("Rimuove i file .deb già installati e non più necessari. Non disinstalla alcun programma."),
            size: cleaner::size(Kind::AptCache),
            will_run: true,
            note: None,
        },
        PreviewStep {
            title: t("Log obsoleti in /var/log"),
            detail: t("Elimina solo i vecchi file di log compressi o ruotati (.gz, .old, numerati)."),
            size: cleaner::size(Kind::OldLogs),
            will_run: true,
            note: None,
        },
        PreviewStep {
            title: t("Miniature (Thumbnails)"),
            detail: t("Pulisce le anteprime salvate. Verranno rigenerate automaticamente se serve."),
            size: cleaner::size(Kind::Thumbnails),
            will_run: true,
            note: None,
        },
        PreviewStep {
            title: t("Compattazione database SQLite"),
            detail: t("Ottimizza i database dei browser chiusi senza cancellare dati o credenziali."),
            size: None,
            will_run: !browser_open,
            note: browser_note,
        },
    ]
}

pub fn estimated_bytes() -> u64 {
    preview_steps()
        .iter()
        .filter(|s| s.will_run)
        .filter_map(|s| s.size)
        .sum()
}

pub fn run_safe_optimize_report() -> OptimizeReport {
    run_safe_optimize_report_with(|_, _, _| {})
}

/// Come `run_safe_optimize_report`, con avanzamento a passi (fase, step, totale).
pub fn run_safe_optimize_report_with<F: FnMut(usize, usize, &str)>(
    mut on_progress: F,
) -> OptimizeReport {
    let mut report = OptimizeReport::default();
    let h = PathBuf::from(home());
    let browser_open = any_browser_running();

    let apt_before = cleaner::size(Kind::AptCache).unwrap_or(0);
    let logs_before = cleaner::size(Kind::OldLogs).unwrap_or(0);
    let browser_before = cleaner::size(Kind::BrowserCache).unwrap_or(0);
    let thumbs_before = cleaner::size(Kind::Thumbnails).unwrap_or(0);

    on_progress(1, 6, t("Riparazione pacchetti e dipendenze..."));
    let sys_script = "\
DEBIAN_FRONTEND=noninteractive dpkg --configure -a; \
apt-get install -f -y; \
apt-get clean; \
apt-get -y autoclean; \
find /var/log -type f \\( -name '*.gz' -o -name '*.old' -o -regex '.*\\.[0-9]+' \\) -delete";

    let (sys_ok, sys_out) = run_shell(sys_script, true);
    let summary = tail(sys_out.trim(), 4).replace('\n', " | ");

    if sys_ok {
        report.entries.push(StepResult {
            status: StepStatus::Done,
            title: "Riparazione pacchetti e dipendenze",
            detail: if summary.is_empty() {
                t("Operazioni di sistema completate.").to_string()
            } else {
                summary.clone()
            },
            freed_bytes: None,
        });
    } else {
        report.entries.push(StepResult {
            status: StepStatus::Warning,
            title: "Riparazione pacchetti e dipendenze",
            detail: if summary.is_empty() {
                t("Servono verifiche manuali.").to_string()
            } else {
                summary.clone()
            },
            freed_bytes: None,
        });
    }

    on_progress(2, 6, t("Cache pacchetti APT..."));
    let apt_after = cleaner::size(Kind::AptCache).unwrap_or(0);
    let logs_after = cleaner::size(Kind::OldLogs).unwrap_or(0);
    let apt_freed = apt_before.saturating_sub(apt_after);
    let logs_freed = logs_before.saturating_sub(logs_after);
    report.total_freed += apt_freed + logs_freed;

    report.entries.push(StepResult {
        status: if sys_ok { StepStatus::Done } else { StepStatus::Warning },
        title: "Cache pacchetti APT",
        detail: tf("Liberati {}", &[&cleaner::fmt_bytes(apt_freed)]),
        freed_bytes: Some(apt_freed),
    });

    report.entries.push(StepResult {
        status: if sys_ok { StepStatus::Done } else { StepStatus::Warning },
        title: "Log obsoleti in /var/log",
        detail: tf("Liberati {}", &[&cleaner::fmt_bytes(logs_freed)]),
        freed_bytes: Some(logs_freed),
    });

    on_progress(3, 6, t("Cache dei browser..."));
    if browser_open {
        report.entries.push(StepResult {
            status: StepStatus::Skipped,
            title: "Cache dei browser",
            detail: t("Saltato ora perché un browser è aperto").to_string(),
            freed_bytes: None,
        });
    } else {
        let errs: usize = BROWSER_CACHE_DIRS
            .iter()
            .map(|d| remove_dir_children(&h.join(d)))
            .sum();
        let browser_after = cleaner::size(Kind::BrowserCache).unwrap_or(0);
        let freed = browser_before.saturating_sub(browser_after);
        report.total_freed += freed;
        let mut det = tf("Liberati {}", &[&cleaner::fmt_bytes(freed)]);
        if errs > 0 {
            det.push_str(&format!(" · {}", tf("{} elementi saltati", &[&errs])));
        }
        report.entries.push(StepResult {
            status: if errs == 0 { StepStatus::Done } else { StepStatus::Warning },
            title: "Cache dei browser",
            detail: det,
            freed_bytes: Some(freed),
        });
    }

    on_progress(4, 6, t("Miniature (thumbnails)..."));
    let thumb_errs = remove_dir_children(&h.join(".cache/thumbnails"));
    let thumb_after = cleaner::size(Kind::Thumbnails).unwrap_or(0);
    let thumb_freed = thumbs_before.saturating_sub(thumb_after);
    report.total_freed += thumb_freed;
    let mut thumb_det = tf("Liberati {}", &[&cleaner::fmt_bytes(thumb_freed)]);
    if thumb_errs > 0 {
        thumb_det.push_str(&format!(" · {}", tf("{} elementi saltati", &[&thumb_errs])));
    }
    report.entries.push(StepResult {
        status: if thumb_errs == 0 { StepStatus::Done } else { StepStatus::Warning },
        title: "Miniature (Thumbnails)",
        detail: thumb_det,
        freed_bytes: Some(thumb_freed),
    });

    on_progress(5, 6, t("Compattazione database SQLite..."));
    if browser_open {
        report.entries.push(StepResult {
            status: StepStatus::Skipped,
            title: "Compattazione database SQLite",
            detail: t("Saltato ora perché un browser è aperto").to_string(),
            freed_bytes: None,
        });
    } else {
        let db_lines = cleaner::optimize_browser_databases();
        let missing_sqlite = db_lines
            .iter()
            .any(|l| l.to_lowercase().contains("sqlite3") && l.to_lowercase().contains("non trovato"));
        if missing_sqlite {
            report.entries.push(StepResult {
                status: StepStatus::Skipped,
                title: "Compattazione database SQLite",
                detail: t("Comando 'sqlite3' non trovato sul sistema.").to_string(),
                freed_bytes: None,
            });
        } else {
            let det = if db_lines.is_empty() {
                t("Ottimizzazione completata.").to_string()
            } else {
                db_lines.join(" | ")
            };
            report.entries.push(StepResult {
                status: StepStatus::Done,
                title: "Compattazione database SQLite",
                detail: det,
                freed_bytes: None,
            });
        }
    }

    on_progress(6, 6, t("Riepilogo..."));
    report
}

pub fn run_safe_optimize() -> Vec<String> {
    run_safe_optimize_report().into_log_lines()
}
