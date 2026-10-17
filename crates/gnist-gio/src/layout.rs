//! Publishes the Herda window layout to the host toolkit.
//!
//! `~/.config/herda/gtk-layout.json` is the authority. This module only reads
//! it and sets the toolkit's own `GtkSettings:gtk-herda-layout`; the toolkit
//! builds and re-lays out its windows from that setting. A toolkit without the
//! property is left exactly as it is.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gio::prelude::*;
use glib::translate::from_glib_none;

use crate::{config, ox_log, symbols};

pub const PROPERTY: &str = "gtk-herda-layout";
pub const SCHEMA: &str = "herda.gtk-layout";
pub const SCHEMA_VERSION: u64 = 1;
const FILE_NAME: &str = "gtk-layout.json";
// Large enough for any valid file; a bigger one is not ours to parse.
const MAX_FILE_BYTES: u64 = 64 * 1024;
// Coalesces the events of one atomic replacement into a single read.
const SETTLE: Duration = Duration::from_millis(40);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Layout {
    #[default]
    Native,
    HerdaDesktop,
}

impl Layout {
    pub fn nick(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::HerdaDesktop => "herda-desktop",
        }
    }

    pub fn from_nick(value: &str) -> Option<Self> {
        match value {
            "native" => Some(Self::Native),
            "herda-desktop" => Some(Self::HerdaDesktop),
            _ => None,
        }
    }

    /// Parse the authoritative file. Anything but a complete version 1
    /// document naming a known layout is the native layout.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|error| format!("invalid JSON: {error}"))?;
        let object = value.as_object().ok_or("not a JSON object")?;
        match object.get("schema").and_then(|schema| schema.as_str()) {
            Some(SCHEMA) => {}
            other => return Err(format!("schema {other:?} is not {SCHEMA:?}")),
        }
        match object.get("version").and_then(|version| version.as_u64()) {
            Some(SCHEMA_VERSION) => {}
            other => return Err(format!("version {other:?} is not {SCHEMA_VERSION}")),
        }
        let layout = object
            .get("layout")
            .and_then(|layout| layout.as_str())
            .ok_or("layout is not a string")?;
        Self::from_nick(layout).ok_or_else(|| format!("unknown layout {layout:?}"))
    }

    pub fn read(path: &Path) -> Self {
        match read_bounded(path) {
            Ok(Some(bytes)) => Self::parse(&bytes).unwrap_or_else(|error| {
                ox_log!("layout file {} ignored: {error}", path.display());
                Self::Native
            }),
            Ok(None) => Self::Native,
            Err(error) => {
                ox_log!("layout file {} unreadable: {error}", path.display());
                Self::Native
            }
        }
    }
}

fn read_bounded(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;

    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(std::io::Error::other("larger than 64 KiB"));
    }
    Ok(Some(bytes))
}

pub fn config_path() -> PathBuf {
    config::config_home().join("herda").join(FILE_NAME)
}

thread_local! {
    static MONITOR: RefCell<Option<gio::FileMonitor>> = const { RefCell::new(None) };
    static SETTLING: Cell<bool> = const { Cell::new(false) };
}

/// Publish the current layout and follow the file from now on. Must run on
/// GTK's main context after `gtk_init()`.
pub fn attach() {
    refresh("attach");
    if MONITOR.with(|monitor| monitor.borrow().is_some()) {
        return;
    }

    // Watch the directory, not the file: Malm replaces the file by renaming
    // over it, which a monitor on the old inode would never report. GLib also
    // keeps watching a directory that does not exist yet.
    let path = config_path();
    let Some(directory) = path.parent() else { return };
    match gio::File::for_path(directory)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
    {
        Ok(monitor) => {
            monitor.connect_changed(|_monitor, file, other, event| {
                if !concerns_layout(file, other) {
                    return;
                }
                ox_log!("layout file event {event:?}");
                schedule_refresh();
            });
            MONITOR.with(|state| state.replace(Some(monitor)));
            ox_log!("watching {}", directory.display());
        }
        Err(error) => ox_log!("cannot watch {}: {error}", directory.display()),
    }
}

pub fn detach() {
    MONITOR.with(|state| {
        if let Some(monitor) = state.borrow_mut().take() {
            monitor.cancel();
        }
    });
}

fn concerns_layout(file: &gio::File, other: Option<&gio::File>) -> bool {
    let named = |file: &gio::File| file.basename().is_some_and(|name| name == Path::new(FILE_NAME));
    // A replaced parent symlink reports the directory itself.
    named(file) || other.is_some_and(named) || file.path().as_deref() == config_path().parent()
}

fn schedule_refresh() {
    if SETTLING.with(|settling| settling.replace(true)) {
        return;
    }
    glib::timeout_add_local_once(SETTLE, || {
        SETTLING.with(|settling| settling.set(false));
        refresh("file");
    });
}

/// Re-read the authoritative file and publish it. Cheap and idempotent, so any
/// hint that something changed may call it.
pub fn refresh(reason: &str) {
    let path = config_path();
    let layout = Layout::read(&path);
    match publish(layout) {
        Publish::Changed => ox_log!("layout={} ({reason}) from {}", layout.nick(), path.display()),
        Publish::Unchanged => ox_log!("layout={} unchanged ({reason})", layout.nick()),
        Publish::Unsupported(why) => ox_log!("layout={} not published ({reason}): {why}", layout.nick()),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Publish {
    Changed,
    Unchanged,
    Unsupported(&'static str),
}

fn publish(layout: Layout) -> Publish {
    let Some(get_settings) = symbols::core().and_then(|symbols| symbols.gtk_settings_get_default)
    else {
        return Publish::Unsupported("no gtk_settings_get_default");
    };
    if !symbols::gtk_initialized() {
        return Publish::Unsupported("GTK not initialized");
    }
    let raw = unsafe { get_settings() };
    if raw.is_null() {
        return Publish::Unsupported("no default GtkSettings");
    }
    let settings: glib::Object = unsafe { from_glib_none(raw.cast::<glib::gobject_ffi::GObject>()) };
    set_layout(&settings, layout)
}

fn set_layout(settings: &glib::Object, layout: Layout) -> Publish {
    let Some(pspec) = settings.find_property(PROPERTY) else {
        return Publish::Unsupported("toolkit has no gtk-herda-layout");
    };
    if !pspec.flags().contains(glib::ParamFlags::WRITABLE) {
        return Publish::Unsupported("gtk-herda-layout is read-only");
    }

    let value_type = pspec.value_type();
    let value = if value_type == glib::Type::STRING {
        layout.nick().to_value()
    } else if value_type.is_a(glib::Type::ENUM) {
        let Some(class) = glib::EnumClass::with_type(value_type) else {
            return Publish::Unsupported("gtk-herda-layout enum class unavailable");
        };
        match class.to_value_by_nick(layout.nick()) {
            Some(value) => value,
            None => return Publish::Unsupported("gtk-herda-layout enum lacks this layout"),
        }
    } else {
        return Publish::Unsupported("gtk-herda-layout has an unexpected type");
    };

    let current = settings.property_value(PROPERTY);
    if values_equal(&current, &value) {
        return Publish::Unchanged;
    }
    settings.set_property_from_value(PROPERTY, &value);
    Publish::Changed
}

fn values_equal(left: &glib::Value, right: &glib::Value) -> bool {
    if left.type_() != right.type_() {
        return false;
    }
    if let (Ok(left), Ok(right)) = (left.get::<Option<String>>(), right.get::<Option<String>>()) {
        return left == right;
    }
    let number = |value| glib::EnumValue::from_value(value).map(|(_, value)| value.value());
    number(left).is_some() && number(left) == number(right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glib::subclass::prelude::*;

    #[test]
    fn parses_known_layouts() {
        let doc = |layout: &str| {
            format!(r#"{{"schema":"herda.gtk-layout","version":1,"layout":"{layout}"}}"#)
        };
        assert_eq!(Layout::parse(doc("native").as_bytes()), Ok(Layout::Native));
        assert_eq!(Layout::parse(doc("herda-desktop").as_bytes()), Ok(Layout::HerdaDesktop));
        let extra = br#"{"schema":"herda.gtk-layout","version":1,"layout":"herda-desktop","note":1}"#;
        assert_eq!(Layout::parse(extra), Ok(Layout::HerdaDesktop));
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            &b""[..],
            b"{",
            b"[]",
            b"\"herda-desktop\"",
            br#"{"version":1,"layout":"herda-desktop"}"#,
            br#"{"schema":"other","version":1,"layout":"herda-desktop"}"#,
            br#"{"schema":"herda.gtk-layout","layout":"herda-desktop"}"#,
            br#"{"schema":"herda.gtk-layout","version":2,"layout":"herda-desktop"}"#,
            br#"{"schema":"herda.gtk-layout","version":"1","layout":"herda-desktop"}"#,
            br#"{"schema":"herda.gtk-layout","version":1}"#,
            br#"{"schema":"herda.gtk-layout","version":1,"layout":"Herda-Desktop"}"#,
            br#"{"schema":"herda.gtk-layout","version":1,"layout":true}"#,
        ] {
            assert!(Layout::parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn missing_malformed_and_oversized_files_are_native() {
        let directory = std::env::temp_dir().join(format!("gnist-layout-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(FILE_NAME);

        assert_eq!(Layout::read(&path), Layout::Native);
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(Layout::read(&path), Layout::Native);
        let mut huge = br#"{"schema":"herda.gtk-layout","version":1,"layout":"herda-desktop","pad":""#.to_vec();
        huge.extend(std::iter::repeat_n(b'x', MAX_FILE_BYTES as usize));
        huge.extend(b"\"}");
        std::fs::write(&path, huge).unwrap();
        assert_eq!(Layout::read(&path), Layout::Native);
        std::fs::write(&path, br#"{"schema":"herda.gtk-layout","version":1,"layout":"herda-desktop"}"#).unwrap();
        assert_eq!(Layout::read(&path), Layout::HerdaDesktop);

        std::fs::remove_dir_all(&directory).unwrap();
    }

    // Stand-ins for a patched and an unpatched GtkSettings.
    mod fake {
        use std::cell::RefCell;
        use std::sync::OnceLock;

        use glib::prelude::*;
        use glib::subclass::prelude::*;

        #[derive(Default)]
        pub struct StringSettings {
            pub layout: RefCell<String>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for StringSettings {
            const NAME: &'static str = "GnistTestStringSettings";
            type Type = super::StringSettingsObject;
        }

        impl ObjectImpl for StringSettings {
            fn properties() -> &'static [glib::ParamSpec] {
                static PROPS: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
                PROPS.get_or_init(|| {
                    vec![glib::ParamSpecString::builder("gtk-herda-layout").default_value(Some("native")).build()]
                })
            }

            fn set_property(&self, _id: usize, value: &glib::Value, _pspec: &glib::ParamSpec) {
                self.layout.replace(value.get::<Option<String>>().unwrap().unwrap_or_default());
            }

            fn property(&self, _id: usize, _pspec: &glib::ParamSpec) -> glib::Value {
                self.layout.borrow().to_value()
            }

            fn constructed(&self) {
                self.parent_constructed();
                self.layout.replace("native".into());
            }
        }

        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, glib::Enum)]
        #[enum_type(name = "GnistTestLayout")]
        pub enum TestLayout {
            #[default]
            #[enum_value(nick = "native")]
            Native,
            #[enum_value(nick = "herda-desktop")]
            HerdaDesktop,
        }

        #[derive(Default)]
        pub struct EnumSettings {
            pub layout: RefCell<TestLayout>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for EnumSettings {
            const NAME: &'static str = "GnistTestEnumSettings";
            type Type = super::EnumSettingsObject;
        }

        impl ObjectImpl for EnumSettings {
            fn properties() -> &'static [glib::ParamSpec] {
                static PROPS: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
                PROPS.get_or_init(|| {
                    vec![glib::ParamSpecEnum::builder::<TestLayout>("gtk-herda-layout").build()]
                })
            }

            fn set_property(&self, _id: usize, value: &glib::Value, _pspec: &glib::ParamSpec) {
                self.layout.replace(value.get().unwrap());
            }

            fn property(&self, _id: usize, _pspec: &glib::ParamSpec) -> glib::Value {
                self.layout.borrow().to_value()
            }
        }
    }

    glib::wrapper! {
        pub struct StringSettingsObject(ObjectSubclass<fake::StringSettings>);
    }

    glib::wrapper! {
        pub struct EnumSettingsObject(ObjectSubclass<fake::EnumSettings>);
    }

    fn notifications(object: &glib::Object) -> std::rc::Rc<Cell<u32>> {
        let count = std::rc::Rc::new(Cell::new(0));
        let seen = count.clone();
        object.connect_notify_local(Some(PROPERTY), move |_, _| seen.set(seen.get() + 1));
        count
    }

    #[test]
    fn string_property_is_set_only_on_change() {
        let settings: glib::Object = glib::Object::new::<StringSettingsObject>().upcast();
        let notified = notifications(&settings);

        assert_eq!(set_layout(&settings, Layout::Native), Publish::Unchanged);
        assert_eq!(notified.get(), 0);
        assert_eq!(set_layout(&settings, Layout::HerdaDesktop), Publish::Changed);
        assert_eq!(settings.property::<String>(PROPERTY), "herda-desktop");
        assert_eq!(set_layout(&settings, Layout::HerdaDesktop), Publish::Unchanged);
        assert_eq!(set_layout(&settings, Layout::Native), Publish::Changed);
        assert_eq!(settings.property::<String>(PROPERTY), "native");
        assert_eq!(notified.get(), 2);
    }

    #[test]
    fn enum_property_uses_nicks() {
        let object = glib::Object::new::<EnumSettingsObject>();
        let settings: glib::Object = object.clone().upcast();
        let notified = notifications(&settings);

        assert_eq!(set_layout(&settings, Layout::Native), Publish::Unchanged);
        assert_eq!(set_layout(&settings, Layout::HerdaDesktop), Publish::Changed);
        assert_eq!(*object.imp().layout.borrow(), fake::TestLayout::HerdaDesktop);
        assert_eq!(set_layout(&settings, Layout::HerdaDesktop), Publish::Unchanged);
        assert_eq!(notified.get(), 1);
    }

    #[test]
    fn unpatched_toolkit_is_left_alone() {
        let settings = glib::Object::new::<glib::Object>();
        assert!(matches!(set_layout(&settings, Layout::HerdaDesktop), Publish::Unsupported(_)));
    }
}
