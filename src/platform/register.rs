//! Per-user (HKCU) registration so Images appears in Explorer's "Open with" menu and in
//! Settings > Default apps. No admin rights needed; `--unregister` removes exactly what it adds.
//!
//! The MSI installer writes the same keys (packaging/wix). When Images runs from its MSIX package,
//! the package manifest declares the file types instead and Windows owns that registration.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};

use std::os::windows::ffi::OsStringExt;
use std::sync::OnceLock;
use windows::core::PWSTR;
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, E_FAIL, WIN32_ERROR,
};
use windows::Win32::Storage::Packaging::Appx::GetCurrentApplicationUserModelId;
use windows::Win32::System::Registry::*;

use windows::core::{w, Interface};
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER,
};
use windows::Win32::UI::Shell::{
    ApplicationAssociationRegistration, FOLDERID_Programs, IApplicationAssociationRegistration,
    IShellLinkW, SHChangeNotify, SHGetKnownFolderPath, ShellExecuteW, ShellLink, AL_EFFECTIVE,
    AT_FILEEXTENSION, KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNF_FLUSH, SHCNF_IDLIST,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::{folder, wide};

const PROGID: &str = "Images.Image";
const PROGID_KEY: &str = r"Software\Classes\Images.Image";
const APP_KEY: &str = r"Software\Classes\Applications\images.exe";
const VENDOR_KEY: &str = r"Software\Images";
const CAPABILITIES: &str = r"Software\Images\Capabilities";
const REGISTERED_APPS: &str = r"Software\RegisteredApplications";
/// Registry value name under RegisteredApplications, and the name users see in "Open with"
/// and Settings > Default apps.
const APP_NAME: &str = "Images";
const DISPLAY_NAME: &str = APP_NAME;
const DESCRIPTION: &str = "A fast, minimal photo viewer";

/// File types declared by the MSIX package and the MSI installer.
fn packaged_extensions() -> impl Iterator<Item = &'static str> {
    include_str!("../../packaging/extensions.txt")
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('.'))
}

/// The app's AppUserModelID when it runs from its MSIX package (Store or sideloaded), else None.
pub fn package_aumid() -> Option<&'static str> {
    static AUMID: OnceLock<Option<String>> = OnceLock::new();
    AUMID
        .get_or_init(|| unsafe {
            let mut len = 0u32;
            if GetCurrentApplicationUserModelId(&mut len, None) != ERROR_INSUFFICIENT_BUFFER {
                return None; // APPMODEL_ERROR_NO_APPLICATION: not packaged
            }
            let mut buf = vec![0u16; len as usize];
            if GetCurrentApplicationUserModelId(&mut len, Some(PWSTR(buf.as_mut_ptr()))).is_err() {
                return None;
            }
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end]))
        })
        .as_deref()
}

/// Extensions Images is offered for in Default apps: the package's declared types, or what
/// `--register` / the MSI recorded.
fn registered_extensions() -> Vec<String> {
    if package_aumid().is_some() {
        packaged_extensions().map(String::from).collect()
    } else {
        unsafe { value_names(&format!(r"{CAPABILITIES}\FileAssociations")) }
    }
}

/// A string value under `root\<path>`, if present.
unsafe fn reg_string(root: HKEY, path: &str, name: &str) -> Option<String> {
    let mut buf = [0u16; 512];
    let mut bytes = std::mem::size_of_val(&buf) as u32;
    RegGetValueW(
        root,
        PCWSTR(wide(path).as_ptr()),
        PCWSTR(wide(name).as_ptr()),
        RRF_RT_REG_SZ,
        None,
        Some(buf.as_mut_ptr() as _),
        Some(&mut bytes),
    )
    .ok()
    .ok()?;
    let len = (bytes as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Whether the running exe is the one the MSI installer put in place (it records its folder), in
/// which case the installer owns the registration and uninstall removes it.
pub fn installed_by_msi() -> bool {
    let Some(dir) = (unsafe { reg_string(HKEY_CURRENT_USER, VENDOR_KEY, "InstallDir") }) else {
        return false;
    };
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()));
    let norm = |p: &std::path::Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    exe_dir.is_some_and(|d| norm(&d) == norm(std::path::Path::new(&dir)))
}

/// Per-user Start menu shortcut, as an installer would create, so Images can be launched from Start.
fn start_menu_shortcut() -> Option<std::path::PathBuf> {
    unsafe {
        let dir = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None).ok()?;
        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(dir.as_wide()))
            .join(format!("{DISPLAY_NAME}.lnk"));
        CoTaskMemFree(Some(dir.0 as _));
        Some(path)
    }
}

fn create_start_menu_shortcut(exe: &std::path::Path) -> Result<()> {
    let target = start_menu_shortcut().ok_or_else(|| windows::core::Error::from(E_FAIL))?;
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(wide(exe).as_ptr()))?;
        if let Some(dir) = exe.parent() {
            link.SetWorkingDirectory(PCWSTR(wide(dir).as_ptr()))?;
        }
        link.SetDescription(PCWSTR(wide(DESCRIPTION).as_ptr()))?;
        link.SetIconLocation(PCWSTR(wide(exe).as_ptr()), 0)?;
        link.cast::<IPersistFile>()?
            .Save(PCWSTR(wide(&target).as_ptr()), true)?;
    }
    Ok(())
}

/// Tell Windows that handlers changed, as Microsoft's registration sample does: flush synchronously,
/// then give system processes (Settings' Default apps index) a moment to process it before we exit.
fn notify_associations_changed() {
    unsafe {
        // SHCNE_ASSOCCHANGED requires SHCNF_IDLIST with both items null (SHChangeNotify reference).
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST | SHCNF_FLUSH, None, None);
    }
    std::thread::sleep(std::time::Duration::from_secs(1));
}

fn check(e: WIN32_ERROR) -> Result<()> {
    e.ok()
}

/// Like `check`, but "already gone" counts as success (for cleanup).
fn check_delete(e: WIN32_ERROR) -> Result<()> {
    if e == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        e.ok()
    }
}

/// Create (or open) `HKCU\<path>` and set `name` (None = default value) to a string.
unsafe fn set_string(path: &str, name: Option<&str>, value: impl AsRef<OsStr>) -> Result<()> {
    let mut key = HKEY::default();
    check(RegCreateKeyExW(
        HKEY_CURRENT_USER,
        PCWSTR(wide(path).as_ptr()),
        None,
        PCWSTR::null(),
        REG_OPTION_NON_VOLATILE,
        KEY_SET_VALUE,
        None,
        &mut key,
        None,
    ))?;
    let data: Vec<u8> = wide(value).iter().flat_map(|c| c.to_le_bytes()).collect();
    let name = name.map(wide);
    let r = RegSetValueExW(
        key,
        name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
        None,
        REG_SZ,
        Some(&data),
    );
    let _ = RegCloseKey(key);
    check(r)
}

/// Value names under `HKCU\<path>` (empty if the key doesn't exist).
unsafe fn value_names(path: &str) -> Vec<String> {
    let mut key = HKEY::default();
    if RegOpenKeyExW(
        HKEY_CURRENT_USER,
        PCWSTR(wide(path).as_ptr()),
        None,
        KEY_READ,
        &mut key,
    )
    .is_err()
    {
        return Vec::new();
    }
    let mut names = Vec::new();
    for i in 0.. {
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        let r = RegEnumValueW(
            key,
            i,
            Some(windows::core::PWSTR(buf.as_mut_ptr())),
            &mut len,
            None,
            None,
            None,
            None,
        );
        if r == ERROR_NO_MORE_ITEMS || r.is_err() {
            break;
        }
        names.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    let _ = RegCloseKey(key);
    names
}

/// True if `HKCU\<path>` exists and has no values and no subkeys.
unsafe fn is_empty_key(path: &str) -> bool {
    let mut key = HKEY::default();
    if RegOpenKeyExW(
        HKEY_CURRENT_USER,
        PCWSTR(wide(path).as_ptr()),
        None,
        KEY_READ,
        &mut key,
    )
    .is_err()
    {
        return false;
    }
    let (mut subkeys, mut values) = (0u32, 0u32);
    let r = RegQueryInfoKeyW(
        key,
        None,
        None,
        None,
        Some(&mut subkeys),
        None,
        None,
        Some(&mut values),
        None,
        None,
        None,
        None,
    );
    let _ = RegCloseKey(key);
    r.is_ok() && subkeys == 0 && values == 0
}

/// Delete `HKCU\<path>` and everything under it. Only used on keys this module owns outright.
unsafe fn delete_tree(path: &str) -> Result<()> {
    let p = wide(path);
    check_delete(RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr())))?;
    check_delete(RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr())))
}

unsafe fn delete_value(path: &str, name: &str) -> Result<()> {
    check_delete(RegDeleteKeyValueW(
        HKEY_CURRENT_USER,
        PCWSTR(wide(path).as_ptr()),
        PCWSTR(wide(name).as_ptr()),
    ))
}

fn os_concat(parts: &[&OsStr]) -> OsString {
    parts.iter().fold(OsString::new(), |mut s, p| {
        s.push(p);
        s
    })
}

/// Register the running exe (in place).
pub fn register() -> Result<()> {
    let exe = std::env::current_exe().map_err(|_| windows::core::Error::from(E_FAIL))?;
    register_exe(&exe)
}

fn register_exe(exe: &std::path::Path) -> Result<()> {
    let exe = exe.as_os_str();
    // Built as OsString so unusual (non-UTF-16-clean) paths survive exactly.
    let command = os_concat(&["\"".as_ref(), exe, "\" \"%1\"".as_ref()]);
    let icon = os_concat(&["\"".as_ref(), exe, "\",0".as_ref()]);
    unsafe {
        set_string(PROGID_KEY, None, "Image")?;
        set_string(&format!(r"{PROGID_KEY}\DefaultIcon"), None, &icon)?;
        // Used by the Windows 10/11 Default apps UI to show the app behind this ProgID.
        let progid_app = format!(r"{PROGID_KEY}\Application");
        set_string(&progid_app, Some("ApplicationName"), DISPLAY_NAME)?;
        set_string(&progid_app, Some("ApplicationDescription"), DESCRIPTION)?;
        set_string(&progid_app, Some("ApplicationIcon"), &icon)?;
        set_string(&format!(r"{PROGID_KEY}\shell\open\command"), None, &command)?;

        // Application entry: friendly name in the "Open with" list.
        set_string(APP_KEY, Some("FriendlyAppName"), DISPLAY_NAME)?;
        set_string(&format!(r"{APP_KEY}\shell\open\command"), None, &command)?;

        // Capabilities make it selectable in Settings > Default apps.
        set_string(CAPABILITIES, Some("ApplicationName"), DISPLAY_NAME)?;
        set_string(CAPABILITIES, Some("ApplicationDescription"), DESCRIPTION)?;
        set_string(CAPABILITIES, Some("ApplicationIcon"), &icon)?;
        set_string(REGISTERED_APPS, Some(APP_NAME), CAPABILITIES)?;

        for ext in folder::supported_extensions() {
            // Record the extension in our own keys first, so unregister can find it even if this loop fails.
            set_string(&format!(r"{APP_KEY}\SupportedTypes"), Some(&ext), "")?;
            set_string(
                &format!(r"{CAPABILITIES}\FileAssociations"),
                Some(&ext),
                PROGID,
            )?;
            set_string(
                &format!(r"Software\Classes\{ext}\OpenWithProgids"),
                Some(PROGID),
                "",
            )?;
        }
        create_start_menu_shortcut(std::path::Path::new(exe))?;
        notify_associations_changed();
    }
    Ok(())
}

pub fn unregister() -> Result<()> {
    unsafe {
        // Extensions we recorded at registration, plus today's codecs in case the record is incomplete.
        let mut exts: BTreeSet<String> = value_names(&format!(r"{APP_KEY}\SupportedTypes"))
            .into_iter()
            .collect();
        exts.extend(value_names(&format!(r"{CAPABILITIES}\FileAssociations")));
        exts.extend(folder::supported_extensions());

        // Keep going on failure so as much as possible is cleaned up, but report the first error.
        let mut first_err: Option<windows::core::Error> = None;
        let mut note = |r: Result<()>| {
            if let Err(e) = r {
                first_err.get_or_insert(e);
            }
        };
        for ext in exts.iter().filter(|e| e.starts_with('.')) {
            // Only our value: the extension keys are shared with other apps.
            note(delete_value(
                &format!(r"Software\Classes\{ext}\OpenWithProgids"),
                PROGID,
            ));
        }
        note(delete_value(REGISTERED_APPS, APP_NAME));
        note(delete_tree(PROGID_KEY));
        note(delete_tree(APP_KEY));
        note(delete_tree(CAPABILITIES));
        if is_empty_key(VENDOR_KEY) {
            note(check_delete(RegDeleteKeyW(
                HKEY_CURRENT_USER,
                PCWSTR(wide(VENDOR_KEY).as_ptr()),
            )));
        }
        // Only our own shortcut, by its exact name.
        if let Some(lnk) = start_menu_shortcut().filter(|p| p.is_file()) {
            if std::fs::remove_file(&lnk).is_err() {
                note(Err(windows::core::Error::from(E_FAIL)));
            }
        }
        notify_associations_changed();
        first_err.map_or(Ok(()), Err)
    }
}

/// How many of the extensions Images registered for currently open with Images (the user's effective
/// default, as Windows resolves it). Returns (defaults, registered), with registered 0 if not registered,
/// or None if Windows couldn't answer for some type (so no possibly-wrong count is shown).
pub fn default_status() -> Option<(usize, usize)> {
    let exts = registered_extensions();
    let mut defaults = 0;
    for ext in &exts {
        if is_default_for(ext)? {
            defaults += 1;
        }
    }
    Some((defaults, exts.len()))
}

/// Whether Images is registered as a handler for `ext` (".jpg"), i.e. offered in Default apps.
pub fn is_registered_for(ext: &str) -> bool {
    registered_extensions()
        .iter()
        .any(|e| e.eq_ignore_ascii_case(ext))
}

/// Whether Images is the effective default for `ext` (".jpg"); None if Windows can't tell.
pub fn is_default_for(ext: &str) -> Option<bool> {
    unsafe {
        let reg: IApplicationAssociationRegistration = CoCreateInstance(
            &ApplicationAssociationRegistration,
            None,
            CLSCTX_INPROC_SERVER,
        )
        .ok()?;
        let progid = reg
            .QueryCurrentDefault(PCWSTR(wide(ext).as_ptr()), AT_FILEEXTENSION, AL_EFFECTIVE)
            .ok()?;
        let name = progid.to_string().ok();
        CoTaskMemFree(Some(progid.0 as _));
        let name = name?;
        match package_aumid() {
            // Package file types get generated "AppX…" ProgIDs that name their app.
            // A failed lookup means "can't tell", not "someone else".
            Some(aumid) => reg_string(
                HKEY_CLASSES_ROOT,
                &format!(r"{name}\Application"),
                "AppUserModelID",
            )
            .map(|id| id.eq_ignore_ascii_case(aumid)),
            None => Some(name.eq_ignore_ascii_case(PROGID)),
        }
    }
}

/// Microsoft's recommended way to let users pick Images as their default: open Images' own page in
/// Settings > Apps > Default apps (Windows 11 2023-04 update and later; older builds open the main page).
/// Must go through ShellExecute; explorer.exe would treat the URI as a path.
pub fn open_default_apps_settings(hwnd: windows::Win32::Foundation::HWND) {
    let uri = wide(match package_aumid() {
        Some(aumid) => format!("ms-settings:defaultapps?registeredAUMID={aumid}"),
        None => format!("ms-settings:defaultapps?registeredAppUser={APP_NAME}"),
    });
    unsafe {
        ShellExecuteW(
            Some(hwnd),
            w!("open"),
            PCWSTR(uri.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}
