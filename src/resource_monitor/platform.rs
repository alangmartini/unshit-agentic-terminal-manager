//! OS calls behind the resource monitor. Everything here is best-effort:
//! a process that exited or cannot be opened yields `None` and the caller
//! counts it as unsampled rather than failing the tick.
//!
//! Only Windows is implemented (the app is Windows-only today). Other
//! platforms report `None` from every probe, which the UI renders as `--`.

use super::cpu::ProcSample;
use super::tree::ProcessRecord;

/// Snapshot of every process on the machine as `(pid, parent pid)`.
pub fn enumerate_processes() -> Option<Vec<ProcessRecord>> {
    imp::enumerate_processes()
}

/// Creation time, cumulative CPU time and working set of one process.
pub fn sample_process(pid: u32) -> Option<ProcSample> {
    imp::sample_process(pid)
}

/// Wall clock in FILETIME units (100ns since 1601-01-01 UTC), comparable
/// with `ProcSample::creation_100ns`.
pub fn now_100ns() -> u64 {
    imp::now_100ns()
}

/// Local wall-clock time as `HH:MM`.
pub fn local_time_hhmm() -> Option<String> {
    imp::local_time_hhmm()
}

pub fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(windows)]
mod imp {
    use super::{ProcSample, ProcessRecord};
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE, SYSTEMTIME};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTimeAsFileTime};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn filetime_to_u64(ft: FILETIME) -> u64 {
        (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
    }

    fn zero_filetime() -> FILETIME {
        FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        }
    }

    pub fn enumerate_processes() -> Option<Vec<ProcessRecord>> {
        // SAFETY: the snapshot handle is checked against INVALID_HANDLE_VALUE
        // before use and closed on every path out. PROCESSENTRY32W is plain
        // data; dwSize is set as the API requires before the first call.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut out = Vec::with_capacity(512);
            let mut ok = Process32FirstW(snapshot, &mut entry);
            while ok != 0 {
                out.push(ProcessRecord {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                });
                ok = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            Some(out)
        }
    }

    pub fn sample_process(pid: u32) -> Option<ProcSample> {
        // SAFETY: OpenProcess returns null on failure, which is checked before
        // the handle is used; every successfully opened handle is closed
        // before returning. Out-parameters are stack values of the exact
        // types the APIs document.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }

            let mut created = zero_filetime();
            let mut exited = zero_filetime();
            let mut kernel = zero_filetime();
            let mut user = zero_filetime();
            let times_ok =
                GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);

            let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let memory_ok = K32GetProcessMemoryInfo(handle, &mut counters, counters.cb);

            CloseHandle(handle);

            if times_ok == 0 {
                return None;
            }
            Some(ProcSample {
                pid,
                creation_100ns: filetime_to_u64(created),
                cpu_100ns: filetime_to_u64(kernel).saturating_add(filetime_to_u64(user)),
                working_set_bytes: if memory_ok != 0 {
                    counters.WorkingSetSize as u64
                } else {
                    0
                },
            })
        }
    }

    pub fn now_100ns() -> u64 {
        let mut now = zero_filetime();
        // SAFETY: writes a FILETIME into a valid stack location.
        unsafe { GetSystemTimeAsFileTime(&mut now) };
        filetime_to_u64(now)
    }

    pub fn local_time_hhmm() -> Option<String> {
        let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
        // SAFETY: writes a SYSTEMTIME into a valid stack location.
        unsafe { GetLocalTime(&mut time) };
        Some(format!("{:02}:{:02}", time.wHour, time.wMinute))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{ProcSample, ProcessRecord};

    pub fn enumerate_processes() -> Option<Vec<ProcessRecord>> {
        None
    }

    pub fn sample_process(_pid: u32) -> Option<ProcSample> {
        None
    }

    pub fn now_100ns() -> u64 {
        // Any monotone-ish clock works for the "born after the previous
        // tick" test when no samples ever arrive.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64 / 100)
            .unwrap_or(0)
    }

    pub fn local_time_hhmm() -> Option<String> {
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn samples_the_current_process_with_limited_access() {
        let sample = sample_process(std::process::id()).expect("own process is sampleable");
        assert!(
            sample.working_set_bytes > 0,
            "working set should be readable: {sample:?}"
        );
        assert!(sample.creation_100ns > 0);
        assert!(sample.creation_100ns <= now_100ns());
    }

    #[test]
    fn impossible_pid_is_unsampled() {
        assert!(sample_process(u32::MAX).is_none());
    }

    #[test]
    fn enumeration_includes_ourselves_and_our_parent_link() {
        let table = enumerate_processes().expect("toolhelp snapshot");
        let me = table
            .iter()
            .find(|r| r.pid == std::process::id())
            .expect("own pid in snapshot");
        assert_ne!(me.parent_pid, 0);
    }

    #[test]
    fn grandchildren_are_discovered_through_the_parent_table() {
        // cmd -> ping: two levels below us, both alive for a few seconds.
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "ping -n 6 127.0.0.1 >nul"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn cmd");
        let cmd_pid = child.id();

        // ping needs a moment to exist; poll rather than sleep a fixed time.
        let mut owner = std::collections::HashMap::new();
        let mut attributed_grandchildren = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        let roots: HashSet<u32> = [std::process::id()].into_iter().collect();
        while std::time::Instant::now() < deadline {
            let table = enumerate_processes().expect("toolhelp snapshot");
            owner = super::super::tree::attribute(&table, &roots, |pid| {
                sample_process(pid).map(|s| s.creation_100ns)
            });
            // Only children of *our* cmd count, so other tests spawning
            // processes in parallel cannot make this pass or fail.
            attributed_grandchildren = table
                .iter()
                .filter(|r| {
                    r.parent_pid == cmd_pid && owner.get(&r.pid) == Some(&std::process::id())
                })
                .count();
            if attributed_grandchildren > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();

        assert_eq!(owner.get(&cmd_pid), Some(&std::process::id()), "{owner:?}");
        assert!(
            attributed_grandchildren >= 1,
            "expected ping under cmd: {owner:?}"
        );
    }

    #[test]
    fn local_time_is_hh_mm() {
        let text = local_time_hhmm().expect("local time");
        assert_eq!(text.len(), 5);
        assert_eq!(&text[2..3], ":");
    }
}
