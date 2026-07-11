//! Process memory sampling used by daemon session inspection.
//!
//! Values are resident/working-set bytes. Sampling is best-effort:
//! missing permissions, exited processes, and unsupported platforms
//! return `None` so session listing stays reliable.

pub fn current_resident_set_bytes() -> Option<u64> {
    resident_set_bytes(std::process::id())
}

pub fn resident_set_bytes(pid: u32) -> Option<u64> {
    platform_resident_set_bytes(pid)
}

#[cfg(windows)]
fn platform_resident_set_bytes(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    // SAFETY: OpenProcess/GetProcessMemoryInfo/CloseHandle are called with
    // OS-owned handles only. Null handles are checked before use, and every
    // opened real handle is closed before returning.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle.is_null() {
            return None;
        }

        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        let ok = K32GetProcessMemoryInfo(handle, &mut counters, counters.cb);
        CloseHandle(handle);

        (ok != 0).then_some(counters.WorkingSetSize as u64)
    }
}

#[cfg(target_os = "linux")]
fn platform_resident_set_bytes(pid: u32) -> Option<u64> {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let resident_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(resident_pages.saturating_mul(linux_page_size()))
}

#[cfg(target_os = "linux")]
fn linux_page_size() -> u64 {
    static PAGE_SIZE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *PAGE_SIZE.get_or_init(detect_linux_page_size)
}

#[cfg(target_os = "linux")]
fn detect_linux_page_size() -> u64 {
    const DEFAULT_PAGE_SIZE: u64 = 4096;
    std::process::Command::new("getconf")
        .arg("PAGESIZE")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .filter(|bytes| *bytes > 0)
        .unwrap_or(DEFAULT_PAGE_SIZE)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_resident_set_bytes(_pid: u32) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_memory_sampling_is_best_effort() {
        let sample = current_resident_set_bytes();
        if let Some(bytes) = sample {
            assert!(bytes > 0);
        }
    }

    #[test]
    fn impossible_pid_returns_none() {
        assert_eq!(resident_set_bytes(u32::MAX), None);
    }
}
