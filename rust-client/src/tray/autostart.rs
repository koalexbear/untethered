//! Starting Untethered when you sign in, minimised to the tray: an XDG
//! autostart entry on Linux, the Run key on Windows. The entry itself is the
//! setting, so removing it in the desktop's own settings shows in ours too.

/// The argument the entry starts Untethered with: stay in the tray.
pub const HIDDEN_FLAG: &str = "--start-minimized";

/// This program, as the entry should start it. A package upgrade can replace
/// the running binary, which Linux then names "… (deleted)".
fn program() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = exe.to_str()?;
    Some(name.strip_suffix(" (deleted)").unwrap_or(name).into())
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{HIDDEN_FLAG, program};
    use crate::app_id::APP_ID;
    use std::path::{Path, PathBuf};

    const FAILED: &str = "Couldn’t change the autostart entry in ~/.config/autostart.";

    fn entry_path() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))?;
        // Named after the app's desktop file, so the desktop's autostart
        // settings show it as Untethered.
        Some(config.join("autostart").join(format!("{APP_ID}.desktop")))
    }

    /// An entry the desktop runs: present, and not switched off in place.
    pub fn runs(entry: &str) -> bool {
        !entry.lines().map(str::trim).any(|l| {
            l.eq_ignore_ascii_case("Hidden=true")
                || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
        })
    }

    pub fn enabled() -> bool {
        entry_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .is_some_and(|e| runs(&e))
    }

    /// A program path as one Exec argument (Desktop Entry spec, "The Exec
    /// key"): quoted, with the quoting's own escapes, then escaped again as a
    /// string value; `%` is the field-code escape.
    pub fn exec_arg(path: &Path) -> String {
        let s = path.to_string_lossy().replace('%', "%%");
        if !s.chars().any(|c| " \t\n\"'\\><~|&;$*?#()`".contains(c)) {
            return s;
        }
        let mut quoted = String::from("\"");
        for c in s.chars() {
            match c {
                '"' | '`' | '$' => {
                    quoted.push_str("\\\\");
                    quoted.push(c);
                }
                '\\' => quoted.push_str("\\\\\\\\"),
                _ => quoted.push(c),
            }
        }
        quoted.push('"');
        quoted
    }

    pub fn entry(program: &Path) -> String {
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Untethered\n\
             Comment=Start Untethered in the system tray\n\
             Exec={} {HIDDEN_FLAG}\n\
             Icon=internet-chat\n\
             Terminal=false\n\
             StartupWMClass={APP_ID}\n\
             X-GNOME-Autostart-enabled=true\n",
            exec_arg(program)
        )
    }

    pub fn set(on: bool) -> Result<(), &'static str> {
        let path = entry_path().ok_or(FAILED)?;
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(FAILED),
                _ => Ok(()),
            };
        }
        let program = program().ok_or(FAILED)?;
        std::fs::create_dir_all(path.parent().ok_or(FAILED)?).map_err(|_| FAILED)?;
        std::fs::write(&path, entry(&program)).map_err(|_| FAILED)
    }
}

#[cfg(windows)]
mod imp {
    use super::{HIDDEN_FLAG, program};
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
    };

    const FAILED: &str = "Couldn’t change Untethered’s entry in the Run key.";
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "Untethered";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    pub fn enabled() -> bool {
        let (key, name) = (wide(RUN), wide(NAME));
        let mut size = 0u32;
        // SAFETY: NUL-terminated strings; a null buffer only asks for the size.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        status == ERROR_SUCCESS
    }

    pub fn set(on: bool) -> Result<(), &'static str> {
        let (key, name) = (wide(RUN), wide(NAME));
        let status = if on {
            let program = program().ok_or(FAILED)?;
            let command = wide(&format!("\"{}\" {HIDDEN_FLAG}", program.display()));
            // SAFETY: NUL-terminated strings, and the data's size in bytes
            // including its NUL.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            }
        } else {
            // SAFETY: NUL-terminated strings.
            match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) } {
                ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                status => status,
            }
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(FAILED)
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    pub fn enabled() -> bool {
        false
    }
    pub fn set(_: bool) -> Result<(), &'static str> {
        Err("Starting at sign-in isn’t supported on this system yet.")
    }
}

pub use imp::{enabled, set};

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::imp::*;
    use std::path::Path;

    #[test]
    fn entry_starts_this_program_in_the_tray() {
        let e = entry(Path::new("/usr/bin/untethered"));
        assert!(e.contains("\nExec=/usr/bin/untethered --start-minimized\n"));
        assert!(e.contains("\nStartupWMClass=io.github.koalexbear.Untethered\n"));
        assert!(runs(&e));
        // Switched off in the desktop's settings without removing the file.
        assert!(!runs(&format!("{e}Hidden=true\n")));
        assert!(!runs(&e.replace("enabled=true", "enabled=false")));
    }

    #[test]
    fn exec_quotes_paths_with_spaces_and_escapes() {
        assert_eq!(
            exec_arg(Path::new("/opt/u/untethered")),
            "/opt/u/untethered"
        );
        assert_eq!(
            exec_arg(Path::new("/home/a b/$x/untethered")),
            r#""/home/a b/\\$x/untethered""#
        );
        assert_eq!(exec_arg(Path::new("/tmp/100%/u")), "/tmp/100%%/u");
    }
}
