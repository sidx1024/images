#![windows_subsystem = "windows"]

mod app;
mod decode;
mod folder;
mod platform;
mod render;
mod touch;
mod view;

use platform::register;
use std::ffi::OsStr;
use std::path::PathBuf;
use windows::core::{w, PCWSTR};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
};

fn main() {
    let first = std::env::args_os().nth(1);
    if first.as_deref() == Some(OsStr::new("--register")) {
        std::process::exit(run_registration(register::register));
    }
    if first.as_deref() == Some(OsStr::new("--unregister")) {
        std::process::exit(run_registration(register::unregister));
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .map(|p| std::path::absolute(&p).unwrap_or(p));
    if let Err(e) = app::run(path) {
        eprintln!("images: {e}");
    }
}

/// Runs a registration command; there's no console (GUI subsystem), so failures get a message box.
fn run_registration(f: fn() -> windows::core::Result<()>) -> i32 {
    if register::package_aumid().is_some() {
        // Packaged (Store/MSIX): the manifest declares the file types; writes here would be virtualized.
        unsafe {
            MessageBoxW(
                None,
                w!("Images was installed as a package, so Windows manages its file types. Choose defaults in Settings > Apps > Default apps."),
                w!("Images"),
                MB_ICONINFORMATION | MB_OK,
            );
        }
        return 0;
    }
    if register::installed_by_msi() {
        unsafe {
            MessageBoxW(
                None,
                w!("Images was installed with its installer, which manages its registration. To remove Images, uninstall it from Settings > Apps > Installed apps."),
                w!("Images"),
                MB_ICONINFORMATION | MB_OK,
            );
        }
        return 0;
    }
    // COM is needed to enumerate installed WIC codecs (the extensions to register).
    let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .and_then(|_| f());
    match result {
        Ok(()) => 0,
        Err(e) => {
            let text = wide(format!("Registration failed: {}", e.message()));
            unsafe {
                MessageBoxW(
                    None,
                    PCWSTR(text.as_ptr()),
                    w!("Images"),
                    MB_ICONERROR | MB_OK,
                );
            }
            1
        }
    }
}

/// Null-terminated UTF-16 for Win32 calls.
pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}
