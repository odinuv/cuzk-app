//! Daily Windows Task Scheduler entry that runs `"<exe>" --check`, created from an XML
//! definition so missed runs catch up and battery power does not block them.

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

/// Escapes text for XML element content and attribute values.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Task Scheduler definition of the daily task: runs `"<exe>" --check` every day at `time`
/// (`HH:MM`, local time) for the logged-on user. A run missed while the PC was off or asleep
/// starts as soon as possible (`StartWhenAvailable`), and battery power neither prevents nor
/// stops it (the `/SC DAILY` defaults do both).
pub fn task_xml(exe: &Path, time: &str) -> String {
    let exe = xml_escape(&exe.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Strážce katastru: denní kontrola sledovaných nemovitostí</Description>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>2024-01-01T{time}:00</StartBoundary>
      <Enabled>true</Enabled>
      <ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT1H</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
      <Arguments>--check</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// File bytes `schtasks /XML` expects: UTF-16 LE with a byte order mark.
pub fn utf16le_with_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

/// Arguments of `schtasks` for `enable`: create (or replace) the task from an XML file.
pub fn schtasks_create_args(xml_file: &Path) -> Vec<String> {
    vec![
        "/Create".into(),
        "/XML".into(),
        xml_file.display().to_string(),
        "/TN".into(),
        TASK_NAME.into(),
        "/F".into(),
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
        let file = std::env::temp_dir().join(format!("strazce-task-{}.xml", std::process::id()));
        std::fs::write(&file, utf16le_with_bom(&task_xml(exe, time)))
            .map_err(|e| CoreError::Db(format!("Definici úlohy nelze zapsat: {e}")))?;
        let o = schtasks(&schtasks_create_args(&file));
        let _ = std::fs::remove_file(&file);
        let o = o?;
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
        /// Makes `status` fail.
        pub fail_status: std::sync::atomic::AtomicBool,
        /// Makes `enable` fail (the attempt is still recorded in `calls`).
        pub fail_enable: std::sync::atomic::AtomicBool,
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
            if self.fail_enable.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(CoreError::Network("mock enable failure".into()));
            }
            Ok(())
        }
        fn disable(&self) -> Result<()> {
            self.calls.lock().unwrap().push(SchedulerCall::Disable);
            Ok(())
        }
        fn status(&self) -> Result<Option<String>> {
            if self.fail_status.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(CoreError::Network("mock status failure".into()));
            }
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
    fn schtasks_args_create_from_xml_file() {
        let xml = PathBuf::from(r"C:\Users\a b\AppData\Local\Temp\strazce-task.xml");
        assert_eq!(
            schtasks_create_args(&xml),
            vec![
                "/Create",
                "/XML",
                r"C:\Users\a b\AppData\Local\Temp\strazce-task.xml",
                "/TN",
                "StrazceKatastru",
                "/F",
            ]
        );
    }

    #[test]
    fn task_xml_catches_up_runs_on_battery_and_quotes_exe() {
        let exe = PathBuf::from(r"C:\Program Files\R&D <x>\strazce.exe");
        let xml = task_xml(&exe, "07:05");
        for needle in [
            "<StartBoundary>2024-01-01T07:05:00</StartBoundary>",
            "<ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>",
            "<StartWhenAvailable>true</StartWhenAvailable>",
            "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>",
            "<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>",
            "<LogonType>InteractiveToken</LogonType>",
            "<ExecutionTimeLimit>PT1H</ExecutionTimeLimit>",
            r#"<Command>"C:\Program Files\R&amp;D &lt;x&gt;\strazce.exe"</Command>"#,
            "<Arguments>--check</Arguments>",
        ] {
            assert!(xml.contains(needle), "missing {needle} in\n{xml}");
        }
        assert!(xml.starts_with(r#"<?xml version="1.0" encoding="UTF-16"?>"#));
        assert!(!xml.contains("R&D"), "unescaped ampersand");
    }

    #[test]
    fn utf16_file_bytes_have_bom_and_little_endian() {
        assert_eq!(
            utf16le_with_bom("Až"),
            vec![0xFF, 0xFE, b'A', 0, 0x7E, 0x01]
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
