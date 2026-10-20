//! Minimal private protocol binding on GDK's connection. GTK alone dispatches
//! that connection; no second connection, roundtrip, socket reader or dependency.
use crate::symbols::symbol;
use std::{
    cell::Cell,
    ffi::{CStr, c_char, c_void},
    ptr,
    rc::Rc,
};

type Proxy = *mut c_void;
#[repr(C)]
struct Interface {
    name: *const c_char,
    version: i32,
    method_count: i32,
    methods: *const Message,
    event_count: i32,
    events: *const Message,
}
#[repr(C)]
struct Message {
    name: *const c_char,
    signature: *const c_char,
    types: *const *const Interface,
}
#[repr(C)]
union Argument {
    i: i32,
    u: u32,
    s: *const c_char,
    o: Proxy,
}
type Marshal = unsafe extern "C" fn(Proxy, u32, *const Interface, u32, u32, *mut Argument) -> Proxy;
type Listen = unsafe extern "C" fn(Proxy, *const c_void, *mut c_void) -> i32;
type Destroy = unsafe extern "C" fn(Proxy);
type Flush = unsafe extern "C" fn(Proxy) -> i32;

pub struct Connection {
    display: Proxy,
    registry: Cell<Proxy>,
    proxy: Cell<Proxy>,
    name: Cell<u32>,
    marshal: Marshal,
    destroy: Destroy,
    flush: Flush,
    // Heap addresses stay stable until all proxies using the metadata die.
    interface: Box<Interface>,
    _messages: Box<[Message; 2]>,
    _types: Box<[*const Interface; 5]>,
}
impl Connection {
    /// `display` is a live wl_display owned and dispatched by GDK. Caller must
    /// close this binding on GdkDisplay::closed, before GDK frees the connection.
    pub unsafe fn new(display: Proxy) -> Option<Rc<Self>> {
        if display.is_null() {
            return None;
        }
        let marshal: Marshal = unsafe { symbol(b"wl_proxy_marshal_array_flags\0")? };
        let listen: Listen = unsafe { symbol(b"wl_proxy_add_listener\0")? };
        let destroy = unsafe { symbol(b"wl_proxy_destroy\0")? };
        let flush = unsafe { symbol(b"wl_display_flush\0")? };
        let registry_interface: *const Interface = unsafe { symbol(b"wl_registry_interface\0")? };
        let surface_interface: *const Interface = unsafe { symbol(b"wl_surface_interface\0")? };
        let types = Box::new([
            surface_interface,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
        ]);
        let messages = Box::new([
            Message {
                name: c"show_snap_layouts".as_ptr(),
                signature: c"oiiii".as_ptr(),
                types: types.as_ptr(),
            },
            Message {
                name: c"dismiss".as_ptr(),
                signature: c"".as_ptr(),
                types: ptr::null(),
            },
        ]);
        let interface = Box::new(Interface {
            name: c"herda_snap_layouts_v1".as_ptr(),
            version: 1,
            method_count: 2,
            methods: messages.as_ptr(),
            event_count: 0,
            events: ptr::null(),
        });
        let registry = unsafe {
            marshal(
                display,
                1,
                registry_interface,
                1,
                0,
                [Argument { o: ptr::null_mut() }].as_mut_ptr(),
            )
        };
        if registry.is_null() {
            return None;
        }
        let connection = Rc::new(Self {
            display,
            registry: Cell::new(registry),
            proxy: Cell::new(ptr::null_mut()),
            name: Cell::new(0),
            marshal,
            destroy,
            flush,
            interface,
            _messages: messages,
            _types: types,
        });
        // Connection is retained by the GTK attachment until close destroys the
        // registry; Wayland then discards any queued callbacks for this proxy.
        if unsafe {
            listen(
                registry,
                (&LISTENER as *const Listener).cast(),
                Rc::as_ptr(&connection).cast_mut().cast(),
            )
        } != 0
        {
            return None;
        }
        unsafe { flush(display) };
        crate::ox_log!("snap layouts registry requested on GDK connection");
        Some(connection)
    }
    pub fn available(&self) -> bool {
        !self.proxy.get().is_null()
    }
    pub fn show(&self, surface: Proxy, rect: [i32; 4]) {
        if !self.available() || surface.is_null() {
            return;
        }
        crate::ox_log!("snap layouts show rect={rect:?}");
        let [x, y, width, height] = rect;
        let mut args = [
            Argument { o: surface },
            Argument { i: x },
            Argument { i: y },
            Argument { i: width },
            Argument { i: height },
        ];
        unsafe {
            (self.marshal)(self.proxy.get(), 0, ptr::null(), 1, 0, args.as_mut_ptr());
            (self.flush)(self.display);
        }
    }
    pub fn dismiss(&self) {
        if !self.available() {
            return;
        }
        unsafe {
            (self.marshal)(self.proxy.get(), 1, ptr::null(), 1, 0, ptr::null_mut());
            (self.flush)(self.display);
        }
    }
    pub fn close(&self) {
        for cell in [&self.proxy, &self.registry] {
            let proxy = cell.replace(ptr::null_mut());
            if !proxy.is_null() {
                unsafe { (self.destroy)(proxy) };
            }
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
    }
}
#[repr(C)]
struct Listener {
    global: unsafe extern "C" fn(*mut c_void, Proxy, u32, *const c_char, u32),
    remove: unsafe extern "C" fn(*mut c_void, Proxy, u32),
}
static LISTENER: Listener = Listener { global, remove };
unsafe extern "C" fn global(
    data: *mut c_void,
    registry: Proxy,
    name: u32,
    interface: *const c_char,
    version: u32,
) {
    let connection = unsafe { &*data.cast::<Connection>() };
    if connection.available()
        || version == 0
        || unsafe { CStr::from_ptr(interface) } != c"herda_snap_layouts_v1"
    {
        return;
    }
    let mut args = [
        Argument { u: name },
        Argument {
            s: c"herda_snap_layouts_v1".as_ptr(),
        },
        Argument { u: 1 },
        Argument { o: ptr::null_mut() },
    ];
    let proxy = unsafe {
        (connection.marshal)(registry, 0, &*connection.interface, 1, 0, args.as_mut_ptr())
    };
    connection.proxy.set(proxy);
    connection.name.set(name);
    if connection.available() {
        crate::ox_log!("snap layouts global bound version=1");
    }
    crate::snap_layouts::queue_scan();
}
unsafe extern "C" fn remove(data: *mut c_void, _: Proxy, name: u32) {
    let connection = unsafe { &*data.cast::<Connection>() };
    if connection.name.get() == name {
        let proxy = connection.proxy.replace(ptr::null_mut());
        if !proxy.is_null() {
            unsafe { (connection.destroy)(proxy) };
        }
    }
}
