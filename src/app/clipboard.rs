//! Copying the current image to the clipboard.

use super::*;

impl App {
    /// Ctrl+C: copy the full-resolution image (decoded off the UI thread). One copy runs at a time so
    /// large images can't pile up in memory; a request made meanwhile replaces any earlier queued one.
    pub(super) fn copy_image(&mut self) {
        let Some(path) = self.current().cloned() else {
            return;
        };
        self.copy_latest += 1;
        if self.copy_busy {
            self.copy_queued = Some(path);
        } else {
            self.copy_busy = true;
            decode::copy_async(self.copy_latest, path, self.notifier.clone());
        }
    }

    pub(super) fn on_copy_done(&mut self, id: u64, result: std::result::Result<ClipData, String>) {
        self.copy_busy = false;
        if let Some(path) = self.copy_queued.take() {
            // A newer request is waiting: this result is outdated.
            self.copy_busy = true;
            decode::copy_async(self.copy_latest, path, self.notifier.clone());
            return;
        }
        if id != self.copy_latest {
            return;
        }
        match result {
            Ok(data) => self.pending_clip = Some(data),
            Err(message) => self.show_notice(&message, false),
        }
    }
}

/// Put the prepared image on the clipboard ("PNG" first when present, then CF_DIB). Memory is already
/// allocated, so a failure here never leaves the user's clipboard emptied for nothing.
/// Returns true if at least the universally readable DIB was published.
pub(super) unsafe fn set_clipboard(hwnd: HWND, data: ClipData) -> bool {
    // Another app can hold the clipboard for a moment; retry briefly.
    if !(0..5).any(|i| {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(15));
        }
        OpenClipboard(Some(hwnd)).is_ok()
    }) {
        return false;
    }
    if EmptyClipboard().is_err() {
        let _ = CloseClipboard();
        return false;
    }
    if let Some(png) = data.png {
        let _ = put_clipboard(RegisterClipboardFormatW(w!("PNG")), png);
    }
    let ok = put_clipboard(CF_DIB.0 as u32, data.dib);
    let _ = CloseClipboard();
    ok
}

unsafe fn put_clipboard(format: u32, buf: decode::GlobalBuf) -> bool {
    let handle = buf.into_handle();
    if SetClipboardData(format, Some(HANDLE(handle.0))).is_ok() {
        return true; // the clipboard owns the memory now
    }
    let _ = GlobalFree(Some(handle));
    false
}
