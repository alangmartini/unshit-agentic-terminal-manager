//! OS calls behind the resource monitor. Everything here is best-effort:
//! a process that exited or cannot be opened yields `None` and the caller
//! counts it as unsampled rather than failing the tick.
//!
//! Windows and macOS use their native process APIs. Other platforms report
//! `None` from every probe, which the UI renders as `--`.

use std::collections::HashMap;

use super::cpu::ProcSample;
use super::tree::ProcessRecord;

/// Snapshot of every process on the machine as `(pid, parent pid)`.
pub fn enumerate_processes() -> Option<Vec<ProcessRecord>> {
    enumerate_processes_named().map(|(records, _)| records)
}

/// The process table plus each pid's image name (`pwsh.exe`), read from
/// the same snapshot so the two never disagree.
pub fn enumerate_processes_named() -> Option<(Vec<ProcessRecord>, HashMap<u32, String>)> {
    imp::enumerate_processes_named()
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
    use super::{HashMap, ProcSample, ProcessRecord};
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

    pub fn enumerate_processes_named() -> Option<(Vec<ProcessRecord>, HashMap<u32, String>)> {
        // SAFETY: the snapshot handle is checked against INVALID_HANDLE_VALUE
        // before use and closed on every path out. PROCESSENTRY32W is plain
        // data; dwSize is set as the API requires before the first call.
        // szExeFile is a fixed NUL-terminated UTF-16 buffer; the slice stops
        // at the first NUL or the buffer end.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut out = Vec::with_capacity(512);
            let mut names = HashMap::with_capacity(512);
            let mut ok = Process32FirstW(snapshot, &mut entry);
            while ok != 0 {
                out.push(ProcessRecord {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                });
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                if len > 0 {
                    names.insert(
                        entry.th32ProcessID,
                        String::from_utf16_lossy(&entry.szExeFile[..len]),
                    );
                }
                ok = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            Some((out, names))
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

#[cfg(target_os = "macos")]
mod imp {
    use super::{HashMap, ProcSample, ProcessRecord};
    use std::mem::{size_of, MaybeUninit};

    // These constants come from Apple's public libproc headers (`libproc.h`
    // and `sys/proc_info.h`). The libc crate supplies the matching C structs;
    // keep the bindings local so non-macOS targets do not need to know about
    // them.
    const PROC_ALL_PIDS: u32 = 1;
    const PROC_PIDTBSDINFO: i32 = libc::PROC_PIDTBSDINFO;
    const PROC_PIDTASKINFO: i32 = libc::PROC_PIDTASKINFO;
    const UNIX_EPOCH_FILETIME_100NS: u64 = 11_644_473_600 * 10_000_000;

    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_listpids(
            kind: u32,
            typeinfo: u32,
            buffer: *mut libc::c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
        fn proc_pidinfo(
            pid: libc::c_int,
            flavor: libc::c_int,
            arg: u64,
            buffer: *mut libc::c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
    }

    fn pid_arg(pid: u32) -> Option<libc::c_int> {
        libc::c_int::try_from(pid).ok()
    }

    fn read_bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
        let pid = pid_arg(pid)?;
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        // SAFETY: `info` is a properly sized writable buffer and libproc only
        // writes the documented `proc_bsdinfo` structure into it. The pid is
        // checked to fit the C API's signed `int` parameter above.
        let bytes = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size_of::<libc::proc_bsdinfo>() as libc::c_int,
            )
        };
        if bytes < size_of::<libc::proc_bsdinfo>() as libc::c_int {
            return None;
        }
        // SAFETY: libproc reported a complete structure in the buffer.
        Some(unsafe { info.assume_init() })
    }

    fn read_task_info(pid: u32) -> Option<libc::proc_taskinfo> {
        let pid = pid_arg(pid)?;
        let mut info = MaybeUninit::<libc::proc_taskinfo>::zeroed();
        // SAFETY: `info` is a properly sized writable buffer and libproc only
        // writes the documented `proc_taskinfo` structure into it.
        let bytes = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDTASKINFO,
                0,
                info.as_mut_ptr().cast(),
                size_of::<libc::proc_taskinfo>() as libc::c_int,
            )
        };
        if bytes < size_of::<libc::proc_taskinfo>() as libc::c_int {
            return None;
        }
        // SAFETY: libproc reported a complete structure in the buffer.
        Some(unsafe { info.assume_init() })
    }

    fn process_name(bytes: &[libc::c_char]) -> Option<String> {
        let len = bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(bytes.len());
        if len == 0 {
            return None;
        }
        let bytes: Vec<u8> = bytes[..len].iter().map(|&byte| byte as u8).collect();
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn process_start_100ns(info: &libc::proc_bsdinfo) -> Option<u64> {
        let seconds = info.pbi_start_tvsec.checked_mul(10_000_000)?;
        let micros = info.pbi_start_tvusec.checked_mul(10)?;
        UNIX_EPOCH_FILETIME_100NS
            .checked_add(seconds)?
            .checked_add(micros)
    }

    pub fn enumerate_processes_named() -> Option<(Vec<ProcessRecord>, HashMap<u32, String>)> {
        // A first zero-sized call asks libproc for the current buffer size.
        // The process table can grow between calls, so retry with a larger
        // buffer when the second call fills it completely.
        let required = unsafe { proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
        if required <= 0 {
            return None;
        }

        let mut pids = vec![0 as libc::pid_t; (required as usize / size_of::<libc::pid_t>()) + 64];
        let bytes = loop {
            let capacity_bytes = pids.len().saturating_mul(size_of::<libc::pid_t>());
            let bytes = unsafe {
                proc_listpids(
                    PROC_ALL_PIDS,
                    0,
                    pids.as_mut_ptr().cast(),
                    capacity_bytes.min(libc::c_int::MAX as usize) as libc::c_int,
                )
            };
            if bytes <= 0 {
                return None;
            }
            if (bytes as usize) < capacity_bytes {
                break bytes as usize;
            }
            // A continuously changing process table should not make this
            // loop unbounded. Grow by a fixed slack amount and try once more;
            // if it still fills, the returned prefix is still useful.
            let next_len = pids.len().saturating_mul(2);
            if next_len <= pids.len() {
                break bytes as usize;
            }
            pids.resize(next_len, 0);
        };

        let count = bytes / size_of::<libc::pid_t>();
        let mut records = Vec::with_capacity(count);
        let mut names = HashMap::with_capacity(count);
        for &raw_pid in pids.iter().take(count) {
            if raw_pid <= 0 {
                continue;
            }
            let pid = raw_pid as u32;
            let Some(info) = read_bsd_info(pid) else {
                continue;
            };
            records.push(ProcessRecord {
                pid,
                parent_pid: info.pbi_ppid,
            });
            if let Some(name) =
                process_name(&info.pbi_name).or_else(|| process_name(&info.pbi_comm))
            {
                names.insert(pid, name);
            }
        }
        Some((records, names))
    }

    pub fn sample_process(pid: u32) -> Option<ProcSample> {
        let bsd = read_bsd_info(pid)?;
        let task = read_task_info(pid)?;
        let creation_100ns = process_start_100ns(&bsd)?;
        // `pti_total_user` and `pti_total_system` are nanoseconds in Apple's
        // proc_taskinfo API; convert to the monitor's 100ns time unit.
        let cpu_ns = task.pti_total_user.saturating_add(task.pti_total_system);
        Some(ProcSample {
            pid,
            creation_100ns,
            cpu_100ns: cpu_ns / 100,
            working_set_bytes: task.pti_resident_size,
        })
    }

    pub fn now_100ns() -> u64 {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        UNIX_EPOCH_FILETIME_100NS
            .saturating_add(elapsed.as_secs().saturating_mul(10_000_000))
            .saturating_add(u64::from(elapsed.subsec_nanos()) / 100)
    }

    pub fn local_time_hhmm() -> Option<String> {
        let now = unsafe { libc::time(std::ptr::null_mut()) };
        if now < 0 {
            return None;
        }
        let mut local = MaybeUninit::<libc::tm>::zeroed();
        // SAFETY: `local` is writable storage for `localtime_r`; on success
        // the returned pointer aliases the initialized value in that storage.
        let result = unsafe { libc::localtime_r(&now, local.as_mut_ptr()) };
        if result.is_null() {
            return None;
        }
        // SAFETY: `localtime_r` returned the pointer to our initialized value.
        let local = unsafe { local.assume_init() };
        if !(0..=23).contains(&local.tm_hour) || !(0..=59).contains(&local.tm_min) {
            return None;
        }
        Some(format!("{:02}:{:02}", local.tm_hour, local.tm_min))
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
mod imp {
    use super::{HashMap, ProcSample, ProcessRecord};

    pub fn enumerate_processes_named() -> Option<(Vec<ProcessRecord>, HashMap<u32, String>)> {
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

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    #[test]
    fn samples_the_current_process() {
        let sample = sample_process(std::process::id()).expect("own process is sampleable");
        assert!(
            sample.working_set_bytes > 0,
            "resident memory should be readable: {sample:?}"
        );
        assert!(sample.creation_100ns > 0);
        assert!(sample.creation_100ns <= now_100ns());
    }

    #[test]
    fn impossible_pid_is_unsampled() {
        assert!(sample_process(u32::MAX).is_none());
    }

    #[test]
    fn enumeration_includes_ourselves_and_a_name() {
        let (table, names) = enumerate_processes_named().expect("libproc process snapshot");
        let pid = std::process::id();
        let me = table
            .iter()
            .find(|record| record.pid == pid)
            .expect("own pid in process snapshot");
        assert_ne!(me.parent_pid, 0);
        assert!(names.get(&pid).is_some_and(|name| !name.is_empty()));
    }

    #[test]
    fn local_time_is_hh_mm() {
        let text = local_time_hhmm().expect("local time");
        assert_eq!(text.len(), 5);
        assert_eq!(&text[2..3], ":");
    }
}
