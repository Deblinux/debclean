use crate::i18n::{self, t, tf};
use std::fs;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct ReportOptions {
    pub hardware: bool,
    pub disk: bool,
    pub packages: bool,
    pub logs: bool,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            hardware: true,
            disk: true,
            packages: true,
            logs: true,
        }
    }
}

fn run_cmd(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stdout.trim().is_empty() {
                stderr.trim().to_string()
            } else {
                stdout.trim().to_string()
            }
        }
        Err(e) => tf("Comando non disponibile ({}): {}", &[&cmd, &e]),
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn generate_html_report(opts: &ReportOptions, output_path: &str) -> Result<String, &'static str> {
    let date = run_cmd("date", &["+%Y-%m-%d %H:%M:%S"]);
    let hostname = run_cmd("hostname", &[]);
    let host_escaped = escape_html(&hostname);

    let mut body = String::new();

    if opts.hardware {
        let uname = run_cmd("uname", &["-a"]);
        let lscpu = run_cmd("lscpu", &[]);
        let mem = run_cmd("free", &["-h"]);
        body.push_str(&format!(
            r#"<section>
                <h2>{title}</h2>
                <h3>{sys}</h3><pre>{uname}</pre>
                <h3>{ram}</h3><pre>{mem}</pre>
                <h3>{cpu}</h3><pre>{lscpu}</pre>
            </section>"#,
            title = t("1. Informazioni Hardware e Kernel"),
            sys = t("Sistema e Kernel"),
            uname = escape_html(&uname),
            ram = t("Memoria RAM e Swap"),
            mem = escape_html(&mem),
            cpu = t("Processore (CPU)"),
            lscpu = escape_html(&lscpu),
        ));
    }

    if opts.disk {
        let df = run_cmd("df", &["-hT", "-x", "tmpfs", "-x", "devtmpfs"]);
        let lsblk = run_cmd("lsblk", &["-f"]);
        body.push_str(&format!(
            r#"<section>
                <h2>{title}</h2>
                <h3>{usage}</h3><pre>{df}</pre>
                <h3>{devices}</h3><pre>{lsblk}</pre>
            </section>"#,
            title = t("2. Spazio Disco e Partizioni"),
            usage = t("Utilizzo File System"),
            df = escape_html(&df),
            devices = t("Dispositivi a Blocchi"),
            lsblk = escape_html(&lsblk),
        ));
    }

    if opts.packages {
        let held = run_cmd("apt-mark", &["showhold"]);
        let audit = run_cmd("dpkg", &["--audit"]);
        let held_txt = if held.is_empty() { t("Nessun pacchetto bloccato (hold).").to_string() } else { held };
        let audit_txt = if audit.is_empty() { t("Nessun pacchetto danneggiato o parzialmente installato.").to_string() } else { audit };
        body.push_str(&format!(
            r#"<section>
                <h2>{title}</h2>
                <h3>{held}</h3><pre>{held_txt}</pre>
                <h3>{audit}</h3><pre>{audit_txt}</pre>
            </section>"#,
            title = t("3. Stato Pacchetti APT"),
            held = t("Pacchetti Bloccati (Hold)"),
            held_txt = escape_html(&held_txt),
            audit = t("Audit Integrità Pacchetti"),
            audit_txt = escape_html(&audit_txt),
        ));
    }

    if opts.logs {
        let journal = run_cmd("journalctl", &["-p", "3", "-xb", "-n", "60", "--no-pager"]);
        let dmesg_err = run_cmd("journalctl", &["-k", "-p", "3", "-n", "30", "--no-pager"]);
        body.push_str(&format!(
            r#"<section>
                <h2>{title}</h2>
                <h3>{system}</h3><pre>{journal}</pre>
                <h3>{kernel}</h3><pre>{dmesg_err}</pre>
            </section>"#,
            title = t("4. Log di Errore Recenti (Avvio Corrente)"),
            system = t("Errori di Sistema (journalctl -p err)"),
            journal = escape_html(&journal),
            kernel = t("Errori del Kernel"),
            dmesg_err = escape_html(&dmesg_err),
        ));
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="{lang}">
<head>
<meta charset="UTF-8">
<title>{page_title} ({host_escaped})</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #f4f6f9; color: #1e293b; margin: 0; padding: 24px; }}
  .container {{ max-width: 960px; margin: 0 auto; background: #ffffff; padding: 32px; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.08); }}
  header {{ border-bottom: 2px solid #00a896; padding-bottom: 16px; margin-bottom: 24px; }}
  h1 {{ margin: 0; color: #0f172a; font-size: 24px; }}
  .meta {{ color: #64748b; font-size: 14px; margin-top: 6px; }}
  section {{ margin-bottom: 28px; }}
  h2 {{ color: #008073; font-size: 18px; border-left: 4px solid #00a896; padding-left: 10px; }}
  h3 {{ font-size: 14px; color: #475569; margin-bottom: 6px; }}
  pre {{ background: #0f172a; color: #e2e8f0; padding: 14px; border-radius: 6px; overflow-x: auto; font-size: 12.5px; line-height: 1.45; }}
  @media print {{ body {{ background: #fff; padding: 0; }} .container {{ box-shadow: none; }} }}
</style>
</head>
<body>
<div class="container">
  <header>
    <h1>{heading}</h1>
    <div class="meta">{host_label}: <strong>{host_escaped}</strong> | {date_label}: <strong>{date}</strong></div>
  </header>
  {body}
</div>
</body>
</html>"#,
        lang = i18n::code_of(i18n::lang()),
        page_title = t("DebUtility - Report Diagnostica"),
        heading = t("DebUtility — Report di Diagnostica di Sistema"),
        host_label = t("Host:"),
        date_label = t("Generato il:"),
        host_escaped = host_escaped,
        date = date,
        body = body,
    );

    fs::write(output_path, html).map_err(|_| t("Impossibile salvare il file del report sul disco."))?;
    Ok(output_path.to_string())
}
