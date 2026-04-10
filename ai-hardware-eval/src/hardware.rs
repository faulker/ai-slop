use std::process::Command;

#[derive(Debug, Clone)]
pub struct CpuInfo {
    pub model_name: String,
    pub architecture: String,
    pub cores: u32,
    pub threads: u32,
    pub has_avx: bool,
    pub has_avx2: bool,
    pub is_apple_silicon: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    AppleSilicon,
    Other(String),
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub vendor: GpuVendor,
    pub name: String,
    pub vram_bytes: Option<u64>,
    pub driver_version: Option<String>,
    pub cuda_version: Option<String>,
    pub metal_support: bool,
}

#[derive(Debug, Clone)]
pub struct MemoryInfo {
    pub total_ram_bytes: u64,
    pub available_ram_bytes: u64,
    pub total_swap_bytes: u64,
    pub available_swap_bytes: u64,
    pub is_unified: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StorageType {
    NVMe,
    SSD,
    HDD,
    Unknown,
}

impl std::fmt::Display for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageType::NVMe => write!(f, "NVMe SSD"),
            StorageType::SSD => write!(f, "SSD"),
            StorageType::HDD => write!(f, "HDD"),
            StorageType::Unknown => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub storage_type: StorageType,
}

#[derive(Debug, Clone)]
pub struct HardwareProfile {
    pub cpu: CpuInfo,
    pub gpus: Vec<GpuInfo>,
    pub memory: MemoryInfo,
    pub disk: DiskInfo,
    pub os: String,
    pub missing_tools: Vec<MissingTool>,
}

#[derive(Debug, Clone)]
pub struct MissingTool {
    pub command: String,
    pub purpose: String,
    pub install_instructions: Vec<(String, String)>, // (distro, command)
}

fn run_command(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
}

fn command_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[allow(dead_code)]
fn detect_distro() -> Option<String> {
    let content = std::fs::read_to_string("/etc/os-release").ok()?;
    for line in content.lines() {
        if let Some(id) = line.strip_prefix("ID=") {
            return Some(id.trim_matches('"').to_lowercase());
        }
    }
    None
}

fn install_instructions(command: &str) -> Vec<(String, String)> {
    match command {
        "lscpu" | "lsblk" => vec![
            ("Ubuntu/Debian/Mint".into(), "sudo apt install util-linux".into()),
            ("RHEL/Fedora/CentOS".into(), "sudo dnf install util-linux".into()),
            ("Arch".into(), "sudo pacman -S util-linux".into()),
        ],
        "free" => vec![
            ("Ubuntu/Debian/Mint".into(), "sudo apt install procps".into()),
            ("RHEL/Fedora/CentOS".into(), "sudo dnf install procps-ng".into()),
            ("Arch".into(), "sudo pacman -S procps-ng".into()),
        ],
        "lspci" => vec![
            ("Ubuntu/Debian/Mint".into(), "sudo apt install pciutils".into()),
            ("RHEL/Fedora/CentOS".into(), "sudo dnf install pciutils".into()),
            ("Arch".into(), "sudo pacman -S pciutils".into()),
        ],
        "nvidia-smi" => vec![
            ("Ubuntu/Debian/Mint".into(), "sudo apt install nvidia-utils-XXX (match your driver version)".into()),
            ("RHEL/Fedora/CentOS".into(), "sudo dnf install nvidia-gpu-firmware".into()),
            ("Arch".into(), "sudo pacman -S nvidia-utils".into()),
        ],
        _ => vec![],
    }
}

fn check_requirements() -> Vec<MissingTool> {
    let mut missing = Vec::new();
    let os = std::env::consts::OS;

    if os == "linux" {
        let checks = [
            ("lscpu", "CPU information detection"),
            ("free", "Memory/swap detection"),
            ("lspci", "GPU detection"),
            ("lsblk", "Storage type detection"),
        ];
        for (cmd, purpose) in checks {
            if !command_exists(cmd) {
                missing.push(MissingTool {
                    command: cmd.to_string(),
                    purpose: purpose.to_string(),
                    install_instructions: install_instructions(cmd),
                });
            }
        }
    }
    // macOS tools (sysctl, system_profiler, df, diskutil) are always pre-installed

    missing
}

// ── Linux Detection ─────────────────────────────────────────────────────────

fn detect_cpu_linux() -> CpuInfo {
    let output = run_command("lscpu", &[]).unwrap_or_default();
    let mut model_name = String::from("Unknown");
    let mut architecture = String::from("Unknown");
    let mut threads: u32 = 1;
    let mut cores_per_socket: u32 = 1;
    let mut sockets: u32 = 1;
    let mut has_avx = false;
    let mut has_avx2 = false;

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() != 2 {
            continue;
        }
        let key = parts[0].trim();
        let val = parts[1].trim();

        match key {
            "Model name" => model_name = val.to_string(),
            "Architecture" => architecture = val.to_string(),
            "CPU(s)" => threads = val.parse().unwrap_or(1),
            "Core(s) per socket" => cores_per_socket = val.parse().unwrap_or(1),
            "Socket(s)" => sockets = val.parse().unwrap_or(1),
            "Flags" => {
                let flags: Vec<&str> = val.split_whitespace().collect();
                has_avx = flags.contains(&"avx");
                has_avx2 = flags.contains(&"avx2");
            }
            _ => {}
        }
    }

    CpuInfo {
        model_name,
        architecture,
        cores: cores_per_socket * sockets,
        threads,
        has_avx,
        has_avx2,
        is_apple_silicon: false,
    }
}

fn detect_gpus_linux() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();

    // Detect via lspci
    let lspci_output = run_command("lspci", &[]);
    if let Some(output) = lspci_output {
        for line in output.lines() {
            let lower = line.to_lowercase();
            if lower.contains("vga") || lower.contains("3d") || lower.contains("display") {
                let vendor = if lower.contains("nvidia") {
                    GpuVendor::Nvidia
                } else if lower.contains("amd") || lower.contains("ati") {
                    GpuVendor::Amd
                } else if lower.contains("intel") {
                    GpuVendor::Intel
                } else {
                    GpuVendor::Other("Unknown".into())
                };

                // Extract name after the bracket description
                let name = line
                    .split(": ")
                    .nth(1)
                    .unwrap_or("Unknown GPU")
                    .to_string();

                gpus.push(GpuInfo {
                    vendor,
                    name,
                    vram_bytes: None,
                    driver_version: None,
                    cuda_version: None,
                    metal_support: false,
                });
            }
        }
    }

    // Enrich NVIDIA GPUs with nvidia-smi data
    if gpus.iter().any(|g| g.vendor == GpuVendor::Nvidia) && command_exists("nvidia-smi") {
        if let Some(smi) = run_command(
            "nvidia-smi",
            &[
                "--query-gpu=name,memory.total,driver_version",
                "--format=csv,noheader,nounits",
            ],
        ) {
            for (i, line) in smi.lines().enumerate() {
                let fields: Vec<&str> = line.split(", ").collect();
                if fields.len() >= 3 {
                    let vram_mib: u64 = fields[1].trim().parse().unwrap_or(0);
                    let nvidia_gpu = if i < gpus.len() {
                        gpus.iter_mut()
                            .filter(|g| g.vendor == GpuVendor::Nvidia)
                            .nth(i)
                    } else {
                        None
                    };
                    if let Some(gpu) = nvidia_gpu {
                        gpu.name = fields[0].trim().to_string();
                        gpu.vram_bytes = Some(vram_mib * 1024 * 1024);
                        gpu.driver_version = Some(fields[2].trim().to_string());
                    }
                }
            }
        }

        // Get CUDA version
        if let Some(smi_full) = run_command("nvidia-smi", &[]) {
            for line in smi_full.lines() {
                if line.contains("CUDA Version") {
                    if let Some(ver) = line.split("CUDA Version:").nth(1) {
                        let cuda_ver = ver.trim().split_whitespace().next().unwrap_or("").to_string();
                        for gpu in gpus.iter_mut().filter(|g| g.vendor == GpuVendor::Nvidia) {
                            gpu.cuda_version = Some(cuda_ver.clone());
                        }
                    }
                }
            }
        }
    }

    gpus
}

fn detect_memory_linux() -> MemoryInfo {
    let output = run_command("free", &["-b"]).unwrap_or_default();
    let mut total_ram: u64 = 0;
    let mut available_ram: u64 = 0;
    let mut total_swap: u64 = 0;
    let mut available_swap: u64 = 0;

    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if line.starts_with("Mem:") && parts.len() >= 7 {
            total_ram = parts[1].parse().unwrap_or(0);
            available_ram = parts[6].parse().unwrap_or(0);
        } else if line.starts_with("Swap:") && parts.len() >= 4 {
            total_swap = parts[1].parse().unwrap_or(0);
            available_swap = parts[3].parse().unwrap_or(0);
        }
    }

    MemoryInfo {
        total_ram_bytes: total_ram,
        available_ram_bytes: available_ram,
        total_swap_bytes: total_swap,
        available_swap_bytes: available_swap,
        is_unified: false,
    }
}

/// Filter out virtual/pseudo/network filesystems from df output.
/// Returns true if this looks like a real local disk filesystem.
fn is_real_filesystem(device: &str, mount: &str) -> bool {
    // Skip network mounts (SMB, NFS, AFP, CIFS, SSHFS, etc.)
    if device.starts_with("//") || device.contains(":/") || device.contains("@") {
        return false;
    }

    // Skip virtual filesystems
    let virtual_prefixes = ["tmpfs", "devtmpfs", "devfs", "none", "sysfs", "proc", "cgroup",
                            "overlay", "shm", "run", "snap", "squashfs", "efivarfs", "map "];
    let device_lower = device.to_lowercase();
    if virtual_prefixes.iter().any(|p| device_lower.starts_with(p)) {
        return false;
    }

    // Skip pseudo mount points
    let virtual_mounts = ["/dev", "/sys", "/proc", "/run", "/snap"];
    if virtual_mounts.iter().any(|m| mount.starts_with(m)) {
        return false;
    }

    // Must start with / (real block device path like /dev/sda1)
    device.starts_with('/')
}

/// Extract the base disk device from a partition path.
/// e.g., "/dev/disk3s1s1" -> "/dev/disk3", "/dev/sda1" -> "/dev/sda",
///       "/dev/nvme0n1p2" -> "/dev/nvme0n1"
fn base_disk_device(device: &str) -> String {
    // macOS APFS: /dev/disk3s1s1, /dev/disk3s5 -> /dev/disk3
    // Find "disk" then skip digits, then look for 's' partition separator
    if let Some(disk_pos) = device.find("disk") {
        let after_disk = &device[disk_pos + 4..]; // skip "disk"
        // Skip the disk number digits
        let digit_count = after_disk.chars().take_while(|c| c.is_ascii_digit()).count();
        if digit_count > 0 {
            let rest = &after_disk[digit_count..];
            if rest.starts_with('s') {
                // Found partition separator — return everything up to it
                return device[..disk_pos + 4 + digit_count].to_string();
            }
        }
    }

    // Linux NVMe: /dev/nvme0n1p2 -> /dev/nvme0n1
    if device.contains("nvme") {
        if let Some(pos) = device.rfind('p') {
            let after = &device[pos + 1..];
            if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) {
                return device[..pos].to_string();
            }
        }
    }

    // Linux standard: /dev/sda1 -> /dev/sda, /dev/vda2 -> /dev/vda
    let trimmed = device.trim_end_matches(|c: char| c.is_ascii_digit());
    trimmed.to_string()
}

fn detect_disk_linux() -> DiskInfo {
    let mut total: u64 = 0;
    let mut available: u64 = 0;
    let mut storage_type = StorageType::Unknown;
    let mut seen_disks = std::collections::HashSet::new();

    // Get disk space across all real filesystems.
    // Deduplicate by base disk device so partitions on the same physical
    // disk are only counted once (uses the largest total from that disk).
    if let Some(output) = run_command("df", &["-B1"]) {
        let mut disk_totals: std::collections::HashMap<String, (u64, u64)> =
            std::collections::HashMap::new();
        for line in output.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 6 {
                let device = parts[0];
                let mount = parts[5];
                if is_real_filesystem(device, mount) {
                    let base = base_disk_device(device);
                    let part_total: u64 = parts[1].parse().unwrap_or(0);
                    let part_avail: u64 = parts[3].parse().unwrap_or(0);
                    let entry = disk_totals.entry(base).or_insert((0, 0));
                    // For partitions sharing a container (like APFS), keep the max total
                    // and sum the available only once (use the first/largest seen)
                    if part_total > entry.0 {
                        entry.0 = part_total;
                    }
                    // Available is shared across APFS volumes so don't sum it
                    if part_avail > entry.1 {
                        entry.1 = part_avail;
                    }
                }
            }
        }
        for (_disk, (disk_total, disk_avail)) in &disk_totals {
            total += disk_total;
            available += disk_avail;
            seen_disks.insert(_disk.clone());
        }
    }

    // Detect storage type
    if let Some(output) = run_command("lsblk", &["-d", "-o", "NAME,ROTA,TRAN"]) {
        for line in output.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let rota = parts[1].trim();
                let tran = parts.get(2).map(|s| s.trim()).unwrap_or("");
                if tran.contains("nvme") {
                    storage_type = StorageType::NVMe;
                    break;
                } else if rota == "0" {
                    storage_type = StorageType::SSD;
                    break;
                } else if rota == "1" {
                    storage_type = StorageType::HDD;
                    break;
                }
            }
        }
    }

    DiskInfo {
        total_bytes: total,
        available_bytes: available,
        storage_type,
    }
}

// ── macOS Detection ─────────────────────────────────────────────────────────

fn detect_cpu_macos() -> CpuInfo {
    let arch = run_command("uname", &["-m"]).unwrap_or_default().trim().to_string();
    let is_apple_silicon = arch == "arm64";

    let model_name = if is_apple_silicon {
        // Try to get the chip name from sysctl
        run_command("sysctl", &["-n", "machdep.cpu.brand_string"])
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| {
                // Fallback: try system_profiler
                run_command("system_profiler", &["SPHardwareDataType"])
                    .and_then(|output| {
                        output
                            .lines()
                            .find(|l| l.contains("Chip:") || l.contains("Processor Name:"))
                            .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
                    })
                    .unwrap_or_else(|| "Apple Silicon".into())
            })
    } else {
        run_command("sysctl", &["-n", "machdep.cpu.brand_string"])
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Unknown".into())
    };

    let cores: u32 = run_command("sysctl", &["-n", "hw.physicalcpu"])
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1);

    let threads: u32 = run_command("sysctl", &["-n", "hw.logicalcpu"])
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(cores);

    let (has_avx, has_avx2) = if !is_apple_silicon {
        let features = run_command("sysctl", &["-n", "machdep.cpu.features"]).unwrap_or_default();
        let leaf7 =
            run_command("sysctl", &["-n", "machdep.cpu.leaf7_features"]).unwrap_or_default();
        (features.contains("AVX"), leaf7.contains("AVX2"))
    } else {
        (false, false) // ARM doesn't use AVX
    };

    CpuInfo {
        model_name,
        architecture: arch,
        cores,
        threads,
        has_avx,
        has_avx2,
        is_apple_silicon,
    }
}

fn detect_gpus_macos(cpu: &CpuInfo, total_ram_bytes: u64) -> Vec<GpuInfo> {
    let mut gpus = Vec::new();

    if cpu.is_apple_silicon {
        // Apple Silicon has integrated GPU with unified memory
        gpus.push(GpuInfo {
            vendor: GpuVendor::AppleSilicon,
            name: format!("{} GPU", cpu.model_name),
            // ~75% of unified memory available for GPU tasks
            vram_bytes: Some((total_ram_bytes as f64 * 0.75) as u64),
            driver_version: None,
            cuda_version: None,
            metal_support: true,
        });
    } else {
        // Intel Mac — check for discrete GPU
        if let Some(output) = run_command("system_profiler", &["SPDisplaysDataType"]) {
            let mut current_name = String::new();
            let mut current_vram: Option<u64> = None;

            for line in output.lines() {
                let trimmed = line.trim();
                if trimmed.ends_with(':') && !trimmed.starts_with("Displays:") && !trimmed.starts_with("Display") {
                    if !current_name.is_empty() {
                        gpus.push(GpuInfo {
                            vendor: if current_name.to_lowercase().contains("amd") {
                                GpuVendor::Amd
                            } else if current_name.to_lowercase().contains("intel") {
                                GpuVendor::Intel
                            } else {
                                GpuVendor::Other(current_name.clone())
                            },
                            name: current_name.clone(),
                            vram_bytes: current_vram,
                            driver_version: None,
                            cuda_version: None,
                            metal_support: true,
                        });
                    }
                    current_name = trimmed.trim_end_matches(':').to_string();
                    current_vram = None;
                }
                if trimmed.starts_with("VRAM") || trimmed.starts_with("Total Number of Cores") {
                    if let Some(val) = trimmed.split(':').nth(1) {
                        let val = val.trim();
                        // Parse "1536 MB" or "2 GB" etc.
                        let parts: Vec<&str> = val.split_whitespace().collect();
                        if parts.len() >= 2 {
                            if let Ok(num) = parts[0].parse::<u64>() {
                                current_vram = Some(match parts[1] {
                                    "GB" => num * 1024 * 1024 * 1024,
                                    "MB" => num * 1024 * 1024,
                                    _ => num,
                                });
                            }
                        }
                    }
                }
            }
            if !current_name.is_empty() {
                gpus.push(GpuInfo {
                    vendor: if current_name.to_lowercase().contains("amd") {
                        GpuVendor::Amd
                    } else if current_name.to_lowercase().contains("intel") {
                        GpuVendor::Intel
                    } else {
                        GpuVendor::Other(current_name.clone())
                    },
                    name: current_name,
                    vram_bytes: current_vram,
                    driver_version: None,
                    cuda_version: None,
                    metal_support: true,
                });
            }
        }
    }

    gpus
}

fn detect_memory_macos() -> MemoryInfo {
    let total: u64 = run_command("sysctl", &["-n", "hw.memsize"])
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    // Parse vm_stat for available memory
    let mut available: u64 = 0;
    if let Some(output) = run_command("vm_stat", &[]) {
        let mut page_size: u64 = 4096;
        let mut free_pages: u64 = 0;
        let mut inactive_pages: u64 = 0;

        for line in output.lines() {
            if line.contains("page size of") {
                // "Mach Virtual Memory Statistics: (page size of 16384 bytes)"
                if let Some(size_str) = line.split("page size of ").nth(1) {
                    if let Some(num_str) = size_str.split_whitespace().next() {
                        page_size = num_str.parse().unwrap_or(4096);
                    }
                }
            }
            if line.starts_with("Pages free:") {
                free_pages = parse_vm_stat_value(line);
            }
            if line.starts_with("Pages inactive:") {
                inactive_pages = parse_vm_stat_value(line);
            }
        }
        available = (free_pages + inactive_pages) * page_size;
    }

    // Check for Apple Silicon unified memory
    let arch = run_command("uname", &["-m"]).unwrap_or_default();
    let is_unified = arch.trim() == "arm64";

    MemoryInfo {
        total_ram_bytes: total,
        available_ram_bytes: available,
        total_swap_bytes: 0, // macOS uses dynamic swap
        available_swap_bytes: 0,
        is_unified,
    }
}

fn parse_vm_stat_value(line: &str) -> u64 {
    line.split(':')
        .nth(1)
        .unwrap_or("")
        .trim()
        .trim_end_matches('.')
        .parse()
        .unwrap_or(0)
}

fn detect_disk_macos() -> DiskInfo {
    let mut total: u64 = 0;
    let mut available: u64 = 0;
    let mut storage_type = StorageType::Unknown;

    // Sum across all real local filesystems.
    // Deduplicate by base disk device — APFS volumes on the same container
    // share the same physical disk and report overlapping totals.
    if let Some(output) = run_command("df", &["-k"]) {
        let mut disk_totals: std::collections::HashMap<String, (u64, u64)> =
            std::collections::HashMap::new();
        for line in output.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 9 {
                let device = parts[0];
                let mount = parts[8];
                if is_real_filesystem(device, mount) {
                    let base = base_disk_device(device);
                    // df -k gives 1K blocks
                    let part_total = parts[1].parse::<u64>().unwrap_or(0) * 1024;
                    let part_avail = parts[3].parse::<u64>().unwrap_or(0) * 1024;
                    let entry = disk_totals.entry(base).or_insert((0, 0));
                    if part_total > entry.0 {
                        entry.0 = part_total;
                    }
                    if part_avail > entry.1 {
                        entry.1 = part_avail;
                    }
                }
            }
        }
        for (_, (disk_total, disk_avail)) in &disk_totals {
            total += disk_total;
            available += disk_avail;
        }
    }

    // Detect storage type via diskutil
    if let Some(output) = run_command("diskutil", &["info", "/"]) {
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Solid State:") {
                if trimmed.contains("Yes") {
                    storage_type = StorageType::SSD;
                } else {
                    storage_type = StorageType::HDD;
                }
            }
            if trimmed.starts_with("Protocol:") && trimmed.contains("NVMe") {
                storage_type = StorageType::NVMe;
            }
        }
    }

    DiskInfo {
        total_bytes: total,
        available_bytes: available,
        storage_type,
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

pub fn detect_all() -> HardwareProfile {
    let os = std::env::consts::OS.to_string();
    let missing_tools = check_requirements();

    match os.as_str() {
        "macos" => {
            let cpu = detect_cpu_macos();
            let memory = detect_memory_macos();
            let gpus = detect_gpus_macos(&cpu, memory.total_ram_bytes);
            let disk = detect_disk_macos();
            HardwareProfile {
                cpu,
                gpus,
                memory,
                disk,
                os,
                missing_tools,
            }
        }
        _ => {
            // Linux (default)
            let cpu = detect_cpu_linux();
            let gpus = detect_gpus_linux();
            let memory = detect_memory_linux();
            let disk = detect_disk_linux();
            HardwareProfile {
                cpu,
                gpus,
                memory,
                disk,
                os,
                missing_tools,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lscpu() {
        // Simulate lscpu parsing with known input
        let sample = "\
Architecture:            x86_64
CPU(s):                  16
Core(s) per socket:      8
Socket(s):               1
Model name:              AMD Ryzen 7 5800X 8-Core Processor
Flags:                   fpu vme avx avx2 sse sse2";

        let mut model_name = String::from("Unknown");
        let mut architecture = String::from("Unknown");
        let mut threads: u32 = 1;
        let mut cores_per_socket: u32 = 1;
        let mut sockets: u32 = 1;
        let mut has_avx = false;
        let mut has_avx2 = false;

        for line in sample.lines() {
            let parts: Vec<&str> = line.splitn(2, ':').collect();
            if parts.len() != 2 { continue; }
            let key = parts[0].trim();
            let val = parts[1].trim();
            match key {
                "Model name" => model_name = val.to_string(),
                "Architecture" => architecture = val.to_string(),
                "CPU(s)" => threads = val.parse().unwrap_or(1),
                "Core(s) per socket" => cores_per_socket = val.parse().unwrap_or(1),
                "Socket(s)" => sockets = val.parse().unwrap_or(1),
                "Flags" => {
                    let flags: Vec<&str> = val.split_whitespace().collect();
                    has_avx = flags.contains(&"avx");
                    has_avx2 = flags.contains(&"avx2");
                }
                _ => {}
            }
        }

        assert_eq!(model_name, "AMD Ryzen 7 5800X 8-Core Processor");
        assert_eq!(architecture, "x86_64");
        assert_eq!(threads, 16);
        assert_eq!(cores_per_socket * sockets, 8);
        assert!(has_avx);
        assert!(has_avx2);
    }

    #[test]
    fn test_parse_free() {
        let sample = "\
              total        used        free      shared  buff/cache   available
Mem:    33554432000  16000000000   8000000000     500000  9054432000  17000000000
Swap:    8388608000           0  8388608000";

        let mut total_ram: u64 = 0;
        let mut available_ram: u64 = 0;
        let mut total_swap: u64 = 0;
        let mut available_swap: u64 = 0;

        for line in sample.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if line.starts_with("Mem:") && parts.len() >= 7 {
                total_ram = parts[1].parse().unwrap_or(0);
                available_ram = parts[6].parse().unwrap_or(0);
            } else if line.starts_with("Swap:") && parts.len() >= 4 {
                total_swap = parts[1].parse().unwrap_or(0);
                available_swap = parts[3].parse().unwrap_or(0);
            }
        }

        assert_eq!(total_ram, 33554432000);
        assert_eq!(available_ram, 17000000000);
        assert_eq!(total_swap, 8388608000);
        assert_eq!(available_swap, 8388608000);
    }

    #[test]
    fn test_parse_vm_stat_value() {
        assert_eq!(parse_vm_stat_value("Pages free:                             1234."), 1234);
        assert_eq!(parse_vm_stat_value("Pages inactive:                         5678."), 5678);
    }

    #[test]
    fn test_parse_nvidia_smi() {
        let sample = "NVIDIA GeForce RTX 3060, 12288, 535.129.03";
        let fields: Vec<&str> = sample.split(", ").collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].trim(), "NVIDIA GeForce RTX 3060");
        let vram_mib: u64 = fields[1].trim().parse().unwrap();
        assert_eq!(vram_mib, 12288);
        assert_eq!(fields[2].trim(), "535.129.03");
    }

    #[test]
    fn test_base_disk_device_macos() {
        assert_eq!(base_disk_device("/dev/disk3s1s1"), "/dev/disk3");
        assert_eq!(base_disk_device("/dev/disk3s5"), "/dev/disk3");
        assert_eq!(base_disk_device("/dev/disk1s2"), "/dev/disk1");
        assert_eq!(base_disk_device("/dev/disk10s3"), "/dev/disk10");
    }

    #[test]
    fn test_base_disk_device_linux() {
        assert_eq!(base_disk_device("/dev/sda1"), "/dev/sda");
        assert_eq!(base_disk_device("/dev/sdb2"), "/dev/sdb");
        assert_eq!(base_disk_device("/dev/vda1"), "/dev/vda");
        assert_eq!(base_disk_device("/dev/nvme0n1p2"), "/dev/nvme0n1");
        assert_eq!(base_disk_device("/dev/nvme0n1p1"), "/dev/nvme0n1");
    }

    #[test]
    fn test_is_real_filesystem() {
        assert!(is_real_filesystem("/dev/disk3s1s1", "/"));
        assert!(is_real_filesystem("/dev/sda1", "/home"));
        assert!(!is_real_filesystem("devfs", "/dev"));
        assert!(!is_real_filesystem("tmpfs", "/run"));
        assert!(!is_real_filesystem("//GUEST:@server/share", "/Volumes/share"));
        assert!(!is_real_filesystem("server:/export", "/mnt/nfs"));
        assert!(!is_real_filesystem("map auto_home", "/home"));
    }

    #[test]
    fn test_detect_all_runs() {
        // Just verify it doesn't panic on the current OS
        let profile = detect_all();
        assert!(!profile.os.is_empty());
        assert!(profile.memory.total_ram_bytes > 0);
    }
}
