# unicompose

A Compose key for Windows, and a bridge that types the characters a keyboard sends over Raw HID. It replaces
WinCompose.

- **Compose key:** tap Right Alt, then type a short sequence: `o` `"` gives ö, `-` `>` gives →, `<` `=`
  gives ≤. `u` followed by hex digits and Enter types any character. It uses the same rules as WinCompose,
  plus your own `.XCompose` file.
- **Raw HID keyboards:** some keyboards, such as the Summa-Cogni Mathpad, send a symbol as a vendor-defined
  HID report carrying its Unicode code point, not as keystrokes. `unicompose` types that character
  directly. Nothing passes through the keyboard layout, so symbols come out right on Hebrew or any other
  layout, and characters outside the Basic Multilingual Plane (𝔸, 𝕜) work too.

> **Status: 1.0, for Windows 10 and 11 (x64 and ARM64).** macOS and Linux are not supported yet; there,
> `--dry-run` prints what would be typed. See [Roadmap](#roadmap).

## Install

Download from the [latest release](https://github.com/shaia/unicompose/releases/latest):

- **MSI** (`unicompose-<version>-windows-x64.msi`, or `-arm64`): installs into Program Files, adds a Start
  menu entry, and starts unicompose. Uninstall it from Settings > Apps like any other program.
- **MSIX bundle** (`.msixbundle`): the same app as a modern package, for both architectures. Windows starts it
  at login; switch that off under Settings > Apps > Startup.
- **Portable zip**: the two programs and nothing else. Run `unicompose-tray.exe` from anywhere.

Or with winget, once the package is published there: `winget install shaia.unicompose`.

Every file is signed. `SHA256SUMS` in the release lists their checksums.

## Build

Requires Rust 1.85 or newer. On Linux, install `libudev-dev` first (hidapi needs it).

```sh
cargo build --release
```

This builds two programs in `target/release`:

- `unicompose-tray.exe`: the app for everyday use, with a tray icon and no console window. See
  [Tray app](#tray-app).
- `unicompose` (`unicompose.exe` on Windows): the same app on the command line, plus tools. See
  [Command line](#command-line).

## Compose key

Tap the Compose key (Right Alt by default) on its own, then type a sequence. Held down with another key,
Right Alt is still AltGr, so AltGr+4 still gives € on a UK keyboard.

After Compose:

- Keys that complete a rule type its text. The built-in rules have about 6,400 sequences.
- `u`, hex digits, then Enter or Space type any code point: Compose `u 1 d 5 3 8` Enter types 𝔸.
  - Where a rule also starts that way (Compose `u a` is ă), `unicompose` waits for the next key.
    More digits make it hex, and any other key, or a second's pause, types the rule.
  - The digits are read on a Latin keyboard layout if one is installed, so hex works while Hebrew or
    another script is active. This also makes the Mathpad's WINDOWS switch position work on Hebrew.
- Compose pressed twice starts WinCompose's emoji names: Compose Compose `w i n k i n g` types 😉.
- Escape cancels, and Backspace takes back the last key.
- A sequence that matches nothing types the keys you pressed, as WinCompose does. `--discard-invalid`
  drops them instead.

`--compose-key` picks another key: `ralt`, `lalt`, `rctrl`, `lctrl`, `rwin`, `lwin`, `menu`, `capslock`,
`scrolllock`, `pause`, `insert` or `printscreen`. `--no-compose` turns the Compose key off and keeps only
the Raw HID typing.

### Rules

`unicompose` loads the same rules as WinCompose, in the same order, so a later file wins:

1. libX11's en_US.UTF-8 rules, the standard Linux set.
2. WinCompose's `Emoji.txt` and `WinCompose.txt`. `--no-wincompose-rules` leaves these out.
3. `%USERPROFILE%\.XCompose`, then `%USERPROFILE%\.XCompose.txt`, if they exist.

Rule files use the Linux XCompose format, for example `<Multi_key> <o> <quotedbl> : "ö"`.
- `include "%L"` pulls in the built-in rules, and `%H` is your home folder.
- WinCompose's shorter key names, such as `<!>` and `< >`, work too.
- Conflicts resolve as in libxkbcommon: a later rule for the same keys wins, and a longer rule replaces a
  shorter rule that is its prefix.
- CI checks that the built-in libX11 rules produce exactly the table libxkbcommon builds.

**Reload Compose rules** in the tray menu picks up changes to your files. To check a file:

```sh
unicompose rules                    # the rules the Compose key uses, including your .XCompose
unicompose rules ~/.XCompose        # one file; problems are listed as file:line
unicompose rules ~/.XCompose --dump # print every rule after includes and overrides
unicompose rules FILE --strict      # read FILE exactly as libxkbcommon on Linux does
```

### Moving from WinCompose

The first time it runs, `unicompose` creates its settings from WinCompose's `settings.ini`:
- the Compose key;
- whether invalid sequences are discarded;
- whether hex entry and the emoji rules are on.

It also reads the same `.XCompose` files, so nothing needs copying.

While WinCompose is running, the Compose key stays off, because both would act on every tap of Right
Alt. The tray menu says so. When WinCompose quits, the Compose key turns on by itself within a few
seconds. After that you can uninstall WinCompose.

WinCompose also loads kragen's dotXCompose rules. Those have no license, so they are not built in. To keep
them, save them as your `.XCompose`.

## Settings

**Settings…** in the tray menu opens the Settings window:
- **Compose key:** on or off; which key; WinCompose's emoji sequences; hex entry; what to do with a sequence
  that matches nothing. **Edit my sequences** opens your `.XCompose` in Notepad, creating it with a short
  guide if it does not exist. **Reload** reads it again and lists any problems.
- **Keyboard:** the Raw HID profile, and optional vendor, product and usage IDs for a keyboard with no profile.
- **Workarounds:** the per-application workarounds.
- **Start at login:** off, normally, or as administrator.

OK and Apply take effect at once, without restarting. The settings are saved in
`%APPDATA%\unicompose\config.toml`, a text file you can also edit by hand; `unicompose settings` shows where it
is and what is in effect. Command-line flags override it for one run, and the Settings window greys out what a
flag has set. If the file has a mistake, unicompose names the line, runs with the defaults, and leaves the
file alone until you save from the Settings window.

## Tray app

`unicompose-tray.exe` runs the Compose key and the Raw HID typing, and shows a tray icon, a keyboard key marked with the Compose
diamond:

- **Green key:** the Compose key is on, or the keyboard is connected and typing.
- **Grey key:** nothing is active (no Compose key, and the keyboard is unplugged or not found).
- **Amber key with a pause sign:** paused. Keys pass untouched and Raw HID symbols are dropped.

Left-click the icon to pause or resume. The right-click menu shows the keyboard's status. It also offers:
- **Compose key** (on or off for now; the Settings window changes it for good);
- **Pause** and **Settings…**;
- **Start at login** and **Start at login as administrator**;
- **Reload Compose sequences**, **Open log** and **Open log folder**;
- **Quit**.

The tray app takes the same flags as `unicompose`, which override the settings for that run. Only one copy
runs per login. Starting a second copy does nothing, since two copies would type every character twice.

It has no console, so it logs to `%LOCALAPPDATA%\unicompose\logs\unicompose.<date>.log`. It keeps a week
of logs, and `RUST_LOG` sets the verbosity (default: info). Errors before the log opens, such as a bad
flag, appear in a message box.

### Start at login

**Start at login** adds `unicompose-tray.exe` to your login programs, with any flags it was started with.
It uses the same per-user list as Task Manager's Startup apps page. If you switch the entry off there,
the menu shows it unticked, and ticking it switches it back on.

**Start at login as administrator** starts it through a Task Scheduler logon task instead, as WinCompose
does. Windows keeps a normal program from reading keys meant for, or typing into, windows that run as
administrator. This option makes the Compose key and the Raw HID typing work there too. Creating or
removing the task asks for permission (UAC). Only one of the two options is on at a time.

The command line can do the same, which helps in scripts:

```sh
unicompose autostart enable                         # flags given here are passed on to unicompose-tray
unicompose autostart enable --elevated              # as administrator
unicompose autostart status
unicompose autostart disable
```

`unicompose-tray.exe` must be in the same folder as `unicompose.exe`. In the MSIX package, Windows manages
starting at login instead, so these options are not shown.

## Command line

```sh
unicompose                          # Compose key and Raw HID typing, logging to the console
unicompose --dry-run                # print U+XXXX<TAB>char instead of typing (no Compose key)
unicompose --device mathpad         # choose a keyboard profile by name
unicompose --vid 1234 --pid 5678    # a keyboard with no profile that sends the same reports
unicompose devices                  # list the profile's HID collections; * marks the one opened
unicompose devices --all            # list every HID collection on the machine
```

`unicompose` waits for the keyboard if it is not plugged in, and reconnects after an unplug. Stop it with
Ctrl+C. Logging goes to stderr; set `RUST_LOG=debug` to log every character.

### Supported keyboards

| Profile   | Keyboard                                  | HID collection                             |
|-----------|-------------------------------------------|--------------------------------------------|
| `mathpad` | Summa-Cogni Mathpad, OS switch set to MAC | `1209:2211`, usage `ff60:61` (QMK Raw HID) |

`mathpad` is the default profile. A keyboard that sends the same reports under different IDs needs no code
change: pass `--vid` and `--pid`, and `--usage-page` and `--usage` if it does not use QMK's Raw HID
collection.

The report format is 32 bytes with command byte `0x01`. Bytes 1–4 hold the code point, either as a
big-endian `u32` or as four ASCII hex digits (BMP only). See [crates/uc-codec/src/lib.rs](crates/uc-codec/src/lib.rs).

The Mathpad's WINDOWS switch position works too. It sends Compose, `u`, hex digits and Enter, which the
Compose key handles on any layout. The MAC position needs no Compose key at all.

### Recording reports

To see exactly what a keyboard sends, record it and decode the recording later:

```sh
unicompose --dry-run --record session.rec
unicompose decode session.rec       # one U+XXXX<TAB>char line per character
```

A recording is a text file: one report per line as `<ms since start><TAB><hex bytes>`, with `#` comment
lines for connect and disconnect. New reports are appended, so a file can hold several sessions.

### Per-application workarounds

A few applications mishandle characters typed the usual way, so `unicompose` types into them
differently. The foreground window's class picks the workaround. The table comes from WinCompose.

| Name          | Applications                          | What changes                                                                                                                          | Default |
|---------------|---------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------|---------|
| `gtk-astral`  | GTK 2/3 apps: GIMP, Inkscape, HexChat | Characters outside the BMP (𝔸) are typed with GTK's Ctrl+Shift+U hex entry, because GTK crashes on the surrogate pairs Windows sends. | on      |
| `office-font` | Word, Outlook                         | Text is typed in front of a zero-width space, so a symbol does not change the font. The zero-width space stays in the document.       | off     |

`--quirk office-font` turns on a workaround that is off by default, and `--no-quirks` turns them all off.

## Adding a keyboard

Each keyboard is a row in `PROFILES` in [crates/uc-app/src/device.rs](crates/uc-app/src/device.rs): a name,
the HID collection to open, and the function that decodes its reports. Firmware that sends Mathpad-style
reports reuses `uc_codec::decode`. A different report format needs a new decoder, and nothing else in the
app changes.

## Workspace

| Crate         | Purpose                                                                 |
|---------------|-------------------------------------------------------------------------|
| `uc-app`      | The `unicompose` and `unicompose-tray` binaries: command line, tray, device profiles, Compose settings, logs |
| `uc-codec`    | Decodes Unicode reports carried over Raw HID                            |
| `uc-config`   | The settings file, `config.toml`                                        |
| `uc-engine`   | The Compose key's state machine: sequences, hex entry, timeouts; no IO  |
| `uc-hid`      | Listens on one HID collection and reconnects after unplug (hidapi)      |
| `uc-keysym`   | X11 keysym names, values and characters, generated from `keysymdef.h`   |
| `uc-platform` | Traits shared by the app and the platform backends                      |
| `uc-win`      | Windows backend: keyboard hook, layout-independent typing, per-application workarounds, login entry and task, the Settings form |
| `uc-xcompose` | XCompose parser and writer with libxkbcommon's semantics; the bundled rules |
| `xtask`       | Regenerates bundled data and icons; builds the zip, MSI, MSIX and winget manifests |

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
```

Two Windows tests are ignored by default, because they take keyboard focus and type into a window:
- `focused_window` types directly under the Hebrew and French AZERTY layouts.
- `compose_hook` installs the real keyboard hook and presses keys as a keyboard would:
  - Compose sequences, hex entry and a failed sequence on UK;
  - AltGr+4 on UK;
  - hex entry while Hebrew is active.

Run them while you are at the machine, since Windows will not give them focus otherwise:

```sh
cargo test -p uc-win --test focused_window --test compose_hook -- --ignored --test-threads=1
```

CI runs formatting, clippy and tests on Windows x64, Windows ARM64, macOS and Linux, and checks that the
workspace builds with Rust 1.85.

### Releasing

1. Update `VERSION`, `version` under `[workspace.package]` in `Cargo.toml` (a test fails if they differ), and
   give [CHANGELOG.md](CHANGELOG.md) a section for the version. The release notes are taken from it.
2. Push a tag `v<version>`. [release.yml](.github/workflows/release.yml) then builds x64 and ARM64, signs
   everything with Azure Trusted Signing, and publishes the GitHub release with its `SHA256SUMS`. It needs
   the secrets and variables listed at the top of that file, and fails rather than publish unsigned files.
3. The release run's `winget-manifests` artifact holds the manifests for
   [winget-pkgs](https://github.com/microsoft/winget-pkgs).

To build the packages locally (unsigned; needs WiX 5 through `dotnet tool install --global wix --version 5.0.2`
and `wix extension add -g WixToolset.Util.wixext/5.0.2`, and the Windows SDK for `makeappx`):

```sh
cargo build --release
cargo xtask package --target x86_64-pc-windows-msvc --bins target/release   # zip, MSI, MSIX in target/dist
cargo xtask bundle
cargo xtask checksums
```

`cargo xtask assets` redraws the icon and logos in `packaging/assets` from `crates/uc-app/src/icon_art.rs`.

### Differential test against libxkbcommon

`crates/uc-xcompose/tests/differential.rs` compares our table with the one libxkbcommon builds from the same
file. It needs a dump from Linux (libxkbcommon 1.6 or newer, no headers needed); CI runs it on Ubuntu. From
WSL or Linux:

```sh
python3 tools/xkbcommon_dump.py crates/uc-xcompose/data/en_US.UTF-8.Compose > target/xkbcommon.dump
UC_XKBCOMMON_DUMP=$PWD/target/xkbcommon.dump cargo test -p uc-xcompose --test differential -- --ignored
```

Set `UC_COMPOSE_FILE` to compare another file, such as `crates/uc-xcompose/tests/data/edge-cases.Compose`.

### Regenerating bundled data

The keysym table and the libX11 rules are checked in. To update them, download xorgproto's
`include/X11/keysymdef.h` and libX11's `nls/en_US.UTF-8/Compose.pre`, then run:

```sh
cargo xtask data path/to/keysymdef.h path/to/Compose.pre
```

WinCompose's rule files in `crates/uc-xcompose/data/wincompose` are copied as they are from its
`src/wincompose/rules`.

## Roadmap

1. ~~Tray icon with pause and logs, start at login, per-app workarounds.~~ Done.
2. ~~XCompose rule parser and compose engine.~~ Done.
3. ~~Compose key on Windows, so WinCompose can be uninstalled.~~ Done.
4. Search popup: insert an emoji or symbol by name.
5. macOS support.
6. Linux support (X11, wlroots, KDE; GNOME via IBus).
7. ~~Settings window, signed installers, 1.0 release.~~ Done. A browser for all the sequences is still to
   come.

## License

[MIT](LICENSE)

Bundled data keeps its own licenses:
- **libX11 Compose rules and the keysym table:** X11/MIT-style licenses. See
  [crates/uc-xcompose/data/COPYING.libX11](crates/uc-xcompose/data/COPYING.libX11) and the notice at the top
  of `crates/uc-keysym/src/generated.rs`.
- **WinCompose's rule files:** WTFPL. See
  [crates/uc-xcompose/data/wincompose/COPYING](crates/uc-xcompose/data/wincompose/COPYING).
