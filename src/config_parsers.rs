//! Parser puri e deterministici per i file di configurazione più delicati.
//!
//! Le funzioni di questo modulo non leggono il filesystem, non avviano processi
//! e non chiedono privilegi: lavorano solo su testo. Sono usate dalla UI (per
//! anteprime, elenchi e validazione) e coperte da unit test con input sintetici,
//! così una regressione nei formati di `sources.list`, GRUB o `fstab` viene
//! intercettata senza toccare `/etc`.

/// Tipo di riga di un file APT classico (`sources.list` / `*.list`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AptLineKind {
    Blank,
    Comment,
    Deb,
    DebSrc,
    /// Campo di un file deb822 (`*.sources`), anche se commentato.
    Deb822Field,
    Other,
}

/// Riga di un file APT con il suo numero originale (1-based).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AptLine<'a> {
    pub number: usize,
    pub raw: &'a str,
    /// Testo senza spazi iniziali/finali.
    pub text: &'a str,
    pub kind: AptLineKind,
    /// `false` per righe vuote, commentate o con voci disabilitate.
    pub enabled: bool,
}

/// Parse di un `sources.list` / `*.list`: commenti, spazi extra e righe
/// malformate vengono classificati senza essere scartati.
pub fn parse_apt_lines(text: &str) -> Vec<AptLine<'_>> {
    text.lines()
        .enumerate()
        .map(|(idx, raw)| {
            let number = idx + 1;
            let trimmed = raw.trim();
            let (kind, enabled) = if trimmed.is_empty() {
                (AptLineKind::Blank, false)
            } else if trimmed.starts_with('#') {
                let unhashed = trimmed.trim_start_matches('#').trim();
                let kind = if unhashed.starts_with("deb-src ") {
                    AptLineKind::DebSrc
                } else if unhashed.starts_with("deb ") {
                    AptLineKind::Deb
                } else if unhashed.starts_with("URIs:") {
                    AptLineKind::Deb822Field
                } else {
                    AptLineKind::Comment
                };
                (kind, false)
            } else if trimmed.starts_with("deb-src ") {
                (AptLineKind::DebSrc, true)
            } else if trimmed.starts_with("deb ") {
                (AptLineKind::Deb, true)
            } else if trimmed
                .split_once(':')
                .is_some_and(|(key, _)| key.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
            {
                (AptLineKind::Deb822Field, true)
            } else {
                (AptLineKind::Other, true)
            };
            AptLine { number, raw, text: trimmed, kind, enabled }
        })
        .collect()
}

/// `true` per una riga `deb`/`deb-src` attiva (usata per gli elenchi selezionabili).
pub fn is_active_repo_line(trimmed: &str) -> bool {
    trimmed.starts_with("deb ") || trimmed.starts_with("deb-src ")
}

/// Rileva una voce APT disabilitata con `#`, come fa l'audit dei repository.
pub fn is_disabled_repo_line(trimmed: &str) -> bool {
    if !trimmed.starts_with('#') {
        return false;
    }
    let unhashed = trimmed.trim_start_matches('#').trim();
    unhashed.starts_with("deb ")
        || unhashed.starts_with("deb-src ")
        || unhashed.starts_with("URIs:")
}

/// Chiave normalizzata per il rilevamento dei duplicati.
/// Le opzioni tra parentesi quadre (`signed-by`, `arch`) non fanno parte della
/// chiave: due righe con opzioni diverse ma stesso repository sono duplicati.
pub fn normalize_repo_line(trimmed: &str) -> Option<String> {
    if is_active_repo_line(trimmed) {
        let without_opts = if let Some(rb) = trimmed.find(']') {
            trimmed[rb + 1..].trim()
        } else {
            trimmed.trim_start_matches("deb-src ").trim_start_matches("deb ").trim()
        };
        Some(without_opts.to_string())
    } else if trimmed.starts_with("URIs:") {
        Some(trimmed.trim_start_matches("URIs:").trim().to_string())
    } else {
        None
    }
}

/// Sostituisce una riga mantenendo tutte le altre (e il newline finale).
/// I numeri di riga sono 1-based; un numero fuori intervallo non cambia nulla.
pub fn replace_line(text: &str, line_no: usize, new_text: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let Some(slot) = line_no.checked_sub(1) else { return text.to_string() };
    if slot >= lines.len() {
        return text.to_string();
    }
    lines[slot] = new_text.to_string();
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Commenta una riga attiva con `# `; le righe già commentate o vuote restano
/// invariate.
pub fn comment_line(text: &str, line_no: usize) -> String {
    let Some(raw) = text.lines().nth(line_no.saturating_sub(1)) else {
        return text.to_string();
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return text.to_string();
    }
    replace_line(text, line_no, &format!("# {trimmed}"))
}

/// Rimuove il `#` iniziale da una riga disabilitata, preservando il testo.
pub fn uncomment_line(text: &str, line_no: usize) -> String {
    let Some(raw) = text.lines().nth(line_no.saturating_sub(1)) else {
        return text.to_string();
    };
    let trimmed = raw.trim_start();
    if !trimmed.starts_with('#') {
        return text.to_string();
    }
    let leading = &raw[..raw.len() - trimmed.len()];
    let unhashed = trimmed.trim_start_matches('#').trim_start();
    replace_line(text, line_no, &format!("{leading}{unhashed}"))
}

/// Stanza di un file deb822 (`.sources`): coppie chiave/valore nell'ordine
/// originale. Le righe di continuazione vengono unite al campo precedente.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deb822Stanza {
    pub fields: Vec<(String, String)>,
}

/// Parse di un file `.sources`; commenti e righe vuote non generano stanze.
pub fn parse_deb822(text: &str) -> Vec<Deb822Stanza> {
    let mut stanzas: Vec<Deb822Stanza> = Vec::new();
    let mut current: Option<Deb822Stanza> = None;
    for raw in text.lines() {
        if raw.trim().is_empty() {
            if let Some(stanza) = current.take() {
                if !stanza.fields.is_empty() {
                    stanzas.push(stanza);
                }
            }
            continue;
        }
        if raw.trim_start().starts_with('#') {
            continue;
        }
        if raw.starts_with(' ') || raw.starts_with('\t') {
            if let Some(stanza) = current.as_mut() {
                if let Some((_, value)) = stanza.fields.last_mut() {
                    let continuation = raw.trim();
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(continuation);
                }
            }
            continue;
        }
        let Some((key, value)) = raw.split_once(':') else { continue };
        let stanza = current.get_or_insert_with(Deb822Stanza::default);
        stanza.fields.push((key.trim().to_string(), value.trim().to_string()));
    }
    if let Some(stanza) = current.take() {
        if !stanza.fields.is_empty() {
            stanzas.push(stanza);
        }
    }
    stanzas
}

/// Valore di un campo deb822, confrontando le chiavi senza distinguere maiuscole.
pub fn deb822_field<'a>(stanzas: &'a [Deb822Stanza], key: &str) -> Option<&'a str> {
    stanzas.iter().flat_map(|s| s.fields.iter()).find_map(|(k, v)| {
        k.eq_ignore_ascii_case(key).then_some(v.as_str())
    })
}

/// Riga di `/etc/default/grub` con chiave e valore separati da `=`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrubLine<'a> {
    pub number: usize,
    pub raw: &'a str,
    pub key: Option<&'a str>,
    pub value: Option<&'a str>,
    pub enabled: bool,
}

/// Separa il valore di un parametro GRUB dal commento finale ` #...`.
/// Un `#` dentro le virgolette fa parte del valore, non è un commento.
fn split_grub_comment(raw: &str) -> (&str, &str) {
    let bytes = raw.as_bytes();
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (idx, byte) in bytes.iter().copied().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, byte) {
            (Some(b'"'), b'\\') => escaped = true,
            (Some(q), b) if b == q => quote = None,
            (None, b'"') => quote = Some(b'"'),
            (None, b'\'') => quote = Some(b'\''),
            (None, b'#') if idx == 0 || bytes[idx - 1].is_ascii_whitespace() => {
                return (raw[..idx].trim_end(), &raw[idx..]);
            }
            _ => {}
        }
    }
    (raw, "")
}

/// Parse di `/etc/default/grub`: righe commentate, spazi extra e voci con
/// chiavi non valide vengono conservate e classificate.
pub fn parse_grub(text: &str) -> Vec<GrubLine<'_>> {
    text.lines()
        .enumerate()
        .map(|(idx, raw)| {
            let trimmed = raw.trim_start();
            let enabled = !trimmed.starts_with('#');
            let clean = trimmed.trim_start_matches('#').trim_start();
            let parsed = clean.split_once('=').and_then(|(key, value)| {
                let key = key.trim();
                let valid = !key.is_empty()
                    && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                let (value, _) = split_grub_comment(value.trim());
                valid.then_some((key, value.trim()))
            });
            GrubLine {
                number: idx + 1,
                raw,
                key: parsed.map(|(k, _)| k),
                value: parsed.map(|(_, v)| v),
                enabled,
            }
        })
        .collect()
}

/// Valore di un parametro GRUB attivo, senza virgolette esterne.
pub fn grub_value(text: &str, key: &str) -> String {
    parse_grub(text)
        .into_iter()
        .find(|line| line.enabled && line.key == Some(key))
        .and_then(|line| line.value)
        .unwrap_or("")
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

/// Aggiorna un parametro GRUB preservando tutte le altre righe.
///
/// Se esiste una riga attiva viene sostituita solo quella (mantenendo
/// indentazione ed eventuale commento finale). Se esiste solo una riga
/// commentata, la commentata resta intatta e la nuova riga attiva viene
/// inserita subito dopo. Se il parametro manca del tutto, viene aggiunto in
/// fondo. Il newline finale del file viene conservato.
pub fn set_grub_param(text: &str, key: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    let mut insert_after = None;
    for line in parse_grub(text) {
        let same_key = line.key == Some(key);
        if same_key && line.enabled && !replaced {
            let raw = line.raw;
            let leading = &raw[..raw.len() - raw.trim_start().len()];
            let after_eq = raw.trim_start().split_once('=').map(|(_, v)| v.trim()).unwrap_or("");
            let (_, comment) = split_grub_comment(after_eq);
            let trailing_comment = if comment.is_empty() { String::new() } else { format!(" {comment}") };
            lines.push(format!("{leading}{key}=\"{escaped}\"{trailing_comment}"));
            replaced = true;
            continue;
        }
        lines.push(line.raw.to_string());
        if same_key && !line.enabled && insert_after.is_none() {
            insert_after = Some(lines.len());
        }
    }
    if !replaced {
        let new_line = format!("{key}=\"{escaped}\"");
        match insert_after {
            Some(at) => lines.insert(at, new_line),
            None => lines.push(new_line),
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Riga di `fstab` con i sei campi separati da spazi e lo stato di validità.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FstabEntry<'a> {
    pub number: usize,
    pub raw: &'a str,
    pub fields: Vec<&'a str>,
    pub enabled: bool,
    /// Vero solo per righe attive con un numero di campi diverso da sei.
    pub malformed: bool,
    pub mount_point: Option<&'a str>,
    pub options: Vec<&'a str>,
}

/// Parse di `fstab`: commenti, righe vuote, spazi extra e righe malformate
/// restano nell'elenco con il loro numero di riga originale.
pub fn parse_fstab(text: &str) -> Vec<FstabEntry<'_>> {
    text.lines()
        .enumerate()
        .map(|(idx, raw)| {
            let trimmed = raw.trim();
            let enabled = !trimmed.starts_with('#');
            let content = trimmed.trim_start_matches('#').trim();
            let fields: Vec<&str> = if content.is_empty() {
                Vec::new()
            } else {
                // Un commento finale non è un campo: evita di segnalare come
                // malformata una riga altrimenti valida.
                content.split_whitespace().take_while(|f| !f.starts_with('#')).collect()
            };
            let malformed = enabled && !fields.is_empty() && fields.len() != 6;
            let mount_point = fields.get(1).copied();
            let options = fields
                .get(3)
                .map(|value| value.split(',').filter(|o| !o.is_empty()).collect())
                .unwrap_or_default();
            FstabEntry { number: idx + 1, raw, fields, enabled, malformed, mount_point, options }
        })
        .collect()
}

/// Numeri di riga delle voci attive di `fstab` con campi errati.
pub fn fstab_malformed(text: &str) -> Vec<usize> {
    parse_fstab(text)
        .into_iter()
        .filter(|line| line.malformed)
        .map(|line| line.number)
        .collect()
}

/// Sostituisce il campo `field_index` mantenendo identici spazi iniziali,
/// separatori originali ed eventuale spazio finale.
fn replace_field_preserving(raw: &str, field_index: usize, new_value: &str) -> String {
    let leading_len = raw.len() - raw.trim_start().len();
    let leading = &raw[..leading_len];
    let body = &raw[leading_len..];
    let mut out = String::from(leading);
    let mut index = 0usize;
    let mut pos = 0usize;
    for word in body.split_whitespace() {
        let gap = body[pos..].find(word).unwrap_or(0);
        out.push_str(&body[pos..pos + gap]);
        if index == field_index {
            out.push_str(new_value);
        } else {
            out.push_str(word);
        }
        pos += gap + word.len();
        index += 1;
    }
    out.push_str(&body[pos..]);
    out
}

/// Aggiunge o rimuove un'opzione dal punto di mount indicato. Le altre righe,
/// gli spazi originali, i commenti e l'eventuale newline finale vengono preservati.
pub fn set_fstab_option(text: &str, mount_point: &str, option: &str, add: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in parse_fstab(text) {
        if line.enabled && !line.malformed && line.mount_point == Some(mount_point) {
            let mut options: Vec<String> = line.fields[3]
                .split(',')
                .filter(|o| !o.is_empty())
                .map(str::to_string)
                .collect();
            let present = options.iter().any(|o| o == option);
            if add && !present {
                options.push(option.to_string());
            } else if !add && present {
                options.retain(|o| o != option);
            }
            lines.push(replace_field_preserving(line.raw, 3, &options.join(",")));
        } else {
            lines.push(line.raw.to_string());
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_parser_classifica_commenti_spazi_e_voci_disabilitate() {
        let text = "# deb http://archive.example/debian bookworm main\n\
                    ##deb http://old.example/debian buster main\n\
                    \n\
                      deb-src  http://archive.example/debian bookworm main  \n\
                    deb [signed-by=/usr/share/keyrings/x.gpg] http://archive.example/debian bookworm main\n\
                    deb http://archive.example/debian bookworm main\n\
                    deblinedi prova\n";
        let lines = parse_apt_lines(text);
        assert_eq!(lines.len(), 7, "nessuna riga deve essere scartata");

        assert_eq!(lines[0].number, 1);
        assert_eq!(lines[0].kind, AptLineKind::Deb);
        assert!(!lines[0].enabled, "voce commentata: disabilitata");
        assert!(is_disabled_repo_line(lines[0].text));

        assert_eq!(lines[1].kind, AptLineKind::Deb);
        assert!(!lines[1].enabled);

        assert_eq!(lines[2].kind, AptLineKind::Blank);
        assert_eq!(lines[2].text, "");

        assert_eq!(lines[3].number, 4);
        assert_eq!(lines[3].kind, AptLineKind::DebSrc);
        assert_eq!(lines[3].raw.trim(), lines[3].text);
        assert!(lines[3].enabled);
        assert_eq!(lines[3].text, "deb-src  http://archive.example/debian bookworm main");
        assert!(is_active_repo_line(lines[3].text));

        // `Other` resta attivo ma non è una voce APT selezionabile.
        assert_eq!(lines[6].kind, AptLineKind::Other);
        assert!(lines[6].enabled);
        let attive: Vec<&AptLine<'_>> = lines
            .iter()
            .filter(|l| l.enabled && matches!(l.kind, AptLineKind::Deb | AptLineKind::DebSrc))
            .collect();
        assert_eq!(attive.len(), 3);

        // Le opzioni tra [] non devono alterare la chiave di duplicato.
        assert_eq!(
            normalize_repo_line(lines[4].text).as_deref(),
            Some("http://archive.example/debian bookworm main")
        );
        assert_eq!(
            normalize_repo_line(lines[5].text).as_deref(),
            Some("http://archive.example/debian bookworm main")
        );
        assert_eq!(normalize_repo_line(lines[6].text), None);
        assert_eq!(normalize_repo_line("# URIs: http://example").as_deref(), None);
        assert!(is_disabled_repo_line("# URIs: http://example"));

        let fields = parse_apt_lines(
            "# solo commento\n  URIs: http://example.invalid/repo\n# URIs: http://old.invalid\n",
        );
        assert_eq!(fields[0].kind, AptLineKind::Comment);
        assert_eq!(fields[1].kind, AptLineKind::Deb822Field);
        assert!(fields[1].enabled);
        assert_eq!(fields[2].kind, AptLineKind::Deb822Field);
        assert!(!fields[2].enabled);
    }

    #[test]
    fn apt_modifiche_preservano_le_righe_esistenti() {
        let text = "deb http://a.example stable main\n# commento utente\ndeb http://b.example stable main\n";
        let commented = comment_line(text, 3);
        assert_eq!(
            commented,
            "deb http://a.example stable main\n# commento utente\n# deb http://b.example stable main\n"
        );
        assert_eq!(uncomment_line(&commented, 3), text);

        let changed = replace_line(text, 1, "deb http://c.example stable main");
        assert_eq!(
            changed,
            "deb http://c.example stable main\n# commento utente\ndeb http://b.example stable main\n"
        );
        // Indici fuori intervallo: nessuna modifica.
        assert_eq!(replace_line(text, 99, "x"), text);
        assert_eq!(comment_line(text, 2), text);
    }

    #[test]
    fn sources_deb822_legge_stanze_e_campi() {
        let text = "# Generato da un PPA\n\
                    Types: deb\n\
                    URIs: https://ppa.example/ubuntu\n\
                    Suites: noble noble-updates\n\
                    Components: main\n\
                    Signed-By:\n  /usr/share/keyrings/ppa.gpg\n\
                    \n\
                    Types: deb-src\n\
                    URIs: https://other.example/repo\n\
                    Suites: noble\n\
                    Components: main\n";
        let stanzas = parse_deb822(text);
        assert_eq!(stanzas.len(), 2);
        assert_eq!(deb822_field(&stanzas, "URIs"), Some("https://ppa.example/ubuntu"));
        assert_eq!(deb822_field(&stanzas, "types"), Some("deb"));
        assert_eq!(
            deb822_field(&stanzas, "Signed-By"),
            Some("/usr/share/keyrings/ppa.gpg"),
            "la riga di continuazione va unita al campo precedente"
        );
        assert_eq!(deb822_field(&stanzas, "Components"), Some("main"));
        assert!(parse_deb822("# solo commenti\n\n").is_empty());
    }

    #[test]
    fn grub_legge_e_aggiorna_i_parametri_senza_perdere_righe() {
        let text = "# GRUB_CMDLINE_LINUX_DEFAULT=\"vecchio disabilitato\"\n\
                    GRUB_DEFAULT=0\n  GRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash\" # commento finale\n\
                    GRUB_TIMEOUT=5\n";
        assert_eq!(grub_value(text, "GRUB_CMDLINE_LINUX_DEFAULT"), "quiet splash");
        assert_eq!(grub_value(text, "GRUB_TIMEOUT"), "5");
        assert_eq!(grub_value(text, "GRUB_MISSING"), "");

        let parsed = parse_grub(text);
        assert_eq!(parsed.len(), 4);
        assert!(!parsed[0].enabled);
        assert_eq!(parsed[0].key, Some("GRUB_CMDLINE_LINUX_DEFAULT"));
        assert_eq!(parsed[2].key, Some("GRUB_CMDLINE_LINUX_DEFAULT"));
        assert_eq!(
            parsed[2].raw,
            "  GRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash\" # commento finale"
        );
        assert_eq!(parsed[3].number, 4);

        let updated = set_grub_param(text, "GRUB_CMDLINE_LINUX_DEFAULT", "quiet nomodeset");
        assert_eq!(grub_value(&updated, "GRUB_CMDLINE_LINUX_DEFAULT"), "quiet nomodeset");
        assert!(updated.contains("# GRUB_CMDLINE_LINUX_DEFAULT=\"vecchio disabilitato\""));
        assert!(updated.contains("GRUB_DEFAULT=0"));
        assert!(updated.contains("GRUB_TIMEOUT=5"));
        assert!(updated.contains(" # commento finale"));
        assert!(updated.contains("  GRUB_CMDLINE_LINUX_DEFAULT=\"quiet nomodeset\""));
        assert!(updated.ends_with('\n'));
        assert_eq!(updated.lines().count(), text.lines().count());

        // Solo riga commentata: il commento resta e la nuova riga va dopo.
        let only_commented = "# GRUB_CMDLINE_LINUX_DEFAULT=\"quiet\"\nGRUB_TIMEOUT=5\n";
        let fixed = set_grub_param(only_commented, "GRUB_CMDLINE_LINUX_DEFAULT", "splash");
        assert_eq!(
            fixed,
            "# GRUB_CMDLINE_LINUX_DEFAULT=\"quiet\"\n\
             GRUB_CMDLINE_LINUX_DEFAULT=\"splash\"\n\
             GRUB_TIMEOUT=5\n"
        );

        // Parametro assente: aggiunto in fondo senza toccare il resto.
        let added = set_grub_param("GRUB_TIMEOUT=5\n", "GRUB_SAVEDEFAULT", "true");
        assert_eq!(added, "GRUB_TIMEOUT=5\nGRUB_SAVEDEFAULT=\"true\"\n");
    }

    #[test]
    fn fstab_parser_e_modifica_opzioni() {
        let text = "# /dev/sda1 / ext4 defaults 0 1\n\
                    \n\
                    UUID=aaaa  /   ext4   defaults,noatime  0  1\n\
                    /dev/sdb1 /mnt/dati ext4 defaults 0\n\
                    # /dev/sdc1 /mnt/old ext4 ro 0 2\n\
                    UUID=bbbb none swap sw 0 0\n";
        let lines = parse_fstab(text);
        assert_eq!(lines.len(), 6);
        assert!(!lines[0].enabled, "riga commentata");
        assert_eq!(lines[0].mount_point, Some("/"));
        assert_eq!(lines[1].fields.len(), 0, "riga vuota");

        assert_eq!(lines[2].number, 3);
        assert_eq!(lines[2].mount_point, Some("/"));
        assert!(!lines[2].malformed);
        assert_eq!(lines[2].options, vec!["defaults", "noatime"]);

        assert_eq!(lines[3].number, 4);
        assert!(lines[3].malformed, "solo 5 campi");
        assert_eq!(fstab_malformed(text), vec![4]);

        assert!(!lines[4].enabled);
        assert_eq!(lines[5].mount_point, Some("none"));

        let with_option = set_fstab_option(text, "/", "discard", true);
        assert_eq!(
            with_option.lines().nth(2).unwrap(),
            "UUID=aaaa  /   ext4   defaults,noatime,discard  0  1"
        );
        // Le righe commentate, vuote e malformate non vengono riscritte.
        assert_eq!(with_option.lines().nth(0).unwrap(), lines[0].raw);
        assert_eq!(with_option.lines().nth(1).unwrap(), lines[1].raw);
        assert_eq!(with_option.lines().nth(3).unwrap(), lines[3].raw);
        assert_eq!(with_option.lines().nth(4).unwrap(), lines[4].raw);
        assert!(with_option.ends_with('\n'));

        // Aggiunta idempotente e rimozione mirata.
        assert_eq!(set_fstab_option(&with_option, "/", "discard", true), with_option);
        let without = set_fstab_option(&with_option, "/", "noatime", false);
        assert_eq!(
            without.lines().nth(2).unwrap(),
            "UUID=aaaa  /   ext4   defaults,discard  0  1"
        );
        assert_eq!(set_fstab_option(text, "/mnt/assente", "x", true), text);
        assert_eq!(fstab_malformed(&without), vec![4]);
    }
}
