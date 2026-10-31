//! Checking finished files with the antivirus.
//!
//! "System" uses what the computer already trusts: Windows Attachment Services (it calls
//! whichever antivirus is installed, Defender or another, and applies its policy), or ClamAV
//! on macOS and Linux when it is installed. A custom command works everywhere.

use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scan {
    System,
    /// A command line; `{file}` becomes the file's path (else the path goes last).
    /// Exit code 0 is clean, anything else is a finding.
    Command(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Clean,
    /// What the scanner said.
    Flagged(String),
    /// No scanner to run; the file is let through (and the log says why).
    Unavailable(String),
}

pub(crate) fn scan(how: &Scan, path: &Path, source: &str, referrer: Option<&str>) -> Verdict {
    match how {
        Scan::System => system(path, source, referrer),
        Scan::Command(line) => command(&words(line), path),
    }
}

/// Splits a command line on spaces, keeping "quoted parts" together.
pub(crate) fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (_, c) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

fn command(words: &[String], path: &Path) -> Verdict {
    let Some((program, args)) = words.split_first() else {
        return Verdict::Unavailable("no scanner command set".into());
    };
    let file = path.to_string_lossy();
    let mut placed = false;
    let mut args: Vec<String> = args
        .iter()
        .map(|a| {
            placed |= a.contains("{file}");
            a.replace("{file}", &file)
        })
        .collect();
    if !placed {
        args.push(file.into_owned());
    }
    let mut cmd = Command::new(program);
    cmd.args(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    match cmd.output() {
        Err(e) => Verdict::Unavailable(format!("{program}: {e}")),
        Ok(out) if out.status.success() => Verdict::Clean,
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
            let said = text
                .lines()
                .map(str::trim)
                .rfind(|l| !l.is_empty())
                .unwrap_or("it reported a problem")
                .to_string();
            Verdict::Flagged(said)
        }
    }
}

#[cfg(windows)]
fn system(path: &Path, source: &str, referrer: Option<&str>) -> Verdict {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Shell::{AttachmentServices, IAttachmentExecute};
    use windows::core::{GUID, HSTRING};
    // Identifies KlyxDown to Attachment Services (its own prompts and policy memory).
    const CLIENT: GUID = GUID::from_u128(0x6f3c_2a51_8b7e_4d0a_9c11_7a52_e8d4_1b90);
    // SAFETY: COM is set up and torn down on this (blocking) thread around the calls, and
    // every string outlives the call it is passed to.
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| -> windows::core::Result<()> {
            let ae: IAttachmentExecute =
                CoCreateInstance(&AttachmentServices, None, CLSCTX_INPROC_SERVER)?;
            ae.SetClientGuid(&CLIENT)?;
            ae.SetLocalPath(&HSTRING::from(path.as_os_str()))?;
            ae.SetSource(&HSTRING::from(source))?;
            if let Some(r) = referrer {
                ae.SetReferrer(&HSTRING::from(r))?;
            }
            // Runs the antivirus and writes the Mark of the Web; fails when the file is blocked.
            ae.Save()
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        match result {
            Ok(()) if path.exists() => Verdict::Clean,
            Ok(()) => Verdict::Flagged("the antivirus removed it".into()),
            Err(e) => Verdict::Flagged(e.message()),
        }
    }
}

#[cfg(not(windows))]
fn system(path: &Path, _: &str, _: Option<&str>) -> Verdict {
    // clamdscan uses the running daemon (fast); clamscan loads the signatures itself.
    for scanner in ["clamdscan", "clamscan"] {
        let found = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join(scanner).is_file()))
            .unwrap_or(false);
        if found {
            return command(
                &[
                    scanner.to_string(),
                    "--no-summary".into(),
                    "--infected".into(),
                ],
                path,
            );
        }
    }
    Verdict::Unavailable("ClamAV is not installed".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_command_lines() {
        assert_eq!(
            words(r#""C:\Program Files\Scan\scan.exe" /quiet "{file}""#),
            [r"C:\Program Files\Scan\scan.exe", "/quiet", "{file}"]
        );
        assert_eq!(
            words("clamscan  --no-summary"),
            ["clamscan", "--no-summary"]
        );
        assert_eq!(words("a '' b"), ["a", "", "b"]);
    }

    #[cfg(unix)]
    #[test]
    fn exit_codes_decide() {
        let file = Path::new("/etc/hostname");
        assert_eq!(
            scan(&Scan::Command("true".into()), file, "", None),
            Verdict::Clean
        );
        let flagged = scan(
            &Scan::Command("sh -c 'echo Eicar-Test-Signature FOUND; exit 1'".into()),
            file,
            "",
            None,
        );
        assert_eq!(
            flagged,
            Verdict::Flagged("Eicar-Test-Signature FOUND".into())
        );
        assert!(matches!(
            scan(
                &Scan::Command("no-such-scanner-here".into()),
                file,
                "",
                None
            ),
            Verdict::Unavailable(_)
        ));
    }
}
