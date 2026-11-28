use crate::i18n::t;
use crate::{ui, App};
use eframe::egui;

impl App {
    pub(crate) fn ui_security_hardening(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, t("Blindatura & Security Hardening"));
        ui::card(ui, |ui| {
            ui.label(
                egui::RichText::new(t("Applica best-practice di sicurezza enterprise per blindare il kernel Linux, proteggere la rete contro attacchi comuni (SYN flood, spoofing), verificare i permessi critici e controllare le porte aperte."))
                    .size(16.0),
            );
        });

        ui.add_space(3.0);
        
        // 1. Blocco Hardening Kernel & Rete
        ui::card(ui, |ui| {
            ui::section_text(ui, "▣", t("🔐 Hardening Kernel & Parametri di Rete (Sysctl)"), "");
            ui.label(t("Applica parametri di sicurezza avanzati per prevenire SYN flood, IP spoofing, limitare i dump di memoria e proteggere il routing. Verrà creato un file di configurazione persistente in /etc/sysctl.d/99-debutility-hardening.conf."));
            ui.add_space(3.0);

            let sysctl_script = r#"
            set -e
            cat << 'EOF' > /etc/sysctl.d/99-debutility-hardening.conf
# Protezione SYN Flood e Cookie
net.ipv4.tcp_syncookies = 1
net.ipv4.tcp_max_syn_backlog = 2048
net.ipv4.tcp_synack_retries = 2

# Prevenzione IP Spoofing e Source Routing
net.ipv4.conf.all.rp_filter = 1
net.ipv4.conf.default.rp_filter = 1
net.ipv4.conf.all.accept_source_route = 0
net.ipv4.conf.default.accept_source_route = 0

# Ignora ICMP Redirects e Broadcast Echo
net.ipv4.conf.all.accept_redirects = 0
net.ipv4.conf.default.accept_redirects = 0
net.ipv4.conf.all.secure_redirects = 0
net.ipv4.icmp_echo_ignore_broadcasts = 1

# Abilita il log dei pacchetti falsificati (Martians)
net.ipv4.conf.all.log_martians = 1

# Protezione da attacchi race condition su /tmp
fs.protected_hardlinks = 1
fs.protected_symlinks = 1

# Disabilita Coredump per processi UID differenti (evita leak di memoria)
fs.suid_dumpable = 0
EOF
sysctl --system
echo '✔ Hardening di rete e kernel applicato con successo!'
echo '--- Valori attuali ---'
sysctl -n net.ipv4.tcp_syncookies net.ipv4.conf.all.rp_filter fs.protected_symlinks fs.suid_dumpable
            "#;

            self.run_btn(
                ui,
                ctx,
                t("🛡 Applica Hardening Kernel e Rete"),
                "hardening",
                sysctl_script,
                true,
                Some(t("Vuoi applicare le impostazioni di sicurezza enterprise al kernel?")),
            );
        });

        ui.add_space(3.0);

        // 2. Controllo Permessi File Critici
        ui::card(ui, |ui| {
            ui::section_text(ui, "▣", t("📂 Verifica Permessi File Critici"), "");
            ui.label(t("Controlla che i file di sistema cruciali (/etc/passwd, /etc/shadow, /etc/sudoers) abbiano i permessi corretti e non siano modificabili da utenti non autorizzati."));
            ui.add_space(3.0);

            let audit_script = r#"
            set -e
            echo "▶ Controllo permessi file critici (/etc/passwd, /etc/shadow, /etc/sudoers)..."
            
            PASSWD_PERM=$(stat -c %a /etc/passwd)
            SHADOW_PERM=$(stat -c %a /etc/shadow)
            SUDOERS_PERM=$(stat -c %a /etc/sudoers)
            
            echo "• /etc/passwd permessi: $PASSWD_PERM (Consigliato: 644)"
            echo "• /etc/shadow permessi: $SHADOW_PERM (Consigliato: 640 o 000)"
            echo "• /etc/sudoers permessi: $SUDOERS_PERM (Consigliato: 440)"
            
            # Correzione automatica sicura: /etc/shadow veniva solo controllato.
            [ "$PASSWD_PERM" = "644" ] || chmod 644 /etc/passwd
            [ "$SUDOERS_PERM" = "440" ] || chmod 440 /etc/sudoers
            [ "$SHADOW_PERM" = "640" ] || [ "$SHADOW_PERM" = "000" ] || chmod 640 /etc/shadow
            stat -c '%n %a' /etc/passwd /etc/shadow /etc/sudoers
            echo "✔ Audit permessi completato e correzioni verificate."
            "#;

            self.run_btn(
                ui,
                ctx,
                t("🔍 Esegui Audit e Ripara Permessi"),
                "hardening",
                audit_script,
                true,
                None,
            );
        });

        ui.add_space(3.0);

        // 3. Porte Aperte e Servizi di Rete
        ui::card(ui, |ui| {
            ui::section_text(ui, "▣", t("🌐 Analisi Porte Locali e Servizi in Ascolto"), "");
            ui.label(t("Elenca tutte le porte di rete aperte sul computer e i relativi programmi associati, per identificare eventuali servizi esposti non necessari."));
            ui.add_space(3.0);

            let ports_script = "ss -tulpn 2>/dev/null || netstat -tulpn 2>/dev/null";

            self.run_btn(
                ui,
                ctx,
                t("📋 Mostra Porte e Servizi in Ascolto"),
                "hardening",
                ports_script,
                true,
                None,
            );
        });

        ui.add_space(3.0);
        self.show_out(ui, "hardening", 220.0);
    }
}
