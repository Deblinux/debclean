//! Scansione e pulizia di artefatti di sviluppo e container.
//!
//! Tutta l'analisi del filesystem e le chiamate alle CLI (docker/podman)
//! girano in un thread di lavoro: la UI riceve solo snapshot tramite `Chan`.

use crate::i18n::{t, tf};
use crate::progress::ProgressReporter;
use crate::{cleaner, ui, util, App, Msg};
use eframe::egui;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CategoryId {
    Container,
    Rust,
    Node,
    Python,
}

#[derive(Clone)]
pub struct Item {
    pub label: String,
    pub path: String,
    pub detail: String,
    pub size: u64,
    pub selected: bool,
    pub stale: bool,
    /// Per gli elementi Docker/Podman: comando di prune da eseguire.
    pub remove_cmd: Option<String>,
}

#[derive(Clone)]
pub struct Category {
    pub id: CategoryId,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub installed: bool,
    /// Versione (o motore) mostrata nell'header della card.
    pub version: String,
    pub items: Vec<Item>,
}

impl Category {
    pub fn recoverable(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }

    pub fn selected_size(&self) -> u64 {
        self.items.iter().filter(|i| i.selected).map(|i| i.size).sum()
    }
}

#[derive(Clone, Default)]
pub struct Report {
    pub categories: Vec<Category>,
}

impl Report {
    pub fn has_selection(&self) -> bool {
        self.categories
            .iter()
            .any(|c| c.items.iter().any(|i| i.selected))
    }
}

pub struct CleanSummary {
    pub freed: u64,
    pub errors: usize,
    pub done: usize,
    pub messages: Vec<String>,
}

// ---- Rilevamento strumenti ---------------------------------------------------------------------

fn has_cmd(name: &str) -> bool {
    util::run_shell(&format!("command -v {name} >/dev/null 2>&1"), false).0
}

fn version_of(cmds: &[&str]) -> String {
    for c in cmds {
        if !has_cmd(c) {
            continue;
        }
        let (_, out) = util::run_shell(&format!("{c} --version 2>&1 | head -n 1"), false);
        let line = out.trim().lines().next().unwrap_or("").trim();
        if !line.is_empty() {
            return line.to_string();
        }
    }
    String::new()
}

// ---- Stats filesystem e date -------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Stats {
    bytes: u64,
    modified: SystemTime,
}

fn dir_stats(path: &Path) -> Stats {
    fn walk(p: &Path, st: &mut Stats) {
        let Ok(md) = fs::symlink_metadata(p) else { return };
        st.bytes += md.len();
        if let Ok(m) = md.modified() {
            if m > st.modified {
                st.modified = m;
            }
        }
        if md.is_dir() {
            if let Ok(rd) = fs::read_dir(p) {
                for e in rd.flatten() {
                    walk(&e.path(), st);
                }
            }
        }
    }

    let mut st = Stats { bytes: 0, modified: UNIX_EPOCH };
    walk(path, &mut st);
    st
}

fn age_days(t: SystemTime) -> u64 {
    SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

/// Data UTC `AAAA-MM-GG` senza dipendenze da crate di date.
fn fmt_date(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    format!("{:04}-{:02}-{:02}", y + if m <= 2 { 1 } else { 0 }, m, d)
}

fn display_path(path: &Path, home: &Path) -> String {
    if !home.as_os_str().is_empty() {
        if let Ok(rel) = path.strip_prefix(home) {
            return format!("~/{}", rel.display());
        }
    }
    path.display().to_string()
}

/// Rimuove il CONTENUTO di una cartella, lasciando la cartella stessa.
/// I link simbolici vengono rimossi come file, mai seguiti.
fn remove_contents(path: &Path) -> usize {
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => return if path.exists() { 1 } else { 0 },
    };
    let mut errors = 0;
    for e in rd.flatten() {
        let p = e.path();
        let res = match fs::symlink_metadata(&p) {
            Ok(md) if md.is_dir() => fs::remove_dir_all(&p),
            Ok(_) => fs::remove_file(&p),
            Err(_) => continue,
        };
        if res.is_err() {
            errors += 1;
        }
    }
    errors
}

struct Finder {
    visited: usize,
    found: Vec<(PathBuf, Stats)>,
}

/// Cerca cartelle con un nome dato nella home, senza seguire link simbolici.
/// Profondità e quantità di cartelle esaminate sono limitate per non rallentare
/// la scansione su home molto popolate.
fn find_named<F: FnMut(f32, &str)>(
    home: &Path,
    name: &str,
    skip: &[&str],
    progress: &mut F,
) -> Vec<(PathBuf, Stats)> {
    fn walk<F: FnMut(f32, &str)>(
        dir: &Path,
        depth: usize,
        name: &str,
        skip: &[&str],
        state: &mut Finder,
        progress: &mut F,
    ) {
        if depth > 6 {
            return;
        }
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let fname = e.file_name().to_string_lossy().to_string();
            if fname.starts_with('.') || skip.iter().any(|s| *s == fname) {
                continue;
            }
            state.visited += 1;
            if state.visited % 25 == 0 {
                let frac = (state.visited.min(400) as f32) / 400.0;
                progress(frac, &format!("{}: {} {}", name, state.visited, t("cartelle esaminate")));
            }
            if fname == name {
                let p = e.path();
                state.found.push((p.clone(), dir_stats(&p)));
                continue;
            }
            walk(&e.path(), depth + 1, name, skip, state, progress);
        }
    }

    let mut state = Finder { visited: 0, found: Vec::new() };
    walk(home, 0, name, skip, &mut state, progress);
    state.found
}

// ---- Parser JSON minimale (nessuna dipendenza aggiuntiva) -------------------------------------

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    h.windows(n.len()).position(|w| w.eq_ignore_ascii_case(n))
}

fn matching_brace(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Contenuto (senza parentesi) di ogni oggetto JSON, array o righe separate.
fn json_objects(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = matching_brace(bytes, i) {
                out.push(text[i + 1..end].to_string());
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn json_value(obj: &str, keys: &[&str]) -> Option<String> {
    for key in keys {
        let pat = format!("\"{key}\"");
        let Some(pos) = find_ci(obj, &pat) else { continue };
        let Some(colon) = obj[pos + pat.len()..].trim_start().strip_prefix(':') else { continue };
        let val = colon.trim_start();
        if let Some(rest) = val.strip_prefix('"') {
            let mut s = String::new();
            let mut esc = false;
            for c in rest.chars() {
                if esc {
                    s.push(c);
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    return Some(s);
                } else {
                    s.push(c);
                }
            }
            continue;
        }
        let num: String = val
            .chars()
            .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
            .collect();
        if !num.is_empty() {
            return Some(num);
        }
    }
    None
}

fn json_num(obj: &str, keys: &[&str]) -> Option<f64> {
    let v = json_value(obj, keys)?;
    if let Ok(n) = v.parse::<f64>() {
        return Some(n);
    }
    // Alcune versioni di Podman restituiscono dimensioni come "1.2GB".
    parse_size(&v).map(|b| b as f64)
}

/// Contenuto di un oggetto annidato sotto una chiave (formato usato da Podman).
fn nested_object<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let pos = find_ci(obj, &pat)?;
    let tail = &obj[pos + pat.len()..];
    let colon = tail.find(':')?;
    let val = &tail[colon + 1..];
    let brace = val.find('{')?;
    let start = pos + pat.len() + colon + 1 + brace;
    let end = matching_brace(obj.as_bytes(), start)?;
    Some(&obj[start + 1..end])
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let n: f64 = s[..split].parse().ok()?;
    let unit = s[split..].trim().to_ascii_lowercase();
    let mult = match unit.as_str() {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024f64.powi(2),
        "g" | "gb" | "gib" => 1024f64.powi(3),
        "t" | "tb" | "tib" => 1024f64.powi(4),
        _ => return None,
    };
    Some((n * mult).max(0.0) as u64)
}

/// (etichetta, suffisso comando prune) per ogni tipo restituito da `system df`.
fn container_action(typ: &str) -> Option<(&'static str, &'static str)> {
    let l = typ.to_ascii_lowercase();
    if l.contains("image") {
        Some((t("Immagini non utilizzate"), "image prune -af"))
    } else if l.contains("container") {
        Some((t("Container fermi"), "container prune -f"))
    } else if l.contains("build") || l.contains("cache") {
        Some((t("Cache di build"), "builder prune -af"))
    } else if l.contains("volume") {
        Some((t("Volumi orfani"), "volume prune -f"))
    } else {
        None
    }
}

fn scan_container(runtime: &str, progress: &mut impl FnMut(f32, &str)) -> Vec<Item> {
    let (ok, out) = util::run_shell(&format!("{runtime} system df --format json 2>/dev/null"), false);
    progress(0.35, t("Lettura dell'utilizzo disco del motore container..."));

    let mut rows: Vec<(String, u64)> = Vec::new();
    if ok {
        for obj in json_objects(&out) {
            if let (Some(typ), Some(rec)) =
                (json_value(&obj, &["Type", "type"]), json_num(&obj, &["Reclaimable", "reclaimable"]))
            {
                rows.push((typ, rec.max(0.0) as u64));
                continue;
            }
            // Podman può restituire oggetti annidati per tipo.
            for (key, alias) in [
                ("Images", t("Immagini non utilizzate")),
                ("Containers", t("Container fermi")),
                ("BuildCache", t("Cache di build")),
                ("Build", t("Cache di build")),
                ("Volumes", t("Volumi orfani")),
            ] {
                if let Some(inner) = nested_object(&obj, key) {
                    let rec = json_num(inner, &["Reclaimable", "reclaimable"]).unwrap_or(0.0);
                    rows.push((alias.to_string(), rec.max(0.0) as u64));
                }
            }
        }
    }

    // Fallback per versioni che non supportano `--format json`.
    if rows.is_empty() {
        let (_, text) = util::run_shell(&format!("{runtime} system df 2>/dev/null"), false);
        for line in text.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                if let Some(size) = parse_size(parts[4]) {
                    rows.push((parts[0].to_string(), size));
                }
            }
        }
    }

    let mut items = Vec::new();
    for (i, (typ, size)) in rows.iter().enumerate() {
        progress(0.4 + 0.55 * (i + 1) as f32 / rows.len().max(1) as f32, typ.as_str());
        let Some((label, action)) = container_action(typ) else { continue };
        if items.iter().any(|it: &Item| it.label == label) {
            continue;
        }
        let cmd = format!("{runtime} {action}");
        items.push(Item {
            label: label.to_string(),
            path: String::new(),
            detail: format!("{}: {}", t("Comando"), cmd),
            size: *size,
            selected: true,
            stale: false,
            remove_cmd: Some(cmd),
        });
    }
    items
}

/// Cartelle da non attraversare durante la ricerca nella home: cache e dati di
/// strumenti che possono contenere migliaia di cartelle senza progetti utili.
const SKIP_RUST: &[&str] = &[
    "node_modules", ".git", ".cache", ".cargo", ".rustup", ".npm", ".config", ".local", ".mozilla", ".steam", "snap",
];

const SKIP_NODE: &[&str] = &[
    "target", ".git", ".cache", ".cargo", ".rustup", ".npm", ".config", ".local", ".mozilla", ".steam", "snap",
];

fn scan_rust(home: &Path, progress: &mut impl FnMut(f32, &str)) -> Vec<Item> {
    let mut items = Vec::new();

    let cache = home.join(".cargo/registry/cache");
    let is_real_dir = fs::symlink_metadata(&cache).map(|m| m.is_dir()).unwrap_or(false);
    if is_real_dir {
        let st = dir_stats(&cache);
        items.push(Item {
            label: t("Cache registry Cargo").to_string(),
            path: cache.to_string_lossy().to_string(),
            detail: format!("{}: {}", t("Ultima modifica"), fmt_date(st.modified)),
            size: st.bytes,
            selected: true,
            stale: false,
            remove_cmd: None,
        });
    }
    progress(0.25, t("Ricerca di cartelle target/ nella home..."));

    let found = find_named(home, "target", SKIP_RUST, progress);
    for (dir, st) in found {
        // Solo vere cartelle di build Cargo, non cartelle `target` qualsiasi.
        let is_cargo = dir
            .parent()
            .map(|p| p.join("Cargo.toml").is_file())
            .unwrap_or(false);
        if !is_cargo {
            continue;
        }
        let days = age_days(st.modified);
        let stale = days > 30;
        let mut detail = format!("{} · {} · {}", cleaner::fmt_bytes(st.bytes), t("Ultima modifica"), fmt_date(st.modified));
        if stale {
            detail.push_str(&format!(" · {} {} {}", t("inattiva da"), days, t("giorni")));
        }
        items.push(Item {
            label: format!("target/ — {}", display_path(&dir, home)),
            path: dir.to_string_lossy().to_string(),
            detail,
            size: st.bytes,
            // Deselezionata: l'utente sceglie consapevolmente di ricostruirla.
            selected: false,
            stale,
            remove_cmd: None,
        });
    }
    items
}

fn cache_item(home: &Path, rel: &str, label: &'static str) -> Option<Item> {
    let dir = home.join(rel);
    if !fs::symlink_metadata(&dir).map(|m| m.is_dir()).unwrap_or(false) {
        return None;
    }
    let st = dir_stats(&dir);
    Some(Item {
        label: t(label).to_string(),
        path: dir.to_string_lossy().to_string(),
        detail: format!("{}: {}", t("Ultima modifica"), fmt_date(st.modified)),
        size: st.bytes,
        selected: true,
        stale: false,
        remove_cmd: None,
    })
}

fn scan_node(home: &Path, progress: &mut impl FnMut(f32, &str)) -> Vec<Item> {
    let mut items = Vec::new();

    progress(0.25, t("Ricerca di cartelle node_modules nella home..."));
    let found = find_named(home, "node_modules", SKIP_NODE, progress);
    for (dir, st) in found {
        let days = age_days(st.modified);
        let stale = days > 30;
        let mut detail = format!("{} · {} · {}", cleaner::fmt_bytes(st.bytes), t("Ultima modifica"), fmt_date(st.modified));
        if stale {
            detail.push_str(&format!(" · {} {} {}", t("inattiva da"), days, t("giorni")));
        }
        items.push(Item {
            label: format!("node_modules — {}", display_path(&dir, home)),
            path: dir.to_string_lossy().to_string(),
            detail,
            size: st.bytes,
            // Deselezionata: l'utente sceglie consapevolmente di ricostruirla.
            selected: false,
            stale,
            remove_cmd: None,
        });
    }

    for (rel, label) in [
        (".npm", t("Cache npm")),
        (".cache/yarn", t("Cache Yarn")),
        (".cache/pnpm", t("Cache pnpm")),
        (".local/share/pnpm/store", t("Store pnpm")),
    ] {
        if let Some(item) = cache_item(home, rel, label) {
            items.push(item);
        }
    }
    items
}

fn scan_python(home: &Path, progress: &mut impl FnMut(f32, &str)) -> Vec<Item> {
    progress(0.4, t("Analisi delle cache Python..."));
    let mut items = Vec::new();
    for (rel, label) in [(".cache/pip", t("Cache pip")), (".cache/uv", t("Cache uv"))] {
        if let Some(item) = cache_item(home, rel, label) {
            items.push(item);
        }
    }
    items
}

/// Scansione completa: aggiorna la frazione di avanzamento mentre procede.
pub fn scan_all<F: FnMut(f32, &str)>(mut progress: F) -> Report {
    progress(0.01, t("Rilevamento degli strumenti installati..."));

    let container_tool = ["docker", "podman"].into_iter().find(|c| has_cmd(c));
    let container_version = container_tool
        .map(|rt| {
            let v = version_of(&[rt]);
            if v.is_empty() { rt.to_string() } else { format!("{rt} · {v}") }
        })
        .unwrap_or_default();
    let cargo_ver = version_of(&["cargo"]);
    let node_ver = version_of(&["node", "npm", "pnpm", "yarn"]);
    let py_ver = version_of(&["python3", "python", "pip3", "uv"]);
    let home = PathBuf::from(util::home());

    let container_items = match container_tool {
        Some(rt) => {
            progress(0.04, &format!("{rt}: system df"));
            scan_container(rt, &mut progress)
        }
        None => Vec::new(),
    };

    progress(0.33, t("Scansione degli artefatti Rust..."));
    let rust_items = if cargo_ver.is_empty() {
        Vec::new()
    } else {
        scan_rust(&home, &mut progress)
    };

    progress(0.62, t("Scansione degli artefatti Node.js..."));
    let node_items = if node_ver.is_empty() {
        Vec::new()
    } else {
        scan_node(&home, &mut progress)
    };

    progress(0.82, t("Scansione degli artefatti Python..."));
    let py_items = if py_ver.is_empty() {
        Vec::new()
    } else {
        scan_python(&home, &mut progress)
    };

    progress(0.95, t("Scansione completata"));
    Report {
        categories: vec![
            Category {
                id: CategoryId::Container,
                title: t("Docker / Podman"),
                subtitle: t("Immagini, container fermi, cache di build e volumi orfani"),
                installed: container_tool.is_some(),
                version: container_version,
                items: container_items,
            },
            Category {
                id: CategoryId::Rust,
                title: t("Rust / Cargo"),
                subtitle: t("Cartelle target/ inattive e cache del registry"),
                installed: !cargo_ver.is_empty(),
                version: cargo_ver,
                items: rust_items,
            },
            Category {
                id: CategoryId::Node,
                title: t("Node.js"),
                subtitle: t("Cartelle node_modules inattive e cache npm / yarn / pnpm"),
                installed: !node_ver.is_empty(),
                version: node_ver,
                items: node_items,
            },
            Category {
                id: CategoryId::Python,
                title: t("Python"),
                subtitle: t("Cache pip e uv"),
                installed: !py_ver.is_empty(),
                version: py_ver,
                items: py_items,
            },
        ],
    }
}

/// Rimuove solo gli elementi spuntati. Restituisce un riepilogo in cui ogni
/// errore viene segnalato senza interrompere le altre operazioni.
pub fn clean_selected<F: FnMut(usize, usize, &str)>(
    report: &Report,
    mut progress: F,
) -> CleanSummary {
    let selected: Vec<(&Category, &Item)> = report
        .categories
        .iter()
        .flat_map(|c| c.items.iter().filter(|i| i.selected).map(move |i| (c, i)))
        .collect();

    let total = selected.len();
    let mut summary = CleanSummary { freed: 0, errors: 0, done: 0, messages: Vec::new() };

    for (idx, (cat, item)) in selected.iter().enumerate() {
        let label = format!("{}: {}", t(cat.title), item.label);
        progress(idx + 1, total, &label);

        let errors = if let Some(cmd) = &item.remove_cmd {
            let (ok, out) = util::run_shell(cmd, false);
            if ok {
                summary.messages.push(tf("✔ {}", &[&label]));
                0
            } else {
                let tail = util::tail(out.trim(), 2).replace('\n', " | ");
                summary.messages.push(tf("✘ {}: {}", &[&label, &tail]));
                1
            }
        } else {
            let errors = remove_contents(Path::new(&item.path));
            if errors == 0 {
                summary.messages.push(tf("✔ {}", &[&label]));
            } else {
                summary.messages.push(tf("⚠ {}: {} {}", &[&label, &errors, &t("errori")]));
            }
            errors
        };

        if errors == 0 {
            summary.freed = summary.freed.saturating_add(item.size);
            summary.done += 1;
        } else {
            summary.errors += errors;
        }
    }

    summary
}

// ---- UI ---------------------------------------------------------------------------------------

fn category_glyph(id: CategoryId) -> &'static str {
    match id {
        CategoryId::Container => "▣",
        CategoryId::Rust => "⚙",
        CategoryId::Node => "◆",
        CategoryId::Python => "⌘",
    }
}

impl App {
    /// Avvia la scansione in background usando la barra condivisa dell'app.
    pub(crate) fn dev_scan(&mut self, ctx: &egui::Context) {
        self.spawn(ctx, |ch| {
            let mut prog = ProgressReporter::start(
                &ch,
                "devcontainers",
                t("Scansione Sviluppo & Container..."),
            );
            let report = scan_all(|frac, detail| {
                prog.set(frac, detail);
                ch.send(Msg::DevProgress { fraction: frac, detail: detail.to_string() });
            });
            ch.send(Msg::DevScan(report));
            prog.finish(t("Scansione completata"));
        });
    }

    /// Avvia la pulizia in background e rilegge i dati al termine.
    pub(crate) fn dev_clean(&mut self, ctx: &egui::Context) {
        let report = self.dev_report.clone();
        self.spawn(ctx, move |ch| {
            let mut prog = ProgressReporter::start(
                &ch,
                "devcontainers",
                t("Pulizia Sviluppo & Container..."),
            );
            let summary = clean_selected(&report, |cur, total, label| {
                let frac = if total == 0 { 1.0 } else { cur as f32 / total as f32 };
                prog.step(cur, total, label);
                ch.send(Msg::DevProgress { fraction: frac, detail: label.to_string() });
            });
            for msg in &summary.messages {
                ch.send(Msg::Log(msg.clone()));
            }
            ch.send(Msg::Log(tf(
                "✔ {}: {} — {} {}",
                &[
                    &t("Pulizia sviluppo completata"),
                    &cleaner::fmt_bytes(summary.freed),
                    &summary.done,
                    &t("elementi"),
                ],
            )));
            let fresh = scan_all(|frac, detail| {
                let overall = 0.85 + 0.14 * frac;
                prog.set(overall, detail);
                ch.send(Msg::DevProgress { fraction: overall, detail: detail.to_string() });
            });
            ch.send(Msg::DevScan(fresh));
            prog.finish(t("Pulizia completata"));
        });
    }

    pub(crate) fn ui_dev_containers(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Sviluppo & Container"));
        ui::hint(
            ui,
            t("Analizza gli strumenti installati (Docker/Podman, Rust, Node.js e Python) e gli spazi recuperabili: immagini e container inutilizzati, cartelle target/ e node_modules, cache dei gestori di pacchetti."),
        );

        ui.horizontal(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Scansiona di nuovo"))).clicked() {
                self.dev_scan(ctx);
            }
            if ui::primary_button(ui, t("Pulisci selezionati"), self.action_enabled() && self.dev_report.has_selection()).clicked() {
                self.dev_clean(ctx);
            }
            if self.busy {
                ui.spinner();
            }
        });

        if let Some((frac, detail)) = &self.dev_progress {
            ui.add_space(3.0);
            ui::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(t("Avanzamento")).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(detail.as_str()).size(12.5).color(ui::text_dim()));
                    });
                });
                ui.add_space(2.0);
                ui.add(
                    ui::progress_bar(*frac)
                        .desired_width(ui.available_width())
                        .animate(true)
                        .show_percentage(),
                );
            });
        }

        ui.add_space(5.0);

        for (ci, cat) in self.dev_report.categories.iter_mut().enumerate() {
            ui::card(ui, |ui| {
                ui.horizontal(|ui| {
                    let (badge, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
                    ui::icon_badge(ui.painter(), badge, ui::ACCENT, category_glyph(cat.id), 16.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(t(cat.title)).strong().size(17.0));
                        ui.label(egui::RichText::new(t(cat.subtitle)).size(12.5).color(ui::text_dim()));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if cat.installed {
                            ui::pill(ui, t("Installato"), ui::GOOD);
                            if !cat.version.is_empty() {
                                ui.label(egui::RichText::new(&cat.version).size(11.5).color(ui::text_dim()));
                            }
                        } else {
                            ui::pill(ui, t("Non installato"), ui::WARN);
                        }
                    });
                });

                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(t("Recuperabile:")).color(ui::text_dim()));
                    ui.label(
                        egui::RichText::new(cleaner::fmt_bytes(cat.recoverable()))
                            .strong()
                            .size(16.0)
                            .color(ui::accent_text()),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "({}: {})",
                            t("selezionato"),
                            cleaner::fmt_bytes(cat.selected_size())
                        ))
                        .size(13.0)
                        .color(ui::text_dim()),
                    );
                });

                ui.add_space(3.0);
                if cat.items.is_empty() {
                    ui.label(
                        egui::RichText::new(if cat.installed {
                            t("Nessun elemento da pulire rilevato.")
                        } else {
                            t("Strumento non installato: nessuna analisi disponibile.")
                        })
                        .color(ui::text_dim()),
                    );
                } else {
                    egui::ScrollArea::vertical().max_height(175.0).show(ui, |ui| {
                        for (idx, item) in cat.items.iter_mut().enumerate() {
                            ui.push_id(format!("dev-{ci}-{idx}"), |ui| {
                                ui.horizontal(|ui| {
                                    ui.checkbox(
                                        &mut item.selected,
                                        egui::RichText::new(item.label.as_str()).strong(),
                                    );
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(
                                            egui::RichText::new(cleaner::fmt_bytes(item.size))
                                                .strong()
                                                .color(ui::accent_text()),
                                        );
                                        if item.stale {
                                            ui::pill(ui, t("Inattivo >30 gg"), ui::WARN);
                                        }
                                    });
                                });
                                ui.label(
                                    egui::RichText::new(item.detail.as_str())
                                        .size(12.5)
                                        .color(ui::text_dim()),
                                );
                                if !item.path.is_empty() {
                                    ui.label(
                                        egui::RichText::new(item.path.as_str())
                                            .monospace()
                                            .size(11.5)
                                            .color(ui::text_dim()),
                                    );
                                }
                                ui.add_space(2.5);
                            });
                        }
                    });
                }
            });
            ui.add_space(3.0);
        }

        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(t("Le cartelle target/ e node_modules sono deselezionate di default: spuntale solo se vuoi ricostruirle."))
                    .size(13.0)
                    .color(ui::text_dim()),
            );
        });
        self.show_out(ui, "devcontainers", 140.0);
    }
}
