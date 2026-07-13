use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

use crate::config::GtkApi;

pub type GetMajorVersion = unsafe extern "C" fn() -> u32;
pub type IsInitialized = unsafe extern "C" fn() -> i32;
pub type CssProviderNew = unsafe extern "C" fn() -> *mut c_void;
pub type CssProviderLoadGtk4 = unsafe extern "C" fn(*mut c_void, *const c_char);
pub type CssProviderLoadGtk3 = unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut c_void) -> i32;
pub type DisplayGetDefault = unsafe extern "C" fn() -> *mut c_void;
pub type ScreenGetDefault = unsafe extern "C" fn() -> *mut c_void;
pub type AddProvider = unsafe extern "C" fn(*mut c_void, *mut c_void, u32);
pub type SettingsGetDefault = unsafe extern "C" fn() -> *mut c_void;
pub type ObjectRef = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
pub type ObjectUnref = unsafe extern "C" fn(*mut c_void);
pub type ObjectNotify = unsafe extern "C" fn(*mut c_void, *const c_char);
pub type WidgetSetOpacity = unsafe extern "C" fn(*mut c_void, f64);
pub type WindowGetChild = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
pub type WindowListToplevels = unsafe extern "C" fn() -> *mut GList;
pub type ListFree = unsafe extern "C" fn(*mut c_void);
pub type DisplayManagerGet = unsafe extern "C" fn() -> *mut c_void;
pub type SignalConnectData = unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    u32,
) -> u64;

#[repr(C)]
pub struct GList {
    pub data: *mut c_void,
    pub next: *mut GList,
    pub prev: *mut GList,
}

/// Function pointers loaded from the GTK process hosting this module.
///
/// Common symbols are required before the module can attach. Version-specific
/// APIs and fade helpers remain optional so one build can support GTK 3 and 4.
pub struct CoreSyms {
    pub gtk_get_major_version: GetMajorVersion,
    pub gtk_is_initialized: Option<IsInitialized>,
    pub gtk_css_provider_new: CssProviderNew,
    pub gtk_css_provider_load_from_path_gtk4: Option<CssProviderLoadGtk4>,
    pub gtk_css_provider_load_from_path_gtk3: Option<CssProviderLoadGtk3>,
    pub gdk_display_get_default: Option<DisplayGetDefault>,
    pub gtk_style_context_add_provider_for_display: Option<AddProvider>,
    pub gdk_screen_get_default: Option<ScreenGetDefault>,
    pub gtk_style_context_add_provider_for_screen: Option<AddProvider>,
    pub gtk_settings_get_default: Option<SettingsGetDefault>,
    pub g_object_ref: ObjectRef,
    pub g_object_unref: ObjectUnref,
    pub g_object_notify: Option<ObjectNotify>,
    pub gtk_widget_set_opacity: Option<WidgetSetOpacity>,
    pub gtk_window_get_child: Option<WindowGetChild>,
    pub gtk_bin_get_child: Option<WindowGetChild>,
    pub gtk_window_list_toplevels: Option<WindowListToplevels>,
    pub g_list_free: Option<ListFree>,
    pub gdk_display_manager_get: Option<DisplayManagerGet>,
    pub g_signal_connect_data: Option<SignalConnectData>,
}

static CORE: OnceLock<CoreSyms> = OnceLock::new();

pub fn core() -> Option<&'static CoreSyms> {
    if let Some(symbols) = CORE.get() {
        return Some(symbols);
    }

    let symbols = unsafe {
        CoreSyms {
            gtk_get_major_version: symbol(b"gtk_get_major_version\0")?,
            gtk_is_initialized: symbol(b"gtk_is_initialized\0"),
            gtk_css_provider_new: symbol(b"gtk_css_provider_new\0")?,
            gtk_css_provider_load_from_path_gtk4: symbol(b"gtk_css_provider_load_from_path\0"),
            gtk_css_provider_load_from_path_gtk3: symbol(b"gtk_css_provider_load_from_path\0"),
            gdk_display_get_default: symbol(b"gdk_display_get_default\0"),
            gtk_style_context_add_provider_for_display: symbol(
                b"gtk_style_context_add_provider_for_display\0",
            ),
            gdk_screen_get_default: symbol(b"gdk_screen_get_default\0"),
            gtk_style_context_add_provider_for_screen: symbol(
                b"gtk_style_context_add_provider_for_screen\0",
            ),
            gtk_settings_get_default: symbol(b"gtk_settings_get_default\0"),
            g_object_ref: symbol(b"g_object_ref\0")?,
            g_object_unref: symbol(b"g_object_unref\0")?,
            g_object_notify: symbol(b"g_object_notify\0"),
            gtk_widget_set_opacity: symbol(b"gtk_widget_set_opacity\0"),
            gtk_window_get_child: symbol(b"gtk_window_get_child\0"),
            gtk_bin_get_child: symbol(b"gtk_bin_get_child\0"),
            gtk_window_list_toplevels: symbol(b"gtk_window_list_toplevels\0"),
            g_list_free: symbol(b"g_list_free\0"),
            gdk_display_manager_get: symbol(b"gdk_display_manager_get\0"),
            g_signal_connect_data: symbol(b"g_signal_connect_data\0"),
        }
    };

    let _ = CORE.set(symbols);
    CORE.get()
}

/// GTK 4.20 made calling into GDK before `gtk_init()` a fatal error, so the
/// module must not touch GDK until the host has initialized GTK. GTK 3 does
/// not export the probe and has no such check, so it counts as initialized.
pub fn gtk_initialized() -> bool {
    let Some(symbols) = core() else { return false };
    match symbols.gtk_is_initialized {
        Some(is_initialized) => (unsafe { is_initialized() }) != 0,
        None => true,
    }
}

pub fn detected_gtk() -> Option<(u32, Option<GtkApi>)> {
    let symbols = core()?;
    let major = unsafe { (symbols.gtk_get_major_version)() };
    Some((major, GtkApi::from_major(major)))
}

/// Look up a symbol in the current process and cast it to a function pointer.
///
/// # Safety
/// `name` must end with a NUL byte. `F` must exactly match the exported
/// function's ABI and signature.
unsafe fn symbol<F: Copy>(name: &[u8]) -> Option<F> {
    debug_assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*mut c_void>());

    let pointer = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr().cast()) };
    (!pointer.is_null()).then(|| unsafe { std::mem::transmute_copy(&pointer) })
}
