//! Analizzatore disco con treemap squarified.
//!
//! La scansione gira in un thread separato e pubblica aggiornamenti sul canale
//! dell'app; la UI non blocca mai durante la lettura del file system.

use crate::i18n::{t, tf};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

/// Numero massimo di file pesanti mostrati nella lista.
pub const TOP_LIMIT: usize = 20;
/// Profondità di sicurezza: evita ricorsioni pathological su link/cicli strani.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskCategory {
    Media,
    Archive,
    Development,
    Documents,
    System,
}

impl DiskCategory {
    pub const ALL: &'static [DiskCategory] = &[
        DiskCategory::Media,
        DiskCategory::Archive,
        DiskCategory::Development,
        DiskCategory::Documents,
        DiskCategory::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DiskCategory::Media => t("Video/Media"),
            DiskCategory::Archive => t("Archivi/ISO"),
            DiskCategory::Development => t("Sviluppo/Build"),
            DiskCategory::Documents => t("Documenti"),
            DiskCategory::System => t("Sistema/Altro"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiskEntry {
    pub name: String,
    pub path: PathBuf,
    /// Dimensione della directory in modo ricorsivo, del file in modo diretto.
    pub size: u64,
    pub is_dir: bool,
    pub category: DiskCategory,
}

#[derive(Clone, Debug, Default)]
pub struct DirScan {
    pub path: PathBuf,
    /// Figli diretti della directory, con dimensione calcolata ricorsivamente.
    pub entries: Vec<DiskEntry>,
    /// File più pesanti dell'intero sottoalbero (non solo figli diretti).
    pub top_files: Vec<DiskEntry>,
    pub total_bytes: u64,
    pub files: usize,
    pub dirs: usize,
    /// Cartelle non leggibili o voci scomparse durante la scansione.
    pub errors: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct TreemapCell {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Default)]
struct Counters {
    files: usize,
    dirs: usize,
    errors: usize,
}

/// Classifica un elemento in base all'estensione (file) o al nome (cartelle di build).
pub fn category_for(path: &Path, is_dir: bool) -> DiskCategory {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if is_dir {
        let build_dirs = [
            "target", "node_modules", "build", "dist", "out", ".git", ".svn", ".hg", "vendor",
            "cmake-build-debug", "cmake-build-release", "debug", "release", "__pycache__",
            ".gradle", ".cargo", ".npm", ".venv", "venv", "bin", "obj",
        ];
        if build_dirs.contains(&name.as_str()) || name.starts_with("cmake-build-") {
            return DiskCategory::Development;
        }
        if matches!(name.as_str(), "video" | "videos" | "musica" | "music") {
            return DiskCategory::Media;
        }
        if matches!(name.as_str(), "documenti" | "documents" | "downloads" | "scaricati" | "foto" | "photos") {
            return DiskCategory::Documents;
        }
        return DiskCategory::System;
    }

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if matches!(
        ext.as_str(),
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
            | "3gp" | "ogv" | "mp3" | "flac" | "wav" | "ogg" | "opus" | "aac" | "m4a" | "wma"
            | "jpg" | "jpeg" | "png" | "gif" | "bmp" | "svg" | "webp" | "heic" | "tif" | "tiff"
            | "psd" | "raw"
    ) {
        DiskCategory::Media
    } else if matches!(
        ext.as_str(),
        "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "iso" | "img" | "dmg"
            | "deb" | "rpm" | "apk" | "tgz" | "txz" | "tbz" | "cab" | "jar"
    ) {
        DiskCategory::Archive
    } else if matches!(
        ext.as_str(),
        "rs" | "c" | "cc" | "cpp" | "h" | "hh" | "hpp" | "py" | "js" | "jsx" | "ts" | "tsx"
            | "go" | "java" | "kt" | "rb" | "php" | "sh" | "bash" | "sql" | "html" | "htm"
            | "css" | "scss" | "json" | "toml" | "yaml" | "yml" | "lock" | "rlib" | "rmeta"
            | "so" | "a" | "o" | "d" | "pdb" | "dll" | "exe" | "make" | "cmake" | "gradle"
    ) {
        DiskCategory::Development
    } else if matches!(
        ext.as_str(),
        "pdf" | "doc" | "docx" | "odt" | "txt" | "md" | "rtf" | "xls" | "xlsx" | "ods"
            | "ppt" | "pptx" | "odp" | "csv" | "epub" | "mobi" | "tex"
    ) {
        DiskCategory::Documents
    } else {
        DiskCategory::System
    }
}

/// true se la cartella va saltata (filesystem virtuali o mount point stranieri).
fn skip_dir(name: &str, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    let virtual_dirs = ["proc", "sys", "dev", "run", "snap", "lost+found", ".gvfs"];
    virtual_dirs.contains(&name)
}

/// Scansione ricorsiva: raccoglie i figli diretti e i 20 file più pesanti del sottoalbero.
/// `cancel` permette di interrompere subito una scansione quando si cambia cartella.
pub fn scan_directory_recursive(
    root: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(usize),
) -> DirScan {
    let mut entries = Vec::new();
    let mut top_files = Vec::new();
    let mut counters = Counters::default();

    walk(
        root,
        0,
        cancel,
        &mut entries,
        &mut top_files,
        &mut counters,
        on_progress,
    );

    entries.sort_unstable_by(compare_entries);
    top_files.sort_unstable_by(compare_entries);

    let total_bytes = entries.iter().map(|e| e.size).sum();
    DirScan {
        path: root.to_path_buf(),
        entries,
        top_files,
        total_bytes,
        files: counters.files,
        dirs: counters.dirs,
        errors: counters.errors,
    }
}

fn compare_entries(a: &DiskEntry, b: &DiskEntry) -> Ordering {
    b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name))
}

fn walk(
    dir: &Path,
    depth: usize,
    cancel: &AtomicBool,
    entries: &mut Vec<DiskEntry>,
    top_files: &mut Vec<DiskEntry>,
    counters: &mut Counters,
    on_progress: &mut dyn FnMut(usize),
) -> u64 {
    if depth > MAX_DEPTH || cancel.load(AtomicOrdering::Relaxed) {
        return 0;
    }

    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => {
            counters.errors += 1;
            return 0;
        }
    };

    let mut total = 0u64;
    for item in read.flatten() {
        if cancel.load(AtomicOrdering::Relaxed) {
            break;
        }
        let path = item.path();
        // I link simbolici non seguono la dimensione reale del bersaglio e
        // possono creare cicli: vengono esclusi.
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                counters.errors += 1;
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            continue;
        }

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        if meta.is_dir() {
            // File system virtuali (proc/sys/dev/...) e cartelle senza permesso
            // vengono saltati: la lettura fallisce o non è significativa.
            if skip_dir(&name, depth + 1) {
                continue;
            }
            counters.dirs += 1;
            on_progress(counters.files + counters.dirs);
            let size = walk(
                &path,
                depth + 1,
                cancel,
                entries,
                top_files,
                counters,
                on_progress,
            );
            total += size;
            let category = category_for(&path, true);
            entries.push(DiskEntry {
                name,
                path,
                size,
                is_dir: true,
                category,
            });
        } else if meta.is_file() {
            counters.files += 1;
            on_progress(counters.files + counters.dirs);
            let size = meta.len();
            total += size;
            let category = category_for(&path, false);
            let entry = DiskEntry {
                name,
                path,
                size,
                is_dir: false,
                category,
            };
            // Lista Top 20 a dimensione costante: niente vettore di milioni di file.
            if top_files.len() < TOP_LIMIT || size > top_files.last().map(|e| e.size).unwrap_or(0) {
                top_files.push(entry);
                top_files.sort_unstable_by(compare_entries);
                top_files.truncate(TOP_LIMIT);
            }
        } else {
            counters.errors += 1;
        }
    }
    total
}

fn worst(row: &[f64], short: f64) -> f64 {
    if row.is_empty() || short <= 0.0 {
        return f64::MAX;
    }
    let sum: f64 = row.iter().sum();
    if sum <= 0.0 {
        return f64::MAX;
    }
    let max = row.iter().cloned().fold(0.0_f64, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    (short * short * max / (sum * sum)).max(sum * sum / (short * short * min))
}

/// Layout squarified (Bruls, Huizing & van Wijk): rettangoli con aspect ratio
/// il più quadrato possibile, area proporzionale alla dimensione.
pub fn squarify(sizes: &[u64], area: (f32, f32, f32, f32), gap: f32) -> Vec<TreemapCell> {
    let (mut x, mut y, mut w, mut h) = area;
    let mut out = Vec::new();
    let total: u64 = sizes.iter().sum();
    if total == 0 || w <= 0.0 || h <= 0.0 {
        return out;
    }

    let mut items: Vec<(usize, f64)> = sizes
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0)
        .map(|(i, v)| (i, *v as f64))
        .collect();
    items.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

    let scale = (w as f64 * h as f64) / total as f64;
    let mut i = 0usize;
    while i < items.len() && w > 0.0 && h > 0.0 {
        let short = w.min(h) as f64;
        let mut row = vec![items[i].1 * scale];
        let mut j = i + 1;
        while j < items.len() {
            let next = items[j].1 * scale;
            let before = worst(&row, short);
            row.push(next);
            if worst(&row, short) > before {
                row.pop();
                break;
            }
            j += 1;
        }

        let row_sum: f64 = row.iter().sum();
        let start = i;
        if w >= h {
            // Colonne verticali: la riga occupa tutta l'altezza disponibile.
            let thickness = (row_sum / h as f64) as f32;
            if thickness <= 0.0 {
                break;
            }
            let mut cursor = y;
            for (k, a) in row.iter().enumerate() {
                let len = (*a / h as f64) as f32;
                if len > 0.0 {
                    out.push(inset(x, cursor, thickness, len, gap, items[start + k].0));
                }
                cursor += len;
            }
            x += thickness;
            w -= thickness;
        } else {
            // Righe orizzontali: la riga occupa tutta la larghezza disponibile.
            let thickness = (row_sum / w as f64) as f32;
            if thickness <= 0.0 {
                break;
            }
            let mut cursor = x;
            for (k, a) in row.iter().enumerate() {
                let len = (*a / w as f64) as f32;
                if len > 0.0 {
                    out.push(inset(cursor, y, len, thickness, gap, items[start + k].0));
                }
                cursor += len;
            }
            y += thickness;
            h -= thickness;
        }
        i = j;
    }
    out
}

fn inset(x: f32, y: f32, w: f32, h: f32, gap: f32, index: usize) -> TreemapCell {
    let g = gap.min(w * 0.4).min(h * 0.4);
    TreemapCell {
        index,
        x: x + g * 0.5,
        y: y + g * 0.5,
        w: w - g,
        h: h - g,
    }
}

/// Dimensione compatta (es. 1.24 GB).
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Dimensione esatta in byte, con separatori di migliaia.
pub fn format_exact(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c);
    }
    if bytes >= 1024 {
        tf("{} ({} byte)", &[&format_size(bytes), &grouped])
    } else {
        tf("{} byte", &[&grouped])
    }
}

/// Percorsi da mostrare nella breadcrumb, già assoluti.
pub fn breadcrumbs(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from("/")];
    let mut cur = PathBuf::from("/");
    for c in path.components() {
        let s = c.as_os_str();
        if s.is_empty() || s == "/" {
            continue;
        }
        cur.push(s);
        out.push(cur.clone());
    }
    out
}

/// Etichetta breve di un componente della breadcrumb.
pub fn short_name(path: &Path, home: &Path) -> String {
    if path == home {
        return t("Home").to_string();
    }
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

