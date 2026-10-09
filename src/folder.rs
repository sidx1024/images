//! Lists the images in a folder in Explorer's natural sort order.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::S_OK;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IEnumUnknown,
    IServiceProvider, CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IEnumIDList, IFolderView2, ILCombine, ILFree, IPersistFolder2, IShellBrowser, IShellWindows,
    SHGetNameFromIDList, SID_STopLevelBrowser, ShellWindows, StrCmpLogicalW, _SVGIO,
    SIGDN_FILESYSPATH, SVGIO_ALLVIEW, SVGIO_FLAG_VIEWORDER,
};

use crate::decode::{Msg, Notifier};
use crate::wide;

const FALLBACK: &[&str] = &[
    ".jpg", ".jpeg", ".jpe", ".jfif", ".png", ".gif", ".bmp", ".dib", ".tif", ".tiff", ".ico",
    ".jxr", ".wdp", ".dds", ".heic", ".heif", ".avif", ".webp",
];

/// Extensions of every installed WIC decoder (includes HEIC/WebP/RAW when their codec packs are present).
pub fn supported_extensions() -> HashSet<String> {
    let mut set: HashSet<String> = FALLBACK.iter().map(|s| s.to_string()).collect();
    unsafe {
        let Ok(factory) = CoCreateInstance::<_, IWICImagingFactory>(
            &CLSID_WICImagingFactory,
            None,
            CLSCTX_INPROC_SERVER,
        ) else {
            return set;
        };
        let Ok(en): windows::core::Result<IEnumUnknown> = factory
            .CreateComponentEnumerator(WICDecoder.0 as u32, WICComponentEnumerateDefault.0 as u32)
        else {
            return set;
        };
        loop {
            let mut items = [None];
            let mut fetched = 0u32;
            if en.Next(&mut items, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(Ok(info)) = items[0].take().map(|u| u.cast::<IWICBitmapDecoderInfo>()) else {
                continue;
            };
            let mut len = 0u32;
            let _ = info.GetFileExtensions(&mut [], &mut len);
            if len == 0 {
                continue;
            }
            let mut buf = vec![0u16; len as usize];
            if info.GetFileExtensions(&mut buf, &mut len).is_ok() {
                let s = String::from_utf16_lossy(
                    &buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())],
                );
                set.extend(
                    s.split(',')
                        .map(|e| e.trim().to_lowercase())
                        .filter(|e| e.starts_with('.')),
                );
            }
        }
    }
    set
}

pub fn list_images(dir: &Path, exts: &HashSet<String>) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(Vec<u16>, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|x| exts.contains(&format!(".{}", x.to_string_lossy().to_lowercase())))
                .unwrap_or(false)
        })
        .map(|p| (wide(p.file_name().unwrap_or_default()), p))
        .collect();
    files.sort_by(|a, b| {
        unsafe { StrCmpLogicalW(PCWSTR(a.0.as_ptr()), PCWSTR(b.0.as_ptr())) }.cmp(&0)
    });
    files.into_iter().map(|(_, p)| p).collect()
}

/// Sends the folder's images in name order right away, then again in Explorer's on-screen order
/// if an Explorer window is showing the folder (that query can be slow, so it doesn't block navigation).
pub fn scan_async(file: PathBuf, generation: u64, notifier: Notifier) {
    std::thread::spawn(move || {
        // STA: the Shell's view interfaces are designed for single-threaded apartments.
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        scan(&file, generation, &notifier);
        if com {
            // Every COM object was created and dropped inside `scan`.
            unsafe { CoUninitialize() };
        }
    });
}

fn scan(file: &Path, generation: u64, notifier: &Notifier) {
    let Some(dir) = file.parent() else { return };
    let exts = supported_extensions();
    let files = list_images(dir, &exts);
    notifier.send(Msg::Folder {
        generation,
        files: files.clone(),
    });

    if let Some(order) = explorer_order(dir, file) {
        // Keep Explorer's order but only for real image files we found (not e.g. a folder named "x.jpg").
        let known: HashSet<String> = files.iter().map(|p| lower(p)).collect();
        let ordered: Vec<PathBuf> = order
            .into_iter()
            .filter(|p| known.contains(&lower(p)))
            .collect();
        if !ordered.is_empty() && ordered != files {
            notifier.send(Msg::Folder {
                generation,
                files: ordered,
            });
        }
    }
}

fn lower(p: &Path) -> String {
    p.as_os_str().to_string_lossy().to_lowercase()
}

fn same_path(a: &Path, b: &Path) -> bool {
    lower(a) == lower(b)
}

unsafe fn pidl_path(pidl: *const ITEMIDLIST) -> Option<PathBuf> {
    let name = SHGetNameFromIDList(pidl, SIGDN_FILESYSPATH).ok()?;
    let path = PathBuf::from(OsString::from_wide(name.as_wide()));
    CoTaskMemFree(Some(name.0 as _));
    Some(path)
}

/// Files in the order an open Explorer window displays `dir` (its sort, grouping and filters).
/// Prefers the window whose focused item is `file`, i.e. the one the photo was opened from.
pub fn explorer_order(dir: &Path, file: &Path) -> Option<Vec<PathBuf>> {
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        let mut fallback = None;
        for i in 0..windows.Count().ok()? {
            let Some(view) = folder_view(&windows, i) else {
                continue;
            };
            let Ok(folder) = view.GetFolder::<IPersistFolder2>() else {
                continue;
            };
            let Ok(folder_pidl) = folder.GetCurFolder() else {
                continue;
            };
            if pidl_path(folder_pidl).is_some_and(|p| same_path(&p, dir)) {
                let items = view_items(&view, folder_pidl);
                let focused = view
                    .GetFocusedItem()
                    .ok()
                    .and_then(|idx| view.Item(idx).ok())
                    .and_then(|child| {
                        let abs = ILCombine(Some(folder_pidl), Some(child));
                        let path = pidl_path(abs);
                        ILFree(Some(abs));
                        CoTaskMemFree(Some(child as _));
                        path
                    });
                if focused.is_some_and(|f| same_path(&f, file)) {
                    CoTaskMemFree(Some(folder_pidl as _));
                    return items;
                }
                fallback = fallback.or(items);
            }
            CoTaskMemFree(Some(folder_pidl as _));
        }
        fallback
    }
}

unsafe fn folder_view(windows: &IShellWindows, index: i32) -> Option<IFolderView2> {
    let mut v = VARIANT::default();
    (*v.Anonymous.Anonymous).vt = VT_I4;
    (*v.Anonymous.Anonymous).Anonymous.lVal = index;
    let browser: IShellBrowser = windows
        .Item(&v)
        .ok()?
        .cast::<IServiceProvider>()
        .ok()?
        .QueryService(&SID_STopLevelBrowser)
        .ok()?;
    browser.QueryActiveShellView().ok()?.cast().ok()
}

unsafe fn view_items(view: &IFolderView2, folder_pidl: *const ITEMIDLIST) -> Option<Vec<PathBuf>> {
    let items: IEnumIDList = view
        .Items(_SVGIO(SVGIO_ALLVIEW.0 | SVGIO_FLAG_VIEWORDER.0))
        .ok()?;
    let mut out = Vec::new();
    loop {
        // Batches keep the number of cross-process calls to Explorer small.
        let mut batch = [std::ptr::null_mut(); 256];
        let mut fetched = 0u32;
        let hr = items.Next(&mut batch, Some(&mut fetched));
        for &child in &batch[..fetched as usize] {
            let abs = ILCombine(Some(folder_pidl), Some(child));
            out.extend(pidl_path(abs));
            ILFree(Some(abs));
            CoTaskMemFree(Some(child as _));
        }
        if hr.is_err() {
            // A partial list would drop files from navigation; fall back to name order instead.
            return None;
        }
        if hr != S_OK || fetched == 0 {
            break;
        }
    }
    Some(out)
}
