//! Traduzioni di DebUtility.
//!
//! Le stringhe originali sono in italiano e fanno da chiave. L'elenco autorevole delle
//! lingue sta in `lang/langs.tsv` (`codice<TAB>nome`; la prima riga è la lingua delle
//! chiavi). Le traduzioni stanno in `lang/strings.tsv` (incorporato nel binario): una
//! riga per chiave, colonne separate da TAB nello stesso ordine di `langs.tsv`:
//!
//!     italiano <TAB> English <TAB> Español <TAB> Français <TAB> Deutsch
//!
//! Per aggiungere una lingua basta una riga in `lang/langs.tsv` e una colonna in
//! `lang/strings.src`, poi rigenerare i dati con `tools/build_strings.py`: questo file
//! non va modificato. Le colonne non ancora tradotte ricadono sul testo originale UTF-8.
//!
//! Il locale di sistema (`LC_ALL`, `LC_MESSAGES`, `LANGUAGE`, `LANG`) viene normalizzato
//! in forma `ll` / `ll_CC` e risolto con fallback ordinato `ll_CC` → `ll` → lingua di
//! fallback (inglese, se tradotta; altrimenti la lingua originale).

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Registro delle lingue: unica fonte di verità, condivisa con gli script in `tools/`.
const LANG_DATA: &str = include_str!("../lang/langs.tsv");
/// Tabella generata da `tools/build_strings.py`, nello stesso ordine di `LANG_DATA`.
const DATA: &str = include_str!("../lang/strings.tsv");
/// Lingua usata quando nessun locale richiesto è disponibile.
const FALLBACK_CODE: &str = "en";

/// Alias storici dei codici lingua a due lettere (Java/Linux legacy).
fn alias_code(lang: &str) -> String {
    match lang {
        "iw" => "he".to_string(),
        "in" => "id".to_string(),
        "ji" => "yi".to_string(),
        _ => lang.to_string(),
    }
}

/// Normalizza un tag locale: minuscole per la lingua, maiuscole per regione/variante,
/// separatore `_`, ignorando charset (`.UTF-8`) e modifier (`@euro`). I pseudo-locale
/// POSIX `C` e `POSIX` sono canonizzati a `C` (non sono codici lingua BCP 47).
fn canonical_code(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches(|c: char| c == '"' || c == '\'');
    let tag = trimmed.split(['.', '@']).next().unwrap_or("").trim();
    if tag.is_empty() {
        return String::new();
    }
    if tag.eq_ignore_ascii_case("C") || tag.eq_ignore_ascii_case("POSIX") {
        return "C".to_string();
    }
    let mut parts = tag.split(['_', '-']).filter(|p| !p.is_empty());
    let mut out: Vec<String> = Vec::new();
    if let Some(lang) = parts.next() {
        out.push(alias_code(&lang.to_ascii_lowercase()));
    }
    for part in parts {
        if part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
            // Sottotaglio script BCP 47 (es. Hans, Latn).
            let mut s = part.to_ascii_lowercase();
            s[..1].make_ascii_uppercase();
            out.push(s);
        } else if part.len() <= 3 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
            out.push(part.to_ascii_uppercase());
        } else {
            out.push(part.to_ascii_lowercase());
        }
    }
    out.join("_")
}

/// Catene di fallback ordinate da un valore d'ambiente: `ll_CC.UTF-8` → `ll_CC` → `ll`.
/// Sono ammessi anche elenchi separati da `:` (stile `LANGUAGE`) e separatori `-`/`_`.
pub fn locale_chain(raw: &str) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    for item in raw.trim().split([':', ',', ';']) {
        let mut code = canonical_code(item);
        while !code.is_empty() {
            if !chain.contains(&code) {
                chain.push(code.clone());
            }
            match code.rfind('_') {
                Some(i) if i > 0 => code.truncate(i),
                _ => break,
            }
        }
    }
    chain
}

fn parse_langs() -> Vec<(&'static str, &'static str)> {
    let mut out: Vec<(&'static str, &'static str)> = Vec::new();
    for line in LANG_DATA.lines() {
        let line = line.trim_start_matches('\u{feff}');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let mut it = line.split('\t');
        let code = it.next().unwrap_or("").trim();
        if code.is_empty() {
            continue;
        }
        let name = it.next().unwrap_or(code).trim();
        let canon = canonical_code(code);
        if out.iter().any(|(c, _)| canonical_code(c) == canon) {
            continue;
        }
        out.push((code, if name.is_empty() { code } else { name }));
    }
    out
}

fn langs_cache() -> &'static Vec<(&'static str, &'static str)> {
    static L: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
    L.get_or_init(parse_langs)
}

/// Tutte le lingue disponibili come `(codice, nome mostrato)`, nell'ordine delle colonne.
pub fn langs() -> &'static [(&'static str, &'static str)] {
    langs_cache()
}

/// Compatibility alias: consente `i18n::LANGS[i]` e `i18n::LANGS.iter()` come in precedenza,
/// ma legge la tabella dal registro `lang/langs.tsv` invece di un array hardcoded.
pub struct LangTable;

impl Deref for LangTable {
    type Target = [(&'static str, &'static str)];

    fn deref(&self) -> &Self::Target {
        langs()
    }
}

pub static LANGS: LangTable = LangTable;

/// Confronto tra codici canonicali (minuscole per la lingua, `ll_CC` per il locale).
fn exact_index(code: &str) -> Option<usize> {
    let want = canonical_code(code);
    if want.is_empty() {
        return None;
    }
    langs().iter().position(|(c, _)| canonical_code(c) == want)
}

/// Una lingua "automatica" è utile solo se ha almeno una traduzione: in caso contrario
/// la rilevazione passa al livello successivo della catena (o alla lingua di fallback).
fn translated(i: usize) -> bool {
    if i == 0 {
        return true; // la lingua originale è sempre disponibile
    }
    table()
        .values()
        .any(|row| row.get(i - 1).is_some_and(|s| !s.trim().is_empty()))
}

/// Risolve un codice o un locale con fallback ordinato `ll_CC` → `ll`.
pub fn resolve(raw: &str) -> Option<usize> {
    locale_chain(raw)
        .into_iter()
        .find_map(|c| exact_index(&c).filter(|&i| translated(i)))
}

/// Risolve un codice salvato nel config senza scartare le lingue registrate ma
/// non ancora tradotte: la scelta dell'utente deve sopravvivere al riavvio e
/// mostrare il testo originale UTF-8. Il fallback finale resta in `detect()`.
pub fn index_of(code: &str) -> Option<usize> {
    locale_chain(code).into_iter().find_map(|c| exact_index(&c))
}

/// Lingua di fallback: inglese se tradotta, altrimenti la lingua originale.
pub fn fallback_lang() -> usize {
    exact_index(FALLBACK_CODE)
        .filter(|&i| translated(i))
        .unwrap_or(0)
}

/// Lingua di sistema: `LC_ALL`, `LC_MESSAGES`, `LANGUAGE` e `LANG`, in ordine di priorità.
pub fn detect() -> usize {
    for key in ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            if let Some(i) = resolve(&value) {
                return i;
            }
        }
    }
    fallback_lang()
}

static CUR: AtomicUsize = AtomicUsize::new(0);

pub fn set_lang(i: usize) {
    CUR.store(i.min(langs().len().saturating_sub(1)), Ordering::Relaxed);
}

pub fn lang() -> usize {
    CUR.load(Ordering::Relaxed)
        .min(langs().len().saturating_sub(1))
}

/// Codice canonicale salvato nel config (es. `en`, `pt_BR`).
pub fn code_of(i: usize) -> &'static str {
    langs()
        .get(i)
        .map(|l| l.0)
        .unwrap_or_else(|| langs().first().map(|l| l.0).unwrap_or("it"))
}

/// Nome mostrato nel selettore della GUI.
pub fn name_of(i: usize) -> &'static str {
    langs().get(i).map(|l| l.1).unwrap_or_else(|| code_of(i))
}

fn table() -> &'static HashMap<&'static str, Vec<&'static str>> {
    static T: OnceLock<HashMap<&'static str, Vec<&'static str>>> = OnceLock::new();
    T.get_or_init(|| {
        let mut m = HashMap::new();
        let cols = langs().len().saturating_sub(1);
        for line in DATA.lines() {
            let line = line.trim_start_matches('\u{feff}');
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let mut it = line.split('\t');
            let key = match it.next() {
                Some(k) if !k.is_empty() => k,
                _ => continue,
            };
            // Colonne mancanti (nuova lingua ancora da tradurre) restano vuote:
            // il fallback riporta il testo originale, senza perdere caratteri UTF-8.
            let mut v = vec![""; cols];
            for slot in v.iter_mut() {
                if let Some(x) = it.next() {
                    *slot = x;
                }
            }
            m.insert(key, v);
        }
        m
    })
}

fn lookup(s: &str) -> Option<&'static str> {
    let i = lang();
    if i == 0 {
        return None; // lingua originale: la chiave è già il testo da mostrare
    }
    let row = table().get(s)?;
    let text = row.get(i - 1).copied().unwrap_or("");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Traduce una stringa statica (sostituzione diretta di un letterale).
pub fn t(s: &'static str) -> &'static str {
    lookup(s).unwrap_or(s)
}

/// Traduce una stringa non statica (chiave calcolata a runtime).
pub fn tr(s: &str) -> String {
    lookup(s).unwrap_or(s).to_string()
}

/// Traduce un modello con segnaposto `{}` e li sostituisce in ordine.
pub fn tf(s: &'static str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::new();
    let mut it = args.iter();
    let mut rest = t(s);
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        if let Some(a) = it.next() {
            out.push_str(&a.to_string());
        }
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const KEYS_FILE: &str = include_str!("../lang/keys.txt");
    const STRINGS_FILE: &str = include_str!("../lang/strings.tsv");

    /// Righe di dati di una tabella TSV (senza commenti e righe vuote).
    fn data_rows(text: &str) -> Vec<(usize, Vec<&str>)> {
        text.lines()
            .enumerate()
            .filter_map(|(idx, line)| {
                let line = line.trim_start_matches('\u{feff}');
                if line.trim().is_empty() || line.trim_start().starts_with('#') {
                    return None;
                }
                Some((idx + 1, line.split('\t').collect()))
            })
            .collect()
    }

    /// Segnaposto `{...}`, come quelli controllati da tools/build_strings.py.
    fn placeholders(s: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut from = 0usize;
        while let Some(rel) = s[from..].find('{') {
            let open = from + rel;
            let Some(close_rel) = s[open + 1..].find('}') else { break };
            let close = open + 1 + close_rel;
            if !s[open + 1..close].contains('{') {
                out.push(&s[open..=close]);
            }
            from = close + 1;
        }
        out
    }

    #[test]
    fn locale_chains_are_ordered() {
        assert_eq!(locale_chain("pt_BR.UTF-8"), vec!["pt_BR", "pt"]);
        assert_eq!(locale_chain("en-US"), vec!["en_US", "en"]);
        assert_eq!(locale_chain("de_DE@euro"), vec!["de_DE", "de"]);
        assert_eq!(locale_chain("it"), vec!["it"]);
        // Il pseudo-locale POSIX `c`/`C` è canonizzato a `C` in maiuscolo:
        // non è un codice lingua BCP 47 e non deve diventare `c`.
        assert_eq!(locale_chain("C.UTF-8"), vec!["C"]);
        assert_eq!(locale_chain("c.UTF-8"), vec!["C"]);
        assert_eq!(locale_chain("de:fr"), vec!["de", "fr"]);
    }

    #[test]
    fn languages_come_from_lang_data() {
        assert!(langs().len() >= 2);
        assert_eq!(code_of(0), "it");
        // Locale non registrato: catena ll_CC -> ll.
        assert_eq!(index_of("EN_us"), Some(1));
        assert_eq!(index_of("it_IT.UTF-8"), Some(0));
        assert_eq!(resolve("xx_YY.UTF-8"), None);
    }

    #[test]
    fn tutte_le_12_lingue_hanno_colonne_coerenti() {
        assert_eq!(langs().len(), 12, "il registro deve contenere 12 lingue");
        assert_eq!(
            langs().iter().map(|(code, _)| *code).collect::<Vec<_>>(),
            vec!["it", "en", "es", "fr", "de", "pt", "nl", "pl", "ru", "uk", "ro", "cs"]
        );
        let codes: HashSet<&str> = (0..langs().len()).map(code_of).collect();
        assert_eq!(codes.len(), langs().len(), "codici lingua duplicati");

        let rows = data_rows(STRINGS_FILE);
        assert!(rows.len() > 1000, "tabella traduzioni troppo piccola");
        // Una colonna è la chiave/italiano, le altre 11 sono le traduzioni.
        let expected_columns = langs().len();
        for (line_no, fields) in rows {
            assert_eq!(
                fields.len(),
                expected_columns,
                "strings.tsv riga {line_no}: colonne errate"
            );
            assert!(!fields[0].is_empty(), "strings.tsv riga {line_no}: chiave vuota");
        }
    }

    #[test]
    fn keys_txt_corrisponde_a_strings_tsv() {
        // Mantieni gli spazi iniziali delle voci dell'aiuto CLI: la prima
        // colonna di strings.tsv li conserva, e `extract_keys.py` li scrive
        // così in keys.txt. Vanno rimossi solo BOM e righe vuote/commento.
        let keys: Vec<&str> = KEYS_FILE
            .lines()
            .map(|line| line.trim_start_matches('\u{feff}'))
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .collect();
        let table_keys: Vec<&str> = data_rows(STRINGS_FILE)
            .into_iter()
            .map(|(_, fields)| fields[0])
            .collect();
        let key_set: HashSet<&str> = keys.iter().copied().collect();
        let table_set: HashSet<&str> = table_keys.iter().copied().collect();
        assert_eq!(key_set, table_set, "keys.txt e strings.tsv non coincidono");
        assert_eq!(keys.as_slice(), table_keys.as_slice(), "keys.txt e strings.tsv hanno ordini diversi");
        assert_eq!(key_set.len(), keys.len(), "chiavi duplicate in keys.txt");
        assert_eq!(table_set.len(), table_keys.len(), "chiavi duplicate in strings.tsv");
    }

    #[test]
    fn segnaposto_coerenti_in_tutte_le_traduzioni() {
        for (line_no, fields) in data_rows(STRINGS_FILE) {
            let expected = placeholders(fields[0]);
            for (col, value) in fields.iter().enumerate().skip(1) {
                if !value.trim().is_empty() {
                    assert_eq!(
                        placeholders(value),
                        expected,
                        "strings.tsv riga {line_no}, colonna {col}: segnaposto diversi"
                    );
                }
            }
        }
    }

    struct LangGuard(usize);

    impl Drop for LangGuard {
        fn drop(&mut self) {
            set_lang(self.0);
        }
    }

    #[test]
    fn traduzioni_e_fallback_funzionano_per_ogni_lingua() {
        let guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _lang = LangGuard(lang());
        for index in 0..langs().len() {
            set_lang(index);
            assert_eq!(lang(), index);
            let expected = if index == 0 {
                "Distribuzione"
            } else {
                table()
                    .get("Distribuzione")
                    .and_then(|row| row.get(index - 1))
                    .copied()
                    .unwrap_or("Distribuzione")
            };
            assert_eq!(tr("Distribuzione"), expected, "lingua {index}");
            assert!(!expected.trim().is_empty(), "traduzione vuota in colonna {index}");
            assert_eq!(tr("_chiave_non_registrata_"), "_chiave_non_registrata_");
        }
        drop(guard);
    }
}
