use crate::i18n::tf;
use crate::util::run_shell;
use std::fs;
use sysinfo::{Components, Disks, System};

pub struct CpuInfo {
    pub model: String,
    pub vendor: String,
    pub details: String,
    pub physical: Option<usize>,
    pub threads: usize,
    pub caches: Vec<String>,
    pub flags: String,
    pub arch: String,
}

pub struct DiskRow {
    pub dev: String,
    pub mount: String,
    pub fs: String,
    pub total: u64,
    pub free: u64,
}

pub struct SysInfo {
    pub sys: System,
    pub cpu: CpuInfo,
    pub os: Vec<(String, String)>,
    pub hw: Vec<(String, String)>,
    pub gpu: Vec<String>,
    pub disks: Vec<DiskRow>,
    pub temps: Vec<(String, f32)>,
    pub net: String,
    pub gateway: String,
    pub lsblk: String,
}

fn rd(p: &str) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn read_cpu(sys: &System) -> CpuInfo {
    let raw = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let block = raw.split("\n\n").next().unwrap_or("");
    let get = |keys: &[&str]| -> String {
        for line in block.lines() {
            if let Some((k, v)) = line.split_once(':') {
                if keys.contains(&k.trim()) {
                    return v.trim().to_string();
                }
            }
        }
        String::new()
    };

    let mut model = get(&["model name", "Model", "Hardware", "cpu model"]);
    if model.is_empty() {
        model = sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_else(|| "n/d".into());
    }
    let mut vendor = get(&["vendor_id", "CPU implementer"]);
    if vendor.is_empty() {
        vendor = sys.cpus().first().map(|c| c.vendor_id().to_string()).unwrap_or_default();
    }

    let fam = get(&["cpu family", "CPU architecture"]);
    let mdl = get(&["model", "CPU part"]);
    let step = get(&["stepping", "CPU revision"]);
    let details = tf("famiglia {} · modello {} · stepping {}", &[&fam, &mdl, &step]);

    // Cache dalla sysfs
    let mut caches = Vec::new();
    for i in 0..8 {
        let base = format!("/sys/devices/system/cpu/cpu0/cache/index{i}");
        if let (Some(level), Some(ty), Some(size)) = (
            rd(&format!("{base}/level")),
            rd(&format!("{base}/type")),
            rd(&format!("{base}/size")),
        ) {
            caches.push(format!("L{level} {ty}: {size}"));
        }
    }

    CpuInfo {
        model,
        vendor,
        details,
        physical: sys.physical_core_count(),
        threads: sys.cpus().len(),
        caches,
        flags: get(&["flags", "Features"]),
        arch: std::env::consts::ARCH.to_string(),
    }
}

fn read_os() -> Vec<(String, String)> {
    let mut v = Vec::new();
    let pretty = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("PRETTY_NAME="))
                .map(|l| l.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "n/d".into());
    v.push(("Distribuzione".into(), pretty));
    if let Some(d) = rd("/etc/debian_version") {
        v.push(("Versione Debian".into(), d));
    }
    v.push(("Kernel".into(), System::kernel_version().unwrap_or_else(|| "n/d".into())));
    v.push(("Nome host".into(), System::host_name().unwrap_or_else(|| "n/d".into())));
    v.push(("Architettura".into(), std::env::consts::ARCH.into()));
    v
}

fn read_hw() -> Vec<(String, String)> {
    let mut v = Vec::new();
    for (label, file) in [
        ("Produttore", "sys_vendor"),
        ("Modello", "product_name"),
        ("Scheda madre", "board_name"),
        ("BIOS/UEFI", "bios_vendor"),
        ("Versione BIOS", "bios_version"),
        ("Data BIOS", "bios_date"),
    ] {
        if let Some(val) = rd(&format!("/sys/class/dmi/id/{file}")) {
            v.push((label.to_string(), val));
        }
    }
    v
}

fn read_gpu() -> Vec<String> {
    let (_, o) = run_shell("lspci 2>/dev/null | grep -Ei 'vga|3d controller|display controller'", false);
    o.lines()
        .map(|l| l.split_once(' ').map(|x| x.1).unwrap_or(l).to_string())
        .filter(|l| !l.trim().is_empty())
        .collect()
}

impl SysInfo {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();
        let cpu = read_cpu(&sys);
        let mut s = SysInfo {
            sys,
            cpu,
            os: read_os(),
            hw: read_hw(),
            gpu: read_gpu(),
            disks: Vec::new(),
            temps: Vec::new(),
            net: String::new(),
            gateway: String::new(),
            lsblk: String::new(),
        };
        s.refresh_dynamic();
        s.reload_static();
        s
    }

    /// Rilegge i dati che cambiano raramente (rete, dischi): da chiamare quando si apre la scheda.
    pub fn reload_static(&mut self) {
        let (_, a) = run_shell("ip -4 -o addr show scope global 2>/dev/null | awk '{print $2\": \"$4}'", false);
        self.net = a.trim().to_string();
        let (_, g) = run_shell("ip route 2>/dev/null | awk '/^default/{print $3; exit}'", false);
        self.gateway = g.trim().to_string();
        let (_, l) = run_shell("lsblk -d -o NAME,SIZE,TYPE,MODEL -e 7,11 2>/dev/null", false);
        self.lsblk = l.trim().to_string();
    }

    pub fn refresh(&mut self) {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.refresh_dynamic();
    }

    fn refresh_dynamic(&mut self) {
        let disks = Disks::new_with_refreshed_list();
        self.disks = disks
            .iter()
            .filter(|d| {
                let fs = d.file_system().to_string_lossy().to_string();
                !["tmpfs", "devtmpfs", "squashfs", "overlay", "efivarfs"].contains(&fs.as_str())
            })
            .map(|d| DiskRow {
                dev: d.name().to_string_lossy().to_string(),
                mount: d.mount_point().to_string_lossy().to_string(),
                fs: d.file_system().to_string_lossy().to_string(),
                total: d.total_space(),
                free: d.available_space(),
            })
            .collect();

        let comps = Components::new_with_refreshed_list();
        self.temps = comps
            .iter()
            .map(|c| (c.label().to_string(), c.temperature()))
            .filter(|(_, t)| t.is_finite() && *t > 0.0)
            .collect();
    }
}
