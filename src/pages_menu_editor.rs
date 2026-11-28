use crate::i18n::{t, tf, tr};
use crate::App;
use eframe::egui;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TUTTI: &str = "Tutti";
const NASCOSTI: &str = "Nascosti";

/// Albero in stile "Menù principale" MATE: gruppi e sottocategorie XDG.
const MENU_TREE: &[(&str, &[&str])] = &[
    (
        "Applicazioni",
        &[
            "Accessori",
            "Accesso universale",
            "Audio e video",
            "Giochi",
            "Grafica",
            "Internet",
            "Istruzione",
            "Programmazione",
            "Strumenti di sistema",
            "Ufficio",
        ],
    ),
    ("Sistema", &["Preferenze", "Amministrazione"]),
];

#[derive(Clone)]
pub struct MenuItemCustom {
    pub name: String,
    pub exec: String,
    pub icon: String,
    /// Categoria dell'albero (italiano, usata come chiave di traduzione).
    pub category: String,
    /// Nome del file .desktop locale (desktop-id).
    pub desktop_id: String,
    /// File da cui è stata letta la voce (locale o di sistema).
    pub source: PathBuf,
    pub no_display: bool,
    pub hidden: bool,
}

impl MenuItemCustom {
    fn is_visible(&self) -> bool {
        !self.no_display && !self.hidden
    }
}

/// Etichetta a larghezza fissa: il testo troppo lungo viene troncato con `...`
/// e il testo completo resta disponibile nel tooltip. `clickable` mantiene
/// l'azione di apertura della voce senza mai allargare la colonna.
fn ellipsis_label(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    rich: egui::RichText,
    clickable: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.set_width(width.max(1.0));
        let mut label = egui::Label::new(rich).truncate();
        if clickable {
            label = label.sense(egui::Sense::click());
        }
        let response = ui.add(label).on_hover_text(text);
        if clickable {
            response.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            response
        }
    })
    .inner
}

fn home_dir() -> Option<PathBuf> {
    let home = crate::util::home();
    if home.is_empty() {
        None
    } else {
        Some(PathBuf::from(home))
    }
}

fn local_apps_dir() -> Option<PathBuf> {
    // La directory di scrittura deve essere quella che i desktop leggono:
    // se XDG_DATA_HOME è impostato, ~/.local/share/applications verrebbe ignorato.
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if xdg.starts_with('/') {
            return Some(PathBuf::from(xdg).join("applications"));
        }
    }
    home_dir().map(|h| h.join(".local/share/applications"))
}

/// Cartelle .desktop considerate: utente, XDG_DATA_HOME, locali e di sistema.
fn desktop_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = local_apps_dir() {
        dirs.push(d);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if xdg.starts_with('/') {
            dirs.push(PathBuf::from(xdg).join("applications"));
        }
    }
    dirs.push(PathBuf::from("/usr/local/share/applications"));
    dirs.push(PathBuf::from("/usr/share/applications"));
    if let Ok(xdg) = std::env::var("XDG_DATA_DIRS") {
        for base in xdg.split(':').filter(|p| p.starts_with('/')) {
            dirs.push(PathBuf::from(base).join("applications"));
        }
    }

    let mut unique: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if !unique.contains(&d) {
            unique.push(d);
        }
    }
    unique
}

/// Legge un campo del gruppo [Desktop Entry], preferendo Name/Commento non
/// localizzati, poi la lingua corrente, poi la prima localizzazione trovata.
fn desktop_value(content: &str, key: &str) -> String {
    let exact_prefix = format!("{key}=");
    let localized_prefix = format!("{key}[");
    // Catena ordinata del locale attivo (es. pt_BR -> pt): consente di usare
    // anche le localizzazioni .desktop della lingua madre.
    let codes = crate::i18n::locale_chain(crate::i18n::code_of(crate::i18n::lang()));
    let mut in_entry = false;
    let mut exact = String::new();
    let mut localized = String::new();
    let mut fallback = String::new();
    let mut best_rank: Option<usize> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(v) = line.strip_prefix(&exact_prefix) {
            exact = v.trim().to_string();
        } else if let Some(rest) = line.strip_prefix(&localized_prefix) {
            if let Some((locale, value)) = rest.split_once("]=") {
                let value = value.trim().to_string();
                let loc = crate::i18n::locale_chain(locale);
                let rank = codes.iter().position(|c| loc.iter().any(|x| x == c));
                if let Some(r) = rank {
                    // Il locale piu' specifico (primo nella catena) ha la priorita'.
                    if best_rank.map_or(true, |best| r < best) {
                        localized = value;
                        best_rank = Some(r);
                    }
                } else if fallback.is_empty() {
                    fallback = value;
                }
            }
        }
    }

    if !exact.is_empty() {
        exact
    } else if !localized.is_empty() {
        localized
    } else {
        fallback
    }
}

fn desktop_bool(content: &str, key: &str) -> bool {
    matches!(
        desktop_value(content, key).to_ascii_lowercase().as_str(),
        "true" | "1" | "yes"
    )
}

fn desktop_categories(content: &str) -> Vec<String> {
    desktop_value(content, "Categories")
        .split(';')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

/// Aggiorna (o aggiunge) una chiave solo dentro [Desktop Entry].
fn set_desktop_value(content: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let mut in_entry = false;
    let mut found: Vec<usize> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_entry = trimmed == "[Desktop Entry]";
            continue;
        }
        // Riscrive tutte le occorrenze: leggere desktop_value prende l'ultima,
        // quindi lasciarne una sola vecchia annullerebbe la modifica.
        if in_entry && trimmed.starts_with(&prefix) {
            found.push(i);
        }
    }

    if !found.is_empty() {
        for i in found {
            lines[i] = format!("{key}={value}");
        }
    } else {
        let insert_at = if let Some(i) = lines
            .iter()
            .position(|l| l.trim() == "[Desktop Entry]")
        {
            i + 1
        } else {
            lines.insert(0, "[Desktop Entry]".to_string());
            1
        };
        lines.insert(insert_at, format!("{key}={value}"));
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn make_desktop_entry(name: &str, exec: &str, icon: &str, category: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nIcon={icon}\nTerminal=false\nCategories={category};\n"
    )
}

/// Applica NoDisplay/Hidden mantenuto il resto del file .desktop.
fn with_visibility(content: &str, show: bool) -> String {
    let mut out = set_desktop_value(content, "NoDisplay", if show { "false" } else { "true" });
    out = set_desktop_value(&out, "Hidden", "false");
    out
}

fn desktop_id_for(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let slug = slug.trim_matches('_');
    if slug.is_empty() {
        "voce-personalizzata.desktop".into()
    } else {
        format!("{slug}.desktop")
    }
}

fn has_category(categories: &[String], wanted: &str) -> bool {
    categories
        .iter()
        .any(|c| c.eq_ignore_ascii_case(wanted))
}

fn looks_administrative(name: &str, exec: &str) -> bool {
    let text = format!("{name} {exec}").to_ascii_lowercase();
    ["admin", "amministr", "pkexec", "gksu", "sudo", "polkit"]
        .iter()
        .any(|k| text.contains(k))
}

/// Mappa le categorie XDG alle voci dell'albero MATE.
fn menu_category_for(categories: &[String], name: &str, exec: &str) -> &'static str {
    if has_category(categories, "Settings")
        || has_category(categories, "X-GNOME-Settings-Panel")
        || has_category(categories, "X-KDE-System-Settings")
    {
        "Preferenze"
    } else if has_category(categories, "X-GNOME-SystemSettings")
        || (has_category(categories, "System") && looks_administrative(name, exec))
    {
        "Amministrazione"
    } else if has_category(categories, "Accessibility") {
        "Accesso universale"
    } else if has_category(categories, "AudioVideo")
        || has_category(categories, "Audio")
        || has_category(categories, "Video")
    {
        "Audio e video"
    } else if has_category(categories, "Game") {
        "Giochi"
    } else if has_category(categories, "Graphics") {
        "Grafica"
    } else if has_category(categories, "Network") {
        "Internet"
    } else if has_category(categories, "Education") {
        "Istruzione"
    } else if has_category(categories, "Development") {
        "Programmazione"
    } else if has_category(categories, "Office") {
        "Ufficio"
    } else if has_category(categories, "System") {
        "Strumenti di sistema"
    } else {
        "Accessori"
    }
}

/// `selected` è una chiave stabile (non tradotta): cambiar lingua non deve
/// svuotare l'elenco né perdere la selezione corrente.
fn menu_matches(item: &MenuItemCustom, selected: &str) -> bool {
    if selected == TUTTI {
        return true;
    }
    if selected == NASCOSTI {
        return !item.is_visible();
    }
    for (group, children) in MENU_TREE {
        if selected == *group {
            return children.iter().any(|c| item.category == *c);
        }
        if children.iter().any(|c| selected == *c) {
            return children.iter().any(|c| item.category == *c && selected == *c);
        }
    }
    false
}

fn env_contains(value: &str) -> bool {
    ["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION", "XDG_SESSION_DESKTOP"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .any(|v| v.to_ascii_lowercase().contains(value))
}

fn escape_quotes(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Sostituisce un blocco marcato in un file di menu, o lo aggiunge prima del
/// tag di chiusura indicato (creando il file se il WM è attivo).
fn write_block_at(
    path: &Path,
    begin: &str,
    end: &str,
    block: &str,
    before_tags: &[&str],
    base_if_missing: &str,
) -> std::io::Result<()> {
    let source = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => base_if_missing.to_string(),
        // File presente ma non leggibile: riscriverlo significherebbe
        // cancellare la configurazione del window manager.
        Err(e) => return Err(e),
    };

    let mut out = String::new();
    if let Some(start) = source.find(begin) {
        if let Some(rel_end) = source[start..].find(end) {
            let finish = start + rel_end;
            out.push_str(&source[..start]);
            out.push_str(begin);
            out.push('\n');
            out.push_str(block);
            out.push_str(end);
            out.push('\n');
            out.push_str(source[finish + end.len()..].trim_start_matches('\n'));
            return write_with_parent(path, out);
        }
    }

    let marked = format!("{begin}\n{block}{end}\n");
    if let Some(tag) = before_tags.iter().find_map(|tag| source.rfind(*tag)) {
        out.push_str(&source[..tag]);
        out.push_str(&marked);
        out.push_str(&source[tag..]);
    } else {
        out.push_str(source.trim_end());
        out.push('\n');
        out.push_str(&marked);
    }
    write_with_parent(path, out)
}

fn write_with_parent(path: &Path, content: String) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

/// Aggiorna la cache XDG e, se disponibile, quella KDE.
/// Un comando assente o fallito non deve impedire di provare il successivo.
fn refresh_external_caches() {
    if let Some(dir) = local_apps_dir() {
        let _ = Command::new("update-desktop-database").arg(dir).output();
    }
    // Su sistemi con Plasma 5 e 6 installati vanno aggiornate entrambe le
    // cache: fermarsi al primo successo lasciava l'altra versione stale.
    for tool in ["kbuildsycoca6", "kbuildsycoca5"] {
        let _ = Command::new(tool).arg("--noincremental").output();
    }
}

/// Rigenera i menu testuali utente per i window manager presenti o attivi.
fn update_text_wm_menus(items: &[MenuItemCustom]) -> Vec<String> {
    let home = match home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    let mut errors = Vec::new();
    let visible: Vec<&MenuItemCustom> = items.iter().filter(|i| i.is_visible()).collect();

    // IceWM: ~/.icewm/menu
    let icewm = home.join(".icewm/menu");
    // Solo se esiste già un menu utente: crearne uno vuoto sopprimerebbe il
    // menu predefinito del WM, lasciando visibili solo le voci DebUtility.
    if icewm.exists() {
        let mut block = String::from("menu \"DebUtility\" {\n");
        for item in &visible {
            block.push_str(&format!(
                "  prog \"{}\" \"{}\" \"{}\"\n",
                escape_quotes(&item.name),
                escape_quotes(&item.icon),
                escape_quotes(&item.exec)
            ));
        }
        block.push_str("}\n");
        if let Err(e) = write_block_at(
            &icewm,
            "# DEBUTILITY-BEGIN",
            "# DEBUTILITY-END",
            &block,
            &[],
            "",
        ) {
            errors.push(format!("IceWM {}: {e}", icewm.display()));
        }
    }

    // Fluxbox: ~/.fluxbox/menu
    let fluxbox = home.join(".fluxbox/menu");
    // Come per IceWM: non creare un menu fluxbox che contenga solo DebUtility.
    if fluxbox.exists() {
        let mut block = String::from("[submenu] (DebUtility)\n");
        for item in &visible {
            block.push_str(&format!(
                "  [exec] ({}) {{{}}}\n",
                escape_quotes(&item.name),
                item.exec.replace('}', "\\}")
            ));
        }
        block.push_str("[end]\n");
        if let Err(e) = write_block_at(
            &fluxbox,
            "# DEBUTILITY-BEGIN",
            "# DEBUTILITY-END",
            &block,
            &[],
            "",
        ) {
            errors.push(format!("Fluxbox {}: {e}", fluxbox.display()));
        }
    }

    // JWM: ~/.jwmrc
    let jwm = home.join(".jwmrc");
    if env_contains("jwm") || jwm.exists() {
        let mut block = String::from("<Menu label=\"DebUtility\">\n");
        for item in &visible {
            block.push_str(&format!(
                "  <Program label=\"{}\" icon=\"{}\">{}</Program>\n",
                escape_xml(&item.name),
                escape_xml(&item.icon),
                escape_xml(&item.exec)
            ));
        }
        block.push_str("</Menu>\n");
        if let Err(e) = write_block_at(
            &jwm,
            "<!-- DEBUTILITY-BEGIN -->",
            "<!-- DEBUTILITY-END -->",
            &block,
            &["</RootMenu>", "</JWM>"],
            "<?xml version=\"1.0\"?>\n<JWM>\n</JWM>\n",
        ) {
            errors.push(format!("JWM {}: {e}", jwm.display()));
        }
    }

    // Openbox: ~/.config/openbox/menu.xml
    let openbox = home.join(".config/openbox/menu.xml");
    if env_contains("openbox") || openbox.exists() {
        let mut block = String::from("<menu id=\"debutility\" label=\"DebUtility\">\n");
        for item in &visible {
            block.push_str(&format!(
                "  <item label=\"{}\">\n    <action name=\"Execute\"><command>{}</command></action>\n  </item>\n",
                escape_xml(&item.name),
                escape_xml(&item.exec)
            ));
        }
        block.push_str("</menu>\n");
        if let Err(e) = write_block_at(
            &openbox,
            "<!-- DEBUTILITY-BEGIN -->",
            "<!-- DEBUTILITY-END -->",
            &block,
            // Solo il tag radice: "</menu>" può appartenere a un sottomenu.
            &["</openbox_menu>"],
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<openbox_menu>\n<menu id=\"root-menu\" label=\"DebUtility\">\n</menu>\n</openbox_menu>\n",
        ) {
            errors.push(format!("Openbox {}: {e}", openbox.display()));
        }
    }
    errors
}

/// Scrive il file .desktop locale, senza toccare i file di sistema protetti.
fn write_visibility(item: &MenuItemCustom, show: bool) -> Result<(), String> {
    let apps_dir = local_apps_dir().ok_or_else(|| t("✘ Nessuna cartella utente disponibile per i lanciatori.").to_string())?;
    fs::create_dir_all(&apps_dir).map_err(|e| tf("✘ Impossibile aggiornare la voce: {}", &[&e]))?;

    let local = apps_dir.join(&item.desktop_id);
    let base = if local.exists() {
        fs::read_to_string(&local).ok()
    } else if item.source.extension().and_then(|s| s.to_str()) == Some("desktop") {
        fs::read_to_string(&item.source).ok()
    } else {
        None
    };
    let content = base.unwrap_or_else(|| {
        make_desktop_entry(&item.name, &item.exec, &item.icon, &item.category)
    });
    let content = with_visibility(&content, show);
    fs::write(&local, content).map_err(|e| tf("✘ Impossibile aggiornare la voce: {}", &[&e]))
}

impl App {
    pub(crate) fn ui_menu_editor(&mut self, ui: &mut egui::Ui) {
        let de = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_uppercase();
        let wm = std::env::var("DESKTOP_SESSION").unwrap_or_default().to_uppercase();
        let sess = std::env::var("XDG_SESSION_DESKTOP").unwrap_or_default().to_uppercase();
        let current_env = if !de.is_empty() { de } else if !wm.is_empty() { wm } else { sess };

        crate::ui::title(ui, t("Editor Menu Applicazioni e Window Manager"));
        crate::ui::card(ui, |ui| {
            ui.label(
                egui::RichText::new(t("Gestisce i lanciatori XDG e rigenera i menu testuali per IceWM, JWM, Openbox e Fluxbox: nascondi o mostra una voce vale su MATE, KDE, Cinnamon, LXQt, LXDE, XFCE e qualsiasi window manager."))
                    .size(15.0),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!(
                    "{}: {}",
                    t("Ambiente rilevato"),
                    if current_env.is_empty() { t("Generico") } else { current_env.as_str() }
                ))
                .strong()
                .color(crate::ui::accent_text())
                .size(14.0),
            );
        });
        ui.add_space(12.0);
        // Stato del backup: il riquadro viene mostrato alla fine, dopo le
        // operazioni principali (ordine GT-STACER: contenuto prima, servizi poi).
        let backup_status = self.out.get("menu_backup").cloned().unwrap_or_default();

        // --- Due colonne in stile Menù principale MATE ---
        let entries = self.custom_menu_items.clone();
        let selected = self.menu_category.clone();
        let filter = self.menu_filter.trim().to_ascii_lowercase();
        let filtered: Vec<String> = entries
            .iter()
            .enumerate()
            .filter(|(_, item)| menu_matches(item, &selected))
            .filter(|(_, item)| {
                filter.is_empty()
                    || item.name.to_ascii_lowercase().contains(&filter)
                    || item.exec.to_ascii_lowercase().contains(&filter)
                    || tr(&item.category).to_ascii_lowercase().contains(&filter)
            })
            // ID stabile: dopo una modifica la lista viene ricaricata e gli
            // indici dello snapshot non sarebbero più affidabili.
            .map(|(i, _)| entries[i].desktop_id.clone())
            .collect();
        let hidden_count = entries.iter().filter(|i| !i.is_visible()).count();
        let status = self.out.get("menu_editor").cloned().unwrap_or_default();

        crate::ui::card(ui, |ui| {
            // Header di colonna unico: "Menù" e "Elementi" sono le sole
            // intestazioni, entrambe in alto. Niente doppio titolo che ruba spazio.
            // La colonna sinistra ha larghezza-controllo (170-190 px), mentre
            // "Elementi" occupa tutta la larghezza residua della card: in questo
            // modo nomi, comandi e pulsanti non vengono più schiacciati a metà.
            let column_gap = ui.spacing().item_spacing.x;
            let full_width = ui.available_width();
            let left_width = if full_width < 520.0 {
                110.0
            } else {
                190.0f32.min((full_width - 280.0).max(120.0))
            };
            let right_width = (full_width - left_width - column_gap).max(1.0);

            // Altezza dell'elenco di destra: almeno 380 px, fino a 450 px quando
            // la finestra lo consente. La tabella mostra così molte voci per
            // volta, come il Menù principale di MATE, senza lasciare lo spazio
            // vuoto che si aveva sotto la prima riga.
            let list_height = ui.available_height().clamp(220.0, 420.0);

            // Allineamento in alto: "Elementi" e "Cerca voce..." partono alla
            // stessa quota di "Menù" / "Tutti" nella colonna sinistra.
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |left| {
                        left.set_width(left_width);
                        left.horizontal(|ui| {
                            ui.label(egui::RichText::new(t("Menù")).strong().size(16.0));
                            ui.label(
                                egui::RichText::new(format!("{}: {}", t("Nascosti"), hidden_count))
                                    .color(crate::ui::text_dim())
                                    .size(13.5),
                            );
                        });
                        left.add_space(4.0);

                        if left
                            .selectable_label(selected.as_str() == TUTTI, t(TUTTI))
                            .clicked()
                        {
                            self.menu_category = TUTTI.to_string();
                        }
                        if left
                            .selectable_label(
                                selected.as_str() == NASCOSTI,
                                format!("{} ({hidden_count})", t(NASCOSTI)),
                            )
                            .clicked()
                        {
                            self.menu_category = NASCOSTI.to_string();
                        }

                        for (group, children) in MENU_TREE {
                            left.add_space(6.0);
                            left.label(
                                egui::RichText::new(t(*group))
                                    .strong()
                                    .color(crate::ui::accent_text()),
                            );
                            for child in *children {
                                let label = t(*child);
                                if left
                                    .selectable_label(selected.as_str() == *child, format!("   {label}"))
                                    .clicked()
                                {
                                    self.menu_category = (*child).to_string();
                                }
                            }
                        }
                    },
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(right_width, 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |right| {
                        right.set_width(right_width);
                        right.horizontal(|ui| {
                            ui.label(egui::RichText::new(t("Elementi")).strong().size(16.0));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} / {}",
                                        filtered.len(),
                                        entries.len()
                                    ))
                                    .color(crate::ui::text_dim())
                                    .size(13.5),
                                );
                            });
                        });
                        let width = right.available_width();
                        right.add(
                            egui::TextEdit::singleline(&mut self.menu_filter)
                                .hint_text(t("Cerca voce..."))
                                .desired_width(width),
                        );
                        right.add_space(4.0);
                        right.separator();

                        if filtered.is_empty() {
                            right.label(t("Nessuna voce corrisponde al filtro."));
                        } else {
                            // Solo scroll verticale: nessuna barra orizzontale può
                            // coprire la prima riga o i pulsanti Mostra/Nascondi.
                            // La griglia resta sempre dentro il viewport disponibile.
                            egui::ScrollArea::vertical()
                                .id_source("menu_editor_items_scroll")
                                .auto_shrink([false, false])
                                .min_scrolled_height(list_height)
                                .max_height(list_height)
                                .show(right, |ui| {
                                    let available = ui.available_width().max(1.0);
                                    // Con poca larghezza si passa da 5 a 3 colonne:
                                    // icona/categoria e comando finiscono sotto il nome
                                    // (sempre con tooltip), così non diventano "…" da 20 px.
                                    let compact_cols = available < 560.0;
                                    let cols = if compact_cols { 3 } else { 5 };
                                    let gaps = ((cols - 1) as f32 * ui.spacing().item_spacing.x).min(available * 0.24);
                                    let cell_gap = gaps / (cols - 1) as f32;

                                    let show_w = 140.0f32.min(available * (if compact_cols { 0.32 } else { 0.40 }));
                                    let actions_w = 104.0f32.min(available * 0.24);
                                    let text_space = (available - gaps - show_w - actions_w).max(0.0);
                                    let name_w = if compact_cols { text_space } else { text_space * 0.34 };
                                    let icon_w = if compact_cols { 0.0 } else { text_space * 0.20 };
                                    let command_w = if compact_cols { 0.0 } else { (text_space - name_w - icon_w).max(0.0) };
                                    let table_width = show_w + name_w + icon_w + command_w + actions_w + gaps;
                                    ui.set_width(table_width);
                                    egui::Grid::new("menu_editor_items_grid")
                                        .num_columns(cols)
                                        .striped(true)
                                        .min_col_width(0.0)
                                        .spacing(egui::vec2(cell_gap, 4.0))
                                        .show(ui, |ui| {
                                            let _ = ellipsis_label(
                                                ui,
                                                t("Mostra"),
                                                show_w,
                                                egui::RichText::new(t("Mostra")).strong().size(15.0),
                                                false,
                                            );
                                            let _ = ellipsis_label(
                                                ui,
                                                t("Nome"),
                                                name_w,
                                                egui::RichText::new(t("Nome")).strong().size(15.0),
                                                false,
                                            );
                                            if !compact_cols {
                                                let _ = ellipsis_label(
                                                    ui,
                                                    t("Icona / Categoria"),
                                                    icon_w,
                                                    egui::RichText::new(t("Icona / Categoria")).strong().size(15.0),
                                                    false,
                                                );
                                                let _ = ellipsis_label(
                                                    ui,
                                                    t("Comando"),
                                                    command_w,
                                                    egui::RichText::new(t("Comando")).strong().size(15.0),
                                                    false,
                                                );
                                            }
                                            let _ = ellipsis_label(
                                                ui,
                                                t("Azioni"),
                                                actions_w,
                                                egui::RichText::new(t("Azioni")).strong().size(15.0),
                                                false,
                                            );
                                            ui.end_row();

                                            for id in &filtered {
                                                let Some(item) = entries.iter().find(|i| i.desktop_id == *id).cloned() else {
                                                    continue;
                                                };
                                                let mut show = item.is_visible();

                                                ui.horizontal(|ui| {
                                                    ui.set_width(show_w);
                                                    if ui.checkbox(&mut show, "").clicked() {
                                                        self.set_entry_visibility(id, show);
                                                    }
                                                    let button = if show { t("Nascondi") } else { t("Mostra") };
                                                    if ui.small_button(button).clicked() {
                                                        self.set_entry_visibility(id, !show);
                                                    }
                                                });

                                                ui.vertical(|ui| {
                                                    ui.set_width(name_w);
                                                    let clicked = ellipsis_label(
                                                        ui,
                                                        item.name.as_str(),
                                                        name_w,
                                                        egui::RichText::new(item.name.as_str())
                                                            .strong()
                                                            .color(crate::ui::accent_text()),
                                                        true,
                                                    )
                                                    .clicked();
                                                    if clicked {
                                                        self.load_entry_into_form(&item);
                                                    }
                                                    let _ = ellipsis_label(
                                                        ui,
                                                        item.desktop_id.as_str(),
                                                        name_w,
                                                        egui::RichText::new(item.desktop_id.as_str())
                                                            .color(crate::ui::text_dim())
                                                            .size(12.5),
                                                        false,
                                                    );
                                                    if compact_cols {
                                                        let category = tr(&item.category);
                                                        let detail = format!("{} · {}", item.icon, category);
                                                        let _ = ellipsis_label(
                                                            ui,
                                                            detail.as_str(),
                                                            name_w,
                                                            egui::RichText::new(detail.as_str())
                                                                .color(crate::ui::accent_text())
                                                                .size(12.0),
                                                            false,
                                                        );
                                                        let _ = ellipsis_label(
                                                            ui,
                                                            item.exec.as_str(),
                                                            name_w,
                                                            egui::RichText::new(item.exec.as_str())
                                                                .monospace()
                                                                .size(12.0),
                                                            false,
                                                        );
                                                    }
                                                });

                                                if !compact_cols {
                                                    ui.vertical(|ui| {
                                                        ui.set_width(icon_w);
                                                        let _ = ellipsis_label(
                                                            ui,
                                                            item.icon.as_str(),
                                                            icon_w,
                                                            egui::RichText::new(item.icon.as_str())
                                                                .color(crate::ui::accent_text())
                                                                .size(13.0),
                                                            false,
                                                        );
                                                        let category = tr(&item.category);
                                                        let _ = ellipsis_label(
                                                            ui,
                                                            category.as_str(),
                                                            icon_w,
                                                            egui::RichText::new(category.as_str())
                                                                .color(crate::ui::text_dim())
                                                                .size(13.0),
                                                            false,
                                                        );
                                                    });

                                                    let _ = ellipsis_label(
                                                        ui,
                                                        item.exec.as_str(),
                                                        command_w,
                                                        egui::RichText::new(item.exec.as_str())
                                                            .monospace()
                                                            .size(13.5),
                                                        false,
                                                    );
                                                }

                                                ui.horizontal(|ui| {
                                                    ui.set_width(actions_w);
                                                    if ui.small_button("✏").on_hover_text(t("✏ Modifica")).clicked() {
                                                        self.load_entry_into_form(&item);
                                                    }
                                                    if ui.small_button("🗑").on_hover_text(t("🗑 Rimuovi")).clicked() {
                                                        self.remove_custom_desktop_entry_by_id(&item.desktop_id);
                                                    }
                                                });
                                                ui.end_row();
                                            }
                                        });
                                });
                        }
                    },
                );
            });

            if !status.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(status.as_str()).color(crate::ui::accent_text()));
            }
        });

        ui.add_space(12.0);

        crate::ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(t("Aggiungi o modifica applicazione nel menu"));
                if let Some(id) = self.menu_editing_id.clone() {
                    ui.label(
                        egui::RichText::new(tf("Modifica: {}", &[&id]))
                            .color(crate::ui::accent_text())
                            .size(12.5),
                    );
                    if ui.button(t("Nuova voce")).clicked() {
                        self.clear_menu_form();
                    }
                }
            });
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(t("Nome:"));
                ui.text_edit_singleline(&mut self.new_menu_name);
            });
            ui.horizontal(|ui| {
                ui.label(t("Comando:"));
                ui.text_edit_singleline(&mut self.new_menu_exec);
            });
            ui.horizontal(|ui| {
                ui.label(t("Icona (opzionale):"));
                ui.text_edit_singleline(&mut self.new_menu_icon);
            });
            ui.horizontal(|ui| {
                ui.label(t("Categoria (es. Internet, Utility):"));
                ui.text_edit_singleline(&mut self.new_menu_category);
            });

            ui.add_space(8.0);
            if ui
                .button(egui::RichText::new(t("➕ Salva e aggiorna menu")).strong().size(14.0))
                .clicked()
            {
                self.add_custom_desktop_entry();
            }
        });

        ui.add_space(12.0);

        // --- Backup e Ripristino Rapido Menu & WM (servizi secondari) ---
        crate::ui::card(ui, |ui| {
            ui.heading(t("Backup e Ripristino Rapido (.desktop e WM)"));
            ui.separator();
            ui.label(t("Esporta tutte le personalizzazioni dei lanciatori (~/.local/share/applications) e delle configurazioni dei Window Manager in un archivio compresso .tar.gz nella tua Home, oppure ripristinale."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(t("📦 Esporta Backup Menu & WM")).clicked() {
                    self.export_menu_backup();
                }
                if ui.button(t("📂 Ripristina Backup")).clicked() {
                    self.restore_menu_backup();
                }
            });
            if !backup_status.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(backup_status.as_str()).color(crate::ui::accent_text()));
            }
        });
    }

    fn clear_menu_form(&mut self) {
        self.new_menu_name.clear();
        self.new_menu_exec.clear();
        self.new_menu_icon.clear();
        self.new_menu_category.clear();
        self.menu_editing_id = None;
    }

    fn load_entry_into_form(&mut self, item: &MenuItemCustom) {
        self.new_menu_name = item.name.clone();
        self.new_menu_exec = item.exec.clone();
        self.new_menu_icon = item.icon.clone();
        // Il campo categoria accetta i token XDG: li ricaviamo dalla sorgente.
        let categories = if item.source.extension().and_then(|s| s.to_str()) == Some("desktop") {
            fs::read_to_string(&item.source)
                .map(|c| desktop_value(&c, "Categories"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        self.new_menu_category = categories.trim_end_matches(';').to_string();
        self.menu_editing_id = Some(item.desktop_id.clone());
    }

    /// Percorsi relativi alla home inclusi nel backup menu/WM.
    fn backup_paths(home: &str) -> Vec<String> {
        let mut paths: Vec<String> = [
            ".local/share/applications",
            ".icewm",
            ".fluxbox",
            ".jwmrc",
            ".config/openbox",
            ".config/fluxbox",
            ".config/jwm",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // Con XDG_DATA_HOME le personalizzazioni non stanno in ~/.local/share.
        if let Some(apps) = local_apps_dir() {
            if let Ok(rel) = apps.strip_prefix(home) {
                let rel = rel.to_string_lossy().trim_matches('/').to_string();
                if !rel.is_empty() && !paths.contains(&rel) {
                    paths.push(rel);
                }
            }
        }
        paths
    }

    /// Elenca i membri di un archivio senza estrarlo.
    fn tar_members(archive: &Path) -> Result<Vec<String>, String> {
        let out = Command::new("tar")
            .arg("tzf")
            .arg(archive)
            .output()
            .map_err(|e| format!("tar non disponibile: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(tf("Archivio non leggibile: {}", &[&crate::util::tail(&err, 2)]));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect())
    }

    /// Un membro è ammesso solo se è uno dei percorsi salvati o un loro
    /// discendente. Niente percorsi assoluti, `..` o nomi con caratteri
    /// speciali da passare alla shell: il controllo avviene in Rust.
    fn allowed_tar_entry(entry: &str, allowed: &[String]) -> bool {
        let e = entry.strip_prefix("./").unwrap_or(entry);
        if e.is_empty() || e.starts_with('/') {
            return false;
        }
        if Path::new(e)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return false;
        }
        allowed.iter().any(|a| e == a || e.starts_with(&format!("{a}/")))
    }

    fn export_menu_backup(&mut self) {
        let home = crate::util::home();
        if home.is_empty() {
            return;
        }
        let backup_path = format!("{home}/debutility_menu_wm_backup.tar.gz");
        let rels = Self::backup_paths(&home);
        let files = rels
            .iter()
            .map(|p| crate::util::shq(p))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "cd {} && tar --ignore-failed-read -czf {} {files}",
            crate::util::shq(&home),
            crate::util::shq(&backup_path)
        );
        let (ok, out) = crate::util::run_shell(&script, false);
        if ok {
            self.out.insert("menu_backup".into(), tf("✔ Backup salvato con successo in: {}", &[&backup_path]));
        } else {
            self.out.insert("menu_backup".into(), tf("✘ Errore durante la creazione del backup: {}", &[&crate::util::tail(out.trim(), 2)]));
        }
    }

    fn restore_menu_backup(&mut self) {
        let home = crate::util::home();
        if home.is_empty() {
            return;
        }
        let backup_path = format!("{home}/debutility_menu_wm_backup.tar.gz");
        let legacy_path = format!("{home}/debclean_menu_wm_backup.tar.gz");
        let chosen = if Path::new(&backup_path).exists() {
            backup_path.clone()
        } else if Path::new(&legacy_path).exists() {
            legacy_path
        } else {
            self.out.insert("menu_backup".into(), tf("✘ File di backup non trovato in: {}", &[&backup_path]));
            return;
        };

        // Estrarre un archivio arbitrario in $HOME permetteva di sovrascrivere
        // dotfile qualunque o di uscire dalla home con `..`: validare i membri
        // in Rust, senza costrutti regex passati alla shell.
        let allowed = Self::backup_paths(&home);
        let members = match Self::tar_members(Path::new(&chosen)) {
            Ok(m) => m,
            Err(e) => {
                self.out.insert(
                    "menu_backup".into(),
                    tf("✘ Errore durante il ripristino del backup: {}", &[&e]),
                );
                return;
            }
        };
        if let Some(bad) = members
            .iter()
            .find(|e| !Self::allowed_tar_entry(e, &allowed))
        {
            self.out.insert(
                "menu_backup".into(),
                tf("✘ Errore durante il ripristino del backup: {}", &[&tf("Archivio non valido: {}", &[&bad])]),
            );
            return;
        }
        let script = format!(
            "set -e; cd {h} && tar -xzf {q}",
            q = crate::util::shq(&chosen),
            h = crate::util::shq(&home),
        );
        let (ok, out) = crate::util::run_shell(&script, false);
        if ok {
            self.out.insert("menu_backup".into(), t("✔ Backup ripristinato con successo!").to_string());
            self.refresh_after_change();
        } else {
            self.out.insert("menu_backup".into(), tf("✘ Errore durante il ripristino del backup: {}", &[&crate::util::tail(out.trim(), 2)]));
        }
    }

    /// Ricarica tutte le voci XDG/WM e aggiorna la cache XDG/KDE.
    fn refresh_after_change(&mut self) {
        self.load_icewm_menu();
        // Rigenera i menu testuali solo dopo una modifica esplicita, non a
        // ogni avvio: `load_icewm_menu` è usata anche in `App::new`.
        let errors = update_text_wm_menus(&self.custom_menu_items);
        refresh_external_caches();
        for e in errors {
            self.log.push_str(&format!("✘ Menu WM non aggiornato: {e}\n"));
        }
    }

    /// Carica tutte le voci .desktop (utente e sistema) e le voci IceWM.
    /// Le voci nascoste restano in elenco così possono essere mostrate di nuovo.
    pub fn load_icewm_menu(&mut self) {
        let mut found: HashMap<String, MenuItemCustom> = HashMap::new();

        for dir in desktop_dirs() {
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for path in entries.flatten().map(|e| e.path()) {
                if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                    continue;
                }
                let id = match path.file_name().and_then(|s| s.to_str()) {
                    Some(id) => id.to_string(),
                    None => continue,
                };
                if found.contains_key(&id) {
                    continue; // il file utente ha priorità su quello di sistema
                }
                let content = match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let kind = desktop_value(&content, "Type");
                let name = desktop_value(&content, "Name");
                let exec = desktop_value(&content, "Exec");
                if name.is_empty() || exec.is_empty() || (!kind.is_empty() && kind != "Application") {
                    continue;
                }
                let categories = desktop_categories(&content);
                let category = menu_category_for(&categories, &name, &exec);
                found.insert(
                    id.clone(),
                    MenuItemCustom {
                        name,
                        exec,
                        icon: desktop_value(&content, "Icon"),
                        category: category.to_string(),
                        desktop_id: id,
                        source: path,
                        no_display: desktop_bool(&content, "NoDisplay"),
                        hidden: desktop_bool(&content, "Hidden"),
                    },
                );
            }
        }

        // Voci definite solo nel menu IceWM: vengono incluse comunque.
        if let Some(home) = home_dir() {
            let icewm = home.join(".icewm/menu");
            if let Ok(content) = fs::read_to_string(&icewm) {
                for line in content.lines() {
                    let line = line.trim();
                    if !line.starts_with("prog") {
                        continue;
                    }
                    let parts: Vec<&str> = line.split('"').collect();
                    let (name, icon, exec) = if parts.len() >= 6 {
                        (parts[1], parts[3], parts[5])
                    } else if parts.len() >= 4 {
                        (parts[1], "", parts[3])
                    } else {
                        continue;
                    };
                    if name.trim().is_empty() || exec.trim().is_empty() {
                        continue;
                    }
                    let id = desktop_id_for(name);
                    if found.contains_key(&id) {
                        continue;
                    }
                    found.insert(
                        id.clone(),
                        MenuItemCustom {
                            name: name.to_string(),
                            exec: exec.replace("\\\"", "\""),
                            icon: icon.to_string(),
                            category: "Accessori".to_string(),
                            desktop_id: id,
                            source: PathBuf::new(),
                            no_display: false,
                            hidden: false,
                        },
                    );
                }
            }
        }

        let mut items: Vec<MenuItemCustom> = found.into_values().collect();
        items.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
        self.custom_menu_items = items;
    }

    fn set_entry_visibility(&mut self, id: &str, show: bool) {
        let item = match self.custom_menu_items.iter().find(|i| i.desktop_id == id).cloned() {
            Some(item) => item,
            None => return,
        };
        if let Err(err) = write_visibility(&item, show) {
            self.out.insert("menu_editor".into(), err);
            return;
        }
        self.refresh_after_change();
        let message = if show {
            tf("✔ Voce «{}» resa visibile su tutti i menu del desktop.", &[&item.name])
        } else {
            tf("✔ Voce «{}» nascosta su tutti i menu del desktop.", &[&item.name])
        };
        self.out.insert("menu_editor".into(), message);
    }

    fn add_custom_desktop_entry(&mut self) {
        let name = self.new_menu_name.trim().to_string();
        let exec = self.new_menu_exec.trim().to_string();
        let icon = self.new_menu_icon.trim().to_string();
        let category = self.new_menu_category.trim().to_string();
        if name.is_empty() || exec.is_empty() {
            self.out.insert(
                "menu_editor".into(),
                t("✘ Inserire nome e comando per creare una voce.").to_string(),
            );
            return;
        }
        let apps_dir = match local_apps_dir() {
            Some(d) => d,
            None => {
                self.out.insert(
                    "menu_editor".into(),
                    t("✘ Nessuna cartella utente disponibile per i lanciatori.").to_string(),
                );
                return;
            }
        };
        if let Err(e) = fs::create_dir_all(&apps_dir) {
            self.out.insert(
                "menu_editor".into(),
                tf("✘ Impossibile aggiornare la voce: {}", &[&e]),
            );
            return;
        }

        let id = self
            .menu_editing_id
            .clone()
            .unwrap_or_else(|| desktop_id_for(&name));
        let local = apps_dir.join(&id);
        let base = if local.exists() {
            fs::read_to_string(&local).ok()
        } else {
            self.custom_menu_items
                .iter()
                .find(|i| i.desktop_id == id)
                .and_then(|i| {
                    if i.source.extension().and_then(|s| s.to_str()) == Some("desktop") {
                        fs::read_to_string(&i.source).ok()
                    } else {
                        None
                    }
                })
        };
        let mut content = base.unwrap_or_else(|| make_desktop_entry(&name, &exec, &icon, &category));
        content = set_desktop_value(&content, "Name", &name);
        content = set_desktop_value(&content, "Exec", &exec);
        content = set_desktop_value(&content, "Icon", &icon);
        let cat = if category.is_empty() { "Utility".to_string() } else { category };
        content = set_desktop_value(&content, "Categories", &format!("{cat};"));
        // forza entrambi i flag: un NoDisplay=true presente nel file originale
        // non deve lasciare la voce nascosta dopo «Salva».
        content = with_visibility(&content, true);
        if let Err(e) = fs::write(&local, content) {
            self.out.insert(
                "menu_editor".into(),
                tf("✘ Impossibile aggiornare la voce: {}", &[&e]),
            );
            return;
        }

        self.clear_menu_form();
        self.refresh_after_change();
        self.out.insert(
            "menu_editor".into(),
            tf("✔ Voce «{}» salvata e menu aggiornati.", &[&name]),
        );
    }

    fn remove_custom_desktop_entry_by_id(&mut self, id: &str) {
        let item = match self.custom_menu_items.iter().find(|i| i.desktop_id == id).cloned() {
            Some(item) => item,
            None => return,
        };
        let apps_dir = match local_apps_dir() {
            Some(d) => d,
            None => return,
        };
        let local = apps_dir.join(id);
        let message = if local.exists() {
            match fs::remove_file(&local) {
                Ok(()) => tf("✔ Voce «{}» rimossa dalle personalizzazioni.", &[&item.name]),
                Err(e) => tf("✘ Impossibile aggiornare la voce: {}", &[&e]),
            }
        } else {
            // I file di sistema non si toccano: la voce viene nascosta con un
            // override locale, così sparisce da tutti i menu.
            match write_visibility(&item, false) {
                Ok(()) => tf(
                    "✔ Voce di sistema «{}» nascosta (i file di sistema non vengono modificati).",
                    &[&item.name],
                ),
                Err(e) => e,
            }
        };
        if self.menu_editing_id.as_deref() == Some(id) {
            self.clear_menu_form();
        }
        self.refresh_after_change();
        self.out.insert("menu_editor".into(), message);
    }
}
