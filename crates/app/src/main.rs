#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Headless agent mode (started by the IntegraRemote scheduled task).
    if std::env::args().any(|a| a == "--remote-run") {
        std::process::exit(integra_app_lib::remote_run_standalone());
    }
    integra_app_lib::run()
}
