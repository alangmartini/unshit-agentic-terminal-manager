//! Windows named-pipe transport.
//!
//! Tokio's named-pipe server works in rounds: each accept requires a
//! fresh `NamedPipeServer` built with the same path. The first instance
//! is created with `first_pipe_instance(true)` so a second bind on the
//! same path fails with `ERROR_ACCESS_DENIED`. That is our single-
//! instance guard per SPEC.md section 2.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};

/// Server-side connection handed out by `Server::accept`.
pub type Connection = NamedPipeServer;

/// Client-side connection returned by [`connect`].
pub type ClientConnection = NamedPipeClient;

/// Listens on a named pipe and yields connections one at a time.
///
/// Holds the path so we can keep re-creating server instances across
/// accepts; the pipe is torn down when `Server` is dropped.
pub struct Server {
    path: PathBuf,
    // Pending instance waiting for a client. `None` between `accept`
    // calls, populated again on the next call.
    pending: Option<NamedPipeServer>,
}

impl Server {
    /// Binds to `path`. Fails with `AlreadyExists` if another daemon
    /// already owns this pipe.
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let server = create_instance(&path, true)?;
        Ok(Self {
            path,
            pending: Some(server),
        })
    }

    /// Waits for a client to connect and returns the resulting
    /// connection. The next accept rebuilds a fresh pending instance
    /// so we keep serving.
    pub async fn accept(&mut self) -> io::Result<Connection> {
        let server = self
            .pending
            .take()
            .expect("pending instance must always be populated between accepts");
        server.connect().await?;
        // Prepare the next instance so the path stays owned by us.
        self.pending = Some(create_instance(&self.path, false)?);
        Ok(server)
    }
}

fn create_instance(path: &Path, first: bool) -> io::Result<NamedPipeServer> {
    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first);
    opts.reject_remote_clients(true);
    let result = create_owner_only_pipe(&opts, path);
    result.map_err(|e| {
        if first && e.kind() == io::ErrorKind::PermissionDenied {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "another daemon owns this pipe",
            )
        } else {
            e
        }
    })
}

/// Create the pipe with a protected DACL. Windows' default named-pipe DACL
/// grants read access to Everyone, which is too broad because attach responses
/// carry the per-PTY capability used to authenticate recovery hooks.
fn create_owner_only_pipe(opts: &ServerOptions, path: &Path) -> io::Result<NamedPipeServer> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    // Protected DACL: object owner, LocalSystem, and local Administrators only.
    // `OW` is the Owner Rights SID and resolves against the descriptor owner
    // assigned from the creating process token.
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;BA)"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated and `descriptor` is a valid out pointer.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: `attributes` and its LocalAlloc-backed descriptor stay alive for
    // the full CreateNamedPipe call. The OS copies the descriptor into the
    // kernel object before returning.
    let result = unsafe {
        opts.create_with_security_attributes_raw(
            path,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>(),
        )
    };
    // SAFETY: successful conversion returns a LocalAlloc allocation owned by
    // the caller. It is no longer needed after CreateNamedPipe returns.
    unsafe {
        LocalFree(descriptor.cast());
    }
    result
}

/// Connects to a daemon already listening on `path`.
pub async fn connect(path: impl AsRef<Path>) -> io::Result<ClientConnection> {
    let connection = ClientOptions::new().open(path.as_ref())?;
    verify_server_owner(&connection)?;
    Ok(connection)
}

/// Fail closed unless the named-pipe server belongs to the same Windows user
/// as this client. A protected DACL secures pipes we create, but it cannot stop
/// another local account from pre-binding a predictable name before startup.
/// This check runs before callers serialize or write any request bytes.
fn verify_server_owner(connection: &ClientConnection) -> io::Result<()> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut server_pid = 0;
    // SAFETY: the Tokio client owns a valid pipe handle for the duration of
    // this call, and `server_pid` is a valid writable out parameter.
    if unsafe { GetNamedPipeServerProcessId(connection.as_raw_handle() as HANDLE, &mut server_pid) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `server_pid` came from the connected kernel pipe object.
    let server_process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, server_pid) };
    if server_process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let server_process = OwnedWinHandle(server_process);
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle that must not be
    // closed and remains valid for the lifetime of this process.
    let current_sid = process_owner_sid(unsafe { GetCurrentProcess() })?;
    let server_sid = process_owner_sid(server_process.0)?;
    ensure_same_owner(&current_sid, &server_sid)
}

#[cfg(windows)]
struct OwnedWinHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for OwnedWinHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper is constructed only from owned handles returned
        // by OpenProcess/OpenProcessToken and drops each handle exactly once.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn process_owner_sid(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{
        GetLengthSid, GetTokenInformation, IsValidSid, TokenUser, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::OpenProcessToken;

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `process` is a live process handle or the documented current-
    // process pseudo-handle; `token` is a valid writable out parameter.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedWinHandle(token);
    let mut required = 0;
    // The first call intentionally supplies no buffer to obtain its size.
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut required);
    }
    if required < std::mem::size_of::<TOKEN_USER>() as u32 {
        return Err(io::Error::last_os_error());
    }
    // `usize` storage gives TOKEN_USER its required pointer alignment.
    let word_count = (required as usize).div_ceil(std::mem::size_of::<usize>());
    let mut storage = vec![0usize; word_count];
    // SAFETY: storage is aligned and at least `required` bytes long; the token
    // remains open while the returned SID pointer is inspected and copied.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful TokenUser output starts with a fully initialized
    // TOKEN_USER structure in our aligned buffer.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    // SAFETY: the SID pointer belongs to the TokenUser buffer above.
    if user.User.Sid.is_null() || unsafe { IsValidSid(user.User.Sid) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pipe server token contained an invalid owner SID",
        ));
    }
    // SAFETY: a validated SID reports its own bounded byte length and remains
    // backed by `storage` until the copy completes.
    let sid_len = unsafe { GetLengthSid(user.User.Sid) } as usize;
    if sid_len == 0 || sid_len > required as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pipe server token contained an invalid owner SID length",
        ));
    }
    // SAFETY: the validated SID contains `sid_len` readable bytes.
    Ok(unsafe { std::slice::from_raw_parts(user.User.Sid.cast::<u8>(), sid_len) }.to_vec())
}

fn ensure_same_owner(expected: &[u8], actual: &[u8]) -> io::Result<()> {
    if expected == actual && !expected.is_empty() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local IPC server belongs to a different user",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn unique_pipe_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        PathBuf::from(format!(r"\\.\pipe\unshit-ptyd-test-{pid}-{n}"))
    }

    #[tokio::test]
    async fn client_and_server_exchange_bytes() {
        let path = unique_pipe_path();
        let mut server = Server::bind(&path).unwrap();

        let client_path = path.clone();
        let client_task = tokio::spawn(async move {
            // Retry briefly because the server might not be waiting yet.
            for _ in 0..50 {
                match connect(&client_path).await {
                    Ok(mut c) => {
                        c.write_all(b"ping").await.unwrap();
                        let mut buf = [0u8; 4];
                        c.read_exact(&mut buf).await.unwrap();
                        return buf;
                    }
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
                }
            }
            panic!("client could not connect");
        });

        let mut conn = server.accept().await.unwrap();
        let mut got = [0u8; 4];
        conn.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ping");
        conn.write_all(b"pong").await.unwrap();

        let client_got = client_task.await.unwrap();
        assert_eq!(&client_got, b"pong");
    }

    #[tokio::test]
    async fn second_bind_on_same_path_is_rejected() {
        let path = unique_pipe_path();
        let _first = Server::bind(&path).unwrap();
        match Server::bind(&path) {
            Ok(_) => panic!("second bind should have failed"),
            Err(e) => assert_eq!(
                e.kind(),
                io::ErrorKind::AlreadyExists,
                "second bind should surface AlreadyExists: {e:?}"
            ),
        }
    }

    #[test]
    fn mismatched_pipe_server_owner_is_rejected() {
        let error = ensure_same_owner(b"current-user-sid", b"squatter-user-sid")
            .expect_err("different user must be rejected");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
