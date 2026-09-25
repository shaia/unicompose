# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). The version is kept in `VERSION` and in
`[workspace.package]` in `Cargo.toml`; a test fails if they differ.

## [Unreleased]

## [1.0.0] - 2026-09-25

The first release, for Windows 10 and 11 on x64 and ARM64.

### Added

- Settings window, from the tray menu: Compose key, sequences (with Edit and Reload), the Raw HID keyboard,
  workarounds and starting at login. Changes apply at once.
- Settings are kept in `%APPDATA%\unicompose\config.toml`, created on the first run from WinCompose's
  settings if it is installed. Command-line flags override it for one run. `unicompose settings` shows the
  file and what is in effect. A broken file is reported by line and left alone.
- Installers: an MSI per architecture (Program Files, Start menu, clean uninstall that also removes the
  login entries), an MSIX bundle that starts at login through Windows' startup tasks, and a portable zip.
  All are signed with Azure Trusted Signing, and winget manifests are produced with each release.
- The programs carry an icon, version information and a manifest for modern controls and per-monitor DPI.

- Compose key on Windows, through a low-level keyboard hook on its own thread. Tap Right Alt (or the key
  from `--compose-key`) and type a sequence. Held with another key, Right Alt stays AltGr.
- Hex entry: `u`, hex digits, then Enter or Space. The digits are read on a Latin layout while another
  script is active, so it works on Hebrew, including from the Mathpad's WINDOWS switch position. Where a
  rule also starts with `u` and a hex digit (`u a` is ă), the engine waits for the next key or a second.
- WinCompose's emoji and extra rules are built in, and `%USERPROFILE%\.XCompose` and `.XCompose.txt` are
  read after them, in WinCompose's order. Compose pressed twice starts WinCompose's emoji names. Rule files
  may use WinCompose's single-character key names such as `<!>` and `< >`.
- Defaults come from WinCompose's `settings.ini` when it exists: Compose key, discarding invalid
  sequences, hex entry, emoji rules. While WinCompose runs, the Compose key stays off. The tray turns it on
  once WinCompose quits.
- A sequence that matches nothing types its keys, or `--discard-invalid` drops them.
- A watchdog reinstalls the hook if it has seen no key for five minutes while the machine is in use, in
  case Windows removed it. The hook thread runs at high priority and opts out of EcoQoS throttling.
- Holding Right Alt with another key replays Windows' fake Left Ctrl along with Right Alt, so AltGr
  characters (€ on UK) still work, and releases that Ctrl afterwards so it cannot stick.
- Tray menu: Compose key on or off, Reload Compose rules, and Start at login as administrator. The
  administrator option uses a Task Scheduler logon task, so the Compose key and typing also work in
  windows that run as administrator. `unicompose autostart enable --elevated` does the same.
- `unicompose rules` without a file checks the full rule set the Compose key uses. With a file, it reads
  WinCompose's key names too, and `--strict` reads exactly as libxkbcommon does.
- XCompose rule parser (`uc-xcompose`) that reads files exactly as libxkbcommon does: includes with `%H`,
  `%L` and `%S`, string escapes, modifiers (accepted and ignored), and the same handling of conflicting
  sequences. It matches libxkbcommon on all 5,145 rules of libX11's en_US.UTF-8 file and on a file of edge
  cases, and CI checks both on Linux.
- libX11's en_US.UTF-8 Compose rules are built in and are what `include "%L"` reads.
- Compose engine (`uc-engine`): walks the rules, types `u` + hex digits + Enter as any code point, lets
  Backspace take back a key and Escape or Compose cancel, and waits with a timeout on a sequence that is
  complete but could continue. It is not connected to the keyboard yet.
- `unicompose rules [FILE] [--dump]` checks a Compose file and lists its problems as `file:line`.
- X11 keysym names and characters (`uc-keysym`), generated from xorgproto's `keysymdef.h` with
  `cargo xtask data`.
- `unicompose` binary: types each character a keyboard sends over Raw HID into the focused window.
- Windows typing through `SendInput` with `KEYEVENTF_UNICODE`, so output does not depend on the active
  keyboard layout (Hebrew works) and characters outside the BMP, such as 𝔸 (U+1D538), arrive as one
  surrogate pair.
- Held Ctrl, Alt and Win keys are released before typing, so a symbol never turns into a shortcut, and
  releasing Alt or Win afterwards does not open the menu bar or the Start menu.
- Device profiles. The Summa-Cogni Mathpad (OS switch set to MAC) ships as the default profile.
  `--device NAME` picks a profile; `--vid`, `--pid`, `--usage-page` and `--usage` reach a keyboard that
  has no profile.
- Reconnects on its own after the keyboard is unplugged and plugged back in.
- `--dry-run` prints `U+XXXX<TAB>char` instead of typing. This is the only mode on macOS and Linux for now.
- `--record FILE` appends every raw report to a text file; `decode FILE` replays a recording through a
  profile's codec.
- `devices` lists HID collections and marks the one the profile opens; `--all` lists every collection.
- Reports carrying a control character, a surrogate or a value above U+10FFFF are logged and dropped
  rather than typed. Reports with an unknown command byte are ignored.
- `unicompose-tray.exe`, a tray app with no console window. Its icon shows whether the keyboard is
  connected, and a left click pauses or resumes typing. The menu shows the status and has Pause, Start at
  login, Open log, Open log folder and Quit. Only one copy runs per login.
- Start at login through the per-user Run key, from the tray menu or with
  `unicompose autostart enable|disable|status`. The entry keeps the device and quirk flags, and the menu
  respects an entry switched off in Task Manager.
- The tray app logs to daily files in `%LOCALAPPDATA%\unicompose\logs` and keeps a week of them.
- Per-application workarounds from WinCompose. `gtk-astral` (on by default) types characters outside the
  BMP into GTK apps through Ctrl+Shift+U, since GTK on Windows crashes on surrogate pairs. `office-font`
  (off by default) stops Word and Outlook from changing font after a symbol. `--quirk NAME` and
  `--no-quirks` control them.
