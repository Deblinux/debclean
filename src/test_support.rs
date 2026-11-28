//! Infrastruttura comune ai test: variabili d'ambiente isolate e cartelle
//! temporanee univoche. Compilata solo con `cfg(test)`, mai nel binario finale.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

/// Le variabili d'ambiente sono globali: tutti i test che le toccano devono
/// prendere questo lock, altrimenti `cargo test` può eseguirli in parallelo.
pub static ENV_LOCK: Mutex<()> = Mutex::new(());

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Salva le variabili indicate, permette di modificarle e le ripristina alla
/// fine del test (anche in caso di panic).
pub struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    pub fn new(vars: &[&'static str]) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = vars.iter().map(|key| (*key, std::env::var_os(*key))).collect();
        Self { _lock: lock, saved }
    }

    pub fn set(&mut self, key: &str, value: &str) {
        std::env::set_var(key, value);
    }

    pub fn remove(&mut self, key: &str) {
        std::env::remove_var(key);
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(v) => std::env::set_var(*key, v),
                None => std::env::remove_var(*key),
            }
        }
    }
}

/// Cartella univoca sotto `std::env::temp_dir()`, rimossa alla fine del test.
/// Nessun test scrive nella home reale, in `/etc` o in percorsi di sistema.
pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        let base = std::env::temp_dir();
        let pid = std::process::id();
        for _ in 0..10_000 {
            let seq = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("debutility-test-{pid}-{seq}-{label}"));
            if std::fs::create_dir(&path).is_ok() {
                return Self { path };
            }
        }
        panic!("impossibile creare una cartella temporanea di test per {label}");
    }

    pub fn join(&self, parts: &str) -> PathBuf {
        self.path.join(parts)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
