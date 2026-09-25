//! Start at login as administrator, through a Task Scheduler logon task, as WinCompose
//! does. Windows does not let a program type into, or read keys meant for, windows that
//! run as administrator unless it runs as administrator too.
//!
//! Creating or deleting the task needs administrator rights, so those run `schtasks.exe`
//! through a UAC prompt unless this process is already elevated. Reading it does not.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, WaitForSingleObject, CREATE_NO_WINDOW, INFINITE,
};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

use crate::app::is_elevated;

/// The command line the task runs, or `None` if there is no such task.
pub fn command(name: &str) -> io::Result<Option<String>> {
    let output = Command::new("schtasks.exe")
        .args(["/Query", "/TN", name, "/XML", "ONE"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let xml = String::from_utf8_lossy(&output.stdout);
    let command = element(&xml, "Command").map(unescape).unwrap_or_default();
    let arguments = element(&xml, "Arguments").map(unescape).unwrap_or_default();
    Ok(Some(if arguments.is_empty() { command } else { format!("{command} {arguments}") }))
}

/// Creates or replaces the task: at this user's logon, run `exe arguments` with the
/// highest rights the user has. Shows a UAC prompt unless already elevated.
pub fn create(name: &str, exe: &Path, arguments: &str) -> io::Result<()> {
    let user = current_user();
    let xml = task_xml(&user, &exe.display().to_string(), arguments);
    let path = std::env::temp_dir().join(format!("unicompose-task-{}.xml", std::process::id()));
    // Task Scheduler reads task XML as UTF-16 with a byte-order mark.
    let bytes: Vec<u8> =
        std::iter::once(0xFEFF).chain(xml.encode_utf16()).flat_map(u16::to_le_bytes).collect();
    std::fs::write(&path, bytes)?;
    let result = schtasks_as_admin(&format!("/Create /TN \"{name}\" /XML \"{}\" /F", path.display()));
    let _ = std::fs::remove_file(&path);
    result
}

/// Deletes the task if it exists. Shows a UAC prompt unless already elevated.
pub fn delete(name: &str) -> io::Result<()> {
    if command(name)?.is_none() {
        return Ok(());
    }
    schtasks_as_admin(&format!("/Delete /TN \"{name}\" /F"))
}

fn current_user() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    }
}

fn task_xml(user: &str, exe: &str, arguments: &str) -> String {
    let (user, exe, arguments) = (escape(user), escape(exe), escape(arguments));
    // Unlike schtasks' defaults: keep running on battery, never time out, and run at
    // normal priority (7, the default, is below normal: bad for a keyboard hook).
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts unicompose as administrator at logon, so it can type into windows that run as administrator.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// Runs `schtasks.exe` with administrator rights and waits for it.
fn schtasks_as_admin(arguments: &str) -> io::Result<()> {
    if is_elevated() {
        let status =
            Command::new("schtasks.exe").raw_arg(arguments).creation_flags(CREATE_NO_WINDOW).status()?;
        return if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("schtasks.exe failed: {status}")))
        };
    }
    let verb = wide("runas");
    let file = wide("schtasks.exe");
    let params = wide(arguments);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: params.as_ptr(),
        nShow: SW_HIDE,
        // SAFETY: the rest of the struct is plain data for which zero means "not used".
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: `info` is filled in as ShellExecuteExW requires and its strings outlive the
    // call; the process handle it returns is waited on and closed.
    unsafe {
        if ShellExecuteExW(&mut info) == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
                return Err(io::Error::other("the administrator prompt was declined"));
            }
            return Err(error);
        }
        if info.hProcess.is_null() {
            return Err(io::Error::other("schtasks.exe did not start"));
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if code == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!("schtasks.exe failed with exit code {code}")))
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain([0]).collect()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// The text of the first `<tag>…</tag>` in `xml`.
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(&xml[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_xml_escapes_and_round_trips_the_command() {
        let xml = task_xml(r"PC\me", r"C:\Program Files\u&c\unicompose-tray.exe", "--device \"my pad\"");
        assert!(xml.contains("<UserId>PC\\me</UserId>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        let command = element(&xml, "Command").map(unescape).unwrap();
        assert_eq!(command, r#""C:\Program Files\u&c\unicompose-tray.exe""#);
        assert_eq!(element(&xml, "Arguments").map(unescape).unwrap(), "--device \"my pad\"");
    }

    #[test]
    fn a_missing_task_reads_as_none() {
        assert_eq!(command("unicompose-test-no-such-task").unwrap(), None);
    }
}
