//! Delete confirmation and moving files to the Recycle Bin.

use super::*;

/// Outcome of moving a file to the Recycle Bin.
pub(super) enum DeleteResult {
    Deleted,
    /// The user declined Windows' "permanently delete?" warning (no Recycle Bin on that drive).
    Cancelled,
    Failed,
}

impl App {
    /// Delete key / ⋯ > Delete: confirm first if the setting asks for it.
    pub(super) fn request_delete(&mut self) {
        if self.delete_busy || self.want_delete.is_some() {
            return;
        }
        let Some(path) = self.current().cloned() else {
            return;
        };
        if self.confirm_delete {
            self.touch.cancel();
            self.drag = None;
            self.delete_prompt = Some(path);
        } else {
            self.want_delete = Some(path);
        }
        self.invalidate();
    }

    pub(super) fn on_deleted(&mut self, path: PathBuf, result: DeleteResult) {
        match result {
            DeleteResult::Deleted => {
                // A folder listing still in flight predates the delete and would bring the file back.
                self.generation += 1;
                if let Some(i) = self.files.iter().position(|p| *p == path) {
                    self.files.remove(i);
                    if i < self.index || self.index >= self.files.len() {
                        self.index = self.index.saturating_sub(1);
                    }
                }
                self.cache.remove(&path);
                self.infos.remove(&path);
                self.view_for = None;
                self.anim = None;
                self.drawer_scroll = 0.0;
                self.schedule();
                self.title_dirty = true;
                self.show_notice("Moved to the Recycle Bin.", true);
            }
            DeleteResult::Cancelled => self.invalidate(),
            DeleteResult::Failed => self.show_notice("Couldn't delete the file.", false),
        }
    }
}

/// Move a file to the Recycle Bin through the shell, like Explorer does. Where the drive has no
/// Recycle Bin, Windows' own "permanently delete?" warning is shown (FOF_WANTNUKEWARNING), so a file
/// is never destroyed silently.
pub(super) unsafe fn recycle(hwnd: HWND, path: &std::path::Path) -> DeleteResult {
    // Re-check right before acting: only a regular file, never a folder; fail closed if unsure.
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return DeleteResult::Failed;
    }
    let run = || -> Result<bool> {
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        op.SetOwnerWindow(hwnd)?;
        op.SetOperationFlags(
            FOF_ALLOWUNDO
                | FOFX_RECYCLEONDELETE
                | FOF_NOCONFIRMATION
                | FOF_WANTNUKEWARNING
                | FOF_SILENT,
        )?;
        let wp = wide(path);
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wp.as_ptr()), None)?;
        op.DeleteItem(&item, None)?;
        op.PerformOperations()?;
        Ok(op.GetAnyOperationsAborted()?.as_bool())
    };
    match run() {
        Ok(_) if !path.exists() => DeleteResult::Deleted,
        Ok(true) => DeleteResult::Cancelled,
        _ => DeleteResult::Failed,
    }
}
