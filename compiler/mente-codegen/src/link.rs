//! Linking an object into an executable with the system linker ([`native-runtime`]): `cc` on
//! Linux, and on Windows MSVC's `link.exe`, found as Rust's own toolchain finds it, through
//! `vswhere` and the Windows SDK's folder.
//!
//! [`native-runtime`]: ../../docs/design-review.md#native-runtime

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{CodegenError, Target};

/// Links `object`, written for `target`, into the executable `exe`.
pub fn link(object: &Path, exe: &Path, target: Target) -> Result<(), CodegenError> {
    let mut cmd = match target {
        Target::X86_64Windows => {
            let msvc = Msvc::find()?;
            let mut c = Command::new(&msvc.link);
            c.arg("/NOLOGO")
                .arg(format!("/OUT:{}", exe.display()))
                .arg(object)
                .arg("/SUBSYSTEM:CONSOLE")
                .arg("/DEBUG");
            for dir in &msvc.libs {
                c.arg(format!("/LIBPATH:{}", dir.display()));
            }
            c.args(["msvcrt.lib", "ucrt.lib", "vcruntime.lib", "kernel32.lib"]);
            c
        }
        _ => {
            let mut c = Command::new("cc");
            c.arg(object).arg("-o").arg(exe);
            c
        }
    };
    let out = cmd
        .output()
        .map_err(|e| CodegenError::new(format!("the linker did not start: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(CodegenError::new(format!(
            "the linker failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )))
    }
}

/// MSVC's linker and the folders of the libraries a console program needs.
struct Msvc {
    link: PathBuf,
    libs: Vec<PathBuf>,
}

impl Msvc {
    fn find() -> Result<Msvc, CodegenError> {
        let missing = |what: &str| CodegenError::new(format!("MSVC: {what} not found"));
        let x86 = std::env::var_os("ProgramFiles(x86)")
            .map(PathBuf::from)
            .ok_or_else(|| missing("Program Files (x86)"))?;
        let vswhere = x86.join("Microsoft Visual Studio/Installer/vswhere.exe");
        let out = Command::new(&vswhere)
            .args([
                "-latest",
                "-products",
                "*",
                "-requires",
                "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                "-property",
                "installationPath",
            ])
            .output()
            .map_err(|_| missing("vswhere"))?;
        let install = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        let version = std::fs::read_to_string(
            install.join("VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt"),
        )
        .map_err(|_| missing("the version of the VC tools"))?;
        let tools = install.join("VC/Tools/MSVC").join(version.trim());
        let kits = x86.join("Windows Kits/10/Lib");
        let sdk = std::fs::read_dir(&kits)
            .map_err(|_| missing("the Windows SDK"))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("ucrt/x64/ucrt.lib").exists())
            .max()
            .ok_or_else(|| missing("the Windows SDK's libraries"))?;
        Ok(Msvc {
            link: tools.join("bin/Hostx64/x64/link.exe"),
            libs: vec![
                tools.join("lib/x64"),
                sdk.join("ucrt/x64"),
                sdk.join("um/x64"),
            ],
        })
    }
}
