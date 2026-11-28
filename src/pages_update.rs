use crate::i18n::{t, tf};
use crate::{ui, util, App, Chan, Job, Msg, ProgressReporter};
use eframe::egui;

/// Opzioni che rendono `apt-get update` più veloce e meno soggetto a blocchi.
const APT_UPDATE: &str =
    "apt-get -o Acquire::Languages=none -o Acquire::Retries=1 -o Acquire::http::Timeout=20 -o APT::Color=0 update";
const APT_UPGRADE: &str =
    "apt-get -y -o APT::Color=0 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold full-upgrade";
const FLATPAK_UPDATE: &str =
    "command -v flatpak >/dev/null 2>&1 || { echo 'Flatpak non installato / not installed'; exit 1; }; flatpak update -y --noninteractive";

/// Pacchetti APT aggiornabili in base all'elenco già scaricato (non richiede privilegi).
pub fn read_apt_upgradable() -> Vec<String> {
    let (_, o) = util::run_shell("apt list --upgradable 2>/dev/null | grep '/'", false);
    o.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

/// App Flatpak aggiornabili (richiede la rete: può essere lento).
pub fn read_flatpak_updates() -> Vec<String> {
    let (_, o) = util::run_shell(
        "command -v flatpak >/dev/null 2>&1 && flatpak remote-ls --updates --columns=application,version 2>/dev/null",
        false,
    );
    o.lines()
        .map(|l| l.replace('\t', "   ").trim().to_string())
        .filter(|l| !l.is_empty() && !l.to_lowercase().starts_with("application"))
        .collect()
}

fn mark(ok: bool) -> &'static str {
    if ok {
        "✔"
    } else {
        "✘"
    }
}

/// Esegue uno script mostrando l'output riga per riga nella casella `key`.
fn stream_into(
    ch: &Chan,
    key: &'static str,
    script: &str,
    privileged: bool,
    prog: &mut ProgressReporter,
) -> bool {
    ch.send(Msg::Out(key, String::new()));
    if privileged {
        let wait = t("Attendo l'autorizzazione di amministratore...").to_string();
        ch.send(Msg::Live(key, wait.clone(), false));
        prog.status(wait);
    }
    util::run_stream(script, privileged, |line, commit| {
        prog.track_line(line);
        ch.send(Msg::Live(key, line.to_string(), commit));
    })
}

/// Cerca aggiornamenti: APT (con output in tempo reale) e Flatpak in parallelo.
fn check_job() -> Job {
    Box::new(move |ch: &Chan| {
        ch.send(Msg::Log(t("▶ Cerco aggiornamenti...").to_string()));
        ch.send(Msg::FlatLoading);
        let ch2 = ch.clone();
        let flat_handle = std::thread::spawn(move || {
            ch2.send(Msg::Out("upd_flat", String::new()));
            let wait = t("Controllo gli aggiornamenti Flatpak...").to_string();
            ch2.send(Msg::Live("upd_flat", wait.clone(), false));
            let mut flat_prog = ProgressReporter::start(&ch2, "upd_flat", t("Controllo aggiornamenti Flatpak..."));
            flat_prog.status(wait);
            let v = read_flatpak_updates();
            ch2.send(Msg::Out("upd_flat", String::new()));
            ch2.send(Msg::UpdFlat(v));
            flat_prog.finish(t("Controllo Flatpak completato"));
        });
        let mut apt_prog =
            ProgressReporter::start(ch, "upd_apt", t("Ricerca aggiornamenti APT in corso..."));
        let ok = stream_into(ch, "upd_apt", APT_UPDATE, true, &mut apt_prog);
        ch.send(Msg::Log(format!("{} apt-get update", mark(ok))));
        ch.send(Msg::UpdApt(read_apt_upgradable(), true));
        apt_prog.finish(t("Ricerca aggiornamenti completata"));
        // Attende anche Flatpak: così `busy` resta true finché entrambe le
        // barre non sono completate e l'utente vede sempre lo stato reale.
        let _ = flat_handle.join();
    })
}

/// Installa gli aggiornamenti APT e/o Flatpak. Le due operazioni vanno in parallelo.
/// `refresh`: se true scarica prima l'elenco aggiornato dei pacchetti APT.
fn upd_job(apt: bool, flat: bool, refresh: bool) -> Job {
    Box::new(move |ch: &Chan| {
        let flat_handle = if flat {
            ch.send(Msg::FlatLoading);
            let ch2 = ch.clone();
            Some(std::thread::spawn(move || {
                ch2.send(Msg::Log(t("▶ Aggiorno le app Flatpak...").to_string()));
                let mut flat_prog =
                    ProgressReporter::start(&ch2, "upd_flat", t("Aggiornamento app Flatpak in corso..."));
                let ok = stream_into(&ch2, "upd_flat", FLATPAK_UPDATE, false, &mut flat_prog);
                ch2.send(Msg::Log(format!("{} Flatpak", mark(ok))));
                ch2.send(Msg::UpdFlat(read_flatpak_updates()));
                flat_prog.finish(t("Aggiornamento Flatpak completato"));
            }))
        } else {
            None
        };
        if apt {
            ch.send(Msg::Log(t("▶ Aggiorno i pacchetti APT...").to_string()));
            let script = if refresh { format!("{APT_UPDATE} && {APT_UPGRADE}") } else { APT_UPGRADE.to_string() };
            let mut apt_prog =
                ProgressReporter::start(ch, "upd_apt", t("Aggiornamento pacchetti APT in corso..."));
            let ok = stream_into(ch, "upd_apt", &script, true, &mut apt_prog);
            ch.send(Msg::Log(format!("{} APT", mark(ok))));
            ch.send(Msg::UpdApt(read_apt_upgradable(), true));
            apt_prog.finish(t("Aggiornamento APT completato"));
        }
        if let Some(h) = flat_handle {
            let _ = h.join();
        }
    })
}

impl App {
    /// All'apertura della scheda legge gli aggiornamenti APT già noti, senza chiedere la password.
    pub(crate) fn load_updates_cache(&self, ctx: &egui::Context) {
        self.bg(ctx, |ch| ch.send(Msg::UpdApt(read_apt_upgradable(), false)));
    }

    pub(crate) fn ui_updates(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui::title(ui, "Aggiornamenti");
        ui::hint(
            ui,
            "Gli aggiornamenti correggono errori e falle di sicurezza e portano novità. Premi «Cerca aggiornamenti» per vedere cosa è disponibile, poi aggiorna i pacchetti APT (il sistema Debian e i suoi programmi), le app Flatpak o tutto insieme.",
        );

        let (na, nf) = (self.upd_apt.len(), self.upd_flat.len());
        let reboot = std::path::Path::new("/var/run/reboot-required").exists();
        ui::card(ui, |ui| {
            if na == 0 && !self.upd_checked {
                ui.label(t("Aggiornamenti non ancora controllati: premi «Cerca aggiornamenti»."));
            } else {
                if na == 0 {
                    ui.label(egui::RichText::new(t("Nessun aggiornamento APT: il sistema è aggiornato.")).size(18.0).strong().color(ui::semantic_text(ui::GOOD)));
                } else {
                    ui.label(egui::RichText::new(tf("{} aggiornamenti APT disponibili", &[&na])).size(18.0).strong());
                }
                if self.upd_flat_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t("Controllo gli aggiornamenti Flatpak..."));
                    });
                } else if nf == 0 {
                    ui.label(egui::RichText::new(t("Nessun aggiornamento Flatpak.")).size(18.0).strong().color(ui::semantic_text(ui::GOOD)));
                } else {
                    ui.label(egui::RichText::new(tf("{} aggiornamenti Flatpak disponibili", &[&nf])).size(18.0).strong());
                }
            }
            if reboot {
                ui.colored_label(ui::semantic_text(ui::WARN), t("Serve riavviare il computer per completare l'aggiornamento."));
            }
        });
        ui.add_space(3.0);

        let confirm_text = t("Aggiornare ora? L'operazione può richiedere diversi minuti: non spegnere il computer e non chiudere DebUtility finché non finisce.");
        let refresh = !self.upd_checked;
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Cerca aggiornamenti"))).clicked() {
                self.spawn(ctx, check_job());
            }
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Aggiorna APT"))).clicked() {
                self.confirm = Some((confirm_text.to_string(), upd_job(true, false, refresh)));
            }
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Aggiorna Flatpak"))).clicked() {
                self.spawn(ctx, upd_job(false, true, false));
            }
            if ui::primary_button(ui, t("Aggiorna tutto"), self.action_enabled()).clicked() {
                self.confirm = Some((confirm_text.to_string(), upd_job(true, true, refresh)));
            }
            if self.busy {
                ui.spinner();
            }
         });
        ui.add_space(3.0);

        // L'avanzamento in tempo reale è disegnato in cima al pannello
        // centrale (e in forma compatta nella barra di stato): qui restano
        // solo i pulsanti e l'output dei due comandi.

        // output in tempo reale
        if self.out.contains_key("upd_apt") || self.live.contains_key("upd_apt") {
            ui.label(egui::RichText::new(t("Output APT")).strong());
            self.show_out_follow(ui, "upd_apt", 220.0);
            ui.add_space(3.0);
        }
        if self.out.contains_key("upd_flat") || self.live.contains_key("upd_flat") {
            ui.label(egui::RichText::new(t("Output Flatpak")).strong());
            self.show_out_follow(ui, "upd_flat", 140.0);
            ui.add_space(3.0);
        }

        if na > 0 {
            ui::card(ui, |ui| {
                egui::CollapsingHeader::new(egui::RichText::new(tf("Pacchetti APT da aggiornare: {}", &[&na])).strong().size(18.0))
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(185.0).show(ui, |ui| {
                            for l in &self.upd_apt {
                                ui.label(egui::RichText::new(l.as_str()).monospace().size(14.0));
                            }
                        });
                    });
            });
            ui.add_space(3.0);
        }
        if nf > 0 {
            ui::card(ui, |ui| {
                egui::CollapsingHeader::new(egui::RichText::new(tf("App Flatpak da aggiornare: {}", &[&nf])).strong().size(18.0))
                    .default_open(false)
                    .show(ui, |ui| {
                        for l in &self.upd_flat {
                            ui.label(egui::RichText::new(l.as_str()).monospace().size(14.0));
                        }
                    });
            });
        }
    }
}
