use std::process::Command;

fn main() {
    // La CI passa lo sha del commit: cambiarlo deve forzare la ricompilazione
    // del crate, perché `update.rs` lo incorpora con `option_env!`.
    println!("cargo:rerun-if-env-changed=SOTTOVOCE_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=SOTTOVOCE_BUILD_TIME");
    build_info();
    tauri_build::build()
}

/// Commit e ora del build mostrati in Impostazioni → Informazioni
/// (`SV_INFO_COMMIT`, `SV_INFO_BUILT`). In CI valgono quelli della release;
/// in locale si leggono da git (`-dirty` = modifiche non committate). Nomi
/// diversi da `SOTTOVOCE_BUILD_*`: i build locali non controllano gli
/// aggiornamenti.
fn build_info() {
    // Si rifà a ogni cambio di sorgenti o di commit, così l'ora è quella vera.
    for path in [
        "src",
        "../crates",
        "../src",
        "../.git/HEAD",
        "../.git/index",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let commit = std::env::var("SOTTOVOCE_BUILD_SHA")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let sha = git(&["rev-parse", "--short=7", "HEAD"])?;
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty());
            Some(if dirty { format!("{sha}-dirty") } else { sha })
        })
        .unwrap_or_default();
    let built = std::env::var("SOTTOVOCE_BUILD_TIME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| run("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"]))
        .unwrap_or_default();
    println!("cargo:rustc-env=SV_INFO_COMMIT={commit}");
    println!("cargo:rustc-env=SV_INFO_BUILT={built}");
}

fn git(args: &[&str]) -> Option<String> {
    run("git", args)
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
