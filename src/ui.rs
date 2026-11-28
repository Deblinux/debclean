use crate::i18n::{t, tf, tr};
use crate::pages_disk_treemap::DiskCategory;
use crate::Page;
use eframe::egui::epaint::{Mesh, TextureId, Vertex, WHITE_UV};
use eframe::egui::{self, vec2, Align2, Color32, FontId, Pos2, Rect, Rounding, Sense, Shape, Stroke, TextStyle};
use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub const ACCENT: Color32 = Color32::from_rgb(0, 196, 168);
/// Accento tenue per il tema scuro.
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(103, 232, 204);
/// Accento scuro e leggibile per il tema chiaro.
pub const ACCENT_DEEP: Color32 = Color32::from_rgb(13, 148, 136);
pub const BLUE_LIGHT: Color32 = Color32::from_rgb(96, 165, 250);
pub const TEXT: Color32 = Color32::from_rgb(238, 242, 248);
pub const TEXT_DIM: Color32 = Color32::from_rgb(172, 183, 199);
/// Testo ad alto contrasto per il tema chiaro.
pub const TEXT_LIGHT: Color32 = Color32::from_rgb(15, 23, 42);       // #0f172a
pub const TEXT_DIM_LIGHT: Color32 = Color32::from_rgb(71, 85, 105);  // #475569
pub const BAR_BLUE: Color32 = Color32::from_rgb(37, 99, 235);

/// Pulsante primario: verde acqua pieno, senza gradienti né maschere.
pub const PRIMARY_BG: Color32 = Color32::from_rgb(20, 184, 166);   // #14b8a6
pub const PRIMARY_BG_HOVER: Color32 = Color32::from_rgb(13, 148, 136); // #0d9488
/// Pulsante secondario globale, usato da tutti i widget standard.
pub const SECONDARY_BG: Color32 = Color32::from_rgb(30, 41, 59);       // #1e293b
pub const SECONDARY_BG_HOVER: Color32 = Color32::from_rgb(38, 50, 73);
pub const SECONDARY_BG_ACTIVE: Color32 = Color32::from_rgb(26, 36, 52);
pub const SECONDARY_BORDER: Color32 = Color32::from_rgb(51, 65, 85);   // #334155
pub const SECONDARY_TEXT: Color32 = Color32::from_rgb(248, 250, 252);  // #f8fafc
// Varianti chiare dei pulsanti secondari / campi di testo.
pub const SECONDARY_BG_LIGHT: Color32 = Color32::from_rgb(241, 245, 249); // #f1f5f9
pub const SECONDARY_BG_HOVER_LIGHT: Color32 = Color32::from_rgb(226, 232, 240);
pub const SECONDARY_BG_ACTIVE_LIGHT: Color32 = Color32::from_rgb(203, 213, 225);
pub const SECONDARY_BORDER_LIGHT: Color32 = Color32::from_rgb(148, 163, 184); // #94a3b8
/// Bordo e sfondo delle card in tutte le schede.
pub const CARD_BORDER: Color32 = Color32::from_rgb(38, 51, 72);        // #263348
pub const CARD_BORDER_LIGHT: Color32 = Color32::from_rgb(203, 213, 225); // #cbd5e1

/// Tema attivo, aggiornato da `apply_theme`: usato da tutti gli helper di testo.
static DARK_MODE: AtomicBool = AtomicBool::new(true);
pub fn is_dark() -> bool {
    DARK_MODE.load(Ordering::Relaxed)
}
/// Testo principale adattato al tema attivo.
pub fn text_primary() -> Color32 {
    if is_dark() { TEXT } else { TEXT_LIGHT }
}
/// Testo secondario adattato al tema attivo.
pub fn text_dim() -> Color32 {
    if is_dark() { TEXT_DIM } else { TEXT_DIM_LIGHT }
}
/// Accento leggibile sullo sfondo del tema attivo.
pub fn accent_bright() -> Color32 {
    if is_dark() { ACCENT_LIGHT } else { ACCENT_DEEP }
}
/// Colore accento per il testo: invariato in dark, scuro in light.
pub fn accent_text() -> Color32 {
    if is_dark() { ACCENT } else { ACCENT_DEEP }
}
/// Adatta un colore semantico (verde/giallo/rosso/blu) al tema attivo:
/// in light viene scurito per garantire contrasto sui fondi chiari.
pub fn semantic_text(c: Color32) -> Color32 {
    if is_dark() { c } else { mix(c, TEXT_LIGHT, 0.35) }
}
pub const GOOD: Color32 = Color32::from_rgb(70, 195, 105);
pub const WARN: Color32 = Color32::from_rgb(244, 168, 40);
pub const BAD: Color32 = Color32::from_rgb(232, 72, 72);

// ---- Livelli di sicurezza / rischio delle sezioni ---------------------------------------------

/// Classificazione unica dei tre livelli di rischio associati alle pagine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum RiskLevel {
    Safe = 0,     // Sicura
    Caution = 1,  // Con cautela
    Delicate = 2, // Delicato
}

impl RiskLevel {
    /// Colore semantico: verde, ambra, rosso.
    pub fn color(self) -> Color32 {
        match self {
            RiskLevel::Safe => Color32::from_rgb(34, 197, 94),    // #22c55e
            RiskLevel::Caution => Color32::from_rgb(245, 158, 11), // #f59e0b
            RiskLevel::Delicate => Color32::from_rgb(239, 68, 68), // #ef4444
        }
    }

    /// Etichetta tradotta tramite il sistema `t(...)`.
    pub fn label(self) -> &'static str {
        match self {
            RiskLevel::Safe => t("Sicura"),
            RiskLevel::Caution => t("Con cautela"),
            RiskLevel::Delicate => t("Delicato"),
        }
    }

    /// Glifo usato nella pillola dell'intestazione.
    pub fn icon(self) -> &'static str {
        match self {
            RiskLevel::Safe => "✔",
            RiskLevel::Caution => "⚠",
            RiskLevel::Delicate => "❗",
        }
    }

    /// Colore del testo con contrasto adattato al tema attivo.
    pub fn text_color(self) -> Color32 {
        let c = self.color();
        if is_dark() {
            mix(c, Color32::WHITE, 0.22)
        } else {
            semantic_text(c)
        }
    }
}

/// Rischio di una pagina: unica fonte per sidebar e intestazioni.
pub fn page_risk(page: Page) -> RiskLevel {
    use Page::*;
    use RiskLevel::*;
    match page {
        // Lettura / diagnostica / impostazioni dell'app: nessuna modifica del sistema.
        Dashboard | Info | Disk | DiskAnalyzer | DiskTreemap | Procs | Temps | Network | Boot
        | History | Report | Guide | About | HardwareHealth | Settings | Backup | Log
        | SafeOptimize => Safe,
        // Pulizie e modifiche utente: utili, ma da eseguire con attenzione.
        Cleaner | AdvancedCleaner | Updates | Timers | Snaps | Containers | AppImages | Apps
        | Themes | MenuEditor | TelemetryGuard | Mounts | DevContainers | Zram | Quarantine => Caution,
        // Cancellazioni definitive, permessi, bootloader e componenti di sistema.
        Fixer | Kernel | KernelBoot | Repos | Startup | Perms | Services | Synaptic | Shredder
        | AppLifecycle | Duplicates | Firewall | SecurityHardening | SystemEditor
        | RestoreHistory => Delicate,
    }
}

/// Livello della pagina correntemente visualizzata (usato da `title`).
static CURRENT_RISK: AtomicU8 = AtomicU8::new(RiskLevel::Safe as u8);

/// Tutte le schede ordinarie adottano il passo compatto della Dashboard.
/// Dashboard ed Editor Menu mantengono invece il loro layout già sistemato.
static COMPACT_PAGES: AtomicBool = AtomicBool::new(true);

pub fn compact_pages() -> bool {
    COMPACT_PAGES.load(Ordering::Relaxed)
}

pub fn set_current_page(page: Page) {
    CURRENT_RISK.store(page_risk(page) as u8, Ordering::Relaxed);
    COMPACT_PAGES.store(!matches!(page, Page::Dashboard | Page::MenuEditor), Ordering::Relaxed);
}

/// Spaziatura verticale della scheda corrente: compatta ovunque tranne che
/// nella Dashboard e nell'Editor Menu, che gestiscono il proprio passo.
pub fn begin_page(ui: &mut egui::Ui) {
    if compact_pages() {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.spacing_mut().button_padding.y = 6.0;
    }
}

// ---- Helper tabelle: filtro rapido, ordinamento, selezione -------------------

/// Direzione di ordinamento di una colonna.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn toggle(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }

    pub fn arrow(self) -> &'static str {
        match self {
            Self::Asc => "▲",
            Self::Desc => "▼",
        }
    }
}

/// Stato generico di ordinamento per colonna.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortState<K: Copy + PartialEq> {
    pub key: K,
    pub dir: SortDir,
}

impl<K: Copy + PartialEq> SortState<K> {
    pub fn new(key: K, dir: SortDir) -> Self {
        Self { key, dir }
    }

    /// Clic su intestazione: stesso campo = toggle, campo diverso = ascendente.
    pub fn click(&mut self, key: K) {
        if self.key == key {
            self.dir = self.dir.toggle();
        } else {
            self.key = key;
            self.dir = SortDir::Asc;
        }
    }
}

/// Disegna un'intestazione di colonna cliccabile con indicatore `▲/▼`.
pub fn sort_header<K: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    key: K,
    sort: &mut SortState<K>,
) {
    let active = sort.key == key;
    let text = if active {
        format!("{} {}", label, sort.dir.arrow())
    } else {
        label.to_string()
    };
    let color = if active { accent_text() } else { text_primary() };
    let resp = ui.add(
        egui::Button::new(egui::RichText::new(text).strong().color(color))
            .frame(false)
            .wrap(),
    );
    if resp.clicked() {
        sort.click(key);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

/// Barra di filtro rapido con contatore "X / Y" e pulsante svuota.
pub fn filter_bar(ui: &mut egui::Ui, filter: &mut String, shown: usize, total: usize) {
    ui.horizontal_wrapped(|ui| {
        ui.label(t("Filtra:"));
        let resp = ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text(t("Filtro rapido..."))
                .desired_width(fit_width(ui, 280.0, 120.0)),
        );
        if !filter.is_empty() {
            if ui
                .small_button("✕")
                .on_hover_text(t("Svuota filtro"))
                .clicked()
            {
                filter.clear();
                resp.request_focus();
            }
        }
        ui.label(
            egui::RichText::new(format!("{shown} / {total}"))
                .color(text_dim())
                .small(),
        );
    });
}

/// Pulsanti uniformi di selezione multipla, adattivi allo spazio.
pub fn selection_controls(ui: &mut egui::Ui, selected: usize, total: usize) -> SelectionAction {
    let mut action = SelectionAction::None;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(tf("{} selezionati", &[&selected.to_string()]))
                .color(text_dim())
                .small(),
        );
        if total > 0 {
            let compact = ui.available_width() < 360.0;
            let all_label = if compact { t("Tutti") } else { t("Seleziona tutto") };
            let none_label = if compact { t("Nessuno") } else { t("Deseleziona tutto") };
            if ui.small_button(all_label).clicked() {
                action = SelectionAction::All;
            }
            if ui.small_button(none_label).clicked() {
                action = SelectionAction::NoneSelected;
            }
        }
    });
    action
}

/// Azione richiesta dai controlli di selezione rapida.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionAction {
    None,
    All,
    NoneSelected,
}

/// Confronto case-insensitive per ordinamento alfabetico.
pub fn cmp_alpha(a: &str, b: &str) -> std::cmp::Ordering {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    a.cmp(&b)
}

/// Estrae un numero da stringhe tipo "12.3", "5%", "1234".
pub fn parse_f64_loose(s: &str) -> f64 {
    let cleaned: String = s
        .trim()
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == ',')
        .collect();
    cleaned.replace(',', ".").parse::<f64>().unwrap_or(0.0)
}

/// Larghezza adattiva: `ideal` viene usata quando c'è spazio, altrimenti si
/// restringe fino a `min` senza mai superare lo spazio effettivamente libero.
/// Serve a evitare che widget con larghezze storiche (420, 520, 640 px)
/// sbordino dalle schede su monitor da 800 px o con traduzioni lunghe.
pub fn fit_width(ui: &egui::Ui, ideal: f32, min: f32) -> f32 {
    let avail = ui.available_width().max(1.0);
    let floor = min.min(avail);
    ideal.min(avail).max(floor)
}

/// Numero di colonne di larghezza `ideal` che entrano davvero nello spazio
/// disponibile (almeno una, al massimo `max`).
pub fn column_count(ui: &egui::Ui, ideal: f32, gap: f32, max: usize) -> usize {
    let avail = ui.available_width().max(1.0);
    let count = ((avail + gap) / (ideal + gap)).floor().max(1.0) as usize;
    count.max(1).min(max)
}

pub fn current_risk() -> RiskLevel {
    match CURRENT_RISK.load(Ordering::Relaxed) {
        1 => RiskLevel::Caution,
        2 => RiskLevel::Delicate,
        _ => RiskLevel::Safe,
    }
}

// ---- Treemap: colori per categoria -------------------------------------------------------------
// I colori base sono fissi; il riempimento viene mixato con lo sfondo del tema
// attivo, così i rettangoli restano leggibili sia in dark che in light mode.

/// Tinta base di una categoria della treemap.
pub fn treemap_base(cat: DiskCategory) -> Color32 {
    match cat {
        DiskCategory::Media => Color32::from_rgb(186, 85, 211),    // viola
        DiskCategory::Archive => Color32::from_rgb(234, 146, 55),  // arancio
        DiskCategory::Development => Color32::from_rgb(64, 140, 235), // blu
        DiskCategory::Documents => Color32::from_rgb(62, 175, 120), // verde
        DiskCategory::System => Color32::from_rgb(120, 134, 156),  // grigio
    }
}

/// Riempimento adattato al tema attivo (`hovered` → più saturo).
pub fn treemap_fill(cat: DiskCategory, hovered: bool) -> Color32 {
    let bg = if is_dark() {
        Color32::from_rgb(25, 31, 42)
    } else {
        Color32::from_rgb(248, 250, 252)
    };
    let strength = if is_dark() {
        if hovered { 0.68 } else { 0.42 }
    } else if hovered {
        0.52
    } else {
        0.30
    };
    mix(bg, treemap_base(cat), strength)
}

/// Bordo del rettangolo; quello selezionato usa l'accento globale.
pub fn treemap_stroke(cat: DiskCategory, selected: bool) -> Stroke {
    if selected {
        Stroke::new(2.5_f32, accent_bright())
    } else {
        Stroke::new(1.0_f32, mix(treemap_base(cat), text_primary(), 0.28))
    }
}

pub const HIST_LEN: usize = 60;

/// Interpolazione tra due colori (t = 0 → `a`, t = 1 → `b`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

/// Copia un colore cambiandone solo l'alfa.
pub fn alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], a)
}

/// Pillola di stato condivisa da tutte le schede: sfondo tenue, bordo sottile,
/// testo leggibile sia nel tema scuro sia in quello chiaro. Un solo frame, senza
/// ombre né gradienti: può essere usata anche dentro elenchi di centinaia di righe.
pub fn pill(ui: &mut egui::Ui, text: &str, tint: Color32) {
    egui::Frame::default()
        .fill(alpha(tint, if is_dark() { 46 } else { 30 }))
        .stroke(Stroke::new(1.0_f32, alpha(tint, if is_dark() { 140 } else { 190 })))
        .rounding(7.0)
        .inner_margin(egui::Margin::symmetric(8.0, 3.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(12.5)
                    .strong()
                    .color(if is_dark() { mix(tint, Color32::WHITE, 0.55) } else { semantic_text(tint) }),
            );
        });
}

/// Badge/riquadro icona universale: quadrato arrotondato 8px, sfondo al 15%
/// del colore accento e glifo centrato. Nessuna maschera, clip-path o ritaglio.
pub fn icon_badge(p: &egui::Painter, rect: Rect, tint: Color32, glyph: &str, font_size: f32) {
    p.rect_filled(rect, 8.0, alpha(tint, 38)); // 38/255 ≈ 15%
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(font_size),
        if is_dark() { mix(tint, Color32::WHITE, 0.65) } else { mix(tint, TEXT_LIGHT, 0.35) },
    );
}

/// Striscia orizzontale sfumata (usata come filetto luminoso sulle card).
pub fn accent_strip(p: &egui::Painter, rect: Rect, from: Color32, to: Color32) {
    let steps = 28usize.max(1);
    let w = rect.width() / steps as f32;
    for i in 0..steps {
        let x0 = rect.left() + w * i as f32;
        let x1 = rect.left() + w * (i as f32 + 1.15);
        p.rect_filled(
            Rect::from_min_max(Pos2::new(x0, rect.top()), Pos2::new(x1.min(rect.right()), rect.bottom())),
            0.0,
            mix(from, to, i as f32 / (steps - 1) as f32),
        );
    }
}

// Funzioni di sfondo dinamiche in base al tema
pub fn sidebar_bg(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(16, 21, 29)
    } else {
        Color32::from_rgb(241, 245, 249)
    }
}

pub fn content_bg(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(25, 31, 42)
    } else {
        Color32::from_rgb(248, 250, 252)
    }
}

pub fn card_bg(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(22, 32, 50) // #162032
    } else {
        Color32::from_rgb(255, 255, 255)
    }
}

pub fn card_border(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        CARD_BORDER
    } else {
        CARD_BORDER_LIGHT
    }
}

pub fn apply_theme(ctx: &egui::Context, dark_mode: bool) {
    // Aggiorna prima lo stato globale: tutti gli helper di testo lo leggono.
    DARK_MODE.store(dark_mode, Ordering::Relaxed);

    let mut v = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = if dark_mode { Color32::from_rgb(25, 31, 42) } else { Color32::from_rgb(248, 250, 252) };
    v.window_fill = if dark_mode { Color32::from_rgb(36, 44, 58) } else { Color32::WHITE };
    v.extreme_bg_color = if dark_mode {
        Color32::from_rgb(14, 28, 58)
    } else {
        Color32::from_rgb(248, 250, 252) // sfondo chiaro dei campi di testo
    };
    v.selection.bg_fill = alpha(ACCENT, 90);
    v.selection.stroke = Stroke::new(1.4_f32, accent_bright());
    v.hyperlink_color = accent_bright();
    v.window_rounding = Rounding::same(14.0);
    v.menu_rounding = Rounding::same(10.0);
    v.window_shadow = egui::Shadow {
        offset: vec2(0.0, 12.0),
        blur: 28.0,
        spread: -4.0,
        color: if dark_mode { Color32::from_black_alpha(110) } else { Color32::from_black_alpha(28) },
    };

    // Testo: scuro su tema chiaro, chiaro su tema scuro.
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text_primary());

    // Pulsanti secondari e campi di testo: stessa palette in tutto il tema.
    let r = egui::Rounding::same(6.0);
    let border = Stroke::new(1.0_f32, if dark_mode { SECONDARY_BORDER } else { SECONDARY_BORDER_LIGHT });
    let widget_text = if dark_mode { SECONDARY_TEXT } else { TEXT_LIGHT };
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
        w.rounding = r;
        w.fg_stroke = Stroke::new(1.0_f32, widget_text);
        w.bg_stroke = border;
    }
    v.widgets.inactive.bg_fill = if dark_mode { SECONDARY_BG } else { SECONDARY_BG_LIGHT };
    v.widgets.inactive.weak_bg_fill = v.widgets.inactive.bg_fill;
    v.widgets.hovered.bg_fill = if dark_mode { SECONDARY_BG_HOVER } else { SECONDARY_BG_HOVER_LIGHT };
    v.widgets.hovered.weak_bg_fill = v.widgets.hovered.bg_fill;
    v.widgets.active.bg_fill = if dark_mode { SECONDARY_BG_ACTIVE } else { SECONDARY_BG_ACTIVE_LIGHT };
    v.widgets.active.weak_bg_fill = v.widgets.active.bg_fill;
    v.button_frame = true;
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    // Dimensioni moderatamente compatte ma sempre leggibili a occhio nudo:
    // testo normale 16 px, note 13.5 px, titoli 26 px. Nessun character minuscolo.
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(26.0)),
        (TextStyle::Body, FontId::proportional(16.0)),
        (TextStyle::Button, FontId::proportional(16.0)),
        (TextStyle::Small, FontId::proportional(13.5)),
        (TextStyle::Monospace, FontId::monospace(14.5)),
    ]
    .into();
    // Spaziatura GT-STACER-like: compatta, ma senza comprimere i pulsanti.
    style.spacing.item_spacing = vec2(8.0, 7.0);
    style.spacing.button_padding = vec2(12.0, 5.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.slider_width = 220.0;
    style.spacing.scroll.bar_width = 11.0;
    ctx.set_style(style);
}

/// Pillola arrotondata con icona ed etichetta del livello di rischio.
fn risk_badge(ui: &mut egui::Ui, risk: RiskLevel) -> egui::Response {
    let w = fit_width(ui, 142.0, 92.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
    let p = ui.painter();
    let fill_alpha = if is_dark() { 46 } else { 34 };
    let stroke_alpha = if is_dark() { 170 } else { 220 };
    p.rect_filled(rect, 13.0, alpha(risk.color(), fill_alpha));
    p.rect_stroke(rect, 13.0, Stroke::new(1.0_f32, alpha(risk.color(), stroke_alpha)));
    let text = format!("{} {}", risk.icon(), risk.label());
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(if w < 118.0 { 11.5 } else { 13.5 }),
        risk.text_color(),
    );
    resp.on_hover_text(risk.label())
}

/// Titolo di pagina con barra accentata, badge del rischio a destra e filetto sfumato sottostante.
pub fn title(ui: &mut egui::Ui, text: &'static str) {
    if compact_pages() {
        title_balanced(ui, text);
        return;
    }
    let title_col = text_primary();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(6.0, 30.0), Sense::hover());
        ui.painter().rect_filled(r, 3.0, if is_dark() { ACCENT } else { ACCENT_DEEP });
        ui.add_space(3.0);
        ui.label(egui::RichText::new(t(text)).size(28.0).strong().color(title_col));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            risk_badge(ui, current_risk());
        });
    });
    let w = ui.available_width().min(420.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 2.0), Sense::hover());
    accent_strip(ui.painter(), r, alpha(accent_bright(), 150), Color32::TRANSPARENT);
    ui.add_space(13.0);
}

/// Titolo compatto e leggibile per tutte le schede standard: stessa struttura
/// della Dashboard, con un corpo leggermente più grande.
fn title_balanced(ui: &mut egui::Ui, text: &'static str) {
    let title_col = text_primary();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(6.0, 28.0), Sense::hover());
        ui.painter().rect_filled(r, 3.0, if is_dark() { ACCENT } else { ACCENT_DEEP });
        ui.add_space(3.0);
        ui.label(egui::RichText::new(t(text)).size(25.0).strong().color(title_col));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            risk_badge(ui, current_risk());
        });
    });
    let w = ui.available_width().min(420.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 2.0), Sense::hover());
    accent_strip(ui.painter(), r, alpha(accent_bright(), 150), Color32::TRANSPARENT);
    ui.add_space(4.0);
}

/// Variante compatta del titolo, usata solo dalla Dashboard.
/// Mantiene barra, badge e filetto ma riduce l'altezza totale.
pub fn title_compact(ui: &mut egui::Ui, text: &'static str) {
    let title_col = text_primary();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(6.0, 26.0), Sense::hover());
        ui.painter().rect_filled(r, 3.0, if is_dark() { ACCENT } else { ACCENT_DEEP });
        ui.add_space(3.0);
        ui.label(egui::RichText::new(t(text)).size(25.0).strong().color(title_col));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            risk_badge(ui, current_risk());
        });
    });
    let w = ui.available_width().min(420.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 2.0), Sense::hover());
    accent_strip(ui.painter(), r, alpha(accent_bright(), 150), Color32::TRANSPARENT);
    ui.add_space(3.0);
}

/// Intestazione di sezione con icona, titolo e sottotitolo.
pub fn section(ui: &mut egui::Ui, glyph: &str, name: &'static str, sub: &'static str) {
    section_text(ui, glyph, t(name), if sub.is_empty() { "" } else { t(sub) });
}

/// Variante di [`section`] per testi già risolti a runtime (non traduce).
pub fn section_text(ui: &mut egui::Ui, glyph: &str, name: &str, sub: &str) {
    if compact_pages() {
        section_text_balanced(ui, glyph, name, sub);
        return;
    }
    ui.horizontal(|ui| {
        let (badge, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
        icon_badge(ui.painter(), badge, ACCENT, glyph, 17.0);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(name).strong().size(18.0));
            if !sub.is_empty() {
                ui.label(egui::RichText::new(sub).size(13.0).color(text_dim()));
            }
        });
    });
    ui.add_space(8.0);
}

/// Intestazione compatta per le schede standard: icona e testo su una riga sola.
fn section_balanced(ui: &mut egui::Ui, glyph: &str, name: &'static str, sub: &'static str) {
    section_text_balanced(ui, glyph, t(name), if sub.is_empty() { "" } else { t(sub) });
}

fn section_text_balanced(ui: &mut egui::Ui, glyph: &str, name: &str, sub: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        let (badge, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
        icon_badge(ui.painter(), badge, ACCENT, glyph, 14.0);
        ui.label(egui::RichText::new(name).strong().size(16.5));
        if !sub.is_empty() {
            ui.label(egui::RichText::new(sub).size(12.5).color(text_dim()));
        }
    });
    ui.add_space(3.0);
}

/// Intestazione compatta usata solo dalla Dashboard: stessa icona, riga singola.
pub fn section_compact(ui: &mut egui::Ui, glyph: &str, name: &'static str, sub: &'static str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (badge, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
        icon_badge(ui.painter(), badge, ACCENT, glyph, 12.0);
        ui.label(egui::RichText::new(t(name)).strong().size(16.0));
        if !sub.is_empty() {
            ui.label(egui::RichText::new(t(sub)).size(11.5).color(text_dim()));
        }
    });
    ui.add_space(2.0);
}

pub fn sub_tabs(ui: &mut egui::Ui, sub: &mut usize, names: &[&'static str]) {
    let (margin, after, size) = if compact_pages() { (7.0, 5.0, 15.5) } else { (10.0, 11.0, 16.0) };
    egui::Frame::default()
        .fill(card_bg(ui.ctx()))
        .rounding(if compact_pages() { 9.0 } else { 10.0 })
        .inner_margin(egui::Margin::same(margin))
        .stroke(Stroke::new(1.0_f32, card_border(ui.ctx())))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 3.0);
                for (i, n) in names.iter().enumerate() {
                    ui.selectable_value(sub, i, egui::RichText::new(t(*n)).size(size));
                }
            });
        });
    ui.add_space(after);
}

/// Card pulita, usata da TUTTE le schede: sfondo #162032, bordo 1px #263348,
/// raggio 10px, padding interno uniforme. Nessun riflesso/header interno.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    card_tinted(ui, ACCENT, add)
}

/// Variante di [`card`] mantenuta per compatibilità: lo stile è identico.
pub fn card_tinted<R>(ui: &mut egui::Ui, _tint: Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if compact_pages() {
        return card_balanced(ui, add);
    }
    egui::Frame::default()
        .fill(card_bg(ui.ctx()))
        .rounding(10.0)
        .inner_margin(egui::Margin::same(16.0))
        .stroke(Stroke::new(1.0_f32, card_border(ui.ctx())))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Card delle schede standard: stesso aspetto della Dashboard, con un padding
/// appena più generoso per non schiacciare tabelle, campi e pulsanti azione.
fn card_balanced<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::default()
        .fill(card_bg(ui.ctx()))
        .rounding(9.0)
        .inner_margin(egui::Margin::symmetric(11.0, 9.0))
        .stroke(Stroke::new(1.0_f32, card_border(ui.ctx())))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            add(ui)
        })
        .inner
}

/// Frame compatto riservato alla Dashboard: stesso stile, padding dimezzato.
pub fn card_compact<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::default()
        .fill(card_bg(ui.ctx()))
        .rounding(9.0)
        .inner_margin(egui::Margin::symmetric(8.0, 8.0))
        .stroke(Stroke::new(1.0_f32, card_border(ui.ctx())))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            add(ui)
        })
        .inner
}

/// Alias compatto di [`card_tinted`] per i riquadri colorati della Dashboard.
pub fn card_tinted_compact<R>(ui: &mut egui::Ui, _tint: Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    card_compact(ui, add)
}

/// Card "KPI" per la dashboard: numero grande, etichetta e icona colorata.
pub fn kpi_card(ui: &mut egui::Ui, glyph: &str, value: &str, name: &'static str, tint: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(192.0, 96.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 10.0, card_bg(ui.ctx()));
    p.rect_stroke(rect, 10.0, Stroke::new(1.0_f32, card_border(ui.ctx())));
    let icon = Rect::from_min_size(Pos2::new(rect.left() + 14.0, rect.top() + 13.0), vec2(32.0, 32.0));
    icon_badge(&p, icon, tint, glyph, 17.0);
    p.text(
        Pos2::new(rect.left() + 56.0, rect.top() + 29.0),
        Align2::LEFT_CENTER,
        value,
        FontId::proportional(23.0),
        tint,
    );
    p.text(
        Pos2::new(rect.left() + 14.0, rect.bottom() - 17.0),
        Align2::LEFT_CENTER,
        t(name),
        FontId::proportional(12.5),
        text_dim(),
    );
}

/// Card KPI compatta per la Dashboard: stessa struttura, altezza 50px.
pub fn kpi_card_compact(ui: &mut egui::Ui, glyph: &str, value: &str, name: &'static str, tint: Color32) {
    let w = fit_width(ui, 156.0, 110.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 50.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 8.0, card_bg(ui.ctx()));
    p.rect_stroke(rect, 8.0, Stroke::new(1.0_f32, card_border(ui.ctx())));
    let icon = Rect::from_min_size(Pos2::new(rect.left() + 7.0, rect.top() + 7.0), vec2(22.0, 22.0));
    icon_badge(&p, icon, tint, glyph, 12.0);
    p.text(
        Pos2::new(rect.left() + 37.0, rect.top() + 17.0),
        Align2::LEFT_CENTER,
        value,
        FontId::proportional(18.0),
        tint,
    );
    p.text(
        Pos2::new(rect.left() + 7.0, rect.bottom() - 9.0),
        Align2::LEFT_CENTER,
        t(name),
        FontId::proportional(11.0),
        text_dim(),
    );
}

pub fn hint(ui: &mut egui::Ui, text: &'static str) {
    let dark_mode = ui.visuals().dark_mode;
    let fill = if dark_mode { Color32::from_rgb(20, 52, 92) } else { Color32::from_rgb(218, 232, 250) };
    let text_col = text_primary();
    let compact = compact_pages();
    let (margin, icon, font, after) = if compact { (8.0, 22.0, 15.0, 5.0) } else { (12.0, 28.0, 16.0, 8.0) };
    egui::Frame::default()
        .fill(fill)
        .rounding(if compact { 9.0 } else { 10.0 })
        .inner_margin(egui::Margin::same(margin))
        .stroke(Stroke::new(1.0_f32, alpha(ACCENT, 160)))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let w = ui.available_width();
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(icon, icon), Sense::hover());
                icon_badge(ui.painter(), r, ACCENT, "i", if compact { 14.0 } else { 18.0 });
                ui.vertical(|ui| {
                    ui.set_max_width((w - icon * 2.0).max(200.0));
                    ui.label(egui::RichText::new(t(text)).size(font).color(text_col));
                });
            });
        });
    ui.add_space(after);
}

/// Pulsante principale globale: sfondo pieno #14b8a6 (hover #0d9488),
/// testo bianco centrato, raggio 8px, padding uniforme 10px 18px.
/// Nessun gradiente, mesh, ombra o maschera: elimina l'artefatto triangolare.
pub fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let old_padding = ui.spacing().button_padding;
    let old_inactive = ui.visuals().widgets.inactive.weak_bg_fill;
    let old_hovered = ui.visuals().widgets.hovered.weak_bg_fill;
    let old_active = ui.visuals().widgets.active.weak_bg_fill;

    let compact = compact_pages();
    ui.spacing_mut().button_padding = if compact { vec2(14.0, 7.0) } else { vec2(18.0, 10.0) };
    ui.visuals_mut().widgets.inactive.weak_bg_fill = if enabled { PRIMARY_BG } else { alpha(PRIMARY_BG, 110) };
    ui.visuals_mut().widgets.hovered.weak_bg_fill = PRIMARY_BG_HOVER;
    ui.visuals_mut().widgets.active.weak_bg_fill = PRIMARY_BG_HOVER;

    let resp = ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(text).size(if compact { 17.0 } else { 19.0 }).strong().color(Color32::WHITE))
            .stroke(Stroke::NONE)
            .rounding(if compact { 7.0 } else { 8.0 }),
    );

    ui.spacing_mut().button_padding = old_padding;
    ui.visuals_mut().widgets.inactive.weak_bg_fill = old_inactive;
    ui.visuals_mut().widgets.hovered.weak_bg_fill = old_hovered;
    ui.visuals_mut().widgets.active.weak_bg_fill = old_active;

    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::NotAllowed })
}

/// Pulsante distruttivo condiviso: sfondo pieno rosso e testo scuro/chiaro in
/// base alla luminanza, così etichette lunghe e tradotte restano leggibili.
pub fn danger_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let resp = ui.add_enabled(
        enabled,
        egui::Button::new(
            egui::RichText::new(text)
                .strong()
                .color(on_fill_text(BAD)),
        )
        .fill(BAD)
        .rounding(if compact_pages() { 7.0 } else { 8.0 }),
    );
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::NotAllowed })
}

/// Intestazione di tabella con colore di testo adattato al tema. Nessun frame:
/// viene disegnata direttamente nella cella dell'header di `TableBuilder`.
pub fn table_head(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).strong().color(text_primary()));
}

/// Pulsante primario compatto, usato solo nella Dashboard: stessa palette, meno padding e testo 16px.
pub fn primary_button_compact(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let old_padding = ui.spacing().button_padding;
    let old_inactive = ui.visuals().widgets.inactive.weak_bg_fill;
    let old_hovered = ui.visuals().widgets.hovered.weak_bg_fill;
    let old_active = ui.visuals().widgets.active.weak_bg_fill;

    ui.spacing_mut().button_padding = vec2(12.0, 6.0);
    ui.visuals_mut().widgets.inactive.weak_bg_fill = if enabled { PRIMARY_BG } else { alpha(PRIMARY_BG, 110) };
    ui.visuals_mut().widgets.hovered.weak_bg_fill = PRIMARY_BG_HOVER;
    ui.visuals_mut().widgets.active.weak_bg_fill = PRIMARY_BG_HOVER;

    let resp = ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(text).size(16.0).strong().color(Color32::WHITE))
            .stroke(Stroke::NONE)
            .rounding(7.0),
    );

    ui.spacing_mut().button_padding = old_padding;
    ui.visuals_mut().widgets.inactive.weak_bg_fill = old_inactive;
    ui.visuals_mut().widgets.hovered.weak_bg_fill = old_hovered;
    ui.visuals_mut().widgets.active.weak_bg_fill = old_active;

    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::NotAllowed })
}

/// Luminanza percepita di un colore pieno (0..255).
fn luma(c: Color32) -> f32 {
    let [r, g, b, _] = c.to_array();
    0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
}

/// Testo ad alto contrasto sopra uno sfondo pieno: verde acqua, giallo e verde
/// brillanti ricevono testo scuro, i colori scuri ricevono testo bianco.
/// Usato da pillole e pulsanti di tutte le schede.
pub fn on_fill_text(fill: Color32) -> Color32 {
    if luma(fill) > 125.0 {
        Color32::from_rgb(15, 23, 42)
    } else {
        Color32::WHITE
    }
}

/// Colore testo ad alto contrasto per una barra con riempimento `fill`.
/// La soglia storica delle barre resta invariata (la Dashboard non cambia).
pub fn progress_text_color(fill: Color32) -> Color32 {
    if luma(fill) > 140.0 {
        Color32::from_rgb(20, 24, 32)
    } else {
        Color32::WHITE
    }
}

/// Wrapper leggero attorno a `egui::ProgressBar` che forza testo ad alto contrasto.
pub struct Bar {
    frac: f32,
    fill: Color32,
    text: Option<egui::WidgetText>,
    desired_height: f32,
    desired_width: Option<f32>,
    rounding: Rounding,
    animate: bool,
    show_percentage: bool,
}

impl Bar {
    pub fn fill(mut self, fill: Color32) -> Self {
        self.fill = fill;
        self
    }
    pub fn text(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.text = Some(text.into());
        self
    }
    pub fn desired_height(mut self, height: f32) -> Self {
        self.desired_height = height;
        self
    }
    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }
    pub fn rounding(mut self, rounding: impl Into<Rounding>) -> Self {
        self.rounding = rounding.into();
        self
    }
    pub fn animate(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }
    pub fn show_percentage(mut self) -> Self {
        self.show_percentage = true;
        self
    }
}

impl egui::Widget for Bar {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let frac = self.frac.clamp(0.0, 1.0);
        let text = if self.show_percentage && self.text.is_none() {
            Some(egui::WidgetText::from(format!("{:.0}%", frac * 100.0)))
        } else {
            self.text
        };
        let mut bar = egui::ProgressBar::new(frac)
            .fill(self.fill)
            .desired_height(self.desired_height)
            .rounding(self.rounding)
            .animate(self.animate);
        if let Some(text) = text {
            bar = bar.text(text);
        }
        if let Some(width) = self.desired_width {
            bar = bar.desired_width(width);
        }
        // Il testo è centrato sull'intera barra: se il riempimento è chiaro ma copre
        // poco spazio, il testo scuro sparirebbe sullo sfondo scuro del tema.
        // Usiamo il contrasto contro il riempimento solo quando copre abbastanza.
        let fill_contrast = progress_text_color(self.fill);
        let text_color = if frac >= 0.45 {
            fill_contrast
        } else if is_dark() {
            Color32::WHITE
        } else {
            TEXT_LIGHT
        };
        ui.scope(|ui| {
            ui.visuals_mut().override_text_color = Some(text_color);
            ui.visuals_mut().selection.stroke.color = text_color;
            ui.add(bar)
        })
        .inner
    }
}

pub fn bar(frac: f32) -> Bar {
    Bar {
        frac,
        fill: BAR_BLUE,
        text: None,
        desired_height: if compact_pages() { 20.0 } else { 26.0 },
        desired_width: None,
        rounding: Rounding::same(7.0),
        animate: false,
        show_percentage: false,
    }
}

/// Barra di avanzamento dei job: verde acqua, alta e animata.
pub fn progress_bar(frac: f32) -> Bar {
    Bar {
        frac,
        fill: ACCENT,
        text: None,
        desired_height: if compact_pages() { 15.0 } else { 18.0 },
        desired_width: None,
        rounding: Rounding::same(8.0),
        animate: false,
        show_percentage: false,
    }
}

pub fn heat_color(celsius: f32) -> Color32 {
    if celsius < 65.0 {
        BAR_BLUE
    } else if celsius < 80.0 {
        Color32::from_rgb(214, 120, 0)
    } else {
        Color32::from_rgb(200, 40, 40)
    }
}

pub fn level_color(f: f32) -> Color32 {
    if f < 0.6 {
        GOOD
    } else if f < 0.85 {
        WARN
    } else {
        BAD
    }
}

fn page_color(p: Page) -> Color32 {
    let (r, g, b) = match p {
        Page::Dashboard => (0, 168, 150),
        Page::Cleaner => (46, 170, 100),
        Page::SafeOptimize => (50, 180, 140),
        Page::Fixer => (232, 140, 40),
        Page::Info => (60, 130, 230),
        Page::Disk => (140, 100, 220),
        Page::Procs => (60, 160, 200),
        Page::Apps => (200, 100, 160),
        Page::Zram => (226, 178, 40),
        Page::Temps => (225, 90, 70),
        Page::Network => (50, 150, 220),
        Page::Kernel => (110, 124, 150),
        Page::Repos => (176, 120, 60),
        Page::Startup => (80, 180, 120),
        Page::Perms => (150, 100, 200),
        Page::Firewall => (205, 60, 70),
        Page::Backup => (60, 140, 200),
        Page::Shredder => (190, 70, 70),
        Page::Log => (110, 124, 150),
        Page::Settings => (120, 130, 150),
        Page::Guide => (0, 150, 200),
        Page::Updates => (30, 160, 120),
        Page::Containers => (40, 140, 200),
        Page::Timers => (140, 100, 200),
        Page::Synaptic => (100, 160, 220),
        Page::About => (100, 160, 220),
        Page::History => (80, 140, 200),
        Page::AdvancedCleaner => (100, 200, 150),
        Page::Duplicates => (100, 180, 220),
        Page::Quarantine => (90, 170, 190),
        Page::RestoreHistory => (70, 160, 180),
        Page::Services => (120, 180, 240),
        Page::Snaps => (120, 180, 240),
        Page::Report => (0, 168, 150),
        Page::DiskAnalyzer => (53, 162, 159),
        Page::DiskTreemap => (37, 150, 190),
        Page::Boot => (53, 162, 159),
        Page::AppImages => (114, 9, 183),
        Page::MenuEditor => (140, 100, 220),
        Page::SecurityHardening => (50, 150, 220),
        Page::HardwareHealth => (70, 180, 120),
        Page::AppLifecycle => (220, 80, 100),
        Page::Themes => (180, 100, 220),
        Page::SystemEditor => (53, 162, 159),
        Page::TelemetryGuard => (46, 139, 87),
        Page::Mounts => (50, 150, 220),
        Page::DevContainers => (40, 150, 180),
        Page::KernelBoot => (70, 140, 190),
    };
    Color32::from_rgb(r, g, b)
}

pub fn page_icon(p: &egui::Painter, c: Pos2, size: f32, page: Page) {
    let tile = Rect::from_center_size(c, vec2(size, size));
    let tint = page_color(page);
    // Badge quadrato arrotondato, sfondo al 15% dell'accento: nessun ritaglio.
    p.rect_filled(tile, 8.0, alpha(tint, 38));
    let s = size * 0.28;
    let dark = p.ctx().style().visuals.dark_mode;
    let w = if dark { mix(tint, Color32::WHITE, 0.75) } else { mix(tint, TEXT_LIGHT, 0.35) };
    let st = Stroke::new((size * 0.075).max(1.5), w);
    let th = Stroke::new((size * 0.055).max(1.2), w);
    let pt = |x: f32, y: f32| Pos2::new(c.x + x * s, c.y + y * s);
    let line = |a: Pos2, b: Pos2, stroke: Stroke| {
        p.line_segment([a, b], stroke);
    };
    let poly = |pts: Vec<Pos2>, stroke: Stroke| {
        p.add(Shape::line(pts, stroke));
    };
    let rr = |a: Pos2, b: Pos2| {
        p.rect_stroke(Rect::from_min_max(a, b), 2.0, st);
    };
    match page {
        Page::Dashboard => {
            for (i, h) in [0.5f32, 1.0, 0.75].iter().enumerate() {
                let x = -0.7 + i as f32 * 0.7;
                p.rect_filled(Rect::from_min_max(pt(x - 0.22, 1.0 - 2.0 * h), pt(x + 0.22, 1.0)), 1.5, w);
            }
        }
        Page::Cleaner => {
            line(pt(0.0, -1.0), pt(0.0, 1.0), st);
            line(pt(-1.0, 0.0), pt(1.0, 0.0), st);
            line(pt(-0.55, -0.55), pt(0.55, 0.55), th);
            line(pt(-0.55, 0.55), pt(0.55, -0.55), th);
        }
        Page::SafeOptimize => {
            p.circle_stroke(c, s, st);
            line(pt(0.0, -0.55), pt(0.0, 0.55), st);
            line(pt(-0.55, 0.0), pt(0.55, 0.0), st);
        }
        Page::Fixer => {
            line(pt(-0.8, 0.8), pt(0.2, -0.2), Stroke::new((size * 0.12).max(2.0), w));
            p.circle_stroke(pt(0.45, -0.45), s * 0.5, st);
        }
        Page::Info => {
            rr(pt(-1.0, -0.8), pt(1.0, 0.4));
            line(pt(0.0, 0.4), pt(0.0, 0.85), st);
            line(pt(-0.5, 0.85), pt(0.5, 0.85), st);
        }
        Page::Disk => {
            p.circle_stroke(c, s, st);
            p.circle_filled(c, s * 0.25, w);
        }
        Page::DiskAnalyzer => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "📊", FontId::proportional(size * 0.55), w);
        }
        Page::DiskTreemap => {
            p.rect_stroke(Rect::from_min_max(pt(-1.0, -0.95), pt(0.15, 0.1)), 2.0, st);
            p.rect_stroke(Rect::from_min_max(pt(0.28, -0.95), pt(1.0, -0.25)), 2.0, st);
            p.rect_stroke(Rect::from_min_max(pt(-1.0, 0.23), pt(-0.05, 1.0)), 2.0, st);
            p.rect_stroke(Rect::from_min_max(pt(0.08, 0.23), pt(1.0, 1.0)), 2.0, st);
        }
        Page::SecurityHardening => {
            // Disegna qui l'icona personalizzata per il modulo di sicurezza (es. uno scudo o linee guida)
            // Oppure lascia temporaneamente todo!() se vuoi implementare il disegno grafico in un secondo momento
            line(pt(0.0, -0.6), pt(0.6, 0.6), st);
            line(pt(0.0, -0.6), pt(-0.6, 0.6), st);
        }
        Page::Boot => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "⏱", FontId::proportional(size * 0.55), w);
        }
        Page::Procs => {
            p.circle_stroke(c, s * 0.55, st);
            for i in 0..8 {
                let a = i as f32 * TAU / 8.0;
                line(pt(0.75 * a.cos(), 0.75 * a.sin()), pt(a.cos(), a.sin()), st);
            }
        }
        Page::Apps => {
            rr(pt(-0.9, -0.4), pt(0.9, 0.95));
            rr(pt(-1.0, -0.95), pt(1.0, -0.4));
        }
        Page::Zram => {
            rr(pt(-0.8, -0.5), pt(0.8, 0.5));
            for i in 0..4 {
                let x = -0.55 + i as f32 * 0.37;
                line(pt(x, -0.5), pt(x, -0.95), th);
                line(pt(x, 0.5), pt(x, 0.95), th);
            }
        }
        Page::Temps => {
            line(pt(0.0, -0.95), pt(0.0, 0.3), Stroke::new((size * 0.11).max(2.0), w));
            p.circle_filled(pt(0.0, 0.6), s * 0.5, w);
        }
        Page::Network => {
            p.circle_stroke(c, s, st);
            line(pt(-1.0, 0.0), pt(1.0, 0.0), th);
            line(pt(0.0, -1.0), pt(0.0, 1.0), th);
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = i as f32 / 24.0 * TAU;
                    pt(0.5 * a.cos(), a.sin())
                })
                .collect();
            poly(pts, th);
        }
        Page::Snaps => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "📦", FontId::proportional(size * 0.55), w);
        }
        Page::Report => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "📋", FontId::proportional(size * 0.55), w);
        }
        Page::Kernel => {
            poly(vec![pt(-0.9, -0.5), pt(-0.2, 0.0), pt(-0.9, 0.5)], st);
            line(pt(0.1, 0.6), pt(0.9, 0.6), st);
        }
        Page::Containers | Page::Timers | Page::Synaptic | Page::History | Page::AdvancedCleaner => {
            p.circle_stroke(c, s, st);
        }
        Page::Repos => {
            for y in [-0.85f32, -0.2, 0.45] {
                rr(pt(-0.9, y), pt(0.9, y + 0.45));
            }
        }
        Page::Startup => {
            p.add(Shape::convex_polygon(
                vec![pt(-0.6, -0.9), pt(0.9, 0.0), pt(-0.6, 0.9)],
                w,
                Stroke::NONE,
            ));
        }
        Page::Perms => {
            p.circle_stroke(pt(0.0, -0.45), s * 0.45, st);
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = PI + i as f32 / 16.0 * PI;
                    pt(0.9 * a.cos(), 0.95 + 0.9 * a.sin())
                })
                .collect();
            poly(pts, st);
        }
        Page::Firewall => {
            p.add(Shape::closed_line(
                vec![pt(-0.8, -0.8), pt(0.8, -0.8), pt(0.8, 0.1), pt(0.0, 0.95), pt(-0.8, 0.1)],
                st,
            ));
        }
        Page::Backup => {
            line(pt(0.0, -0.9), pt(0.0, 0.3), st);
            poly(vec![pt(-0.5, -0.2), pt(0.0, 0.35), pt(0.5, -0.2)], st);
            poly(vec![pt(-0.9, 0.4), pt(-0.9, 0.9), pt(0.9, 0.9), pt(0.9, 0.4)], st);
        }
        Page::Shredder => {
            line(pt(-0.9, -0.6), pt(0.9, -0.6), st);
            poly(vec![pt(-0.3, -0.6), pt(-0.3, -0.9), pt(0.3, -0.9), pt(0.3, -0.6)], th);
            rr(pt(-0.7, -0.6), pt(0.7, 0.95));
            line(pt(-0.25, -0.2), pt(-0.25, 0.6), th);
            line(pt(0.25, -0.2), pt(0.25, 0.6), th);
        }
        Page::Log => {
            rr(pt(-0.7, -0.95), pt(0.7, 0.95));
            for y in [-0.4f32, 0.0, 0.4] {
                line(pt(-0.4, y), pt(0.4, y), th);
            }
        }
        Page::Settings => {
            for (y, k) in [(-0.6f32, -0.3f32), (0.0, 0.4), (0.6, -0.1)] {
                line(pt(-0.9, y), pt(0.9, y), th);
                p.circle_filled(pt(k, y), s * 0.22, w);
            }
        }
        Page::Updates => {
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = 0.3 * PI + i as f32 / 20.0 * 1.4 * PI;
                    pt(0.9 * a.cos(), 0.9 * a.sin())
                })
                .collect();
            poly(pts, st);
            p.add(Shape::convex_polygon(vec![pt(0.894, -0.465), pt(0.736, -1.013), pt(0.324, -0.447)], w, Stroke::NONE));
        }
        Page::Guide => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "?", FontId::proportional(size * 0.55), w);
        }
        Page::About => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "i", FontId::proportional(size * 0.55), w);
        }
        Page::Duplicates => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "📁", FontId::proportional(size * 0.55), w);
        }
        Page::RestoreHistory | Page::Quarantine => {
            // Freccia di ripristino: cerchio aperto con punta in alto.
            let pts: Vec<Pos2> = (1..=20)
                .map(|i| {
                    let a = -PI * 0.65 + i as f32 / 20.0 * PI * 1.5;
                    pt(0.85 * a.cos(), 0.85 * a.sin())
                })
                .collect();
            poly(pts, st);
            line(pt(-0.25, -0.85), pt(0.2, -0.95), st);
            line(pt(0.2, -0.95), pt(0.55, -0.6), st);
        }
        Page::Services => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "⚙", FontId::proportional(size * 0.55), w);
        }
        Page::AppImages => {
          p.circle_stroke(c, s, st);
          p.text(c, Align2::CENTER_CENTER, "📦", FontId::proportional(size * 0.55), w);
        }
        Page::MenuEditor => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "📋", FontId::proportional(size * 0.55), w);
        }
        Page::HardwareHealth => { 
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "🔋", FontId::proportional(size * 0.55), w);
        }
        Page::AppLifecycle => { 
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "✂", FontId::proportional(size * 0.55), w);
        }
        Page::Themes => {
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "🎨", FontId::proportional(size * 0.55), w);
        }
        Page::SystemEditor => { 
            p.circle_stroke(c, s, st);
            p.text(c, Align2::CENTER_CENTER, "🛠", FontId::proportional(size * 0.55), w);
        }
        Page::TelemetryGuard => {
         p.circle_stroke(c, s, st);
         p.text(c, Align2::CENTER_CENTER, "🛡", FontId::proportional(size * 0.55), w);
        }
        Page::Mounts => {
            rr(pt(-0.9, -0.55), pt(0.9, 0.55));
            line(pt(-0.55, 0.2), pt(0.55, 0.2), th);
        }
        Page::DevContainers => {
            // Contenitore con scomparti: stessa icona per sidebar e tile.
            rr(pt(-0.95, -0.65), pt(0.95, 0.65));
            line(pt(-0.95, -0.2), pt(0.95, -0.2), th);
            line(pt(0.0, -0.65), pt(0.0, 0.65), th);
        }
        Page::KernelBoot => {
            // Stivale di kernel: tre livelli sovrapposti.
            rr(pt(-0.95, -0.75), pt(0.95, -0.15));
            rr(pt(-0.95, -0.15), pt(0.95, 0.35));
            rr(pt(-0.95, 0.35), pt(0.95, 0.85));
        }
    }
}

/// Voce della sidebar. `name` è l'etichetta corta disegnata, `full_name` il nome
/// completo mostrato nel tooltip insieme al livello di rischio.
pub fn sidebar_item(
    ui: &mut egui::Ui,
    page: Page,
    name: &str,
    full_name: &str,
    selected: bool,
    enabled: bool,
) -> bool {
    // Geometria del pallino: il testo può occupare solo lo spazio che lo precede.
    const DOT_INSET: f32 = 15.0;
    const DOT_RADIUS: f32 = 4.5;
    const DOT_CLEARANCE: f32 = 20.0;
    const TEXT_LEFT: f32 = 54.0;

    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), sense);
    let dark_mode = ui.visuals().dark_mode;
    let risk = page_risk(page);
    let resp = if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp.on_hover_cursor(egui::CursorIcon::Wait)
    };
    let tooltip = if enabled {
        format!("{}\n{}: {}", full_name, t("Livello di rischio"), risk.label())
    } else {
        format!(
            "{}\n{}: {}\n{}",
            full_name,
            t("Livello di rischio"),
            risk.label(),
            t("Menu bloccato: attendi il completamento dell'operazione.")
        )
    };
    let resp = resp.on_hover_text(tooltip);

    let p = ui.painter();
    if selected {
        let sel_bg = if dark_mode { Color32::from_rgb(34, 54, 66) } else { Color32::from_rgb(209, 224, 234) };
        p.rect_filled(rect, 10.0, sel_bg);
    } else if enabled && resp.hovered() {
        let hover_bg = if dark_mode { Color32::from_white_alpha(16) } else { Color32::from_black_alpha(14) };
        p.rect_filled(rect, 10.0, hover_bg);
    }
    // Con la sidebar a "binario" (finestre strette) restano solo icona e
    // tooltip: il nome completo continua a essere leggibile passando il mouse.
    let compact = rect.width() < 118.0;
    let icon_x = if compact { rect.center().x } else { rect.left() + 30.0 };
    let icon = Rect::from_center_size(Pos2::new(icon_x, rect.center().y), vec2(30.0, 30.0));
    page_icon(p, icon.center(), 27.0, page);
    let text_color = if !enabled {
        alpha(text_primary(), 110)
    } else if dark_mode {
        if selected { Color32::WHITE } else { text_primary() }
    } else {
        text_primary()
    };

    if !compact {
        // Elide il testo prima del pallino: nessuna etichetta, anche molto lunga
        // o con finestra stretta, può sovrapporsi al indicatore di rischio.
        let text_left = rect.left() + TEXT_LEFT;
        let dot_center = Pos2::new(rect.right() - DOT_INSET, rect.center().y);
        let text_right = dot_center.x - DOT_RADIUS - DOT_CLEARANCE;
        let max_width = (text_right - text_left).max(0.0);
        let mut job = egui::text::LayoutJob::single_section(
            name.to_string(),
            egui::text::TextFormat {
                font_id: FontId::proportional(if selected { 17.5 } else { 16.5 }),
                color: text_color,
                ..Default::default()
            },
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
        job.break_on_newline = false;
        let galley = p.layout_job(job);
        p.galley(
            Pos2::new(text_left, rect.center().y - galley.size().y / 2.0),
            galley,
            text_color,
        );

        // Il pallino del livello di rischio è sempre visibile, non solo sulla voce attiva.
        p.circle_filled(dot_center, DOT_RADIUS, risk.color());
        p.circle_stroke(
            dot_center,
            DOT_RADIUS,
            Stroke::new(1.0_f32, alpha(if dark_mode { Color32::WHITE } else { Color32::BLACK }, 90)),
        );
    }
    enabled && resp.clicked()
}

pub fn tile(ui: &mut egui::Ui, page: Page, title: &str, sub: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(290.0, 104.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    let dark_mode = ui.visuals().dark_mode;
    let tint = page_color(page);
    let fill = if resp.hovered() {
        if dark_mode { Color32::from_rgb(45, 57, 74) } else { Color32::from_rgb(230, 238, 246) }
    } else {
        card_bg(ui.ctx())
    };
    p.rect_filled(rect, 10.0, fill);
    let stroke_color = if resp.hovered() { alpha(tint, 150) } else { card_border(ui.ctx()) };
    p.rect_stroke(rect, 10.0, Stroke::new(1.0_f32, stroke_color));
    // icona vettoriale colorata (page_icon disegna già il proprio riquadro)
    page_icon(p, Pos2::new(rect.left() + 52.0, rect.center().y), 56.0, page);
    let title_color = text_primary();
    p.text(
        Pos2::new(rect.left() + 96.0, rect.center().y - 14.0),
        Align2::LEFT_CENTER,
        tr(title),
        FontId::proportional(20.0),
        title_color,
    );
    p.text(
        Pos2::new(rect.left() + 96.0, rect.center().y + 13.0),
        Align2::LEFT_CENTER,
        tr(sub),
        FontId::proportional(14.0),
        text_dim(),
    );
    // freccia di navigazione
    p.text(
        Pos2::new(rect.right() - 16.0, rect.center().y),
        Align2::CENTER_CENTER,
        "›",
        FontId::proportional(24.0),
        if resp.hovered() { tint } else { text_dim() },
    );
    resp.clicked()
}

/// Tessera compatta per "Accessi rapidi": titolo e sottotitolo su due righe, tooltip esteso.
pub fn tile_compact(ui: &mut egui::Ui, page: Page, title: &str, sub: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(168.0, 52.0), Sense::click());
    let sub_text = tr(sub);
    let resp = resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(sub_text.clone());
    let p = ui.painter();
    let dark_mode = ui.visuals().dark_mode;
    let tint = page_color(page);
    let fill = if resp.hovered() {
        if dark_mode { Color32::from_rgb(45, 57, 74) } else { Color32::from_rgb(230, 238, 246) }
    } else {
        card_bg(ui.ctx())
    };
    p.rect_filled(rect, 8.0, fill);
    let stroke_color = if resp.hovered() { alpha(tint, 150) } else { card_border(ui.ctx()) };
    p.rect_stroke(rect, 8.0, Stroke::new(1.0_f32, stroke_color));
    page_icon(p, Pos2::new(rect.left() + 23.0, rect.center().y), 32.0, page);
    p.text(
        Pos2::new(rect.left() + 42.0, rect.center().y - 8.0),
        Align2::LEFT_CENTER,
        tr(title),
        FontId::proportional(15.0),
        text_primary(),
    );
    let short_sub = if sub_text.chars().count() > 22 {
        format!("{}…", sub_text.chars().take(21).collect::<String>())
    } else {
        sub_text
    };
    p.text(
        Pos2::new(rect.left() + 42.0, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        short_sub,
        FontId::proportional(11.0),
        text_dim(),
    );
    p.text(
        Pos2::new(rect.right() - 9.0, rect.center().y),
        Align2::CENTER_CENTER,
        "›",
        FontId::proportional(19.0),
        if resp.hovered() { tint } else { text_dim() },
    );
    resp.clicked()
}

pub fn dash_tile(ui: &mut egui::Ui, page: Page, title: &str, width: f32, height: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(width.max(150.0), height.max(120.0)), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    let dark_mode = ui.visuals().dark_mode;

    let tint = page_color(page);
    let fill = if resp.hovered() {
        if dark_mode { Color32::from_rgb(48, 60, 80) } else { Color32::from_rgb(228, 234, 242) }
    } else {
        card_bg(ui.ctx())
    };
    p.rect_filled(rect, 10.0, fill);

    let stroke_color = if resp.hovered() { alpha(tint, 170) } else { card_border(ui.ctx()) };
    p.rect_stroke(rect, 10.0, Stroke::new(1.0_f32, stroke_color));

    let title_color = text_primary();
    p.text(
        Pos2::new(rect.center().x, rect.top() + 26.0),
        Align2::CENTER_CENTER,
        tr(title),
        FontId::proportional(17.0),
        title_color,
    );

    let icon_center = Pos2::new(rect.center().x, rect.top() + height * 0.58);
    let ring_r = (height * 0.25).clamp(26.0, 40.0);
    page_icon(p, icon_center, ring_r * 1.15, page);
    resp.clicked()
}

pub fn cpu_dial(ui: &mut egui::Ui, frac: f32, label: &str) {
    let frac = frac.clamp(0.0, 1.0);
    let size = 132.0;
    let (rect, _) = ui.allocate_exact_size(vec2(size + 16.0, size + 12.0), Sense::hover());
    let p = ui.painter_at(rect);
    let c = rect.center();
    let r = size * 0.42;
    let start = 0.75 * PI;
    let sweep = 1.5 * PI;

    let bg_ring = if ui.visuals().dark_mode { Color32::from_white_alpha(26) } else { Color32::from_black_alpha(22) };
    p.add(Shape::line(arc(c, r, start, start + sweep), Stroke::new(11.0_f32, bg_ring)));
    if frac > 0.005 {
        let pts = arc(c, r, start, start + sweep * frac);
        let col = level_color(frac);
        p.add(Shape::line(pts.clone(), Stroke::new(16.0_f32, alpha(col, 18))));
        let segs = pts.len().saturating_sub(1);
        for i in 0..segs {
            let t = i as f32 / segs.max(1) as f32;
            p.line_segment([pts[i], pts[i + 1]], Stroke::new(11.0_f32, mix(ACCENT, col, t)));
        }
    }

    let text_col = text_primary();
    p.text(
        Pos2::new(c.x, c.y - 6.0),
        Align2::CENTER_CENTER,
        format!("{:.0}%", frac * 100.0),
        FontId::proportional(24.0),
        text_col,
    );
    p.text(
        Pos2::new(c.x, c.y + 18.0),
        Align2::CENTER_CENTER,
        tr(label),
        FontId::proportional(13.0),
        text_dim(),
    );
}

fn arc(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    let n = 48;
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

pub fn gauge(ui: &mut egui::Ui, frac: f32, title: &str, sub: &str) {
    let frac = frac.clamp(0.0, 1.0);
    let size = 160.0;
    let (rect, _) = ui.allocate_exact_size(vec2(size + 24.0, size + 52.0), Sense::hover());
    let p = ui.painter_at(rect);
    let c = Pos2::new(rect.center().x, rect.top() + size / 2.0 + 6.0);
    let r = size * 0.40;
    let start = 0.75 * PI;
    let sweep = 1.5 * PI;
    let dark = ui.visuals().dark_mode;
    p.add(Shape::line(arc(c, r, start, start + sweep), Stroke::new(13.0_f32, if dark { Color32::from_white_alpha(22) } else { Color32::from_black_alpha(18) })));
    if frac > 0.005 {
        let pts = arc(c, r, start, start + sweep * frac);
        let col = level_color(frac);
        p.add(Shape::line(pts.clone(), Stroke::new(18.0_f32, alpha(col, 18))));
        let segs = pts.len().saturating_sub(1);
        for i in 0..segs {
            let t = i as f32 / segs.max(1) as f32;
            p.line_segment([pts[i], pts[i + 1]], Stroke::new(12.0_f32, mix(ACCENT, col, t)));
        }
        if let Some(&end) = pts.last() {
            p.circle_filled(end, 5.5, col);
            p.circle_filled(end, 2.5, Color32::WHITE);
        }
    }
    let text_col = text_primary();
    p.text(c, Align2::CENTER_CENTER, format!("{:.0}%", frac * 100.0), FontId::proportional(31.0), text_col);
    p.text(Pos2::new(c.x, c.y + r + 26.0), Align2::CENTER_CENTER, tr(title), FontId::proportional(18.0), text_col);
    p.text(Pos2::new(c.x, c.y + r + 46.0), Align2::CENTER_CENTER, sub, FontId::proportional(14.0), text_dim());
}

/// Gauge compatto per la Dashboard: quadrante 54px, dettagli in tooltip.
pub fn gauge_compact(ui: &mut egui::Ui, frac: f32, title: &str, sub: &str) {
    let frac = frac.clamp(0.0, 1.0);
    let size = 54.0_f32;
    let (rect, resp) = ui.allocate_exact_size(vec2(size + 6.0, size + 26.0), Sense::hover());
    let _resp = resp.on_hover_text(sub);
    let p = ui.painter_at(rect);
    let c = Pos2::new(rect.center().x, rect.top() + size / 2.0 + 1.0);
    let r = size * 0.40;
    let start = 0.75 * PI;
    let sweep = 1.5 * PI;
    let dark = ui.visuals().dark_mode;
    p.add(Shape::line(arc(c, r, start, start + sweep), Stroke::new(7.0_f32, if dark { Color32::from_white_alpha(22) } else { Color32::from_black_alpha(18) })));
    if frac > 0.005 {
        let pts = arc(c, r, start, start + sweep * frac);
        let col = level_color(frac);
        p.add(Shape::line(pts.clone(), Stroke::new(10.0_f32, alpha(col, 18))));
        let segs = pts.len().saturating_sub(1);
        for i in 0..segs {
            let t = i as f32 / segs.max(1) as f32;
            p.line_segment([pts[i], pts[i + 1]], Stroke::new(7.0_f32, mix(ACCENT, col, t)));
        }
        if let Some(&end) = pts.last() {
            p.circle_filled(end, 3.5, col);
        }
    }
    let text_col = text_primary();
    p.text(c, Align2::CENTER_CENTER, format!("{:.0}%", frac * 100.0), FontId::proportional(16.0), text_col);
    p.text(
        Pos2::new(c.x, rect.bottom() - 10.0),
        Align2::CENTER_CENTER,
        tr(title),
        FontId::proportional(11.0),
        text_col,
    );
}

pub fn sparkline(ui: &mut egui::Ui, data: &VecDeque<f32>, color: Color32, height: f32) {
    let w = ui.available_width().max(100.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    let p = ui.painter_at(rect);
    let dark_mode = ui.visuals().dark_mode;
    let bg = if dark_mode { Color32::from_rgb(20, 27, 38) } else { Color32::from_rgb(236, 241, 247) };
    p.rect_filled(rect, 10.0, bg);
    p.rect_stroke(rect, 10.0, Stroke::new(1.0_f32, if dark_mode { Color32::from_white_alpha(14) } else { Color32::from_black_alpha(16) }));
    let n = data.len().min(HIST_LEN);
    if n < 2 {
        return;
    }
    let skip = data.len() - n;
    let pts: Vec<Pos2> = data
        .iter()
        .skip(skip)
        .enumerate()
        .map(|(i, v)| {
            let x = rect.left() + rect.width() * (i + HIST_LEN - n) as f32 / (HIST_LEN - 1) as f32;
            Pos2::new(x, rect.bottom() - 5.0 - (rect.height() - 12.0) * v.clamp(0.0, 1.0))
        })
        .collect();
    // area sfumata sotto la curva
    let base = rect.bottom() - 5.0;
    let mut mesh = Mesh::with_texture(TextureId::Managed(0));
    for i in 0..pts.len().saturating_sub(1) {
        let idx = mesh.vertices.len() as u32;
        mesh.vertices.push(Vertex { pos: pts[i], uv: WHITE_UV, color: alpha(color, 45) });
        mesh.vertices.push(Vertex { pos: Pos2::new(pts[i].x, base), uv: WHITE_UV, color: alpha(color, 2) });
        mesh.vertices.push(Vertex { pos: pts[i + 1], uv: WHITE_UV, color: alpha(color, 45) });
        mesh.vertices.push(Vertex { pos: Pos2::new(pts[i + 1].x, base), uv: WHITE_UV, color: alpha(color, 2) });
        mesh.indices.extend_from_slice(&[idx, idx + 1, idx + 2, idx + 2, idx + 1, idx + 3]);
    }
    p.add(Shape::mesh(mesh));
    p.add(Shape::line(pts.clone(), Stroke::new(5.0_f32, alpha(color, 18))));
    p.add(Shape::line(pts, Stroke::new(2.5_f32, color)));
}

pub fn led(ui: &mut egui::Ui, state: Option<bool>, text: &str) {
    let col = match state {
        Some(true) => GOOD,
        Some(false) => BAD,
        None => Color32::GRAY,
    };
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 10.0, alpha(col, 45));
        ui.painter().circle_filled(r.center(), 6.0, col);
        ui.label(egui::RichText::new(text).strong());
    });
}

pub fn output_box(ui: &mut egui::Ui, text: &str, height: f32) {
    let dark_mode = ui.visuals().dark_mode;
    let bg = if dark_mode { Color32::from_rgb(17, 22, 30) } else { Color32::from_rgb(235, 240, 245) };
    let compact = compact_pages();
    let height = if compact { (height * 0.72).max(110.0) } else { height };
    egui::Frame::default()
        .fill(bg)
        .inner_margin(egui::Margin::same(if compact { 8.0 } else { 10.0 }))
        .rounding(8.0)
        .show(ui, |ui| {
            egui::ScrollArea::both().max_height(height).auto_shrink([false, true]).show(ui, |ui| {
                let shown = if text.trim().is_empty() { t("(nessun output)") } else { text };
                ui.label(egui::RichText::new(shown).monospace());
            });
        });
}

pub fn output_box_follow(ui: &mut egui::Ui, text: &str, height: f32) {
    let dark_mode = ui.visuals().dark_mode;
    let bg = if dark_mode { Color32::from_rgb(17, 22, 30) } else { Color32::from_rgb(235, 240, 245) };
    let compact = compact_pages();
    let height = if compact { (height * 0.72).max(110.0) } else { height };
    egui::Frame::default()
        .fill(bg)
        .inner_margin(egui::Margin::same(if compact { 8.0 } else { 10.0 }))
        .rounding(8.0)
        .show(ui, |ui| {
            egui::ScrollArea::both().max_height(height).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                let shown = if text.trim().is_empty() { t("(nessun output)") } else { text };
                ui.label(egui::RichText::new(shown).monospace());
            });
        });
}

pub fn kv(ui: &mut egui::Ui, k: &str, v: &str) {
    let size = if compact_pages() { 15.0 } else { 16.0 };
    ui.label(egui::RichText::new(tr(k)).strong().size(size));
    ui.label(egui::RichText::new(v).size(size));
    ui.end_row();
}

/// Coppia chiave/valore compatta per il riquadro informativo della Dashboard.
pub fn kv_compact(ui: &mut egui::Ui, k: &str, v: &str) {
    ui.label(egui::RichText::new(tr(k)).strong().size(12.0));
    ui.label(egui::RichText::new(v).size(12.0));
    ui.end_row();
}
