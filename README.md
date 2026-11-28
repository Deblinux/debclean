# DebUtility 0.3.0

Clone di Stacer in Rust per Debian, con interfaccia grafica (egui): barra laterale scura, dashboard con indicatori circolari, tessere colorate e le schede del Deblinux Toolkit. Multilingua: Italiano, English, Español, Français, Deutsch, Português, Nederlands, Polski, Русский, Українська, Română e Čeština (elenco estendibile in `lang/langs.tsv`).

**Novità della versione corrente:** [DebUtility 0.3.0 — note di rilascio](docs/DebUtility_0.3.0_Note_rilascio.md)

**Manuale PDF per principianti (edizione 0.2.0):** [Guida completa a DebUtility v0.2.0](docs/DebUtility_0.2.0_Guida_completa.pdf)

**Novità in 5 lingue:** [Rimozione repository, punti di ripristino e blocco del menu laterale](docs/NUOVE_FUNZIONALITA_multilingua.md)

## Schede

**Dashboard**: stato del sistema, "Ottimizza ora", accessi rapidi, CPU / RAM / swap / disco con storico a 60 secondi

**Pulizia** (con sottoschede)
- Pulizia: file temporanei, cache APT, pacchetti orfani, browser (cache, cronologia, cookie), cache RAM, log, cache utente, cestino
- Pulizia avanzata: cartelle vuote e file da 0 byte nella home, con elenco, selezione e conferma prima di eliminare
- Risparmio energetico (solo notebook): Bilanciato, Performance, Powersave, Origine, con spiegazioni per principianti. Su un PC fisso la scheda lo segnala e non fa nulla
- Servizi di sistema: verifica stato, avvia, arresta, con filtro rapido, ordinamento per colonna e selezione multipla
- Ottimizzazione performance: analisi avvio, swappiness, TRIM, processi pesanti, ottimizza tutto
- Log sistema: ultimi eventi di journalctl

**Fixer**: controlli automatici con "Correggi" + riparazione APT, dipendenze, chiavi, integrità

**Aggiornamenti**: cerca e installa gli aggiornamenti dei pacchetti APT e delle app Flatpak (solo APT, solo Flatpak o tutto insieme), con avviso se serve riavviare

**Sistema**
- Info hardware: Info / Hardware e rete / Memoria RAM (marca, tipo e velocità dei moduli, con dmidecode) / Scheda video (modello, driver, memoria video, monitor) / Tutti i componenti (scheda madre, audio, rete, dischi, USB, input, webcam, monitor, batteria) / About
- Disco: tipo di disco (SSD, NVMe, HDD, USB...), analisi SMART, file grandi, spazio, stress test
- Processi, App installate (APT/Flatpak), Ottimizza RAM (zRAM) con filtro rapido, ordinamento per colonna, contatore elementi filtrati e selezione multipla coerente
- Temperature, con pulsante "Modalità di raffreddamento"
- Rete: diagnosi gateway/DNS/Internet e ripristino

**Amministratore**
- Kernel manager, governor CPU, swappiness, GRUB (con **Anteprima** delle modifiche), recovery
- Repository e driver: Driver manager che rileva l'hardware e propone i pacchetti; in «App repository» e «Repository Manager» puoi rimuovere o disabilitare righe di `/etc/apt/sources.list` e file in `/etc/apt/sources.list.d/`, con conferma, backup e aggiornamento degli elenchi APT
- Startup, permessi utente

**Sicurezza e dati**: firewall UFW, sicurezza avanzata, backup (con **Punti di ripristino**: elenco snapshot Timeshift e cartelle, selezione singola/multipla, rimozione con conferma e avanzamento), file shredder, log debug, impostazioni (lingua inclusa)

**Spiegazioni per principianti**: ogni scheda importante ha in alto un riquadro blu che spiega in breve a cosa serve e come usarla.

## Coda operazioni, tabelle e layout adattivo (0.3.0)

- **Coda FIFO**: le operazioni lunghe (scansioni, pulizie, aggiornamenti APT/Flatpak, rimozioni, ripristini) non vengono più scartate se il programma è occupato: vengono accodate ed eseguite una alla volta, nell'ordine in cui sono state richieste. La coda è limitata a 12 operazioni per evitare accumuli accidentali.
- **Tabelle principali più usabili**: Processi, App installate, Servizi di sistema e Storico APT hanno filtro rapido con pulsante di svuotamento, contatore compatto `X / Y`, intestazioni ordinabili con indicatore `▲/▼` e ordinamento coerente per tipo (alfabetico case-insensitive, numerico per CPU/RAM/PID, cronologico per data/ora). Dove applicabile, la selezione multipla usa controlli uniformi e azioni in blocco adattive allo spazio disponibile.
- **Pannello in basso**: quando ci sono operazioni attive o in attesa compare una barra discreta con l'attività in corso, il numero e le prime etichette di quelle in attesa e il pulsante «Annulla le operazioni in attesa». L'annullo rimuove solo i task in coda: quello in esecuzione continua normalmente fino alla fine.
- **Conferme**: confermare un'azione mentre un'altra è in corso la accoda invece di perderla.
- **Finestre compatte**: la dimensione minima è scesa da 1120×700 a 800×560. Sotto i 1040 px la sidebar diventa una striscia di icone con tooltip, mentre tabelle, griglie, campi di testo, grafici e KPI usano la larghezza disponibile invece di larghezze fisse.

## Rimozione repository, punti di ripristino e menu laterale

- **Rimozione/disabilitazione repository**: in «Repository e driver» → «App repository» o «Repository Manager» selezioni le righe `deb`/`deb-src` di `/etc/apt/sources.list` e dei file `.list`, oppure interi file `.list`/`.sources` di `/etc/apt/sources.list.d/`. «Rimuovi selezionati» elimina le righe o i file, «Disabilita selezionati» commenta le righe o rinomina i file con `.disabled`. Prima della modifica viene chiesta conferma, viene creato un backup in `/etc/apt/debutility-backups/` tramite `pkexec` e al termine vengono aggiornati gli elenchi APT, l'output e l'elenco selezionabile. Symlink e percorsi fuori dalle cartelle ammesse vengono ignorati; errori di permesso o di `apt-get update` sono mostrati senza bloccare la UI.
- **Punti di ripristino**: in «Backup → Punti di ripristino» premi «Aggiorna elenco» per leggere `timeshift --list --scripted` e le cartelle snapshot in `/timeshift/snapshots` (anche su backup esterni montati). La tabella mostra nome, data, tipo e dimensione; puoi selezionare una riga o spuntarne più, poi usare «Rimuovi selezionati» o «Rimuovi tutto». Ogni rimozione chiede conferma e viene eseguita con `pkexec`: gli snapshot Timeshift con `timeshift --delete --snapshot <id> --yes`, le cartelle solo se figlie dirette delle root ammesse, con nomi validati e symlink ignorati. La scansione e la rimozione mostrano la percentuale nella barra di avanzamento, tengono bloccato il menu laterale e aggiornano l'elenco al termine. Se Timeshift non è installato vengono comunque elencate le cartelle presenti.
- **Blocco del menu laterale**: solo mentre un'operazione è realmente in esecuzione (aggiornamenti, scansioni, installazioni, rimozione repository, scansione/rimozione dei punti di ripristino, job in background) le voci del menu a sinistra non sono cliccabili e compare un avviso. Le operazioni richieste in quel momento vengono comunque accodate dal pannello in basso; il menu si riabilita appena il job corrente termina e parte il successivo.

Queste funzioni sono descritte anche nella guida interna dell'applicazione e nel documento [multilingua](docs/NUOVE_FUNZIONALITA_multilingua.md) (Italiano, English, Español, Français, Deutsch).

**Errori di sistema**: in Log debug (e nel Log sistema) gli errori noti del journal vengono spiegati e accompagnati da una soluzione. Le regole sono in `src/solutions.rs`.

**Chiusura**: premendo la X in alto a destra il programma chiede conferma e, se c'è un'operazione in corso (pulizia, aggiornamento...), avvisa prima di chiudere.

**Guida**: spiega in linguaggio semplice tutte le funzioni di DebUtility, con avvertenze e ricerca. Per aggiungere o modificare un testo della guida vedi `src/pages_guide.rs`, poi rigenera le traduzioni come descritto sotto

## Compilare

Installa la toolchain con rustup:

    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

Dipendenze di sviluppo:

    sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev \
        libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev

Build ed esecuzione:

    cargo build --release
    ./target/release/debutility

Per compatibilità con tutte le Debian compila sulla versione più vecchia che vuoi supportare.

## Lingue

All'avvio DebUtility rileva la lingua da `LC_ALL` / `LC_MESSAGES` / `LANGUAGE` / `LANG`; si cambia in Impostazioni → Lingua. Sono accettati codici `ll`, `ll_CC` e `ll_CC.UTF-8` (con eventuali modifier come `@euro`): la ricerca prova il locale completo, poi la lingua madre, poi la lingua di fallback (inglese). Non serve più toccare il codice per supportare nuovi locale.

L'elenco autorevole delle lingue è `lang/langs.tsv` (`codice<TAB>nome mostrato`, prima riga = lingua originale delle chiavi). Le traduzioni incorporate stanno in `lang/strings.tsv`, generato da `lang/strings.src` nello stesso ordine. I font di sistema per i caratteri non latini vengono aggiunti automaticamente all'avvio (`src/fonts.rs`), con fontconfig e fallback per Debian.

Oltre alle sei lingue originarie (Italiano, English, Español, Français, Deutsch, Português) sono disponibili Nederlands (`nl`), Polski (`pl`), Русский (`ru`) e Українська (`uk`), insieme a Română (`ro`) e Čeština (`cs`): tutte le 12 lingue hanno le 1.853 voci tradotte, senza modificare il numero di righe. `python3 tools/build_strings.py` riporta `COLONNE ERRATE: 0`, nessuna traduzione mancante e nessun segnaposto diverso; il controllo confronta tutti i token tra parentesi graffe (`{}`, `{0}`, `{name}`, ecc.). Le voci presenti solo in `lang/strings.src` e non più usate vengono segnalate ma non scritte in `lang/strings.tsv`.

Per correggerle o aggiungere una lingua:

1. aggiungi una riga in `lang/langs.tsv`, per esempio `pt_BR<TAB>Português (Brasil)`;
2. aggiungi la colonna corrispondente in `lang/strings.src`, nello stesso ordine (puoi anche lasciarla vuota e tradurre in seguito);
3. esegui, da qualsiasi cartella:

       python3 tools/extract_keys.py     # rigenera l'elenco delle frasi (lang/keys.txt)
       python3 tools/build_strings.py    # rigenera lang/strings.tsv e segnala frasi mancanti

Non ci sono array o costanti da aggiornare in `src/i18n.rs`: numeri di colonne, nomi e selettore della GUI derivano tutti da `lang/langs.tsv`. Se una frase visibile non viene intercettata dagli schemi di `tools/extract_keys.py`, aggiungila a `lang/extra_keys.txt` (una chiave per riga) e poi esegui di nuovo i due script.

Le frasi senza traduzione vengono mostrate con il testo originale UTF-8. I messaggi di registro e l'output dei comandi di sistema restano nella lingua originale.

## Pacchetti opzionali usati a runtime

`policykit-1` / `pkexec`, `zenity` (selettore di file e cartelle del File shredder), `dmidecode` (marca e tipo della RAM), `mesa-utils` (informazioni OpenGL), `x11-xserver-utils` (xrandr, monitor), `pciutils`, `usbutils`, `smartmontools`, `ufw`, `timeshift`, `flatpak`, `lm-sensors`, `rsync`, `power-profiles-daemon` (risparmio energetico).

## Sicurezza

La password di root non viene mai salvata: le operazioni privilegiate passano da `pkexec` (Polkit). Le operazioni distruttive chiedono sempre conferma.

## Compatibilità

DebUtility legge configurazione, pianificazione e stato dell'ultima esecuzione anche dai vecchi percorsi `debclean`; al primo salvataggio scrive i nuovi percorsi `debutility`. Le vecchie unità systemd utente `debclean-maint.timer` e `debclean-maint.service` vengono riconosciute e disattivate quando si configura il nuovo timer.
