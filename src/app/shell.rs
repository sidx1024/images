//! Shell integration: the Open dialog, dropped files, and revealing a file in Explorer.

use super::*;

pub(super) unsafe fn open_dialog(hwnd: HWND) -> Option<PathBuf> {
    let dlg: IFileOpenDialog =
        CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
    let mut exts: Vec<String> = folder::supported_extensions()
        .into_iter()
        .map(|e| format!("*{e}"))
        .collect();
    exts.sort();
    let spec = wide(exts.join(";"));
    let filters = [
        COMDLG_FILTERSPEC {
            pszName: w!("Images"),
            pszSpec: PCWSTR(spec.as_ptr()),
        },
        COMDLG_FILTERSPEC {
            pszName: w!("All files"),
            pszSpec: w!("*.*"),
        },
    ];
    dlg.SetFileTypes(&filters).ok()?;
    dlg.Show(Some(hwnd)).ok()?;
    let item = dlg.GetResult().ok()?;
    let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    let path = PathBuf::from(OsString::from_wide(p.as_wide()));
    CoTaskMemFree(Some(p.0 as _));
    Some(path)
}

pub(super) unsafe fn dropped_file(hdrop: HDROP) -> Option<PathBuf> {
    let len = DragQueryFileW(hdrop, 0, None);
    let mut buf = vec![0u16; len as usize + 1];
    DragQueryFileW(hdrop, 0, Some(&mut buf));
    DragFinish(hdrop);
    (len > 0).then(|| PathBuf::from(OsString::from_wide(&buf[..len as usize])))
}

/// Opens Explorer on the file's folder with the file selected.
pub(super) unsafe fn reveal_in_explorer(path: &std::path::Path) {
    let w = wide(path);
    let pidl = ILCreateFromPathW(PCWSTR(w.as_ptr()));
    if !pidl.is_null() {
        let _ = SHOpenFolderAndSelectItems(pidl, None, 0);
        ILFree(Some(pidl));
    }
}
