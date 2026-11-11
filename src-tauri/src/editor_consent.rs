//! Linux native consent, independent of web content and renderer-supplied flags.
//! Uses GTK 3 already required by Tauri; no additional crate or helper process.
use crate::editor::{error, Result};

#[cfg(target_os = "linux")]
pub fn confirm(description: &str) -> Result<bool> {
    use std::{
        ffi::{c_char, c_int, c_void, CString},
        sync::atomic::{AtomicBool, Ordering},
    };
    static OPEN: AtomicBool = AtomicBool::new(false);
    if OPEN.swap(true, Ordering::SeqCst) {
        return Err(error(
            "grant",
            "Another native editor confirmation is open.",
        ));
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            OPEN.store(false, Ordering::SeqCst);
        }
    }
    let _reset = Reset;
    let description =
        CString::new(description).map_err(|_| error("grant", "Invalid folder description."))?;
    #[link(name = "gtk-3")]
    unsafe extern "C" {
        fn gtk_message_dialog_new(
            parent: *mut c_void,
            flags: c_int,
            kind: c_int,
            buttons: c_int,
            format: *const c_char,
            ...
        ) -> *mut c_void;
        fn gtk_window_set_title(window: *mut c_void, title: *const c_char);
        fn gtk_dialog_set_default_response(dialog: *mut c_void, response: c_int);
        fn gtk_dialog_run(dialog: *mut c_void) -> c_int;
        fn gtk_widget_destroy(widget: *mut c_void);
    }
    // Called only by run_on_main_thread. GTK owns the dialog until destroy.
    // Plain text plus a constant format prevents markup/format interpretation.
    // GTK_MODAL=1, MESSAGE_QUESTION=2, BUTTONS_YES_NO=4; default NO=-9,
    // only RESPONSE_YES=-8 grants access. Close/Escape never grant access.
    unsafe {
        let dialog = gtk_message_dialog_new(
            std::ptr::null_mut(),
            1,
            2,
            4,
            c"%s".as_ptr(),
            description.as_ptr(),
        );
        if dialog.is_null() {
            return Err(error("grant", "Native editor confirmation unavailable."));
        }
        gtk_window_set_title(dialog, c"CorpsTex — Grant editor access".as_ptr());
        gtk_dialog_set_default_response(dialog, -9);
        let response = gtk_dialog_run(dialog);
        gtk_widget_destroy(dialog);
        Ok(response == -8)
    }
}

#[cfg(not(target_os = "linux"))]
pub fn confirm(_: &str) -> Result<bool> {
    Err(error(
        "grant",
        "Native editor consent is unavailable on this platform.",
    ))
}
