// Prevents additional console window on Windows, including autostart debug builds.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    delta_lib::run()
}
