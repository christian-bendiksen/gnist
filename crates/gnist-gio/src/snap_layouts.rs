//! Maximize hover for native GTK client headers. No toolkit linkage or theme
//! mutations: symbols are resolved from the host just like the CSS attachment.
use crate::{
    config::GtkApi,
    snap_wayland::Connection,
    symbols::{self, symbol},
};
use glib::{
    prelude::*,
    translate::{from_glib_full, from_glib_none},
};
use std::{
    cell::{Cell, RefCell},
    ffi::{c_char, c_void},
    rc::{Rc, Weak},
    time::Duration,
};
type Ptr = *mut c_void;
type Getter = unsafe extern "C" fn(Ptr) -> Ptr;
type Test = unsafe extern "C" fn(Ptr) -> i32;
const MARKER: &str = "gnist-snap-layouts-controller-v1";

struct DisplayBinding {
    display: glib::Object,
    connection: Rc<Connection>,
    closed: Option<glib::SignalHandlerId>,
}
impl Drop for DisplayBinding {
    fn drop(&mut self) {
        self.connection.close();
        if let Some(signal) = self.closed.take() {
            self.display.disconnect(signal);
        }
    }
}
struct Attachment {
    gtk: GtkApi,
    signal: u32,
    hook: libc::c_ulong,
    displays: Vec<DisplayBinding>,
}
thread_local! {
    static ATTACHED: RefCell<Option<Attachment>> = const { RefCell::new(None) };
    static SCAN_PENDING: Cell<bool> = const { Cell::new(false) };
}
pub fn attach(gtk: GtkApi) {
    detach();
    let Some(widget_type): Option<unsafe extern "C" fn() -> glib::ffi::GType> =
        (unsafe { symbol(b"gtk_widget_get_type\0") })
    else {
        return;
    };
    // Queue one scan after maps settle, including new windows and header rebuilds.
    // No periodic tree walking or frame callbacks in an application's hot path.
    // A module may attach before the application's first widget exists. The
    // class initializer registers GtkWidget signals, not get_type() alone.
    let signal = unsafe {
        let ty = widget_type();
        let class = glib::gobject_ffi::g_type_class_ref(ty);
        let signal = glib::gobject_ffi::g_signal_lookup(c"map".as_ptr(), ty);
        glib::gobject_ffi::g_type_class_unref(class);
        signal
    };
    if signal == 0 {
        return;
    }
    let hook = unsafe {
        glib::gobject_ffi::g_signal_add_emission_hook(
            signal,
            0,
            Some(mapped),
            std::ptr::null_mut(),
            None,
        )
    };
    ATTACHED.with(|slot| {
        *slot.borrow_mut() = Some(Attachment {
            gtk,
            signal,
            hook,
            displays: Vec::new(),
        })
    });
    queue_scan();
}
pub fn detach() {
    let old = ATTACHED.with(|slot| slot.borrow_mut().take());
    if let Some(old) = old {
        unsafe { glib::gobject_ffi::g_signal_remove_emission_hook(old.signal, old.hook) };
    }
}
unsafe extern "C" fn mapped(
    _: *mut glib::gobject_ffi::GSignalInvocationHint,
    _: u32,
    _: *const glib::gobject_ffi::GValue,
    _: Ptr,
) -> i32 {
    queue_scan();
    1
}
pub(crate) fn queue_scan() {
    if SCAN_PENDING.with(|pending| pending.replace(true)) {
        return;
    }
    glib::idle_add_local_once(|| {
        SCAN_PENDING.with(|pending| pending.set(false));
        scan();
    });
}
fn scan() {
    let Some(core) = symbols::core() else {
        return;
    };
    let Some((list, free)) = core.gtk_window_list_toplevels.zip(core.g_list_free) else {
        return;
    };
    let gtk = ATTACHED.with(|slot| slot.borrow().as_ref().map(|a| a.gtk));
    let Some(gtk) = gtk else {
        return;
    };
    unsafe {
        let head = list();
        let mut item = head;
        while !item.is_null() {
            visit((*item).data, gtk);
            item = (*item).next;
        }
        free(head.cast());
    }
}
unsafe fn visit(widget: Ptr, gtk: GtkApi) {
    if widget.is_null() {
        return;
    }
    let object: glib::Object = unsafe { from_glib_none(widget.cast()) };
    if is_type(&object, "GtkButton") && unsafe { is_maximize(widget, gtk) } {
        attach_button(&object, gtk);
    }
    let Some(core) = symbols::core() else {
        return;
    };
    match gtk {
        GtkApi::Gtk4 => {
            if let Some((first, next)) = core
                .gtk_widget_get_first_child
                .zip(core.gtk_widget_get_next_sibling)
            {
                let mut child = unsafe { first(widget) };
                while !child.is_null() {
                    unsafe { visit(child, gtk) };
                    child = unsafe { next(child) };
                }
            }
        }
        GtkApi::Gtk3 => {
            if is_type(&object, "GtkContainer")
                && let Some(forall) = core.gtk_container_forall
            {
                unsafe { forall(widget, visit_gtk3, std::ptr::null_mut()) };
            }
        }
    }
}
unsafe extern "C" fn visit_gtk3(widget: Ptr, _: Ptr) {
    unsafe { visit(widget, GtkApi::Gtk3) };
}
fn is_type(object: &glib::Object, name: &str) -> bool {
    glib::Type::from_name(name).is_some_and(|ty| object.type_().is_a(ty))
}
unsafe fn has_class(widget: Ptr, gtk: GtkApi, name: *const c_char) -> bool {
    let name_fn = match gtk {
        GtkApi::Gtk4 => b"gtk_widget_has_css_class\0".as_slice(),
        GtkApi::Gtk3 => b"gtk_style_context_has_class\0".as_slice(),
    };
    let Some(has): Option<unsafe extern "C" fn(Ptr, *const c_char) -> i32> =
        (unsafe { symbol(name_fn) })
    else {
        return false;
    };
    let object = match gtk {
        GtkApi::Gtk4 => widget,
        GtkApi::Gtk3 => {
            let Some(context): Option<Getter> =
                (unsafe { symbol(b"gtk_widget_get_style_context\0") })
            else {
                return false;
            };
            unsafe { context(widget) }
        }
    };
    unsafe { has(object, name) != 0 }
}
unsafe fn is_maximize(widget: Ptr, gtk: GtkApi) -> bool {
    if !unsafe { has_class(widget, gtk, c"maximize".as_ptr()) } {
        return false;
    }
    let Some(parent): Option<Getter> = (unsafe { symbol(b"gtk_widget_get_parent\0") }) else {
        return false;
    };
    let mut ancestor = unsafe { parent(widget) };
    let mut header = false;
    while !ancestor.is_null() {
        if unsafe { has_class(ancestor, gtk, c"herda-titlebar".as_ptr()) } {
            return false;
        }
        let object: glib::Object = unsafe { from_glib_none(ancestor.cast()) };
        header |= is_type(
            &object,
            match gtk {
                GtkApi::Gtk4 => "GtkWindowControls",
                GtkApi::Gtk3 => "GtkHeaderBar",
            },
        );
        ancestor = unsafe { parent(ancestor) };
    }
    header
}
fn connection(widget: Ptr) -> Option<Rc<Connection>> {
    unsafe {
        let get_display: Getter = symbol(b"gtk_widget_get_display\0")?;
        let display_ptr = get_display(widget);
        if display_ptr.is_null() {
            return None;
        }
        let display: glib::Object = from_glib_none(display_ptr.cast());
        if !is_type(&display, "GdkWaylandDisplay") {
            return None;
        }
        let is_closed: Test = symbol(b"gdk_display_is_closed\0")?;
        if is_closed(display_ptr) != 0 {
            return None;
        }
        ATTACHED.with(|slot| {
            let mut slot = slot.borrow_mut();
            let attachment = slot.as_mut()?;
            if let Some(binding) = attachment
                .displays
                .iter()
                .find(|binding| binding.display == display)
            {
                return Some(binding.connection.clone());
            }
            let get_wayland: Getter = symbol(b"gdk_wayland_display_get_wl_display\0")?;
            let wire = Connection::new(get_wayland(display_ptr))?;
            let weak = Rc::downgrade(&wire);
            let closed = display.connect_local("closed", false, move |_| {
                if let Some(wire) = weak.upgrade() {
                    wire.close();
                }
                None
            });
            attachment.displays.push(DisplayBinding {
                display,
                connection: wire.clone(),
                closed: Some(closed),
            });
            Some(wire)
        })
    }
}
struct Hover {
    widget: glib::WeakRef<glib::Object>,
    wire: Weak<Connection>,
    gtk: GtkApi,
    timer: RefCell<Option<glib::SourceId>>,
    shown: Cell<bool>,
}
impl Hover {
    fn leave(&self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        if self.shown.replace(false)
            && let Some(wire) = self.wire.upgrade()
        {
            wire.dismiss();
        }
    }
    fn enter(self: &Rc<Self>) {
        self.leave();
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(Duration::from_millis(500), move || {
            let Some(hover) = weak.upgrade() else {
                return;
            };
            hover.timer.borrow_mut().take();
            let Some(wire) = hover.wire.upgrade().filter(|wire| wire.available()) else {
                return;
            };
            let Some(widget) = hover.widget.upgrade() else {
                return;
            };
            // A reparented control can move to another display. Never marshal
            // that display's surface through the old Wayland connection.
            if !connection(widget.as_ptr().cast())
                .is_some_and(|current| Rc::ptr_eq(&current, &wire))
            {
                return;
            }
            if let Some((surface, rect)) = unsafe { bounds(widget.as_ptr().cast(), hover.gtk) } {
                wire.show(surface, rect);
                hover.shown.set(true);
            }
        });
        *self.timer.borrow_mut() = Some(timer);
    }
}
impl Drop for Hover {
    fn drop(&mut self) {
        self.leave();
    }
}
fn attach_button(widget: &glib::Object, gtk: GtkApi) {
    unsafe {
        if widget.data::<glib::Object>(MARKER).is_some() {
            return;
        }
        let Some(wire) = connection(widget.as_ptr().cast()).filter(|wire| wire.available()) else {
            return;
        };
        let controller: glib::Object = match gtk {
            GtkApi::Gtk4 => {
                let Some(new): Option<unsafe extern "C" fn() -> Ptr> =
                    symbol(b"gtk_event_controller_motion_new\0")
                else {
                    return;
                };
                let Some(add): Option<unsafe extern "C" fn(Ptr, Ptr)> =
                    symbol(b"gtk_widget_add_controller\0")
                else {
                    return;
                };
                let controller = new();
                if controller.is_null() {
                    return;
                }
                add(widget.as_ptr().cast(), controller);
                from_glib_none(controller.cast())
            }
            GtkApi::Gtk3 => {
                let Some(new): Option<Getter> = symbol(b"gtk_event_controller_motion_new\0") else {
                    return;
                };
                let controller = new(widget.as_ptr().cast());
                if controller.is_null() {
                    return;
                }
                from_glib_full(controller.cast())
            }
        };
        let hover = Rc::new(Hover {
            widget: widget.downgrade(),
            wire: Rc::downgrade(&wire),
            gtk,
            timer: RefCell::new(None),
            shown: Cell::new(false),
        });
        let enter = hover.clone();
        controller.connect_local("enter", false, move |_| {
            enter.enter();
            None
        });
        let leave = hover.clone();
        controller.connect_local("leave", false, move |_| {
            leave.leave();
            None
        });
        widget.connect_local("unmap", false, move |_| {
            hover.leave();
            None
        });
        // GTK3 controllers require a retained reference. Also marks GTK4 buttons
        // so repeated maps never attach a second motion controller.
        widget.set_data(MARKER, controller);
    }
}
#[repr(C)]
#[derive(Default)]
struct FloatRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}
#[repr(C)]
#[derive(Default)]
struct Allocation {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}
fn enclosing(x: f64, y: f64, width: f64, height: f64) -> Option<[i32; 4]> {
    let edges = [
        x.floor(),
        y.floor(),
        (x + width).ceil(),
        (y + height).ceil(),
    ];
    if width <= 0.0
        || height <= 0.0
        || !edges
            .iter()
            .all(|n| n.is_finite() && *n >= f64::from(i32::MIN) && *n <= f64::from(i32::MAX))
    {
        return None;
    }
    let [x, y, right, bottom] = edges.map(|n| n as i32);
    Some([x, y, right.checked_sub(x)?, bottom.checked_sub(y)?])
}
unsafe fn bounds(widget: Ptr, gtk: GtkApi) -> Option<(Ptr, [i32; 4])> {
    unsafe {
        let mapped: Test = symbol(b"gtk_widget_get_mapped\0")?;
        let sensitive: Test = symbol(b"gtk_widget_is_sensitive\0")?;
        if mapped(widget) == 0 || sensitive(widget) == 0 || !is_maximize(widget, gtk) {
            return None;
        }
        let (surface, rect) = match gtk {
            GtkApi::Gtk4 => {
                let native: Getter = symbol(b"gtk_widget_get_native\0")?;
                let native = native(widget);
                if native.is_null() {
                    return None;
                }
                let object: glib::Object = from_glib_none(native.cast());
                if !is_type(&object, "GtkWindow") {
                    return None;
                }
                let surface: Getter = symbol(b"gtk_native_get_surface\0")?;
                let surface = surface(native);
                let wl_surface: Getter = symbol(b"gdk_wayland_surface_get_wl_surface\0")?;
                let compute: unsafe extern "C" fn(Ptr, Ptr, *mut FloatRect) -> i32 =
                    symbol(b"gtk_widget_compute_bounds\0")?;
                let transform: unsafe extern "C" fn(Ptr, *mut f64, *mut f64) =
                    symbol(b"gtk_native_get_surface_transform\0")?;
                let mut rect = FloatRect::default();
                if surface.is_null() || compute(widget, native, &mut rect) == 0 {
                    return None;
                }
                let (mut dx, mut dy) = (0., 0.);
                transform(native, &mut dx, &mut dy);
                // GTK documents this as surface -> widget: subtract to go back.
                let rect = enclosing(
                    f64::from(rect.x) - dx,
                    f64::from(rect.y) - dy,
                    rect.width.into(),
                    rect.height.into(),
                )?;
                (wl_surface(surface), rect)
            }
            GtkApi::Gtk3 => {
                let toplevel: Getter = symbol(b"gtk_widget_get_toplevel\0")?;
                let top = toplevel(widget);
                if top.is_null() {
                    return None;
                }
                let object: glib::Object = from_glib_none(top.cast());
                if !is_type(&object, "GtkWindow") {
                    return None;
                }
                let window: Getter = symbol(b"gtk_widget_get_window\0")?;
                let window = window(top);
                if window.is_null() {
                    return None;
                }
                let wl_surface: Getter = symbol(b"gdk_wayland_window_get_wl_surface\0")?;
                let translate: unsafe extern "C" fn(Ptr, Ptr, i32, i32, *mut i32, *mut i32) -> i32 =
                    symbol(b"gtk_widget_translate_coordinates\0")?;
                let allocation: unsafe extern "C" fn(Ptr, *mut Allocation) =
                    symbol(b"gtk_widget_get_allocation\0")?;
                let (mut x, mut y) = (0, 0);
                if translate(widget, top, 0, 0, &mut x, &mut y) == 0 {
                    return None;
                }
                let mut button = Allocation::default();
                let mut root = Allocation::default();
                allocation(widget, &mut button);
                allocation(top, &mut root);
                // translate_coordinates subtracts the toplevel allocation origin;
                // add it back to express the button in its GdkWindow/wl_surface.
                let rect = enclosing(
                    f64::from(x) + f64::from(root.x),
                    f64::from(y) + f64::from(root.y),
                    button.width.into(),
                    button.height.into(),
                )?;
                (wl_surface(window), rect)
            }
        };
        (!surface.is_null()).then_some((surface, rect))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surface_bounds_enclose_fractional_edges_without_buffer_scaling() {
        assert_eq!(enclosing(10.25, 4.75, 30.5, 28.5), Some([10, 4, 31, 30]));
        assert_eq!(enclosing(-2., -3., 40., 32.), Some([-2, -3, 40, 32]));
        assert_eq!(enclosing(f64::NAN, 0., 20., 20.), None);
        assert_eq!(enclosing(0., 0., 0., 20.), None);
        assert_eq!(enclosing(f64::from(i32::MAX), 0., 20., 20.), None);
        assert_eq!(
            enclosing(f64::from(i32::MIN), 0., f64::from(u32::MAX), 20.),
            None
        );
    }
}
