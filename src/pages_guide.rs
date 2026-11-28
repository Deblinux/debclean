use crate::i18n::{t, tf};
use crate::{ui, App};
use eframe::egui;

/// (titolo, spiegazione, avvertenza: stringa vuota se non serve)
type Entry = (&'static str, &'static str, &'static str);

// GUIDE-BEGIN
const GUIDE: &[(&str, &[Entry])] = &[
    (
        "PANORAMICA",
       &[
            (
                "Dashboard",
                "È la schermata iniziale. Mostra in un colpo d'occhio se il sistema è in buono stato, l'uso di CPU, RAM, swap e disco e il loro storico degli ultimi 60 secondi. «Ottimizza ora» esegue la pulizia con le voci già selezionate; le tessere portano alle funzioni principali.",
                "",
            ),
            (
                "Perché è lento adesso?",
                "Esegue una diagnostica rapida in un clic su carico CPU e load average, uso di RAM e swap, spazio libero sulle partizioni, processi più esosi e unità systemd in stato failed, offrendo pulsanti diretti per aprire la sezione relativa.",
                "",
            ),
            (
                "Barre di avanzamento",
                "Le operazioni lunghe (scansioni, pulizie, aggiornamenti, installazioni, rimozione repository, scansione e rimozione dei punti di ripristino) mostrano una barra con percentuale e la descrizione del passaggio in corso. Finché il lavoro è attivo il menu laterale sinistro è disabilitato e compare un avviso: attendi il completamento prima di cambiare scheda. Il menu si riabilita automaticamente appena l'operazione finisce e i risultati compaiono da soli.",
                "",
            ),
        ],
    ),
    (
        "MANUTENZIONE",
        &[
            (
                "Pulizia",
                "Libera spazio eliminando i file inutili: temporanei, cache di APT, dei browser, miniature di sistema, cache di app popolari (Discord, Telegram, Steam, Slack), pacchetti orfani, log vecchi, cache utente e cestino. Spunta le voci, premi «Analizza» per vedere quanto spazio recuperi e poi «Pulisci selezionati».",
                "La cancellazione della cache delle chat o dei browser richiede la chiusura delle relative applicazioni.",
            ),
            (
                 "Ottimizza in sicurezza",
                 "Percorso guidato per principianti diviso in due passaggi: mostra prima un'anteprima di ciò che farà e poi il riepilogo di quanto eseguito. Ripara pacchetti e dipendenze APT/dpkg, svuota la cache dei browser, rimuove i file .deb obsoleti, i vecchi log in /var/log e le miniature, e compatta i database SQLite senza toccare password, preferiti, cronologia, cookie o Cestino.",
                  "Le operazioni su cache e database dei browser vengono saltate o richiedono che il browser sia chiuso.",
            ),
            (
                "Pulizia Profonda Browser (SQLite)",
                "Esegue VACUUM e REINDEX sui database interni dei browser (places.sqlite, History, Web Data di Firefox, Chrome, Chromium e Brave) verificando prima che siano chiusi. Velocizza l'avvio e la barra degli indirizzi senza cancellare cronologia o segnalibri.",
                "Richiede che il browser sia completamente chiuso prima di avviare l'ottimizzazione.",
            ),
            (
                "Residui di sistema",
                "Scansiona e rimuove le configurazioni residue dei pacchetti disinstallati (pacchetti RC), i backup di configurazione dimenticati, i runtime e le app Flatpak non più utilizzati, liberando spazio prezioso. Vedi l'elenco completo, scegli cosa eliminare e confermi prima che venga cancellato qualcosa. I singoli backup di configurazione vengono spostati in quarantena, non eliminati subito.",
                "",
            ),
            (
                "Snapshot pre-pulizia",
                "In «Pulizia», «Residui di sistema», «Fixer» e «Pulizia Chirurgica» puoi spuntare «Crea snapshot di sicurezza prima di procedere». Se Timeshift o Snapper è installato, DebUtility crea un'istantanea del sistema prima di rimuovere qualsiasi elemento.",
                "L'opzione è disponibile solo se Timeshift o Snapper è installato. Se la creazione dello snapshot fallisce, la pulizia viene annullata e nessun file viene rimosso.",
            ),
            (
                "Gestione AppImage",
                "Gestisce le applicazioni portabili in formato AppImage. Permette di scansionare le cartelle principali, integrarle automaticamente nel menu delle applicazioni di sistema con un file .desktop, avviarle o rimuoverle facilmente.",
                "",
            ),
            (
                "Pulizia avanzata",
                "Cerca nella tua cartella personale le cartelle vuote e i file da 0 byte, oppure individua i residui di configurazione dei pacchetti disinstallati (rc), i runtime Flatpak inutilizzati e le configurazioni utente orfane in ~/.config di programmi non più installati sul computer. Vedi l'elenco completo, scegli cosa eliminare e confermi prima che venga cancellato qualcosa.",
                "",
            ),
            (
                "File duplicati",
                "Scansiona le cartelle selezionate per individuare i file identici tramite impronta digitale SHA-256, indipendentemente dal nome. Lascia deselezionata la prima copia di ogni gruppo e spunta automaticamente i doppioni: quelli selezionati vengono spostati in quarantena, non cancellati subito.",
                "Controlla sempre l'elenco prima di confermare: il primo file di ogni gruppo viene conservato. Le copie spostate restano recuperabili per 7 giorni dalla scheda Quarantena / Ripristino.",
            ),
            (
                "Quarantena / Ripristino",
                "Raccoglie i singoli file spostati via dalle schede File duplicati e Residui di sistema (backup di configurazione). Per ogni file mostra il percorso originale, la data, la dimensione e il modulo di origine. Puoi ripristinarlo esattamente dov'era, eliminarlo definitivamente dopo averlo controllato oppure lasciare che scada automaticamente dopo 7 giorni. La cartella è privata e non sostituisce gli snapshot di sistema.",
                "Livello di rischio: con cautela. Il ripristino non sovrascrive un file già presente nel percorso originale: se esiste, prima sposta o rinomina quel file. Dopo la scadenza i file vengono eliminati senza recupero; per operazioni su cartelle o su più file usa gli snapshot di Timeshift/Snapper, non questa scheda.",
            ),
            (
                "Cronologia e ripristino",
                "Riunisce in un unico elenco i backup delle sorgenti APT creati in /etc/apt/debutility-backups/, i file .bak di GRUB e dei file di sistema e le voci ancora presenti in Quarantena. Puoi filtrare per tipo, cercare per percorso o stato, ordinare per data e selezionare una voce per ripristinarla o eliminarla. La vista segnala quali voci richiedono autorizzazione di amministratore.",
                "Il ripristino sovrascrive il file attuale con la copia salvata e, per GRUB, aggiorna la configurazione di avvio: la conferma è obbligatoria. L'eliminazione è definitiva. Se l'elenco in /etc non è leggibile, usa «Ricarica con privilegi»; un rifiuto della finestra pkexec lascia il sistema invariato e mostra un messaggio di annullamento.",
            ),
            (
                "Pianificatore di manutenzione",
                "Crea un timer systemd utente che avvia direttamente DebUtility in modalità headless (--clean --yes --json) ogni giorno, ogni settimana o una volta al mese alle 03:30. Puoi scegliere le categorie da includere: cache utente e dei browser, miniature, residui APT/Flatpak, log, file temporanei e file/cartelle vuote. La scheda mostra lo stato del timer, la prossima esecuzione, l'esito dell'ultima pulizia con lo spazio liberato e il log; «Esegui test ora» avvia subito il servizio senza aspettare la scadenza.",
                "Le categorie contrassegnate con • richiedono l'autorizzazione di amministratore tramite pkexec durante l'esecuzione pianificata. Disattivando o rimuovendo la pianificazione vengono fermati sia il timer sia il servizio; l'ultimo log resta disponibile per la diagnosi.",
            ),
            (
              "Risparmio energetico",
              "Solo per i notebook: sceglie come il processore usa l'energia. Bilanciato è per l'uso di tutti i giorni, Performance dà la massima velocità consumando di più, Powersave fa durare di più la batteria, Origine ripristina le impostazioni precedenti. Su un PC fisso la funzione non è disponibile.",
              "Le impostazioni valgono fino al riavvio del computer.",
            ),
            (
                "Servizi di sistema",
                "I servizi sono programmi che girano sempre in background (rete, SSH, Bluetooth, firewall...). Puoi verificare se un servizio è attivo, avviarlo o arrestarlo per la sessione corrente.",
                "Fermare servizi critici come networking o NetworkManager può togliere la connessione a Internet.",
            ),
            (
                "Ottimizzazione performance",
                "Raccoglie gli strumenti per velocizzare il computer: analisi dell'avvio, regolazione della swappiness, TRIM per gli SSD, elenco dei processi più pesanti e «Ottimizza tutto» per una manutenzione completa in un colpo solo.",
                "Alcune operazioni richiedono i privilegi di amministratore.",
            ),
            (
                "Log sistema",
                "Mostra gli ultimi eventi e messaggi di sistema registrati da journalctl, con filtri per errori, avvio corrente e kernel. Include l'Assistente Correzione Crash & Errori, che riconosce automaticamente i guasti critici (memoria esaurita OOM, pacchetti bloccati, servizi falliti, firmware mancanti) e propone pulsanti di riparazione automatica con un clic.",
                "",
            ),
            (
                "Fixer",
                "Controlla lo stato di salute del sistema (pacchetti, dipendenze, servizi, spazio, orologio...) e per ogni problema offre il pulsante «Correggi», incluso lo sblocco rapido dei pacchetti bloccati in «hold» e la riparazione automatica di grub-pc quando serve configurare il disco di avvio. Dispone inoltre di una scheda dedicata per la risoluzione automatica delle chiavi GPG mancanti (NO_PUBKEY) dai repository PPA. «Correggi tutto» applica tutte le correzioni insieme; gli strumenti manuali riparano il database APT, le dipendenze e le chiavi, verificano l'integrità dei pacchetti, permettono di controllare o ripulire i file .dpkg-old, .dpkg-dist, .ucf-dist e i pacchetti residui in stato rc e offrono il controllo o il riavvio rapido delle unità systemd fallite (systemctl --failed).",
                "Le correzioni modificano pacchetti e servizi: leggi il problema prima di applicarle, verifica i file .dpkg-dist prima di eliminarli se hai personalizzato una configurazione e controlla quali unità sono fallite prima di riavviarle in blocco.",
            ),
            (
                "Aggiornamenti",
                "Controlla se ci sono aggiornamenti per i pacchetti APT (il sistema Debian e i suoi programmi) e per le app Flatpak. Puoi aggiornare solo APT, solo Flatpak o tutto con un clic.",
                "Durante l'aggiornamento non spegnere il computer: alla fine potrebbe servire un riavvio.",
            ),
            (
                "Storico APT",
                "Analizza il registro di sistema (/var/log/apt/history.log) per mostrare l'elenco cronologico di tutte le transazioni APT eseguite. Puoi filtrare rapidamente per data, comando o pacchetti, ordinare per data/ora o comando e vedere il contatore «X / Y» delle transazioni visibili. Per ogni operazione visualizza data, comando esatto, pacchetti toccati e l'eventuale annullamento.",
                "",
            ),
            (
                "Pacchetti Snap",
                "Gestisci i pacchetti Snap installati sul sistema. Puoi visualizzare l'elenco dei pacchetti con le relative versioni ed editori, aggiornarli tutti contemporaneamente o rimuovere singolarmente quelli non necessari.",
                "La rimozione richiede i privilegi di amministratore e l'esecuzione del demone snapd sul sistema.",
            ),
            (
                "Sviluppo & Container",
                "Analizza Docker/Podman e gli strumenti di sviluppo (Rust/Cargo, Node.js, Python). Mostra immagini e container inutilizzati, cartelle target/ e node_modules inattive e cache dei gestori di pacchetti, con dimensione e data dell'ultima modifica. Spunta gli elementi da eliminare e premi «Pulisci selezionati».",
                "Le cartelle target/ e node_modules sono deselezionate di default: ricostruirle può richiedere tempo. Il prune dei container rimuove definitivamente immagini e volumi orfani.",
            ),
        ],
    ),
    (
        "SISTEMA",
        &[
            (
                "Info hardware",
                "Sei schede: Info (sistema, memoria, dischi, rete), Hardware e rete (processore, scheda madre, modelli dei dischi, gateway), Memoria RAM (marca, tipo e velocità di ogni modulo), Scheda video (modello, driver, memoria video, monitor), Tutti i componenti (elenco completo del computer) e About. Serve a sapere esattamente che computer hai.",
                "",
            ),
            (
                "Disco",
                "Mostra il tipo di ogni disco (SSD, NVMe, HDD, USB). Analisi disco controlla la salute con SMART, File grandi trova i file pesanti e inutilizzati, Spazio disco calcola la dimensione delle cartelle e Stress test mette sotto sforzo CPU e RAM per scoprire instabilità.",
                "Lo stress test porta il computer al limite: salva il lavoro prima di avviarlo.",
            ),
            (
                "Guardiano Predittivo",
                "Analisi predittiva dello stato di salute di SSD/NVMe e batteria: calcola l'usura reale delle celle, i cicli di carica e i Terabyte scritti per prevenire guasti hardware.",
                "",
            ),
            (
                "Analizzatore spazio",
                "Esplora le cartelle del file system per individuare a colpo d'occhio quali file o directory occupano più spazio, permettendo di navigare tra le directory e analizzarne i contenuti in modo visivo.",
                "",
            ),
            (
                "Analizzatore Disco (Treemap)",
                "Scansiona una cartella e ne visualizza il contenuto come mappa di rettangoli proporzionali alla dimensione: clic su un rettangolo per entrare nella cartella, «Torna su» per risalire. Il pannello «Top 20 file più pesanti» elenca i file più grandi dell'intero sottoalbero.",
                "Il tasto destro offre «Sposta nel cestino» ed «Elimina definitivamente»: quest'ultima operazione non è reversibile.",
            ),
            (
                "Processi",
                "Elenca i programmi in esecuzione con la CPU e la memoria che usano. Puoi filtrare rapidamente per nome, PID o utente, ordinare le colonne (PID, utente, CPU, memoria, comando) con indicatore ▲/▼ e selezionare più processi per terminarli o chiuderli con la forza in blocco. Il contatore «X / Y» mostra quanti elementi sono visibili rispetto al totale.",
                "Chiudere un processo di sistema può rendere instabile il computer. Le azioni in blocco si applicano solo ai processi selezionati visibili.",
            ),
            (
                "App installate",
                "Elenca i pacchetti APT e le app Flatpak installati e permette di disinstallarli. La ricerca filtra in tempo reale, le intestazioni Pacchetto e Versione sono ordinabili, e puoi selezionare più elementi con selezione/deselezione rapida per disinstallarli in un'unica conferma. Il contatore «X / Y» indica gli elementi filtrati rispetto al totale della scheda.",
                "La disinstallazione chiede conferma ma non si può annullare. Nelle azioni multiple verifica bene la selezione prima di confermare.",
            ),
            (
                "Container (Docker / Podman)",
                "Permette di monitorare i container in esecuzione o fermi, leggere gli ultimi log, avviarli, arrestarli ed eseguire «Prune» per liberare lo spazio occupato da immagini orfane e volumi abbandonati.",
                "Il Prune elimina definitivamente i container fermi e le immagini non associate ad alcun contenitore.",
            ),
            (
                "Ottimizza RAM",
                "Crea una zRAM, cioè una memoria compressa dentro la RAM, per evitare rallentamenti quando la memoria scarseggia. Scegli la dimensione con il cursore e premi «Ottimizza RAM»; «Disattiva zRAM» la rimuove.",
                "La configurazione vale fino al riavvio.",
            ),
            (
                "Temperature",
                "Mostra le temperature dei sensori. Il pulsante «Attiva modalità di raffreddamento» limita la velocità del processore per produrre meno calore e far girare più piano le ventole; «Disattiva e ripristina» torna alla velocità normale.",
                "",
            ),
            (
                "Rete",
                "Controlla con tre spie il gateway (router), il DNS e la connessione a Internet. «Ripristina rete e Wi-Fi» riavvia NetworkManager per risolvere i problemi di connessione.",
                "La connessione cade per qualche secondo.",
            ),
        ],
    ),
    (
        "AMMINISTRATORE",
        &[
            (
                "Kernel, Grub e Recovery",
                "Include quattro schede e il controllo dei kernel orfani: Kernel manager rimuove i vecchi kernel (quello in uso resta protetto) mostrando lo spazio recuperabile in MB/GB per ciascuno, Kernel e prestazioni regola governor e swappiness, Grub modifica i parametri di avvio con anteprima, salvataggio del backup (/etc/default/grub.debutility.bak) e ripristino rapido del backup, e Recovery offre strumenti di emergenza per audio, rete, ripristino/reinstallazione di GRUB e Flathub.",
                "Modificare GRUB può impedire l'avvio del computer: controlla sempre l'anteprima e tieni un backup.",
            ),
            (
                "Kernel & Boot",
                "Mostra lo spazio occupato e libero della partizione /boot e l'elenco dei pacchetti linux-image-* e linux-headers-* installati con le relative dimensioni. Premi «Scansiona kernel e /boot», spunta i kernel vecchi da rimuovere e poi «Rimuovi kernel selezionati»: la rimozione usa pkexec apt-get purge e aggiorna GRUB.",
                "Il kernel in esecuzione (uname -r) e il secondo kernel più recente sono protetti: mostrano il badge verde «In uso - Protetto» e non possono essere selezionati. Lascia sempre almeno un kernel di riserva funzionante.",
            ),
            (
                "Repository e driver",
                "Driver manager rileva l'hardware e propone i driver e i firmware da installare. App repository mostra, aggiunge, rimuove e disabilita le sorgenti software; Repository Manager abilita i Debian Backports, aggiorna gli elenchi dei pacchetti e offre gli stessi strumenti di rimozione sicura. Il Gestore Chiavi GPG verifica la scadenza delle chiavi, le rinnova o permette di disabilitare e riabilitare rapidamente chiavi e repository associati.",
                "Aggiungi solo repository di cui ti fidi e fai attenzione quando disabiliti o riabiliti sorgenti APT.",
            ),
            (
                "Rimozione e disabilitazione repository",
                "In «Repository e driver» → «App repository» o «Repository Manager» puoi selezionare le righe di /etc/apt/sources.list e dei file .list in /etc/apt/sources.list.d/, oppure interi file .list/.sources. «Rimuovi selezionati» elimina le righe o i file, «Disabilita selezionati» commenta le righe o rinomina i file con il suffisso .disabled. Prima di ogni modifica DebUtility crea un backup in /etc/apt/debutility-backups/ con pkexec, lavora in background senza bloccare l'interfaccia, poi aggiorna gli elenchi APT e l'elenco visualizzato.",
                "Controlla bene cosa selezioni: rimuovere repository ufficiali può lasciare il sistema senza aggiornamenti. I symlink e i percorsi fuori da /etc/apt/sources.list* vengono ignorati per evitare modifiche non sicure. Se apt-get update segnala errori, ripristina il file dal backup.",
            ),
            (
                "Startup",
                "Elenca le applicazioni che partono automaticamente al login. Togli la spunta a quelle che non ti servono per ridurre i tempi di avvio; puoi riattivarle quando vuoi.",
                "",
            ),
            (
                "Avvio e Servizi",
                "Permette di attivare o disattivare i programmi avviati automaticamente al login (file .desktop) con ricerca/filtro rapido per nome o file, ricaricare l'elenco e gestire le estensioni desktop dalla scheda dedicata.",
                "Disattivare un servizio o un programma di avvio automatico può impedire notifiche, sincronizzazioni o funzionalità del desktop: verifica sempre a cosa serve prima di disabilitarlo.",
            ),
            (
                "Dischi e condivisioni",
                "Mostra i file system montati e le condivisioni di rete (NFS/SMB) attive sul sistema, con punti di mount, tipi di filesystem e opzioni.",
                "",
            ),
            (
                "Tempi di avvio",
                "Analizza i servizi di sistema ordinati per tempo di caricamento all'accensione tramite systemd-analyze blame e gestisce i programmi all'avvio utente (~/.config/autostart/) con supporto per i ritardi (delay) e l'aggiunta personalizzata.",
                "Disabilitare servizi essenziali di rete o di sistema può compromettere il corretto avvio del computer.",
            ),
            (
                "Permessi utente",
                "Abilita o disabilita l'accesso del tuo utente a funzioni speciali: amministratore (sudo), porte seriali per Arduino e USB, VirtualBox e Docker.",
                "Servono un logout o un riavvio perché le modifiche diventino attive.",
            ),
            (
                "Gestore Pacchetti",
                "Cerca pacchetti nei repository ufficiali APT, visualizza lo stato (installato o disponibile) e la descrizione dettagliata. Permette di installare, rimuovere o purgare qualsiasi pacchetto con un clic, sfruttando i permessi di amministrazione in sicurezza.",
                "La rimozione o la purga di pacchetti di sistema essenziali può compromettere il funzionamento del sistema operativo.",
            ),
            (
                "Temi e Font Utente",
                "Gestisce i temi visivi dell'interfaccia, i font personalizzati e la configurazione delle icone utente per adattare l'aspetto grafico di DebUtility alle tue preferenze.",
                "",
            ),
            (
            "Editor Menu",
            "Gestisce i lanciatori utente (.desktop), permette il backup e il ripristino rapido in un archivio compresso (.tar.gz) di tutte le personalizzazioni e dei file di configurazione dei Window Manager (IceWM, JWM, Openbox, Fluxbox), e rigenera i menu testuali per window manager leggeri e ambienti standard.",
        "",
            ),
            (
                "Gestore File di Sistema",
                "Modifica file di configurazione critici di sistema (/etc/fstab, /etc/hosts, /etc/environment). Viene creato automaticamente un backup (.bak) e validata la sintassi prima di salvare per evitare blocchi del boot.",
                "",
            ),
            (
                "Servizi di sistema",
                "I servizi sono programmi che girano sempre in background (rete, SSH, Bluetooth, firewall...). Puoi verificare se un servizio è attivo, avviarlo o arrestarlo per la sessione corrente.",
                "Fermare servizi critici come networking o NetworkManager può togliere la connessione a Internet.",
            ),
        ],
    ),
    (
        "SICUREZZA E DATI",
        &[
            (
                "Sicurezza e firewall",
                "Firewall UFW attiva o disattiva la protezione contro gli accessi non autorizzati dalla rete. Sicurezza avanzata elenca i servizi in ascolto, scansiona le porte locali e mostra i log del firewall.",
                "Disattivare il firewall espone di più il PC alla rete.",
            ),
            (
        "Privacy & Telemetry Guard",
        "Scansiona il sistema alla ricerca di agenti di telemetria, tracker o demoni di diagnostica invisibili, permettendoti di disattivarli o bloccarli con un clic. Include inoltre un blocco rapido dei principali server di telemetria a livello locale tramite il file hosts.",
        "Disattivare demoni di telemetria di sistema o bloccare domini di rete potrebbe alterare la reportistica di diagnostica di alcune applicazioni di terze parti.",
    ),
            (
                "Hardening di sicurezza",
                "Applica best-practice di sicurezza enterprise per blindare il kernel Linux, proteggere la rete contro attacchi comuni (SYN flood, spoofing), verificare i permessi critici e controllare le porte aperte.",
                "",
            ),
            (
                "Disinstallazione Chirurgica",
                "Monitora e pulisce a fondo l'intero ciclo di vita di un'applicazione. Rimuove non solo il pacchetto ma anche tutti i frammenti invisibili sparsi nel sistema (regole udev, file in /etc, registrazioni D-Bus, policy Polkit e script cron dimenticati), impedendo l'accumulo di rifiuti invisibili.",
                "",
            ),
            (
                "Backup",
                "File e cartelle copia i tuoi dati in una destinazione a scelta. Pacchetti di sistema salva l'elenco dei pacchetti e dei repository e li ripristina dopo una reinstallazione. Punti di ripristino crea e ripristina istantanee dell'intero sistema con Timeshift e permette di elencare e rimuovere quelle esistenti.",
                "Ripristinare un punto riporta il sistema indietro: le modifiche successive vanno perse.",
            ),
            (
                "Gestione dei punti di ripristino",
                "In «Backup → Punti di ripristino» premi «Aggiorna elenco»: DebUtility legge `timeshift --list --scripted` e cerca anche le cartelle snapshot in /timeshift/snapshots e nella posizione dei backup esterni montati. La tabella mostra nome, data, tipo e dimensione di ogni punto, con selezione singola o multipla. «Rimuovi selezionati» cancella solo gli elementi spuntati, «Rimuovi tutto» li rimuove tutti: in entrambi i casi compare una conferma e la rimozione viene eseguita con pkexec. Durante scansione e cancellazione la barra di avanzamento mostra la percentuale e il menu laterale resta bloccato; a fine operazione l'elenco si aggiorna da solo. Se Timeshift non è installato vengono comunque mostrate le cartelle snapshot presenti.",
                "Pericolo: eliminare uno snapshot è irreversibile e può togliere l'unica copia di sicurezza del sistema. Controlla nome, data e dimensione prima di confermare; non usare la rimozione tutto se hai bisogno di tornare a una versione precedente.",
            ),
            (
                "Kit di Migrazione (1-Click)",
                "Esporta l'intero profilo della macchina in un unico archivio compresso (.tar.gz) contenente pacchetti APT manuali, app Flatpak installate, repository aggiuntivi (/etc/apt/sources.list.d/), lanciatori utente (.desktop) e configurazioni dei Window Manager, includendo uno script automatico per ripristinare tutto sul nuovo sistema.",
                "",
            ),
            (
                "File shredder",
                "Sovrascrive più volte un file o una cartella con dati casuali e poi li elimina, in modo che non siano recuperabili. Scegli il numero di passaggi.",
                "È irreversibile; su SSD e file system con snapshot la sovrascrittura non dà garanzie complete.",
            ),
            (
                "Log debug",
                "Mostra le operazioni eseguite da DebUtility e gli errori del journal di sistema; se trova errori noti ti suggerisce come risolverli. Puoi mettere in pausa il registro, pulirlo o ridurre lo spazio occupato su disco dai log di journald impostando una soglia massima in MB.",
            "",
            ),
            (
                "Report diagnostica",
                "Raccoglie automaticamente lo stato dell'hardware, lo spazio disco, eventuali pacchetti APT bloccati e gli errori recenti di sistema in un unico file HTML formattato e pronto per essere condiviso o stampato in PDF.",
    "",
),
            (
                "Impostazioni",
"Qui scegli la lingua dell'interfaccia, attivi la pulizia rapida della cache utente all'avvio e puoi esportare o importare l'intero profilo delle preferenze di DebUtility in un file TOML per trasferirlo facilmente su altri sistemi. DebUtility non salva la password di root: usa pkexec e il sistema chiede l'autorizzazione quando serve.",
                "",
            ),
        ],
    ),
    (
        "MODALITÀ CLI",
        &[
            (
                "Modalità CLI / headless",
                "Avvia DebUtility senza interfaccia grafica: è utile su server senza X11/Wayland, negli script e nelle automazioni. Senza argomenti si apre sempre la GUI; con --scan, --clean o --dry-run i comandi vengono gestiti prima dell'avvio della GUI e prima del lock di istanza singola.",
                "",
            ),
            (
                "--scan e --dry-run",
                "--scan mostra il riepilogo della scansione sicura (cache, residui APT/Flatpak/Snap, file temporanei e file vuoti) con lo spazio totale recuperabile. --dry-run elenca esattamente cosa verrebbe rimosso senza cancellare nulla.",
                "Aggiungi --json per ottenere un output strutturato: in quel modo i log superflui non finiscono su stdout.",
            ),
            (
                "--clean --yes",
                "Esegue la pulizia sicura in modo non interattivo: richiede --yes per evitare cancellazioni accidentali. Senza conferma il programma mostra un avviso e termina con codice 2.",
                "Le revisioni Snap disabilitate vengono rimosse solo dalla GUI.",
            ),
            (
                "--include e --exclude",
                "Limita la pulizia alle categorie indicate o esclude alcune categorie dalla selezione predefinita. Gli identificatori sono: user_cache, browser_cache, thumbnails, empty_dirs, empty_files, apt_cache, old_logs, temp, journal, rc_packages e flatpak_unused. Esempio: debutility --clean --yes --json --include=user_cache,empty_dirs.",
                "Il Pianificatore usa --include con le categorie scelte nella scheda.",
            ),
            (
                "Codici di uscita e JSON",
                "Il codice 0 indica successo, 1 un errore operativo, 2 argomenti non validi o --yes mancante. Con --json i campi sono mode, timestamp, status, exit_code, selected, total_recoverable_bytes, freed_bytes, freed_human, entries, logs e warnings. Con --clean viene scritto anche lo stato dell'ultima esecuzione in ~/.local/state/debutility, dove lo legge il Pianificatore.",
                "",
            ),
        ],
    ),
    (
        "AIUTO",
        &[(
            "Guida",
            "Questa schermata spiega ogni funzione di DebUtility. Usa la ricerca per trovare subito l'argomento che ti interessa.",
            "",
        )],
    ),
];
// GUIDE-END

impl App {
    pub(crate) fn ui_guide(&mut self, ui: &mut egui::Ui) {
        ui::title(ui, t("Guida di DebUtility"));
        ui::card(ui, |ui| {
            ui.label(
                egui::RichText::new(tf("DebUtility v{}", &[&crate::APP_VERSION]))
                    .size(17.0)
                    .strong()
                    .color(ui::accent_text()),
            );
            ui.add_space(2.0);
            ui.label(egui::RichText::new(t("DebUtility riunisce in un'unica applicazione pulizia, riparazione, monitoraggio e amministrazione del sistema. Quando una funzione richiede i privilegi di amministratore, il sistema mostra una finestra per l'autorizzazione; le operazioni delicate chiedono sempre conferma prima di procedere. Qui sotto trovi la spiegazione di ogni funzione.")).size(17.0));
        });
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.label(t("Cerca nella guida:"));
            ui.text_edit_singleline(&mut self.guide_filter);
        });

        let q = self.guide_filter.trim().to_lowercase();
        let searching = !q.is_empty();
        let hit = |e: &Entry| -> bool {
            !searching
                || t(e.0).to_lowercase().contains(&q)
                || t(e.1).to_lowercase().contains(&q)
                || t(e.2).to_lowercase().contains(&q)
        };

        let mut shown = 0;
        for (group, entries) in GUIDE.iter() {
            let hits: Vec<&Entry> = entries.iter().filter(|e| hit(e)).collect();
            if hits.is_empty() {
                continue;
            }
            ui.add_space(5.0);
            ui.label(egui::RichText::new(t(*group)).size(14.0).strong().color(ui::text_dim()));
            ui.add_space(2.0);
            for e in hits {
                shown += 1;
                ui::card(ui, |ui| {
                    egui::CollapsingHeader::new(egui::RichText::new(t(e.0)).strong().size(19.0))
                        .open(if searching { Some(true) } else { None })
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(t(e.1)).size(17.0));
                            if !e.2.is_empty() {
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new(format!("{}: {}", t("Attenzione"), t(e.2)))
                                        .color(ui::semantic_text(ui::WARN)),
                                );
                            }
                        });
                });
                ui.add_space(3.0);
            }
        }
        if shown == 0 {
            ui.add_space(3.0);
            ui.label(t("Nessun risultato per questa ricerca."));
        }
    }
}
