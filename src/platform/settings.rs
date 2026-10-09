//! Tiny per-user settings store: DWORD values under HKCU\Software\Images.

use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD,
};

use crate::wide;

const KEY: &str = r"Software\Images";

pub fn get_bool(name: &str) -> Option<bool> {
    get_u32(name).map(|v| v != 0)
}

pub fn set_bool(name: &str, value: bool) {
    set_u32(name, value as u32);
}

pub fn get_u32(name: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(KEY).as_ptr()),
            PCWSTR(wide(name).as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as _),
            Some(&mut size),
        )
    };
    r.is_ok().then_some(value)
}

pub fn set_u32(name: &str, v: u32) {
    unsafe {
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(KEY).as_ptr()),
            PCWSTR(wide(name).as_ptr()),
            REG_DWORD.0,
            Some(&v as *const _ as _),
            4,
        );
    }
}
