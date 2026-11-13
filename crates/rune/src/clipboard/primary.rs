//! Independent primary-selection requests; never alias the regular clipboard.
#[derive(Clone, Default)]
pub(crate) struct Requests {
    pub copy: Option<String>,
    pub paste: Option<Option<String>>,
}

impl std::fmt::Debug for Requests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrimaryRequests").finish_non_exhaustive()
    }
}

impl crate::Tx {
    /// Export a non-secret text selection on Linux. Other platforms ignore this request.
    pub fn copy_primary(&mut self, text: impl Into<String>) {
        if cfg!(target_os = "linux") {
            self.primary.copy = Some(text.into());
        }
    }

    /// Read Linux's primary selection asynchronously into the focused paste target.
    /// A terminal can blur the field and call this from its explicit middle-button handler.
    pub fn request_primary_paste(&mut self) {
        if cfg!(target_os = "linux") {
            self.primary.paste = Some(None);
        }
    }
}
