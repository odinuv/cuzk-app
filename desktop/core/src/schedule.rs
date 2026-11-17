//! Daily Windows Task Scheduler entry that runs `"<exe>" --check`.

use crate::error::CoreError;
use std::path::Path;

type Result<T> = std::result::Result<T, CoreError>;

pub const TASK_NAME: &str = "StrazceKatastru";

pub trait Scheduler: Send + Sync {
    /// Creates or replaces the daily task. `time` is `HH:MM`.
    fn enable(&self, exe: &Path, time: &str) -> Result<()>;
    /// Succeeds when the task does not exist.
    fn disable(&self) -> Result<()>;
    /// `Some(raw /Query output)` when the task exists.
    fn status(&self) -> Result<Option<String>>;
}

/// `HH:MM` (24 h) check.
pub fn validate_time(time: &str) -> Result<()> {
    let parsed = time.split_once(':').and_then(|(h, m)| {
        let two_digits = |s: &str| s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit());
        if two_digits(h) && two_digits(m) {
            Some((h.parse::<u8>().ok()?, m.parse::<u8>().ok()?))
        } else {
            None
        }
    });
    match parsed {
        Some((h, m)) if h < 24 && m < 60 => Ok(()),
        _ => Err(CoreError::Db(format!(
            "Neplatný čas spuštění „{time}“, očekávám HH:MM."
        ))),
    }
}

/// Arguments of `schtasks` for `enable`. The `/TR` value is `"<exe>" --check` with the
/// path quoted, so paths with spaces work.
pub fn schtasks_create_args(exe: &Path, time: &str) -> Vec<String> {
    vec![
        "/Create".into(),
        "/F".into(),
        "/SC".into(),
        "DAILY".into(),
        "/TN".into(),
        TASK_NAME.into(),
        "/ST".into(),
        time.into(),
        "/TR".into(),
        format!("\"{}\" --check", exe.display()),
    ]
}

/// Drives `schtasks.exe`. Output is never parsed (it is localized); only exit codes.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsTaskScheduler;

struct Output {
    ok: bool,
    code: Option<i32>,
    stdout: String,
}

fn schtasks(args: &[String]) -> Result<Output> {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| CoreError::Db(format!("Plánovač úloh nelze spustit: {e}")))?;
    Ok(Output {
        ok: out.status.success(),
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    })
}

fn failed(what: &str, o: &Output) -> CoreError {
    let code = o.code.map_or("?".to_string(), |c| c.to_string());
    CoreError::Db(format!("Plánovač úloh: {what} selhalo (kód {code})."))
}

fn query_args() -> Vec<String> {
    vec!["/Query".into(), "/TN".into(), TASK_NAME.into()]
}

impl Scheduler for WindowsTaskScheduler {
    fn enable(&self, exe: &Path, time: &str) -> Result<()> {
        validate_time(time)?;
        let o = schtasks(&schtasks_create_args(exe, time))?;
        if o.ok {
            Ok(())
        } else {
            Err(failed("vytvoření úlohy", &o))
        }
    }

    fn disable(&self) -> Result<()> {
        if !schtasks(&query_args())?.ok {
            return Ok(()); // not present
        }
        let o = schtasks(&[
            "/Delete".into(),
            "/F".into(),
            "/TN".into(),
            TASK_NAME.into(),
        ])?;
        if o.ok {
            Ok(())
        } else {
            Err(failed("smazání úlohy", &o))
        }
    }

    fn status(&self) -> Result<Option<String>> {
        let o = schtasks(&query_args())?;
        Ok(o.ok.then_some(o.stdout))
    }
}

#[cfg(any(test, feature = "test-util"))]
pub use mock::{MockScheduler, SchedulerCall};

#[cfg(any(test, feature = "test-util"))]
mod mock {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum SchedulerCall {
        Enable { exe: PathBuf, time: String },
        Disable,
    }

    /// Records calls; `status` reports whether the last call was an `enable`.
    #[derive(Debug, Default)]
    pub struct MockScheduler {
        calls: Mutex<Vec<SchedulerCall>>,
    }

    impl MockScheduler {
        pub fn calls(&self) -> Vec<SchedulerCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Scheduler for MockScheduler {
        fn enable(&self, exe: &Path, time: &str) -> Result<()> {
            validate_time(time)?;
            self.calls.lock().unwrap().push(SchedulerCall::Enable {
                exe: exe.to_path_buf(),
                time: time.to_string(),
            });
            Ok(())
        }
        fn disable(&self) -> Result<()> {
            self.calls.lock().unwrap().push(SchedulerCall::Disable);
            Ok(())
        }
        fn status(&self) -> Result<Option<String>> {
            Ok(match self.calls.lock().unwrap().last() {
                Some(SchedulerCall::Enable { .. }) => Some("enabled".to_string()),
                _ => None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn schtasks_args_quote_exe_with_spaces() {
        let exe = PathBuf::from(r"C:\Program Files\Strazce\strazce.exe");
        let args = schtasks_create_args(&exe, "08:00");
        assert_eq!(
            args,
            vec![
                "/Create",
                "/F",
                "/SC",
                "DAILY",
                "/TN",
                "StrazceKatastru",
                "/ST",
                "08:00",
                "/TR",
                r#""C:\Program Files\Strazce\strazce.exe" --check"#,
            ]
        );
    }

    #[test]
    fn time_validation() {
        for ok in ["00:00", "08:00", "23:59"] {
            assert!(validate_time(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "8:00", "24:00", "12:60", "12-00", "ab:cd", "+1:00", "08:000",
        ] {
            assert!(validate_time(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn mock_scheduler_records() {
        let m = MockScheduler::default();
        assert_eq!(m.status().unwrap(), None);
        m.enable(Path::new("a.exe"), "09:30").unwrap();
        assert!(m.status().unwrap().is_some());
        assert!(m.enable(Path::new("a.exe"), "9:30").is_err());
        m.disable().unwrap();
        assert_eq!(m.status().unwrap(), None);
        assert_eq!(m.calls().len(), 2);
    }
}
