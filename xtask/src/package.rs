//! Release packaging, from binaries already built (and, in CI, already signed):
//!
//! - `package --target <triple> --bins <dir> [--out <dir>] [--publisher <subject>]`:
//!   the portable zip, the MSI (WiX 5) and the MSIX for one architecture.
//! - `bundle [--out <dir>]`: the architectures' MSIX packages in one `.msixbundle`.
//! - `checksums [--out <dir>]`: `SHA256SUMS` for everything in the folder.
//! - `winget --url-base <url> [--out <dir>]`: winget manifests for the MSIs there.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::{root, Error};

/// Where `package` writes, unless `--out` says otherwise.
const DEFAULT_OUT: &str = "target/dist";
/// The publisher for local, unsigned test builds of the MSIX.
const DEV_PUBLISHER: &str = "CN=unicompose-dev";
const PACKAGE_ID: &str = "shaia.unicompose";

struct Arch {
    /// What WiX, MSIX and winget call it.
    name: &'static str,
}

fn arch(triple: &str) -> Result<Arch, Error> {
    match triple {
        "x86_64-pc-windows-msvc" => Ok(Arch { name: "x64" }),
        "aarch64-pc-windows-msvc" => Ok(Arch { name: "arm64" }),
        other => {
            Err(format!("unsupported target {other}; use x86_64-pc-windows-msvc or aarch64-pc-windows-msvc")
                .into())
        }
    }
}

/// `--name value` pairs.
fn option<'a>(args: &'a [&'a str], name: &str) -> Option<&'a str> {
    args.windows(2).find(|pair| pair[0] == name).map(|pair| pair[1])
}

fn out_dir(args: &[&str]) -> PathBuf {
    let out = option(args, "--out").map_or_else(|| root().join(DEFAULT_OUT), PathBuf::from);
    if out.is_absolute() {
        out
    } else {
        root().join(out)
    }
}

pub fn version() -> Result<String, Error> {
    Ok(std::fs::read_to_string(root().join("VERSION"))?.trim().to_owned())
}

pub fn package(args: &[&str]) -> Result<(), Error> {
    let triple = option(args, "--target").ok_or("--target is required")?;
    let arch = arch(triple)?;
    let bins = PathBuf::from(option(args, "--bins").ok_or("--bins is required")?);
    let publisher = option(args, "--publisher").unwrap_or(DEV_PUBLISHER);
    let out = out_dir(args);
    let version = version()?;
    std::fs::create_dir_all(&out)?;
    for exe in ["unicompose.exe", "unicompose-tray.exe"] {
        if !bins.join(exe).is_file() {
            return Err(format!("{} not found", bins.join(exe).display()).into());
        }
    }
    let stem = format!("unicompose-{version}-windows-{}", arch.name);
    zip(&bins, &out, &stem)?;
    msi(&bins, &out, &stem, &arch, &version)?;
    msix(&bins, &out, &stem, &arch, &version, publisher)?;
    Ok(())
}

/// The documents that go next to the programs.
fn docs() -> [(PathBuf, &'static str); 4] {
    let root = root();
    [
        (root.join("LICENSE"), "LICENSE.txt"),
        (root.join("README.md"), "README.md"),
        (root.join("crates/uc-xcompose/data/COPYING.libX11"), "COPYING.libX11.txt"),
        (root.join("crates/uc-xcompose/data/wincompose/COPYING"), "COPYING.WinCompose.txt"),
    ]
}

fn fresh_dir(path: &Path) -> Result<(), Error> {
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    std::fs::create_dir_all(path)?;
    Ok(())
}

fn zip(bins: &Path, out: &Path, stem: &str) -> Result<(), Error> {
    let stage = out.join("stage-zip").join(stem);
    fresh_dir(&stage)?;
    for exe in ["unicompose.exe", "unicompose-tray.exe"] {
        std::fs::copy(bins.join(exe), stage.join(exe))?;
    }
    for (source, name) in docs() {
        std::fs::copy(source, stage.join(name))?;
    }
    let zip = out.join(format!("{stem}.zip"));
    let _ = std::fs::remove_file(&zip);
    // Windows' own tar writes zip files with -a.
    run(Command::new("tar").arg("-a").arg("-cf").arg(&zip).arg("-C").arg(stage.parent().unwrap()).arg(stem))?;
    println!("wrote {}", zip.display());
    Ok(())
}

fn msi(bins: &Path, out: &Path, stem: &str, arch: &Arch, version: &str) -> Result<(), Error> {
    let msi = out.join(format!("{stem}.msi"));
    let root = root();
    let mut wix = Command::new(tool("wix", &[dotnet_tools().join("wix.exe")])?);
    wix.env("DOTNET_ROLL_FORWARD", "Major")
        .arg("build")
        .arg(root.join("packaging/wix/unicompose.wxs"))
        .args(["-arch", arch.name, "-ext", "WixToolset.Util.wixext"])
        .arg("-d")
        .arg(format!("Version={version}"))
        .arg("-d")
        .arg(format!("Bins={}", bins.display()))
        .arg("-d")
        .arg(format!("Assets={}", root.join("packaging/assets").display()))
        .arg("-d")
        .arg(format!("Docs={}", root.display()))
        .arg("-o")
        .arg(&msi);
    run(&mut wix)?;
    let _ = std::fs::remove_file(msi.with_extension("wixpdb"));
    println!("wrote {}", msi.display());
    Ok(())
}

fn msix(
    bins: &Path,
    out: &Path,
    stem: &str,
    arch: &Arch,
    version: &str,
    publisher: &str,
) -> Result<(), Error> {
    let stage = out.join(format!("stage-msix-{}", arch.name));
    fresh_dir(&stage)?;
    std::fs::create_dir_all(stage.join("assets"))?;
    for exe in ["unicompose.exe", "unicompose-tray.exe"] {
        std::fs::copy(bins.join(exe), stage.join(exe))?;
    }
    for logo in ["Square44x44Logo.png", "Square150x150Logo.png", "StoreLogo.png"] {
        std::fs::copy(root().join("packaging/assets").join(logo), stage.join("assets").join(logo))?;
    }
    for (source, name) in docs() {
        std::fs::copy(source, stage.join(name))?;
    }
    let template = std::fs::read_to_string(root().join("packaging/msix/AppxManifest.xml"))?;
    let manifest = template
        .replace("{version}", &format!("{version}.0"))
        .replace("{arch}", arch.name)
        .replace("{publisher}", &xml_escape(publisher));
    std::fs::write(stage.join("AppxManifest.xml"), manifest)?;
    let msix = out.join(format!("{stem}.msix"));
    run(Command::new(makeappx()?).arg("pack").arg("/o").arg("/d").arg(&stage).arg("/p").arg(&msix))?;
    println!("wrote {}", msix.display());
    Ok(())
}

pub fn bundle(args: &[&str]) -> Result<(), Error> {
    let out = out_dir(args);
    let version = version()?;
    let stage = out.join("stage-bundle");
    fresh_dir(&stage)?;
    let mut count = 0;
    for entry in std::fs::read_dir(&out)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "msix") {
            std::fs::copy(&path, stage.join(path.file_name().unwrap()))?;
            count += 1;
        }
    }
    if count == 0 {
        return Err(format!("no .msix files in {}", out.display()).into());
    }
    let bundle = out.join(format!("unicompose-{version}-windows.msixbundle"));
    run(Command::new(makeappx()?)
        .arg("bundle")
        .arg("/o")
        .arg("/bv")
        .arg(format!("{version}.0"))
        .arg("/d")
        .arg(&stage)
        .arg("/p")
        .arg(&bundle))?;
    println!("wrote {} ({count} architectures)", bundle.display());
    Ok(())
}

/// Files a release publishes: everything but staging folders and loose MSIX packages,
/// which the bundle replaces.
fn release_files(out: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(out)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| p.extension().is_some_and(|e| e == "zip" || e == "msi" || e == "msixbundle"))
        .collect();
    files.sort();
    Ok(files)
}

pub fn sha256_hex(path: &Path) -> Result<String, Error> {
    let digest = Sha256::digest(std::fs::read(path)?);
    Ok(digest.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    }))
}

pub fn checksums(args: &[&str]) -> Result<(), Error> {
    let out = out_dir(args);
    let mut text = String::new();
    for file in release_files(&out)? {
        writeln!(text, "{}  {}", sha256_hex(&file)?, file.file_name().unwrap().to_string_lossy())?;
    }
    std::fs::write(out.join("SHA256SUMS"), &text)?;
    print!("{text}");
    Ok(())
}

pub fn winget(args: &[&str]) -> Result<(), Error> {
    let url_base = option(args, "--url-base").ok_or("--url-base is required")?.trim_end_matches('/');
    let out = out_dir(args);
    let version = version()?;
    let mut installers = Vec::new();
    for arch in ["x64", "arm64"] {
        let name = format!("unicompose-{version}-windows-{arch}.msi");
        let path = out.join(&name);
        if path.is_file() {
            installers.push((arch, format!("{url_base}/{name}"), sha256_hex(&path)?.to_uppercase()));
        }
    }
    if installers.is_empty() {
        return Err(format!("no MSI files in {}", out.display()).into());
    }
    let dir = out.join("winget/manifests/s/shaia/unicompose").join(&version);
    std::fs::create_dir_all(&dir)?;
    for (name, text) in winget_manifests(&version, &installers) {
        std::fs::write(dir.join(name), text)?;
    }
    println!("wrote {}", dir.display());
    Ok(())
}

/// The version, installer and default-locale manifests, by file name.
fn winget_manifests(version: &str, installers: &[(&str, String, String)]) -> Vec<(String, String)> {
    let header = |kind: &str| {
        format!("# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.1.6.0.schema.json\n\n")
    };
    let version_manifest = format!(
        "{}PackageIdentifier: {PACKAGE_ID}\nPackageVersion: {version}\nDefaultLocale: en-US\nManifestType: version\nManifestVersion: 1.6.0\n",
        header("version")
    );
    let mut installer = format!(
        "{}PackageIdentifier: {PACKAGE_ID}\nPackageVersion: {version}\nInstallerType: wix\nScope: machine\nUpgradeBehavior: install\nInstallerSwitches:\n  Custom: LAUNCHAPP=0\nInstallers:\n",
        header("installer")
    );
    for (arch, url, sha) in installers {
        let _ =
            write!(installer, "- Architecture: {arch}\n  InstallerUrl: {url}\n  InstallerSha256: {sha}\n");
    }
    installer.push_str("ManifestType: installer\nManifestVersion: 1.6.0\n");
    let locale = format!(
        "{}PackageIdentifier: {PACKAGE_ID}\nPackageVersion: {version}\nPackageLocale: en-US\nPublisher: shaia\n\
PublisherUrl: https://github.com/shaia\nPackageName: unicompose\nPackageUrl: https://github.com/shaia/unicompose\n\
License: MIT\nLicenseUrl: https://github.com/shaia/unicompose/blob/main/LICENSE\n\
ShortDescription: A Compose key for Windows, and typing for keyboards that send Unicode over Raw HID.\n\
Description: |-\n  Tap Right Alt, then type a short sequence: o and \" give \u{f6}, - and > give \u{2192}. Uses the same rules as\n  WinCompose, plus your own .XCompose file. Also types the characters keyboards such as the\n  Summa-Cogni Mathpad send over Raw HID, on any keyboard layout.\n\
Tags:\n- compose-key\n- keyboard\n- unicode\n- wincompose\n- xcompose\n\
ReleaseNotesUrl: https://github.com/shaia/unicompose/releases/tag/v{version}\nManifestType: defaultLocale\nManifestVersion: 1.6.0\n",
        header("defaultLocale")
    );
    vec![
        (format!("{PACKAGE_ID}.yaml"), version_manifest),
        (format!("{PACKAGE_ID}.installer.yaml"), installer),
        (format!("{PACKAGE_ID}.locale.en-US.yaml"), locale),
    ]
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn dotnet_tools() -> PathBuf {
    PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join(".dotnet").join("tools")
}

/// `name` from PATH, else the first of `fallbacks` that exists.
fn tool(name: &str, fallbacks: &[PathBuf]) -> Result<PathBuf, Error> {
    let on_path =
        std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>());
    on_path
        .map(|dir| dir.join(format!("{name}.exe")))
        .chain(fallbacks.iter().cloned())
        .find(|p| p.is_file())
        .ok_or_else(|| format!("{name} not found; see the README's release section").into())
}

/// makeappx.exe: on PATH, else from the newest Windows SDK.
fn makeappx() -> Result<PathBuf, Error> {
    let kits = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&kits)
        .map(|dir| dir.filter_map(Result::ok).map(|e| e.path().join("x64").join("makeappx.exe")).collect())
        .unwrap_or_default();
    versions.sort();
    versions.reverse();
    tool("makeappx", &versions)
}

fn run(command: &mut Command) -> Result<(), Error> {
    let status = command.status().map_err(|e| format!("cannot run {:?}: {e}", command.get_program()))?;
    if !status.success() {
        return Err(format!("{:?} failed: {status}", command.get_program()).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winget_manifests_list_every_installer() {
        let installers = [
            ("x64", "https://x/a.msi".to_owned(), "AB".to_owned()),
            ("arm64", "https://x/b.msi".to_owned(), "CD".to_owned()),
        ];
        let manifests = winget_manifests("1.0.0", &installers);
        assert_eq!(manifests.len(), 3);
        let installer = &manifests[1].1;
        assert!(installer
            .contains("- Architecture: x64\n  InstallerUrl: https://x/a.msi\n  InstallerSha256: AB\n"));
        assert!(installer.contains("- Architecture: arm64"));
        assert!(manifests.iter().all(|(_, text)| text.contains("PackageVersion: 1.0.0")));
        assert!(manifests[2].1.contains("License: MIT"));
    }

    #[test]
    fn sha256_matches_a_known_digest() {
        let path = std::env::temp_dir().join(format!("xtask-sha-{}", std::process::id()));
        std::fs::write(&path, "abc").unwrap();
        assert_eq!(
            sha256_hex(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn targets_map_to_package_architectures() {
        assert_eq!(arch("x86_64-pc-windows-msvc").unwrap().name, "x64");
        assert_eq!(arch("aarch64-pc-windows-msvc").unwrap().name, "arm64");
        assert!(arch("i686-pc-windows-msvc").is_err());
    }
}
