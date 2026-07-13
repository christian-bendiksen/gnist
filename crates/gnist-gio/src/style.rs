use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use crate::{config, config::GtkApi, config::Mode, ox_log, symbols};

#[derive(Clone, Copy)]
struct GObjectPtr(NonNull<c_void>);

impl GObjectPtr {
    fn from_raw(pointer: *mut c_void) -> Option<Self> {
        NonNull::new(pointer).map(Self)
    }

    fn as_ptr(self) -> *mut c_void {
        self.0.as_ptr()
    }
}

struct OwnedGObject {
    pointer: GObjectPtr,
}

impl OwnedGObject {
    fn from_owned_raw(pointer: *mut c_void) -> Option<Self> {
        GObjectPtr::from_raw(pointer).map(|pointer| Self { pointer })
    }

    fn retain(pointer: GObjectPtr) -> Option<Self> {
        let symbols = symbols::core()?;
        let retained = unsafe { (symbols.g_object_ref)(pointer.as_ptr()) };
        Self::from_owned_raw(retained)
    }

    fn as_ptr(&self) -> *mut c_void {
        self.pointer.as_ptr()
    }

    fn pointer(&self) -> GObjectPtr {
        self.pointer
    }
}

impl Drop for OwnedGObject {
    fn drop(&mut self) {
        let Some(symbols) = symbols::core() else { return };
        unsafe { (symbols.g_object_unref)(self.pointer.as_ptr()) };
    }
}

struct StyleState {
    owner: GObjectPtr,
    provider: OwnedGObject,
    path: PathBuf,
    gtk: GtkApi,
}

impl StyleState {
    fn snapshot(&self) -> ReloadTarget {
        ReloadTarget {
            owner: self.owner,
            provider: self.provider.pointer(),
            path: self.path.clone(),
            gtk: self.gtk,
        }
    }
}

#[derive(Clone)]
struct ReloadTarget {
    owner: GObjectPtr,
    provider: GObjectPtr,
    path: PathBuf,
    gtk: GtkApi,
}

struct FadeState {
    targets: Vec<OwnedGObject>,
    set_opacity: symbols::WidgetSetOpacity,
    reload: ReloadTarget,
    mode: Mode,
    start: Instant,
    half_duration: Duration,
    midpoint_done: bool,
}

thread_local! {
    static STYLE: RefCell<Option<StyleState>> = const { RefCell::new(None) };
    static FADE: RefCell<Option<FadeState>> = const { RefCell::new(None) };
    static PENDING_DISPLAY_ATTACH: Cell<Option<GtkApi>> = const { Cell::new(None) };
}

const FADE_FRAME: Duration = Duration::from_millis(16);

unsafe extern "C" {
    fn g_object_set(object: *mut c_void, first_property_name: *const c_char, ...);
}

pub fn attach(gtk: GtkApi) {
    if STYLE.with(|state| state.borrow().is_some()) {
        return;
    }

    let Some(symbols) = symbols::core() else { return };
    let Some(owner) = gtk_owner(symbols, gtk) else {
        // gdk_display_manager_get() aborts on GTK >= 4.20 unless gtk_init()
        // has run, and once it has the default display already exists, so
        // waiting for display-opened only makes sense on GTK 3.
        if gtk.is_gtk3() {
            wait_for_display(gtk);
        } else {
            ox_log!("{gtk} has no default display; skipping attach");
        }
        return;
    };

    let Some(provider) = new_css_provider(symbols) else {
        ox_log!("{gtk} failed to allocate GtkCssProvider");
        return;
    };

    let priority = config::css_priority();
    let provider_ptr = provider.pointer();
    if !attach_provider(symbols, gtk, owner, provider_ptr, priority) {
        return;
    }

    let path = config::default_css_path(gtk);
    if config::css_path_loadable(&path) {
        load_css(provider_ptr, &path, gtk);
    } else {
        ox_log!("CSS not loadable at attach: {}", path.display());
    }

    ox_log!(
        "attached {gtk} owner={:p} provider={:p} priority={priority} path={}",
        owner.as_ptr(),
        provider_ptr.as_ptr(),
        path.display(),
    );

    STYLE.with(|state| {
        state.replace(Some(StyleState { owner, provider, path, gtk }));
    });
}

pub fn update_and_reload(signal_path: &str, mode: Mode) {
    let Some(reload) = update_path(signal_path) else { return };

    ox_log!(
        "{} reload owner={:p} provider={:p} path={}",
        reload.gtk,
        reload.owner.as_ptr(),
        reload.provider.as_ptr(),
        reload.path.display(),
    );

    let transition = config::transition_ms();
    if transition == 0 || fade_active() {
        reload_instant(&reload, mode, "instant");
        return;
    }

    let Some(set_opacity) = symbols::core().and_then(|symbols| symbols.gtk_widget_set_opacity) else {
        reload_instant(&reload, mode, "no-opacity-symbol");
        return;
    };

    let targets = collect_fade_targets(reload.gtk);
    if targets.is_empty() {
        reload_instant(&reload, mode, "no-windows");
        return;
    }

    let gtk = reload.gtk;
    let half_duration = Duration::from_millis(((transition as u64) / 2).max(1));
    FADE.with(|state| {
        state.replace(Some(FadeState {
            targets,
            set_opacity,
            reload,
            mode,
            start: Instant::now(),
            half_duration,
            midpoint_done: false,
        }));
    });

    glib::timeout_add_local(FADE_FRAME, tick_fade);

    // GTK 3 recomputes styles synchronously. Start it one frame early so that
    // work finishes near zero opacity and the fade-in stays aligned with GTK 4.
    if gtk.is_gtk3() {
        let pre = half_duration.saturating_sub(FADE_FRAME);
        glib::timeout_add_local(pre, move || {
            apply_gtk3_mode(mode);
            glib::ControlFlow::Break
        });
    }

    ox_log!("fade started ms={transition}");
}

pub fn cleanup() {
    FADE.with(|state| {
        if let Some(fade) = state.borrow_mut().take() {
            for target in &fade.targets {
                unsafe { (fade.set_opacity)(target.as_ptr(), 1.0) };
            }
        }
    });

    STYLE.with(|state| {
        state.replace(None);
    });
}

fn update_path(signal_path: &str) -> Option<ReloadTarget> {
    STYLE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(style) = state.as_mut() else {
            ox_log!("update_and_reload: provider was never attached");
            return None;
        };

        if !signal_path.is_empty() {
            let candidate = PathBuf::from(signal_path);
            if config::css_path_loadable(&candidate) {
                if style.path != candidate {
                    ox_log!(
                        "{} CSS path: {} -> {}",
                        style.gtk,
                        style.path.display(),
                        candidate.display(),
                    );
                    style.path = candidate;
                }
            } else {
                ox_log!(
                    "signal path not loadable, keeping: {}",
                    style.path.display()
                );
            }
        }

        Some(style.snapshot())
    })
}

fn fade_active() -> bool {
    FADE.with(|state| state.borrow().is_some())
}

fn reload_instant(reload: &ReloadTarget, mode: Mode, reason: &'static str) {
    if reload.gtk.is_gtk3() {
        apply_gtk3_mode(mode);
    }
    reload_css_and_notify(reload, reason);
}

fn reload_css_and_notify(reload: &ReloadTarget, reason: &'static str) {
    load_css(reload.provider, &reload.path, reload.gtk);
    notify_gtk_theme_two_phase(reason);
    ox_log!(
        "reloaded {} provider={:p} path={}",
        reload.gtk,
        reload.provider.as_ptr(),
        reload.path.display(),
    );
}

fn load_css(provider: GObjectPtr, path: &Path, gtk: GtkApi) {
    let Some(symbols) = symbols::core() else { return };
    let Some(path) = path.to_str() else {
        ox_log!("CSS path is not valid UTF-8: {}", path.display());
        return;
    };
    let Ok(c_path) = CString::new(path) else {
        ox_log!("CSS path contains NUL: {path}");
        return;
    };

    match gtk {
        GtkApi::Gtk4 => match symbols.gtk_css_provider_load_from_path_gtk4 {
            Some(load) => unsafe { load(provider.as_ptr(), c_path.as_ptr()) },
            None => ox_log!("GTK4 load_from_path symbol missing"),
        },
        GtkApi::Gtk3 => match symbols.gtk_css_provider_load_from_path_gtk3 {
            Some(load) => {
                let ok = unsafe { load(provider.as_ptr(), c_path.as_ptr(), std::ptr::null_mut()) };
                ox_log!(
                    "GTK3 load_from_path ok={ok} provider={:p} path={path}",
                    provider.as_ptr(),
                );
            }
            None => ox_log!("GTK3 load_from_path symbol missing"),
        },
    }
}

fn apply_gtk3_mode(mode: Mode) {
    if !mode.is_known() || !config::set_gtk3_theme_enabled() {
        return;
    }

    let Some(symbols) = symbols::core() else { return };
    let Some(get_settings) = symbols.gtk_settings_get_default else { return };
    let Some(settings) = GObjectPtr::from_raw(unsafe { get_settings() }) else { return };
    let Some(prefer_dark) = mode.prefer_dark() else { return };

    let prefer_dark: libc::c_int = prefer_dark.into();
    let theme = config::gtk3_theme_name(mode);

    if let Some(theme) = theme {
        let Ok(theme_c) = CString::new(theme) else { return };
        unsafe {
            g_object_set(
                settings.as_ptr(),
                c"gtk-theme-name".as_ptr(),
                theme_c.as_ptr(),
                c"gtk-application-prefer-dark-theme".as_ptr(),
                prefer_dark,
                std::ptr::null::<c_char>(),
            )
        };
        ox_log!("GTK3 mode={mode} theme={theme} prefer-dark={prefer_dark}");
    } else {
        unsafe {
            g_object_set(
                settings.as_ptr(),
                c"gtk-application-prefer-dark-theme".as_ptr(),
                prefer_dark,
                std::ptr::null::<c_char>(),
            )
        };
        ox_log!("GTK3 mode={mode} theme=unchanged prefer-dark={prefer_dark}");
    }
}

fn notify_gtk_theme_two_phase(reason: &'static str) {
    notify_gtk_theme("phase=1");
    glib::idle_add_local_once(move || {
        notify_gtk_theme("phase=2");
        ox_log!("theme notify complete: {reason}");
    });
}

fn notify_gtk_theme(phase: &str) {
    let Some(symbols) = symbols::core() else { return };
    let (Some(get_settings), Some(notify)) =
        (symbols.gtk_settings_get_default, symbols.g_object_notify)
    else {
        return;
    };

    let Some(settings) = GObjectPtr::from_raw(unsafe { get_settings() }) else { return };
    unsafe {
        notify(settings.as_ptr(), c"gtk-theme-name".as_ptr());
        notify(
            settings.as_ptr(),
            c"gtk-application-prefer-dark-theme".as_ptr(),
        );
    }
    ox_log!("notified GtkSettings {phase}");
}


fn tick_fade() -> glib::ControlFlow {
    let mut midpoint_reload = None;
    let mut done = false;
    let mut had_fade = false;

    FADE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(fade) = state.as_mut() else { return };

        had_fade = true;
        let elapsed = fade.start.elapsed();
        let half = fade.half_duration;

        if elapsed >= half && !fade.midpoint_done {
            fade.midpoint_done = true;
            midpoint_reload = Some((fade.reload.clone(), fade.mode));
        }

        let opacity = if elapsed < half {
            1.0 - elapsed.as_secs_f64() / half.as_secs_f64()
        } else {
            ((elapsed - half).as_secs_f64() / half.as_secs_f64()).min(1.0)
        };

        for target in &fade.targets {
            unsafe { (fade.set_opacity)(target.as_ptr(), opacity) };
        }

        if elapsed >= half.saturating_mul(2) {
            for target in &fade.targets {
                unsafe { (fade.set_opacity)(target.as_ptr(), 1.0) };
            }
            done = true;
        }
    });

    if let Some((reload, _mode)) = midpoint_reload {
        reload_css_and_notify(&reload, "fade-midpoint");
        ox_log!("fade midpoint: CSS swapped");
    }

    if done {
        FADE.with(|state| {
            state.borrow_mut().take();
        });
        return glib::ControlFlow::Break;
    }

    if had_fade {
        glib::ControlFlow::Continue
    } else {
        glib::ControlFlow::Break
    }
}

fn collect_fade_targets(gtk: GtkApi) -> Vec<OwnedGObject> {
    let Some(symbols) = symbols::core() else { return Vec::new() };
    let get_child = match gtk {
        GtkApi::Gtk4 => symbols.gtk_window_get_child,
        GtkApi::Gtk3 => symbols.gtk_bin_get_child,
    };

    let Some(windows) = collect_windows(symbols) else {
        return Vec::new();
    };

    windows
        .iter()
        .filter_map(|window| {
            let target = get_child
                .and_then(|get_child| GObjectPtr::from_raw(unsafe { get_child(window.as_ptr()) }))
                .unwrap_or(window);
            OwnedGObject::retain(target)
        })
        .collect()
}

struct GListOwned {
    head: *mut symbols::GList,
    free: symbols::ListFree,
}

impl GListOwned {
    fn iter(&self) -> GListIter {
        GListIter { node: self.head }
    }
}

impl Drop for GListOwned {
    fn drop(&mut self) {
        if !self.head.is_null() {
            unsafe { (self.free)(self.head.cast()) };
        }
    }
}

struct GListIter {
    node: *mut symbols::GList,
}

impl Iterator for GListIter {
    type Item = GObjectPtr;

    fn next(&mut self) -> Option<Self::Item> {
        let node = unsafe { self.node.as_ref()? };
        self.node = node.next;
        GObjectPtr::from_raw(node.data)
    }
}

fn collect_windows(symbols: &symbols::CoreSyms) -> Option<GListOwned> {
    let list_toplevels = symbols.gtk_window_list_toplevels?;
    let free = symbols.g_list_free?;
    let head = unsafe { list_toplevels() };
    Some(GListOwned { head, free })
}

fn gtk_owner(symbols: &symbols::CoreSyms, gtk: GtkApi) -> Option<GObjectPtr> {
    let pointer = unsafe {
        match gtk {
            GtkApi::Gtk4 => symbols.gdk_display_get_default?(),
            GtkApi::Gtk3 => symbols.gdk_screen_get_default?(),
        }
    };
    GObjectPtr::from_raw(pointer)
}

fn new_css_provider(symbols: &symbols::CoreSyms) -> Option<OwnedGObject> {
    let provider = unsafe { (symbols.gtk_css_provider_new)() };
    OwnedGObject::from_owned_raw(provider)
}

fn attach_provider(
    symbols: &symbols::CoreSyms,
    gtk: GtkApi,
    owner: GObjectPtr,
    provider: GObjectPtr,
    priority: u32,
) -> bool {
    match gtk {
        GtkApi::Gtk4 => {
            let Some(add_provider) = symbols.gtk_style_context_add_provider_for_display else {
                ox_log!("GTK4 provider attach symbol missing");
                return false;
            };
            unsafe { add_provider(owner.as_ptr(), provider.as_ptr(), priority) };
        }
        GtkApi::Gtk3 => {
            let Some(add_provider) = symbols.gtk_style_context_add_provider_for_screen else {
                ox_log!("GTK3 provider attach symbol missing");
                return false;
            };
            unsafe { add_provider(owner.as_ptr(), provider.as_ptr(), priority) };
        }
    }

    true
}

fn wait_for_display(gtk: GtkApi) {
    let Some(symbols) = symbols::core() else { return };
    let (Some(get_manager), Some(connect)) =
        (symbols.gdk_display_manager_get, symbols.g_signal_connect_data)
    else {
        return;
    };

    let Some(manager) = GObjectPtr::from_raw(unsafe { get_manager() }) else { return };
    PENDING_DISPLAY_ATTACH.with(|pending| pending.set(Some(gtk)));

    unsafe {
        connect(
            manager.as_ptr(),
            c"display-opened".as_ptr(),
            // This FFI signature models GLib's `GCallback` parameter as `*mut c_void`.
            on_display_opened as *mut c_void,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };

    ox_log!("waiting for display-opened signal");
}

/// # Safety
/// GLib must invoke this on the main thread through the registered signal. The
/// GDK pointers are ignored; only thread-local pending state is accessed.
unsafe extern "C" fn on_display_opened(
    _manager: *mut c_void,
    _display: *mut c_void,
    _data: *mut c_void,
) {
    let gtk = PENDING_DISPLAY_ATTACH.with(|pending| pending.take());
    if let Some(gtk) = gtk {
        attach(gtk);
    }
}
