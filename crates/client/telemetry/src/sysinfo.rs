/// The operating system part of the current target triplet.
pub const TARGET_OS: &str = include_str!(concat!(env!("OUT_DIR"), "/target_os.txt"));

/// The CPU ISA architecture part of the current target triplet.
pub const TARGET_ARCH: &str = include_str!(concat!(env!("OUT_DIR"), "/target_arch.txt"));

/// The environment part of the current target triplet.
pub const TARGET_ENV: &str = include_str!(concat!(env!("OUT_DIR"), "/target_env.txt"));

pub struct SysInfo {
    pub core_count: Option<u64>,
    pub cpu: Option<String>,
    pub linux_distro: Option<String>,
    pub linux_kernel: Option<String>,
    pub memory: Option<u64>,
    pub cpu_arch: Option<String>,
}

impl SysInfo {
    pub fn probe() -> Self {
        let probe = sysinfo::System::new_all();

        SysInfo {
            core_count: if probe.cpus().is_empty() { None } else { Some(probe.cpus().len() as _) },
            cpu: probe.cpus().first().map(|cpu| cpu.brand().into()),
            linux_distro: sysinfo::System::long_os_version(),
            linux_kernel: sysinfo::System::kernel_version(),
            memory: Some(probe.total_memory()),
            cpu_arch: sysinfo::System::cpu_arch(),
        }
    }

    pub fn show(&self) {
        if let Some(val) = &self.linux_distro {
            log::info!(target: "madara", "💻 Operating system: {}", val)
        }
        if let Some(val) = &self.cpu_arch {
            log::info!(target: "madara", "💻 CPU architecture: {}", val)
        }
        if let Some(val) = &self.cpu {
            log::info!(target: "madara", "💻 CPU: {}", val)
        }
        if let Some(val) = &self.core_count {
            log::info!(target: "madara", "💻 CPU cores: {}", val)
        }
        if let Some(val) = &self.memory {
            log::info!(target: "madara", "💻 Memory: {}MB", val / 1024 / 1024)
        }
        if let Some(val) = &self.linux_kernel {
            log::info!(target: "madara", "💻 Kernel: {}", val)
        }
    }
}
