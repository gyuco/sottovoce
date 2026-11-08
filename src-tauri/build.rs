fn main() {
    // La CI passa lo sha del commit: cambiarlo deve forzare la ricompilazione
    // del crate, perché `update.rs` lo incorpora con `option_env!`.
    println!("cargo:rerun-if-env-changed=SOTTOVOCE_BUILD_SHA");
    tauri_build::build()
}
