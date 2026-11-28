//! Font dell'interfaccia per le lingue internazionali.
//!
//! I font inclusi in egui coprono sostanzialmente solo il latino: senza fallback
//! i caratteri cirillici, greci, arabi, devanagari e CJK verrebbero disegnati come
//! quadratini. Qui si chiedono a fontconfig i font di sistema per ogni scrittura e li
//! si aggiunge in coda alle famiglie `Proportional` e `Monospace`: egui prova i font
//! nell'ordine della lista e usa il primo font che contiene il glifo, quindi l'ordine
//! preserva emoji e font latini già presenti. Se fontconfig non è disponibile si
//! usano i percorsi Debian più comuni; una lingua senza font disponibile non blocca
//! l'avvio dell'applicazione.

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

/// Tag interno, query fontconfig, indice della faccia nei file `.ttc` (CJK).
const SCRIPTS: &[(&str, &str, u32)] = &[
    ("latin-ext", "sans-serif:lang=pl", 0),
    ("cyrillic", "sans-serif:lang=ru", 0),
    ("greek", "sans-serif:lang=el", 0),
    ("arabic", "sans-serif:lang=ar", 0),
    ("devanagari", "sans-serif:lang=hi", 0),
    ("cjk-sc", "sans-serif:lang=zh-cn", 2),
    ("cjk-jp", "sans-serif:lang=ja", 0),
    ("cjk-kr", "sans-serif:lang=ko", 1),
];

/// Percorsi usati quando `fc-match` non è installato o non trova nulla.
const GENERIC: &[&str] = &[
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
];
const ARABIC: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
    "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
];
const DEVANAGARI: &[&str] = &[
    "/usr/share/fonts/truetype/lohit-devanagari/Lohit-Devanagari.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansDevanagari-Regular.ttf",
];
const CJK_SC: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    "/usr/share/fonts/truetype/arphic/uming.ttc",
];
const CJK_JP: &[&str] = &[
    "/usr/share/fonts/truetype/vlgothic/VL-Gothic-Regular.ttf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
];
const CJK_KR: &[&str] = &[
    "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
];

fn first_existing(paths: &[&'static str]) -> Option<&'static str> {
    paths.iter().copied().find(|p| Path::new(p).is_file())
}

fn from_fontconfig(query: &str) -> Option<String> {
    let out = Command::new("fc-match")
        .arg("-f")
        .arg("%{file}")
        .arg(query)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() || !Path::new(&path).is_file() {
        return None;
    }
    Some(path)
}

/// Aggiunge un file di font a una famiglia, evitando duplicati con lo stesso indice.
fn add_font(
    fonts: &mut FontDefinitions,
    seen: &mut HashSet<(String, u32)>,
    name: &str,
    path: &str,
    index: u32,
) {
    if !seen.insert((path.to_string(), index)) {
        return;
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return,
    };
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    fonts.font_data.insert(name.to_string(), data);
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let key = family.clone();
        match fonts.families.get_mut(&key) {
            Some(list) => {
                if !list.iter().any(|n| n == name) {
                    list.push(name.to_string());
                }
            }
            None => {
                fonts.families.insert(key, vec![name.to_string()]);
            }
        }
    }
}

/// Installa i fallback UTF-8. Da chiamare una sola volta, prima del primo frame.
pub fn install(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    let mut seen = HashSet::new();
    for &(tag, query, ttc_index) in SCRIPTS {
        let path = from_fontconfig(query);
        let (path, index) = match path {
            Some(p) => {
                let index = if p.to_ascii_lowercase().ends_with(".ttc") {
                    ttc_index
                } else {
                    0
                };
                (p, index)
            }
            None => {
                let candidates = match tag {
                    "arabic" => ARABIC,
                    "devanagari" => DEVANAGARI,
                    "cjk-sc" => CJK_SC,
                    "cjk-jp" => CJK_JP,
                    "cjk-kr" => CJK_KR,
                    _ => GENERIC,
                };
                match first_existing(candidates) {
                    Some(p) => (p.to_string(), ttc_index),
                    None => continue,
                }
            }
        };
        add_font(
            &mut fonts,
            &mut seen,
            &format!("debutility-intl-{tag}-{index}"),
            &path,
            index,
        );
    }
    ctx.set_fonts(fonts);
}
