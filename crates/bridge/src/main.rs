//! KlyxDown native messaging host.
//!
//! The browser starts this process and talks to it over stdin/stdout with
//! native messaging frames (u32 little-endian length, then JSON). Each message
//! is relayed unchanged to the running app over a local socket (a named pipe on
//! Windows, a Unix socket elsewhere) using the same framing, and the app's reply
//! goes back to the browser. If the app is not running, it is started hidden.

// No console window when the browser starts it.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{
    io::{self, Read, Write},
    process::{Command, Stdio},
    thread::sleep,
    time::Duration,
};

/// Browsers cap host-to-extension messages at 1 MB; the app never needs more either way.
const MAX_FRAME: usize = 1 << 20;

// Keep in sync with src-tauri/src/bridge.rs.
#[cfg(windows)]
const PIPE: &str = r"\\.\pipe\net.spacie.klyxdown";

fn main() {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    loop {
        let msg = match read_frame(&mut stdin) {
            Ok(Some(msg)) => msg,
            Ok(None) => return, // browser closed the port
            Err(e) => return reply_error(&mut stdout, &e),
        };
        let reply = relay(&msg);
        let result = match reply {
            Ok(bytes) => write_frame(&mut stdout, &bytes),
            Err(e) => write_frame(&mut stdout, error_json(&e).as_bytes()),
        };
        if result.is_err() {
            return;
        }
    }
}

fn relay(msg: &[u8]) -> io::Result<Vec<u8>> {
    let mut conn = connect_or_launch()?;
    write_frame(&mut conn, msg)?;
    read_frame(&mut conn)?.ok_or_else(|| io::Error::other("KlyxDown closed the connection"))
}

fn connect_or_launch() -> io::Result<impl Read + Write> {
    if let Ok(conn) = connect() {
        return Ok(conn);
    }
    launch_app()?;
    // Cold start of the app takes a moment; wait up to about 10 seconds.
    for _ in 0..40 {
        sleep(Duration::from_millis(250));
        if let Ok(conn) = connect() {
            return Ok(conn);
        }
    }
    Err(io::Error::other("KlyxDown did not start in time"))
}

#[cfg(windows)]
fn connect() -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE)
}

#[cfg(unix)]
fn connect() -> io::Result<std::os::unix::net::UnixStream> {
    std::os::unix::net::UnixStream::connect(socket_path())
}

/// Keep in sync with src-tauri/src/bridge.rs.
#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => std::path::PathBuf::from(dir).join("net.spacie.klyxdown.sock"),
        None => std::env::temp_dir().join(format!("net.spacie.klyxdown-{}.sock", user_tag())),
    }
}

#[cfg(unix)]
fn user_tag() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".into())
}

/// Starts the app in the background (it shows its window only if a download needs confirming).
/// The app sits next to this binary, both in development (target/debug) and when installed.
///
/// The app must not inherit the browser's pipes: it would hold them open after this helper
/// exits, and the browser would wait on them. So the standard handles are made
/// non-inheritable first, and the app gets null stdio and its own process group.
fn launch_app() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let app = exe.with_file_name(if cfg!(windows) {
        "klyxdown.exe"
    } else {
        "klyxdown"
    });
    let mut cmd = Command::new(app);
    cmd.arg("--background")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        stop_inheriting_stdio();
        // Leave the browser's job object so closing the browser doesn't close KlyxDown.
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        if cmd.spawn().is_ok() {
            return Ok(());
        }
        // Some job objects forbid breaking away; start it inside the job instead.
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map(drop)
}

#[cfg(windows)]
fn stop_inheriting_stdio() {
    use std::os::windows::io::AsRawHandle;
    unsafe extern "system" {
        fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    }
    const HANDLE_FLAG_INHERIT: u32 = 1;
    for handle in [
        io::stdin().as_raw_handle(),
        io::stdout().as_raw_handle(),
        io::stderr().as_raw_handle(),
    ] {
        // SAFETY: these are this process's own standard handles; clearing a flag on an
        // invalid or null handle just fails, which is harmless here.
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
    }
}

fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::other("message too large"));
    }
    let mut buf = vec![0; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

fn write_frame(w: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(bytes)?;
    w.flush()
}

fn error_json(e: &io::Error) -> String {
    let text = e.to_string().replace('\\', "\\\\").replace('"', "\\\"");
    format!(r#"{{"ok":false,"error":"{text}"}}"#)
}

fn reply_error(out: &mut impl Write, e: &io::Error) {
    let _ = write_frame(out, error_json(e).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, br#"{"type":"ping"}"#).unwrap();
        assert_eq!(&buf[..4], &15u32.to_le_bytes());
        let back = read_frame(&mut buf.as_slice()).unwrap().unwrap();
        assert_eq!(back, br#"{"type":"ping"}"#);
        assert!(read_frame(&mut [].as_slice()).unwrap().is_none());
    }

    #[test]
    fn oversized_frames_are_refused() {
        let mut buf = ((MAX_FRAME + 1) as u32).to_le_bytes().to_vec();
        buf.extend([0; 8]);
        assert!(read_frame(&mut buf.as_slice()).is_err());
    }

    #[test]
    fn errors_are_valid_json_strings() {
        let json = error_json(&io::Error::other(r#"bad "pipe" C:\x"#));
        assert_eq!(json, r#"{"ok":false,"error":"bad \"pipe\" C:\\x"}"#);
    }
}
