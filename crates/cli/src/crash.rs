//! When Gutenberg panics, on any thread, a report goes to `crashes/` beside
//! the store (`~/.local/share/gutenberg/crashes/`), and stderr says where.
//! The report has what a bug report needs and nothing from the store:
//! the version, the thread, the panic's message and place, and a backtrace.
//! The newest [`KEPT`] reports are kept.

use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reports kept; older ones are removed as new ones are written.
const KEPT: usize = 20;

/// Writes a report on every panic, then does what Rust does anyway: the
/// message on stderr.
pub fn install() {
    let Ok(store) = gutenberg_store::Store::default_path() else {
        return;
    };
    install_in(store.with_file_name("crashes"));
}

fn install_in(dir: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        match write(&dir, &report(info, &Backtrace::force_capture())) {
            Ok(path) => eprintln!("gutenberg: crashed; the report is {}", path.display()),
            Err(err) => eprintln!("gutenberg: crashed, and the report couldn't be written: {err}"),
        }
    }));
}

fn report(info: &PanicHookInfo<'_>, backtrace: &Backtrace) -> String {
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned());
    let place = info.location().map_or("(unknown)".to_owned(), |l| {
        format!("{}:{}:{}", l.file(), l.line(), l.column())
    });
    let thread = std::thread::current()
        .name()
        .unwrap_or("(unnamed)")
        .to_owned();
    let mut out = String::new();
    let _ = writeln!(out, "Gutenberg {} crashed.", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "When: {} (seconds since 1970, UTC)", unix_now());
    let _ = writeln!(out, "System: {}", system());
    let _ = writeln!(out, "Thread: {thread}");
    let _ = writeln!(out, "Panic: {message}");
    let _ = writeln!(out, "At: {place}");
    let _ = writeln!(out, "\nBacktrace:\n{backtrace}");
    out
}

/// The distribution's name, from `os-release`.
#[cfg(target_os = "linux")]
fn system() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                Some(
                    line.strip_prefix("PRETTY_NAME=")?
                        .trim_matches('"')
                        .to_owned(),
                )
            })
        })
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

/// `macOS 15.1`, from `sw_vers`. Nothing here may panic: this runs inside
/// the panic hook.
#[cfg(target_os = "macos")]
fn system() -> String {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| format!("macOS {}", String::from_utf8_lossy(&out.stdout).trim()))
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn system() -> String {
    std::env::consts::OS.to_owned()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `crash-<seconds>-<pid>.txt`; the oldest beyond [`KEPT`] go.
fn write(dir: &Path, report: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("crash-{}-{}.txt", unix_now(), std::process::id()));
    std::fs::write(&path, report)?;
    let mut reports: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    if reports.len() > KEPT {
        reports.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        for old in &reports[..reports.len() - KEPT] {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_on_any_thread_writes_a_report() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("crashes");
        install_in(dir.clone());
        let crashed = std::thread::Builder::new()
            .name("refresh".into())
            .spawn(|| panic!("the feed was {}", "odd"))
            .unwrap()
            .join();
        let _ = std::panic::take_hook();
        assert!(crashed.is_err());

        let reports: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(reports.len(), 1);
        let text = std::fs::read_to_string(reports[0].path()).unwrap();
        assert!(text.starts_with(&format!("Gutenberg {} crashed.", env!("CARGO_PKG_VERSION"))));
        assert!(text.contains("\nThread: refresh\n"), "{text}");
        assert!(text.contains("\nPanic: the feed was odd\n"), "{text}");
        assert!(text.contains("\nAt: crates/cli/src/crash.rs:"), "{text}");
        assert!(text.contains("\nBacktrace:\n"), "{text}");
    }

    #[test]
    fn only_the_newest_reports_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..KEPT + 3 {
            std::fs::create_dir_all(tmp.path()).unwrap();
            let old = tmp.path().join(format!("crash-{i}-1.txt"));
            std::fs::write(&old, "old").unwrap();
            let file = std::fs::File::options().write(true).open(&old).unwrap();
            file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(i as u64))
                .unwrap();
        }
        std::fs::write(tmp.path().join("notes.txt"), "").unwrap();
        let newest = write(tmp.path(), "new").unwrap();
        let left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
        // KEPT reports, the newest among them, and the file that isn't one.
        assert_eq!(left.len(), KEPT + 1);
        assert!(newest.exists());
        assert!(!tmp.path().join("crash-0-1.txt").exists());
        assert!(!tmp.path().join("crash-3-1.txt").exists());
        assert!(tmp.path().join("crash-4-1.txt").exists());
    }
}
