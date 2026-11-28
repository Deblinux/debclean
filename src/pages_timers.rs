use crate::i18n::{t, tf};
use crate::progress::ProgressReporter;
use crate::timers_state as ts;
use crate::{ui, util, App, Chan, Job, Msg};
use eframe::egui;
use std::thread;
use std::time::Duration;

/// Operazioni avviabili dalla scheda: vengono eseguite fuori dal frame egui.
enum Action {
    Apply,
    Save,
    Test,
    Disable,
    Remove,
    Refresh,
}

fn finish(ch: &Chan, prog: ProgressReporter, message: String, ok: bool) {
    ch.send(Msg::Log(format!("{} {}", if ok { "✔" } else { "✘" }, message)));
    ch.send(Msg::TimerMessage(message, !ok));
    prog.finish(if ok {
        t("Completato")
    } else {
        t("Operazione terminata con errori")
    });
}

/// Scrive le unità e attiva il timer utente. Ogni passaggio controlla l'esito.
fn apply_job(freq: String, include: Vec<String>) -> Job {
    Box::new(move |ch: &Chan| {
        let mut prog = ProgressReporter::start(ch, "timers", t("Configurazione della pianificazione in corso..."));
        let schedule = ts::Schedule {
            frequency: freq,
            include,
        };
        if let Err(e) = ts::install_units(&schedule, None) {
            finish(
                ch,
                prog,
                tf("Errore nella configurazione del timer: {}", &[&e]),
                false,
            );
            return;
        }
        prog.status(t("Attivazione del timer systemd utente..."));
        match ts::enable_units() {
            Ok(out) => {
                ch.send(Msg::Out("timers", out));
                finish(
                    ch,
                    prog,
                    t("Pianificazione configurata e attivata").to_string(),
                    true,
                );
            }
            Err(e) => finish(
                ch,
                prog,
                tf("Errore nella configurazione del timer: {}", &[&e]),
                false,
            ),
        }
    })
}

/// Disattiva sempre timer e servizio; se `remove` rimuove anche i file.
fn disable_job(remove: bool) -> Job {
    Box::new(move |ch: &Chan| {
        let label = if remove {
            t("Rimozione della pianificazione in corso...")
        } else {
            t("Disattivazione della pianificazione in corso...")
        };
        let mut prog = ProgressReporter::start(ch, "timers", label);
        prog.status(t("Disattivazione di timer e servizio systemd..."));
        match ts::disable_units() {
            Ok(out) => ch.send(Msg::Out("timers", out)),
            Err(e) => {
                finish(
                    ch,
                    prog,
                    tf("Errore nella disattivazione del timer: {}", &[&e]),
                    false,
                );
                return;
            }
        }
        if remove {
            prog.status(t("Rimozione dei file di configurazione..."));
            if let Err(e) = ts::remove_units() {
                finish(
                    ch,
                    prog,
                    tf("Errore nella rimozione del timer: {}", &[&e]),
                    false,
                );
                return;
            }
        }
        let message = if remove {
            t("Pianificazione e file del timer rimossi").to_string()
        } else {
            t("Pianificazione disattivata").to_string()
        };
        finish(ch, prog, message, true);
    })
}

/// Avvia subito il servizio, senza attendere la scadenza del timer.
fn test_job() -> Job {
    Box::new(move |ch: &Chan| {
        let mut prog = ProgressReporter::start(ch, "timers", t("Esecuzione di prova in corso..."));
        let unit = ts::service_unit();
        prog.status(tf("Avvio di {}...", &[&unit]));
        let (ok, out) = util::run_shell(
            &format!("systemctl --user start {unit}"),
            false,
        );
        ch.send(Msg::Out("timers", out.clone()));
        if !ok {
            finish(
                ch,
                prog,
                tf("Errore durante il test del timer: {}", &[&out.trim()]),
                false,
            );
            return;
        }

        prog.status(t("Attesa del completamento della pulizia di prova..."));
        let mut failure: Option<String> = None;
        let mut timed_out = true;
        for _ in 0..180 {
            match ts::service_state().as_str() {
                "failed" => {
                    failure = Some(ts::service_failure_detail());
                    timed_out = false;
                    break;
                }
                "inactive" | "" | "unknown" => {
                    timed_out = false;
                    break;
                }
                _ => thread::sleep(Duration::from_millis(500)),
            }
        }
        if timed_out && failure.is_none() {
            failure = Some(t("Tempo limite raggiunto: la pulizia di prova è ancora in corso.").to_string());
        }
        let status = ts::read_status(t("Nessuna esecuzione pianificata"));
        ch.send(Msg::TimerStatus(status));
        if let Some(detail) = failure {
            let detail = if detail.is_empty() {
                t("Il servizio di pulizia è terminato con errore.").to_string()
            } else {
                detail
            };
            let message = tf("Errore durante il test del timer: {}", &[&detail]);
            finish(ch, prog, message, false);
        } else {
            finish(
                ch,
                prog,
                t("Test completato: esito e log aggiornati nella scheda.").to_string(),
                true,
            );
        }
    })
}

impl App {
    /// Rilegge lo stato reale da systemd e dal file di stato della CLI.
    pub(crate) fn refresh_timers(&self, ctx: &egui::Context) {
        self.bg(ctx, move |ch| {
            let status = ts::read_status(t("Nessuna esecuzione pianificata"));
            ch.send(Msg::TimerStatus(status));
        });
    }

    pub(crate) fn ui_timers(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.timer_loaded {
            self.timer_loaded = true;
            self.refresh_timers(ctx);
        }

        ui::title(ui, t("Pianificatore di Manutenzione"));
        ui::hint(
            ui,
            t("Configura un'attività di sistema in background che avvia DebUtility in modalità headless alle ore impostate. L'esito e lo spazio liberato vengono salvati in un file di stato leggibile da questa scheda."),
        );

        let active = self
            .timer_status
            .as_ref()
            .map(|s| s.active)
            .unwrap_or(false);
        let mut action: Option<Action> = None;

        ui::card(ui, |ui| {
            ui::section(ui, "⏱", "Automazione con Timer Systemd", "");
            ui.horizontal_wrapped(|ui| {
                ui::led(
                    ui,
                    Some(active),
                    if active {
                        t("Pianificazione attiva")
                    } else {
                        t("Pianificazione disattivata")
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(self.action_enabled(), egui::Button::new(t("Aggiorna stato")))
                        .clicked()
                    {
                        // Il refresh reale viene eseguito dopo il frame: qui
                        // non si tocca lo stato mentre la card è in prestito.
                        action = Some(Action::Refresh);
                    }
                });
            });
            ui.add_space(3.0);
            if let Some(status) = &self.timer_status {
                ui.label(
                    egui::RichText::new(tf("Prossima esecuzione: {}", &[&status.next]))
                        .color(ui::text_dim()),
                );
                if let Some(last) = &status.last {
                    ui.label(
                        egui::RichText::new(tf("Ultima esecuzione: {}", &[&last.timestamp]))
                            .strong(),
                    );
                    let esito = if last.ok() {
                        t("Esecuzione riuscita")
                    } else {
                        t("Esecuzione con errori")
                    };
                    ui.label(
                        egui::RichText::new(tf("Esito: {}", &[&esito])).color(ui::semantic_text(
                            if last.ok() { ui::GOOD } else { ui::BAD },
                        )),
                    );
                    ui.label(
                        egui::RichText::new(tf("Spazio liberato: {}", &[&last.freed_human]))
                            .color(ui::accent_text()),
                    )
                    .on_hover_text(tf(
                        "Spazio liberato (byte): {}",
                        &[&last.freed_bytes],
                    ));
                    if !last.selected.is_empty() {
                        let labels: Vec<String> = last
                            .selected
                            .iter()
                            .map(|id| {
                                ts::CLEAN_OPTIONS
                                    .iter()
                                    .find(|(known, _, _)| *known == id)
                                    .map(|(_, label, _)| t(*label).to_string())
                                    .unwrap_or_else(|| id.clone())
                            })
                            .collect();
                        ui.label(
                            egui::RichText::new(tf("Categorie: {}", &[&labels.join(", ")]))
                                .color(ui::text_dim()),
                        );
                    }
                    for warning in &last.warnings {
                        ui.label(
                            egui::RichText::new(format!("⚠ {warning}"))
                                .color(ui::semantic_text(ui::WARN)),
                        );
                    }
                } else {
                    ui.label(
                        egui::RichText::new(t("Nessuna esecuzione registrata"))
                            .color(ui::text_dim()),
                    );
                }
                if let Some(err) = &status.error {
                    ui.label(
                        egui::RichText::new(tf("Errore nella lettura dello stato del timer: {}", &[err]))
                            .color(ui::semantic_text(ui::BAD)),
                    );
                }
            }
            match ts::last_run_path() {
                Ok(p) => {
                    ui.label(
                        egui::RichText::new(tf("File di stato: {}", &[&p.display().to_string()]))
                            .color(ui::text_dim())
                            .size(12.5),
                    );
                }
                Err(e) => {
                    ui.label(
                        egui::RichText::new(tf("Errore nella lettura dello stato del timer: {}", &[&e]))
                            .color(ui::semantic_text(ui::BAD)),
                    );
                }
            }
        });
        ui.add_space(3.0);

        let mut freq = self.timer_freq.min(ts::FREQUENCIES.len() - 1);
        let mut include = self.timer_includes.clone();
        ui::card(ui, |ui| {
            ui::section(ui, "⚙", "Configurazione", "");
            ui.add_space(2.0);
            egui::ComboBox::from_label(t("Frequenza"))
                .selected_text(t(ts::FREQUENCIES[freq].1))
                .show_ui(ui, |ui| {
                    for (i, (_, label)) in ts::FREQUENCIES.iter().enumerate() {
                        ui.selectable_value(&mut freq, i, t(*label));
                    }
                });
            let id = ts::FREQUENCIES[freq].0;
            ui.label(
                egui::RichText::new(tf("OnCalendar: {}", &[&ts::on_calendar(id)]))
                    .color(ui::text_dim())
                    .size(13.0),
            );
            ui.label(
                egui::RichText::new(t("Ogni giorno alle 03:30, ogni lunedì alle 03:30 o il primo del mese alle 03:30."))
                    .color(ui::text_dim())
                    .size(13.0),
            );
            ui.add_space(3.0);
            ui.label(egui::RichText::new(t("Da includere nella pulizia")).strong());
            ui.label(
                egui::RichText::new(t("Seleziona almeno una categoria: verrà passata alla CLI come opzione --include."))
                    .color(ui::text_dim())
                    .size(13.0),
            );
            for &(option_id, label, privileged) in ts::CLEAN_OPTIONS {
                let mut checked = include.iter().any(|v| v == option_id);
                let text = if privileged {
                    format!("{} •", t(label))
                } else {
                    t(label).to_string()
                };
                if ui.checkbox(&mut checked, text).clicked() {
                    if checked {
                        if !include.iter().any(|v| v == option_id) {
                            include.push(option_id.to_string());
                        }
                    } else {
                        include.retain(|v| v != option_id);
                    }
                }
            }
            ui.label(
                egui::RichText::new(t("Le voci contrassegnate con • richiedono l'autorizzazione di amministratore durante l'esecuzione pianificata."))
                    .color(ui::text_dim())
                    .size(12.5),
            );
            ui.label(
                egui::RichText::new(t("Il comando pianificato è: debutility --clean --yes --json --include=..."))
                    .color(ui::text_dim())
                    .monospace()
                    .size(12.5),
            );
        });
        self.timer_freq = freq;
        self.timer_includes = include;

        ui.horizontal_wrapped(|ui| {
            if ui::primary_button(ui, if active { t("Aggiorna pianificazione") } else { t("Salva e attiva") }, self.action_enabled()).clicked() {
                action = Some(Action::Apply);
            }
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Salva impostazioni"))).clicked() {
                action = Some(Action::Save);
            }
            if ui.add_enabled(self.action_enabled(), egui::Button::new(t("Esegui test ora"))).clicked() {
                action = Some(Action::Test);
            }
            if active
                && ui
                    .add_enabled(self.action_enabled(), egui::Button::new(t("Disattiva pianificazione")))
                    .clicked()
            {
                action = Some(Action::Disable);
            }
            if ui
                .add_enabled(self.action_enabled(), egui::Button::new(t("Rimuovi pianificazione")))
                .clicked()
            {
                action = Some(Action::Remove);
            }
        });
        if self.busy {
            ui.spinner();
        }
        if let Some((message, is_error)) = &self.timer_msg {
            ui.label(
                egui::RichText::new(message).color(ui::semantic_text(if *is_error {
                    ui::BAD
                } else {
                    ui::GOOD
                })),
            );
        }
        ui.add_space(3.0);

        match action {
            Some(Action::Apply) | Some(Action::Save) => {
                if self.timer_includes.is_empty() {
                    let msg = t("Seleziona almeno una categoria da includere nella pulizia pianificata.").to_string();
                    self.timer_msg = Some((msg.clone(), true));
                    self.log.push_str(&format!("✘ {msg}\n"));
                } else {
                    let schedule = ts::Schedule {
                        frequency: ts::FREQUENCIES[self.timer_freq].0.to_string(),
                        include: self.timer_includes.clone(),
                    };
                    if matches!(action, Some(Action::Apply)) {
                        self.spawn(ctx, apply_job(schedule.frequency, schedule.include));
                    } else {
                        match schedule.save() {
                            Ok(()) => {
                                self.timer_msg = Some((t("Impostazioni salvate").to_string(), false));
                                if active {
                                    self.timer_msg = Some((
                                        t("Le modifiche salvate richiedono «Aggiorna pianificazione» per avere effetto.").to_string(),
                                        false,
                                    ));
                                }
                            }
                            Err(e) => {
                                let msg = tf("Errore nella configurazione del timer: {}", &[&e]);
                                self.timer_msg = Some((msg.clone(), true));
                                self.log.push_str(&format!("✘ {msg}\n"));
                            }
                        }
                    }
                }
            }
            Some(Action::Test) => {
                self.timer_msg = Some((t("Esecuzione di prova avviata.").to_string(), false));
                self.spawn(ctx, test_job());
            }
            Some(Action::Disable) => self.spawn(ctx, disable_job(false)),
            Some(Action::Remove) => self.spawn(ctx, disable_job(true)),
            Some(Action::Refresh) => self.timer_loaded = false,
            None => {}
        }

        ui.add_space(3.0);
        match ts::read_log() {
            Ok(log) if !log.trim().is_empty() => {
                ui.label(
                    egui::RichText::new(t("Log dell'ultima esecuzione"))
                        .strong()
                        .color(ui::text_dim()),
                );
                ui::output_box(ui, &log, 150.0);
            }
            Ok(_) => {}
            Err(e) => {
                ui.label(
                    egui::RichText::new(tf(
                        "Errore nella lettura dello stato del timer: {}",
                        &[&e],
                    ))
                    .color(ui::semantic_text(ui::BAD)),
                );
            }
        }

        ui.add_space(5.0);
        ui::section(ui, "◷", "Timer attivi nel sistema", "");
        ui.horizontal_wrapped(|ui| {
            self.run_btn(ui, ctx, t("Mostra tutti i timer utente"), "timers", "systemctl --user list-timers --all", false, None);
            self.run_btn(ui, ctx, t("Stato timer TRIM SSD (root)"), "timers", "systemctl status fstrim.timer", false, None);
        });
        self.show_out(ui, "timers", 280.0);
    }
}
