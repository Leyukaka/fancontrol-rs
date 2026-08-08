//! Host CPU activity: global load % + top processes (CPU + RAM).
//!
//! Windows: `GetSystemTimes` + Toolhelp + `GetProcessTimes` + working set.
//! No PowerShell, no WMI. Enabled from UI (thread idles when disabled).
//! Process enumeration can be disabled (Load-only mode) to save CPU.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// One process row for the Activity deck.
#[derive(Debug, Clone)]
pub struct ProcessRow {
    pub pid: u32,
    pub name: String,
    /// Machine-relative CPU % (0–100 scale, Task Manager–like).
    pub cpu_pct: f64,
    pub ram_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ActivitySnapshot {
    /// Global CPU load 0–100. `None` until the second sample.
    pub load_pct: Option<f64>,
    pub processes: Vec<ProcessRow>,
    pub updated: Option<Instant>,
}

struct Shared {
    enabled: AtomicBool,
    /// When false, only global CPU load is sampled (no Toolhelp / per-PID).
    sample_processes: AtomicBool,
    snap: Mutex<ActivitySnapshot>,
}

fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(|| Shared {
        enabled: AtomicBool::new(false),
        sample_processes: AtomicBool::new(true),
        snap: Mutex::new(ActivitySnapshot::default()),
    })
}

/// Enable/disable background sampling (UI Activity toggle).
pub fn set_enabled(on: bool) {
    let s = shared();
    s.enabled.store(on, Ordering::Relaxed);
    if on {
        ensure_thread();
    } else {
        // Clear so UI doesn't show stale rows after disable.
        if let Ok(mut g) = s.snap.lock() {
            *g = ActivitySnapshot::default();
        }
    }
}

/// Whether to enumerate processes (false = load % only — much cheaper).
pub fn set_sample_processes(on: bool) {
    shared().sample_processes.store(on, Ordering::Relaxed);
    if !on && let Ok(mut g) = shared().snap.lock() {
        g.processes.clear();
    }
}

pub fn is_enabled() -> bool {
    shared().enabled.load(Ordering::Relaxed)
}

/// Latest snapshot (empty when disabled or not yet sampled).
pub fn snapshot() -> ActivitySnapshot {
    shared().snap.lock().map(|g| g.clone()).unwrap_or_default()
}

fn ensure_thread() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        thread::Builder::new()
            .name("cpu-activity".into())
            .spawn(|| {
                #[cfg(windows)]
                let mut sampler = windows_impl::Sampler::new();
                loop {
                    let s = shared();
                    if !s.enabled.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(400));
                        continue;
                    }
                    #[cfg(windows)]
                    {
                        let want_procs = s.sample_processes.load(Ordering::Relaxed);
                        let next = sampler.sample(want_procs);
                        if let Ok(mut g) = s.snap.lock() {
                            *g = next;
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            })
            .ok();
    });
}

#[cfg(windows)]
mod windows_impl {
    use super::{ActivitySnapshot, ProcessRow};
    use std::collections::HashMap;
    use std::mem::{size_of, zeroed};
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, GetSystemTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn filetime_u64(ft: FILETIME) -> u64 {
        (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
    }

    fn system_times() -> Option<(u64, u64, u64)> {
        unsafe {
            let mut idle: FILETIME = zeroed();
            let mut kernel: FILETIME = zeroed();
            let mut user: FILETIME = zeroed();
            if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
                return None;
            }
            Some((filetime_u64(idle), filetime_u64(kernel), filetime_u64(user)))
        }
    }

    pub struct Sampler {
        prev_idle: Option<u64>,
        prev_kernel: Option<u64>,
        prev_user: Option<u64>,
        prev_proc_cpu: HashMap<u32, u64>,
        prev_wall: Option<Instant>,
        ncpus: f64,
    }

    impl Sampler {
        pub fn new() -> Self {
            let ncpus = std::thread::available_parallelism()
                .map(|n| n.get() as f64)
                .unwrap_or(1.0)
                .max(1.0);
            Self {
                prev_idle: None,
                prev_kernel: None,
                prev_user: None,
                prev_proc_cpu: HashMap::new(),
                prev_wall: None,
                ncpus,
            }
        }

        pub fn sample(&mut self, want_processes: bool) -> ActivitySnapshot {
            let now = Instant::now();
            let load_pct = self.sample_load();
            let processes = if want_processes {
                self.sample_processes(now)
            } else {
                // Keep prev_proc_cpu coherent if we re-enable processes later:
                // drop map so next process pass starts clean.
                self.prev_proc_cpu.clear();
                self.prev_wall = None;
                Vec::new()
            };
            ActivitySnapshot {
                load_pct,
                processes,
                updated: Some(now),
            }
        }

        fn sample_load(&mut self) -> Option<f64> {
            let (idle, kernel, user) = system_times()?;
            let load = match (self.prev_idle, self.prev_kernel, self.prev_user) {
                (Some(pi), Some(pk), Some(pu)) => {
                    let d_idle = idle.saturating_sub(pi) as f64;
                    let d_kernel = kernel.saturating_sub(pk) as f64;
                    let d_user = user.saturating_sub(pu) as f64;
                    // Kernel time includes idle on Windows.
                    let total = d_kernel + d_user;
                    if total <= 0.0 {
                        None
                    } else {
                        let busy = (d_kernel + d_user - d_idle).max(0.0);
                        Some(((busy / total) * 100.0).clamp(0.0, 100.0))
                    }
                }
                _ => None,
            };
            self.prev_idle = Some(idle);
            self.prev_kernel = Some(kernel);
            self.prev_user = Some(user);
            load
        }

        fn sample_processes(&mut self, now: Instant) -> Vec<ProcessRow> {
            let wall_secs = self
                .prev_wall
                .map(|t| now.duration_since(t).as_secs_f64())
                .unwrap_or(0.0);
            self.prev_wall = Some(now);

            let mut current_cpu: HashMap<u32, u64> = HashMap::new();
            let mut rows: Vec<ProcessRow> = Vec::new();

            unsafe {
                let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
                if snap == INVALID_HANDLE_VALUE {
                    self.prev_proc_cpu = current_cpu;
                    return rows;
                }

                let mut entry: PROCESSENTRY32W = zeroed();
                entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

                if Process32FirstW(snap, &mut entry) != 0 {
                    loop {
                        let pid = entry.th32ProcessID;
                        if pid > 0
                            && let Some((row, cpu_abs)) = probe_process(
                                pid,
                                &entry,
                                &self.prev_proc_cpu,
                                wall_secs,
                                self.ncpus,
                            )
                        {
                            current_cpu.insert(pid, cpu_abs);
                            rows.push(row);
                        }
                        if Process32NextW(snap, &mut entry) == 0 {
                            break;
                        }
                    }
                }
                CloseHandle(snap);
            }

            self.prev_proc_cpu = current_cpu;

            rows.sort_by(|a, b| {
                b.cpu_pct
                    .partial_cmp(&a.cpu_pct)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.ram_bytes.cmp(&a.ram_bytes))
            });
            rows.truncate(64);
            rows
        }
    }

    fn wide_to_string(buf: &[u16]) -> String {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }

    /// Returns (row, absolute cpu time 100ns units).
    fn probe_process(
        pid: u32,
        entry: &PROCESSENTRY32W,
        prev: &HashMap<u32, u64>,
        wall_secs: f64,
        ncpus: f64,
    ) -> Option<(ProcessRow, u64)> {
        unsafe {
            let h: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() || h == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut create: FILETIME = zeroed();
            let mut exit: FILETIME = zeroed();
            let mut kernel: FILETIME = zeroed();
            let mut user: FILETIME = zeroed();
            let times_ok = GetProcessTimes(h, &mut create, &mut exit, &mut kernel, &mut user);

            let mut mem: PROCESS_MEMORY_COUNTERS = zeroed();
            mem.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let mem_ok = GetProcessMemoryInfo(h, &mut mem, mem.cb);

            CloseHandle(h);

            if times_ok == 0 {
                return None;
            }
            let cpu_abs = filetime_u64(kernel).saturating_add(filetime_u64(user));
            let name = wide_to_string(&entry.szExeFile);
            if name.is_empty() {
                return None;
            }
            let ram = if mem_ok != 0 {
                mem.WorkingSetSize as u64
            } else {
                0
            };

            let cpu_pct = if wall_secs > 0.05 {
                if let Some(&prev_cpu) = prev.get(&pid) {
                    // FILETIME is 100ns units → seconds
                    let d_cpu = cpu_abs.saturating_sub(prev_cpu) as f64 * 1e-7;
                    let pct = (d_cpu / (wall_secs * ncpus)) * 100.0;
                    pct.clamp(0.0, 100.0)
                } else {
                    0.0
                }
            } else {
                0.0
            };

            Some((
                ProcessRow {
                    pid,
                    name,
                    cpu_pct,
                    ram_bytes: ram,
                },
                cpu_abs,
            ))
        }
    }
}
