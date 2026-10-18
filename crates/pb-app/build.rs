//! On Windows, puts the app's icon and version into the executable, compiled with the
//! Windows SDK's `rc` or LLVM's `llvm-rc`. Without either the build goes on, iconless.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || target_env != "msvc" {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let out = PathBuf::from(env::var("OUT_DIR").expect("set by cargo"));
    let icon = manifest.join("assets").join("icon.ico");
    let version = env::var("CARGO_PKG_VERSION").expect("set by cargo");
    let part = |name: &str| env::var(name).unwrap_or_else(|_| "0".to_owned());
    let numbers = format!(
        "{},{},{},0",
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH")
    );
    let script = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "PaintBoard"
      VALUE "ProductName", "PaintBoard"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "paintboard.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "/"),
    );
    let rc = out.join("paintboard.rc");
    let res = out.join("paintboard.res");
    std::fs::write(&rc, script).expect("OUT_DIR is writable");

    let compilers = [
        PathBuf::from("rc.exe"),
        PathBuf::from("llvm-rc.exe"),
        PathBuf::from(r"C:\Program Files\LLVM\bin\llvm-rc.exe"),
    ];
    let compiled = compilers.iter().any(|compiler| {
        Command::new(compiler)
            .arg("/nologo")
            .arg("/fo")
            .arg(&res)
            .arg(&rc)
            .output()
            .is_ok_and(|o| o.status.success())
    });
    if compiled {
        println!("cargo:rustc-link-arg-bins={}", res.display());
    } else {
        println!("cargo:warning=no resource compiler (rc or llvm-rc): the executable has no icon");
    }
}
