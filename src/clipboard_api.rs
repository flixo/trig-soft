use arboard::{Clipboard, Error};

#[cfg(target_os = "linux")]
use arboard::{GetExtLinux, LinuxClipboardKind, SetExtLinux};

// Simple string-only clipboard API backed by arboard.
pub struct ClipboardApi {
    inner: Clipboard,
}

impl ClipboardApi {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            inner: Clipboard::new()?,
        })
    }

    #[cfg(target_os = "linux")]
    pub fn write_text(&mut self, text: &str) -> Result<(), Error> {
        let clipboard_result = self
            .inner
            .set()
            .clipboard(LinuxClipboardKind::Clipboard)
            .text(text.to_owned());

        let primary_result = self
            .inner
            .set()
            .clipboard(LinuxClipboardKind::Primary)
            .text(text.to_owned());

        if clipboard_result.is_ok() || primary_result.is_ok() {
            Ok(())
        } else {
            clipboard_result.and(primary_result)
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn write_text(&mut self, text: &str) -> Result<(), Error> {
        self.inner.set_text(text.to_owned())
    }

    #[cfg(target_os = "linux")]
    pub fn read_text(&mut self) -> Result<String, Error> {
        let clipboard_result = self
            .inner
            .get()
            .clipboard(LinuxClipboardKind::Clipboard)
            .text();

        if clipboard_result.is_ok() {
            return clipboard_result;
        }

        self.inner
            .get()
            .clipboard(LinuxClipboardKind::Primary)
            .text()
    }

    #[cfg(not(target_os = "linux"))]
    pub fn read_text(&mut self) -> Result<String, Error> {
        self.inner.get_text()
    }
}
