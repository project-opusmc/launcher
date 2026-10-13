#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = opus_launcher::client_account_resolver_mode() {
        std::process::exit(code);
    }
    opus_launcher::run();
}
