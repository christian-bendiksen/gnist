mod config;
mod dbus;
mod style;
mod symbols;

use std::{ffi::c_void, time::Duration};

const ATTACH_RETRIES: u32 = 50;
const ATTACH_RETRY: Duration = Duration::from_millis(100);

#[macro_export]
macro_rules! ox_log {
    ($($arg:tt)*) => {
        if $crate::config::debug_enabled() {
            eprintln!("[gnist-gio] {}", format_args!($($arg)*));
        }
    };
}

/// GIO module entry point that starts attachment to the host GTK process.
///
/// # Safety
/// GIO must pass either null or a valid `GTypeModule *` as `module`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn g_io_module_load(module: *mut c_void) {
    if config::disabled() {
        return;
    }

    if !module.is_null() {
        unsafe extern "C" {
            fn g_type_module_use(module: *mut c_void) -> i32;
        }
        unsafe { g_type_module_use(module) };
    }

    ox_log!(
        "module loaded pid={} version={}",
        std::process::id(),
        config::VERSION
    );

    glib::idle_add_local_once(|| attach_or_retry(0));
}

/// GIO module entry point that releases process-local state.
///
/// # Safety
/// This must be called through GIO's module teardown path. The module pointer is
/// ignored and is never dereferenced.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn g_io_module_unload(_module: *mut c_void) {
    dbus::unsubscribe();
    style::cleanup();
    ox_log!("module unloaded version={}", config::VERSION);
}

/// Return this module's empty list of extension points.
///
/// # Safety
/// The returned pointer refers to process-lifetime static storage. The caller
/// must not write through it or free it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn g_io_module_query() -> *mut *mut i8 {
    // Raw pointers are not Sync, so store the null terminator as an integer.
    static FEATURES: [usize; 1] = [0];
    FEATURES.as_ptr().cast::<*mut i8>().cast_mut()
}

fn attach_or_retry(attempt: u32) {
    let Some((major, gtk)) = symbols::detected_gtk() else {
        retry_or_skip(attempt, "GTK symbols not ready");
        return;
    };

    let Some(gtk) = gtk else {
        ox_log!("unsupported GTK major version {major}; skipping");
        return;
    };

    // Remote GApplication instances never run gtk_init(); retries expire
    // harmlessly before such a process exits.
    if !symbols::gtk_initialized() {
        retry_or_skip(attempt, "GTK not initialized");
        return;
    }

    ox_log!(
        "attach {gtk} display={} version={}",
        std::env::var("WAYLAND_DISPLAY").unwrap_or_default(),
        config::VERSION,
    );

    style::attach(gtk);
    dbus::subscribe();
}

fn retry_or_skip(attempt: u32, reason: &'static str) {
    if attempt >= ATTACH_RETRIES {
        ox_log!("{reason}; skipping");
        return;
    }

    ox_log!("{reason}; retry {attempt}/{ATTACH_RETRIES}");
    glib::timeout_add_local(ATTACH_RETRY, move || {
        attach_or_retry(attempt + 1);
        glib::ControlFlow::Break
    });
}
