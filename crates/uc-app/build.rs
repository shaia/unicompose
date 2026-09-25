//! Windows resources for both programs: the icon, version information, and a manifest
//! that asks for modern controls, per-monitor DPI awareness and UTF-8.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../packaging/assets/unicompose.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets the version");
    let numeric = version.split('-').next().unwrap_or("0.0.0");
    let manifest = MANIFEST.replace("{version}", &format!("{numeric}.0"));
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let manifest_path = out.join("unicompose.manifest");
    std::fs::write(&manifest_path, manifest).expect("write the manifest");

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("../../packaging/assets/unicompose.ico")
        .set_manifest_file(manifest_path.to_str().expect("OUT_DIR is UTF-8"))
        .set("ProductName", "unicompose")
        .set("FileDescription", "unicompose: a Compose key for Windows")
        .set("LegalCopyright", "Copyright (c) 2026 shaia. MIT License.");
    resource.compile().expect("compile the Windows resources (needs rc.exe from the Windows SDK)");
}

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="shaia.unicompose" version="{version}"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <!-- Windows 10 and 11 -->
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;
