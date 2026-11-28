use crate::i18n::t;
use crate::{ui, App};
use eframe::egui;

const TOKENS: &[&[&[&str]]] = &[
    &[&["out of memory"], &["oom-killer"], &["oom_reaper"]],
    &[&["no space left on device"], &["disk full"], &["not enough free space"]],
    &[&["i/o error"], &["ext4-fs error"], &["blk_update_request"], &["buffer i/o error"], &["medium error"], &["failed command"]],
    &[&["failed to start"], &["entered failed state"], &["failed with result"]],
    &[&["segfault"], &["general protection fault"], &["core dumped"]],
    &[&["direct firmware load"], &["failed to load firmware"], &["firmware: failed"], &["missing firmware"], &["firmware", "not found"]],
    &[&["network is unreachable"], &["temporary failure in name resolution"], &["dhcp", "timeout"], &["carrier lost"]],
    &[&["temperature above threshold"], &["cpu clock throttled"], &["thermal", "critical"], &["overheat"]],
    &[&["authentication failure"], &["failed password"], &["pam_unix", "failure"]],
    &[&["unable to enumerate usb device"], &["device descriptor read"], &["usb", "error -71"]],
    &[&["acpi", "error"]],
    &[&["bluetooth", "fail"], &["bluetooth", "error"], &["hci", "timeout"]],
    &[&["pipewire", "fail"], &["pulseaudio", "fail"], &["pipewire", "error"], &["wireplumber", "fail"]],
    &[&["dpkg", "error"], &["dpkg", "interrupted"], &["unmet dependencies"], &["broken", "packages"]],
    &[&["gpu hang"], &["gpu fault"], &["amdgpu", "error"], &["nouveau", "error"], &["drm", "error"]],
];

// SOLUTIONS-BEGIN
const TEXTS: &[(&str, &str)] = &[
    (
        "Memoria RAM esaurita: il sistema ha chiuso dei programmi",
        "Chiudi i programmi che non usi e controlla i «Processi» più pesanti. Attiva la zRAM in «Ottimizza RAM» per avere più margine. Se succede spesso, il computer ha poca RAM per l'uso che ne fai.",
    ),
    (
        "Disco pieno",
        "Libera spazio con «Pulizia» (anche «Pulizia avanzata») ed elimina i file grandi da «Disco» → «File grandi».",
    ),
    (
        "Errori di lettura o scrittura del disco: il disco potrebbe essere in avaria",
        "Fai subito un backup dei dati importanti, poi controlla la salute del disco in «Disco» → «Analisi disco» → «Stato e attributi». Se il disco risulta danneggiato, valuta di sostituirlo.",
    ),
    (
        "Un servizio non si è avviato",
        "Apri il «Fixer» e premi «Correggi» su «Servizi systemd», oppure in «Pulizia» → «Servizi di sistema» controlla lo stato del servizio e riavvialo.",
    ),
    (
        "Un programma si è chiuso in modo anomalo (crash)",
        "Riavvia il programma. Se succede ancora, aggiornalo dalla scheda «Aggiornamenti» oppure reinstallalo da «App installate».",
    ),
    (
        "Manca un firmware per una periferica (Wi-Fi, scheda video, audio...)",
        "Apri «Repository e driver», premi «Rileva hardware e driver» e installa i firmware consigliati. Poi riavvia il computer.",
    ),
    (
        "Problemi con la connessione di rete",
        "Apri la scheda «Rete»: premi «Test diagnostico» e, se una spia è rossa, «Ripristina rete e Wi-Fi». Controlla anche il cavo o il segnale Wi-Fi.",
    ),
    (
        "Il processore si surriscalda",
        "Pulisci dalla polvere le prese d'aria e le ventole. In «Temperature» attiva la «Modalità di raffreddamento» finché il problema non si risolve.",
    ),
    (
        "Tentativi di accesso con password errata",
        "Se non sei stato tu, cambia la password e attiva il firewall in «Sicurezza e firewall». Se sei stato tu, puoi ignorare il messaggio.",
    ),
    (
        "Un dispositivo USB non viene riconosciuto",
        "Prova un'altra porta USB o un altro cavo e collega il dispositivo direttamente al computer, senza hub.",
    ),
    (
        "Avviso ACPI del BIOS (molto comune)",
        "Di solito è innocuo e si può ignorare se il computer funziona bene. Un aggiornamento del BIOS del produttore può eliminarlo.",
    ),
    (
        "Problema con il Bluetooth",
        "In «Pulizia» → «Servizi di sistema» controlla il servizio bluetooth e avvialo; in «Repository e driver» installa i firmware consigliati.",
    ),
    (
        "Problema con l'audio",
        "In «Kernel, Grub e Recovery» → «Recovery» premi «Riavvia audio». Se non basta, riavvia il computer.",
    ),
    (
        "Problemi con i pacchetti installati",
        "Apri il «Fixer» e premi «Correggi tutto», oppure «Correggi dipendenze rotte» e «Riconfigura pacchetti».",
    ),
    (
        "Errori della scheda video",
        "Installa i driver adatti in «Repository e driver» → «Driver manager» e aggiorna il sistema da «Aggiornamenti». Se il problema resta, prova ad avviare con un kernel diverso.",
    ),
];
// SOLUTIONS-END

const _: () = assert!(TOKENS.len() == TEXTS.len());

fn matching(out: &str) -> Vec<&'static (&'static str, &'static str)> {
    let lines: Vec<String> = out.lines().map(|l| l.to_lowercase()).collect();
    let mut hits = Vec::new();
    for (i, alts) in TOKENS.iter().enumerate() {
        let hit = lines.iter().any(|l| alts.iter().any(|toks| toks.iter().all(|tok| l.contains(*tok))));
        if hit {
            hits.push(&TEXTS[i]);
        }
    }
    hits
}

pub struct CrashFixOption {
    pub problem_title: &'static str,
    pub explanation: &'static str,
    pub fix_label: &'static str,
    pub fix_script: &'static str,
    pub needs_root: bool,
}

pub fn get_active_fixes(out: &str) -> Vec<CrashFixOption> {
    let lines: Vec<String> = out.lines().map(|l| l.to_lowercase()).collect();
    let mut fixes = Vec::new();

    if lines.iter().any(|l| l.contains("out of memory") || l.contains("oom-killer")) {
        fixes.push(CrashFixOption {
            problem_title: t("Memoria RAM esaurita: il sistema ha chiuso dei programmi"),
            explanation: t("Il kernel ha attivato l'OOM Killer. Ottimizziamo la swappiness e liberiamo la cache."),
            fix_label: t("Ottimizza RAM e Swappiness"),
            fix_script: "sync; echo 3 > /proc/sys/vm/drop_caches; sysctl -w vm.swappiness=20 && echo 'vm.swappiness=20' > /etc/sysctl.d/99-debutility-oom.conf && echo '✔ Memoria ottimizzata'",
            needs_root: true,
        });
    }

    if lines.iter().any(|l| l.contains("failed to start") || l.contains("entered failed state")) {
        fixes.push(CrashFixOption {
            problem_title: t("Un servizio di sistema non si è avviato"),
            explanation: t("Uno o più servizi systemd sono in errore. Possiamo azzerare lo stato di errore."),
            fix_label: t("Resetta servizi systemd falliti"),
            fix_script: "systemctl reset-failed && echo '✔ Stato servizi falliti resettato'",
            needs_root: true,
        });
    }

    if lines.iter().any(|l| l.contains("failed to load firmware") || l.contains("firmware: failed")) {
        fixes.push(CrashFixOption {
            problem_title: t("Manca un firmware per una periferica"),
            explanation: t("Il kernel non trova i binari di alcune periferiche. Aggiorniamo i pacchetti firmware."),
            fix_label: t("Installa/aggiorna pacchetti firmware"),
            fix_script: "apt-get update && apt-get install -y firmware-linux firmware-misc-nonfree && echo '✔ Firmware aggiornati'",
            needs_root: true,
        });
    }

    if lines.iter().any(|l| l.contains("dpkg") && (l.contains("error") || l.contains("interrupted"))) {
        fixes.push(CrashFixOption {
            problem_title: t("Problemi con i pacchetti installati (DPKG/APT bloccato)"),
            explanation: t("Il gestore pacchetti è rimasto bloccato da un'installazione interrotta."),
            fix_label: t("Sblocca e ripara database pacchetti"),
            fix_script: "if command -v fuser >/dev/null 2>&1 && fuser /var/lib/dpkg/lock-frontend /var/lib/dpkg/lock >/dev/null 2>&1; then echo 'apt/dpkg è in esecuzione: attendi che finisca prima di riparare.'; exit 1; fi; rm -f /var/lib/dpkg/lock-frontend /var/lib/dpkg/lock && dpkg --configure -a && apt-get install -f -y && echo '✔ DPKG riparato'",
            needs_root: true,
        });
    }

    fixes
}

impl App {
    pub(crate) fn solutions_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, key: &'static str, report_ok: bool) {
        let Some(out) = self.out.get(key) else { return };
        let has_errors = out.lines().any(|l| {
            let l = l.trim();
            !l.is_empty() && !l.starts_with("-- ") && !l.starts_with("Hint:") && !l.starts_with("Notice:")
        });
        if !has_errors {
            if report_ok && !out.trim().is_empty() {
                ui.add_space(3.0);
                ui::card(ui, |ui| {
                    ui.label(
                        egui::RichText::new(t("Nessun errore trovato: il sistema non ha registrato errori in questo avvio."))
                            .size(17.0)
                            .color(ui::semantic_text(ui::GOOD)),
                    );
                });
            }
            return;
        }
        let hits = matching(out);
        let active_fixes = get_active_fixes(out);

        if hits.is_empty() && active_fixes.is_empty() && !report_ok {
            return;
        }

        ui.add_space(3.0);

        // Sezione Pulsanti di Correzione Automatica
        if !active_fixes.is_empty() {
            ui::card(ui, |ui| {
                ui.label(egui::RichText::new(t("Assistente Correzione Crash & Errori")).strong().size(19.0).color(ui::semantic_text(ui::WARN)));
                ui.label(t("In base agli errori riscontrati nei log, DebUtility può applicare le seguenti correzioni automatiche:"));
                
                for fix in active_fixes {
                    ui.add_space(3.0);
                    ui.label(egui::RichText::new(format!("• {}", t(fix.problem_title))).strong().size(16.0));
                    ui.label(t(fix.explanation));
                    ui.add_space(2.0);
                    
                    self.run_btn(
                        ui,
                        ctx,
                        t(fix.fix_label),
                        key,
                        fix.fix_script,
                        fix.needs_root,
                        Some(t("Vuoi applicare questa correzione automatica al sistema?")),
                    );
                }
            });
            ui.add_space(3.0);
        }

        // Sezione Soluzioni Testuali Consigliate
        ui::card(ui, |ui| {
            ui.label(egui::RichText::new(t("Possibili soluzioni")).strong().size(19.0));
            if hits.is_empty() {
                ui.label(t("Non ho una soluzione specifica per questi errori. Molti messaggi sono innocui se il computer funziona bene; se noti problemi, copia la riga dell'errore e cercala online."));
                return;
            }
            ui.label(egui::RichText::new(t("In base agli errori trovati ecco cosa puoi provare:")).color(ui::text_dim()));
            for (problem, fix) in hits {
                ui.add_space(3.0);
                ui.label(egui::RichText::new(format!("• {}", t(problem))).strong().size(17.0).color(ui::semantic_text(ui::WARN)));
                ui.label(egui::RichText::new(t(fix)).size(16.0));
            }
        });
    }
}

pub fn fix_missing_gpg_keys_script() -> String {
    r#"
    echo "▶ Analizzo i repository alla ricerca di chiavi GPG mancanti (NO_PUBKEY)..."
    MISSING_KEYS=$(apt update 2>&1 | grep -o 'NO_PUBKEY [A-F0-9]*' | awk '{print $2}' | sort -u)

    if [ -z "$MISSING_KEYS" ]; then
        echo "✔ Nessuna chiave GPG mancante rilevata nell'ultimo aggiornamento."
    else
        for key in $MISSING_KEYS; do
            echo "🔑 Chiave mancante trovata: $key. Recupero in corso..."
            gpg --no-default-keyring --keyring /etc/apt/trusted.gpg.d/debutility-fixed.gpg --keyserver keyserver.ubuntu.com --recv-keys "$key"
        done
        echo "▶ Aggiornamento indici APT con le nuove chiavi..."
        apt update
        echo "✔ Tutte le chiavi GPG sono state ripristinate con successo!"
    fi
    "#.to_string()
}
