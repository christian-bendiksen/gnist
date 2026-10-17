//! Publishes live changes of the Herda window layout to the host toolkit.
//!
//! `$XDG_CONFIG_HOME/herda/gtk-layout.ini` is the authority. The patched
//! toolkit reads it itself when it creates a `GtkSettings`, so the first frame
//! is right without this module. This module follows later changes: it
//! re-reads the file and sets `GtkSettings:gtk-herda-layout` on every display,
//! and the toolkit re-lays out its windows. A toolkit without the property is
//! left exactly as it is.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gio::prelude::*;
use glib::translate::from_glib_none;

use crate::{config::GtkApi, ox_log, symbols};

pub const PROPERTY: &str = "gtk-herda-layout";
pub const GROUP: &str = "GtkLayout";
pub const SCHEMA: &str = "org.herda.GtkLayout";
pub const SCHEMA_VERSION: i32 = 1;
const DIRECTORY: &str = "herda";
const FILE_NAME: &str = "gtk-layout.ini";
// Coalesces the events of one replacement into a single read.
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

    /// The layout a loaded key file names. Anything but the complete version
    /// 1 group naming a known layout is an error, which callers read as
    /// native.
    pub fn from_key_file(file: &glib::KeyFile) -> Result<Self, String> {
        let schema = file.string(GROUP, "Schema").map_err(|error| error.to_string())?;
        if schema != SCHEMA {
            return Err(format!("schema {schema:?} is not {SCHEMA:?}"));
        }
        let version = file.integer(GROUP, "Version").map_err(|error| error.to_string())?;
        if version != SCHEMA_VERSION {
            return Err(format!("version {version} is not {SCHEMA_VERSION}"));
        }
        let layout = file.string(GROUP, "Layout").map_err(|error| error.to_string())?;
        Self::from_nick(&layout).ok_or_else(|| format!("unknown layout {layout:?}"))
    }

    /// Read the authoritative file with the same GLib loader the toolkit
    /// uses, so the first frame and later changes agree on every input.
    pub fn read(path: &Path) -> Self {
        let file = glib::KeyFile::new();
        if let Err(error) = file.load_from_file(path, glib::KeyFileFlags::NONE) {
            if !error.matches(glib::FileError::Noent) {
                ox_log!("layout file {} unreadable: {error}", path.display());
            }
            return Self::Native;
        }
        Self::from_key_file(&file).unwrap_or_else(|error| {
            ox_log!("layout file {} ignored: {error}", path.display());
            Self::Native
        })
    }
}

/// GLib's user configuration directory, exactly as the toolkit's startup
/// read resolves it. Unlike the CSS path this has no Flatpak exception: both
/// readers must name the same file in every sandbox.
pub fn config_path() -> PathBuf {
    glib::user_config_dir().join(DIRECTORY).join(FILE_NAME)
}

struct Watch {
    directory: PathBuf,
    monitor: gio::FileMonitor,
}

struct DisplayHook {
    manager: glib::Object,
    handler: glib::SignalHandlerId,
}

thread_local! {
    static GTK: Cell<Option<GtkApi>> = const { Cell::new(None) };
    static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
    static DISPLAYS: RefCell<Option<DisplayHook>> = const { RefCell::new(None) };
    static SETTLING: Cell<bool> = const { Cell::new(false) };
}

/// Publish the current layout and follow the file from now on. Must run on
/// GTK's main context after `gtk_init()`.
pub fn attach(gtk: GtkApi) {
    GTK.with(|state| state.set(Some(gtk)));
    watch_displays();
    rearm();
    refresh("attach");
}

pub fn detach() {
    WATCH.with(|state| {
        if let Some(watch) = state.borrow_mut().take() {
            watch.monitor.cancel();
        }
    });
    DISPLAYS.with(|state| {
        if let Some(hook) = state.borrow_mut().take() {
            hook.manager.disconnect(hook.handler);
        }
    });
}

/// Watch the file's directory, or while it does not exist, the nearest
/// ancestor that does, moving down as each level appears. A directory monitor
/// sees Malm's rename over the file, its removal, and its creation.
fn rearm() {
    let path = config_path();
    let Some(target) = path.parent() else { return };
    let directory = nearest_existing(target);
    if WATCH.with(|state| state.borrow().as_ref().is_some_and(|watch| watch.directory == directory)) {
        return;
    }

    let monitor = match gio::File::for_path(&directory)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
    {
        Ok(monitor) => monitor,
        Err(error) => {
            ox_log!("cannot watch {}: {error}", directory.display());
            return;
        }
    };
    monitor.connect_changed(|_monitor, file, other, event| {
        if !concerns_layout(file, other) {
            return;
        }
        ox_log!("layout event {event:?} {}", file.path().unwrap_or_default().display());
        schedule_refresh();
    });
    ox_log!("watching {}", directory.display());

    WATCH.with(|state| {
        if let Some(previous) = state.replace(Some(Watch { directory, monitor })) {
            previous.monitor.cancel();
        }
    });
}

fn nearest_existing(target: &Path) -> PathBuf {
    target
        .ancestors()
        .find(|directory| directory.is_dir())
        .unwrap_or(Path::new("/"))
        .to_path_buf()
}

/// Whether an event can change the layout: the file itself, or any directory
/// on the way to it appearing, moving or going away.
fn concerns_layout(file: &gio::File, other: Option<&gio::File>) -> bool {
    let path = config_path();
    let relevant = |file: &gio::File| file.path().is_some_and(|changed| path.starts_with(&changed));
    relevant(file) || other.is_some_and(relevant)
}

fn schedule_refresh() {
    if SETTLING.with(|settling| settling.replace(true)) {
        return;
    }
    glib::timeout_add_local_once(SETTLE, || {
        SETTLING.with(|settling| settling.set(false));
        rearm();
        refresh("file");
    });
}

/// Re-read the authoritative file and publish it. Cheap and idempotent, so any
/// hint that something changed may call it.
pub fn refresh(reason: &str) {
    let Some(gtk) = GTK.with(Cell::get) else { return };
    let layout = Layout::read(&config_path());
    let mut changed = 0;
    let mut unchanged = 0;
    let mut unsupported = None;
    for settings in all_settings(gtk) {
        match set_layout(&settings, layout) {
            Publish::Changed => changed += 1,
            Publish::Unchanged => unchanged += 1,
            Publish::Unsupported(why) => unsupported = Some(why),
        }
    }
    let layout = layout.nick();
    match unsupported {
        Some(why) if changed + unchanged == 0 => ox_log!("layout={layout} not published ({reason}): {why}"),
        _ if changed == 0 => ox_log!("layout={layout} unchanged ({reason})"),
        _ => ox_log!("layout={layout} ({reason}) on {changed} display(s)"),
    }
}

fn display_manager() -> Option<glib::Object> {
    let symbols = symbols::core()?;
    if !symbols::gtk_initialized() {
        return None;
    }
    let manager = unsafe { symbols.gdk_display_manager_get?() };
    (!manager.is_null()).then(|| unsafe { from_glib_none(manager.cast::<glib::gobject_ffi::GObject>()) })
}

/// Publish to displays opened after attach, such as GTK 3's first display.
fn watch_displays() {
    if DISPLAYS.with(|state| state.borrow().is_some()) {
        return;
    }
    let Some(manager) = display_manager() else { return };
    let handler = manager.connect_local("display-opened", true, |_| {
        refresh("display-opened");
        None
    });
    DISPLAYS.with(|state| state.replace(Some(DisplayHook { manager, handler })));
}

fn all_settings(gtk: GtkApi) -> Vec<glib::Object> {
    let Some(symbols) = symbols::core() else { return Vec::new() };
    let Some(manager) = display_manager() else { return Vec::new() };
    let Some(list_displays) = symbols.gdk_display_manager_list_displays else { return Vec::new() };

    let mut settings = Vec::new();
    unsafe {
        let head = list_displays(manager.as_ptr().cast());
        let mut node = head;
        while let Some(entry) = node.as_ref() {
            if let Some(object) = settings_for_display(symbols, gtk, entry.data.cast()) {
                settings.push(object);
            }
            node = entry.next;
        }
        glib::ffi::g_slist_free(head);
    }
    settings
}

fn settings_for_display(symbols: &symbols::CoreSyms, gtk: GtkApi, display: *mut c_void) -> Option<glib::Object> {
    if display.is_null() {
        return None;
    }
    let settings = unsafe {
        match gtk {
            GtkApi::Gtk4 => symbols.gtk_settings_get_for_display?(display),
            GtkApi::Gtk3 => {
                let screen = symbols.gdk_display_get_default_screen?(display);
                if screen.is_null() {
                    return None;
                }
                symbols.gtk_settings_get_for_screen?(screen)
            }
        }
    };
    (!settings.is_null()).then(|| unsafe { from_glib_none(settings.cast::<glib::gobject_ffi::GObject>()) })
}

#[derive(Debug, Eq, PartialEq)]
enum Publish {
    Changed,
    Unchanged,
    Unsupported(&'static str),
}

fn set_layout(settings: &glib::Object, layout: Layout) -> Publish {
    let Some(pspec) = settings.find_property(PROPERTY) else {
        return Publish::Unsupported("toolkit has no gtk-herda-layout");
    };
    if !pspec.flags().contains(glib::ParamFlags::READWRITE) {
        return Publish::Unsupported("gtk-herda-layout is not read-write");
    }
    if pspec.value_type() != glib::Type::STRING {
        return Publish::Unsupported("gtk-herda-layout is not a string");
    }

    let current = settings.property::<Option<String>>(PROPERTY);
    if current.as_deref() == Some(layout.nick()) {
        return Publish::Unchanged;
    }
    settings.set_property(PROPERTY, layout.nick());
    Publish::Changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Layout, String> {
        let file = glib::KeyFile::new();
        file.load_from_data(text, glib::KeyFileFlags::NONE).map_err(|error| error.to_string())?;
        Layout::from_key_file(&file)
    }

    fn doc(layout: &str) -> String {
        format!("[GtkLayout]\nSchema=org.herda.GtkLayout\nVersion=1\nLayout={layout}\n")
    }

    #[test]
    fn parses_known_layouts() {
        assert_eq!(parse(&doc("native")), Ok(Layout::Native));
        assert_eq!(parse(&doc("herda-desktop")), Ok(Layout::HerdaDesktop));
        let extra = "# Generated\n[GtkLayout]\nSchema=org.herda.GtkLayout\nVersion=1\nLayout=herda-desktop\nNote=1\n[Other]\nA=b\n";
        assert_eq!(parse(extra), Ok(Layout::HerdaDesktop));
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            String::new(),
            "[GtkLayout".into(),
            "Layout=herda-desktop\n".into(),
            "[GtkLayout]\nVersion=1\nLayout=herda-desktop\n".into(),
            "[GtkLayout]\nSchema=other\nVersion=1\nLayout=herda-desktop\n".into(),
            "[GtkLayout]\nSchema=org.herda.GtkLayout\nLayout=herda-desktop\n".into(),
            "[GtkLayout]\nSchema=org.herda.GtkLayout\nVersion=2\nLayout=herda-desktop\n".into(),
            "[GtkLayout]\nSchema=org.herda.GtkLayout\nVersion=one\nLayout=herda-desktop\n".into(),
            "[GtkLayout]\nSchema=org.herda.GtkLayout\nVersion=1\n".into(),
            "[Other]\nSchema=org.herda.GtkLayout\nVersion=1\nLayout=herda-desktop\n".into(),
            doc("Herda-Desktop"),
            doc("windows"),
            doc(""),
            "{\"schema\":\"org.herda.GtkLayout\",\"version\":1,\"layout\":\"herda-desktop\"}".into(),
        ] {
            assert!(parse(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn missing_and_malformed_files_are_native() {
        let directory = std::env::temp_dir().join(format!("gnist-layout-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(FILE_NAME);

        assert_eq!(Layout::read(&path), Layout::Native);
        std::fs::write(&path, b"[GtkLayout\n").unwrap();
        assert_eq!(Layout::read(&path), Layout::Native);
        std::fs::write(&path, b"\xff\xfe[GtkLayout]\n").unwrap();
        assert_eq!(Layout::read(&path), Layout::Native);
        std::fs::write(&path, doc("herda-desktop")).unwrap();
        assert_eq!(Layout::read(&path), Layout::HerdaDesktop);
        assert_eq!(Layout::read(&directory), Layout::Native);

        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The fixture cases shared with the patched toolkit's reader. An explicit
    /// `GTK_LAYOUT_FIXTURES` must hold them; Herda's tree beside this
    /// repository is only used when present.
    #[test]
    fn shared_fixtures() {
        let manifest_in = |directory: &Path| std::fs::read_to_string(directory.join("expected.tsv"));
        let (directory, manifest) = match std::env::var_os("GTK_LAYOUT_FIXTURES") {
            Some(directory) => {
                let directory = PathBuf::from(directory);
                let manifest = manifest_in(&directory)
                    .unwrap_or_else(|error| panic!("GTK_LAYOUT_FIXTURES={}: {error}", directory.display()));
                (directory, manifest)
            }
            None => {
                let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../herda/distro/patches/gtk-layout-fixtures");
                let Ok(manifest) = manifest_in(&directory) else {
                    eprintln!("no shared fixtures at {}; set GTK_LAYOUT_FIXTURES to require them", directory.display());
                    return;
                };
                (directory, manifest)
            }
        };
        // Every case listed exists, and every file present is a listed case.
        let mut listed = std::collections::BTreeSet::new();
        for line in manifest.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
            let (name, expected) = line.split_once(char::is_whitespace).expect("NAME EXPECTED");
            let expected = Layout::from_nick(expected.trim()).expect("expected layout");
            let path = directory.join(name);
            assert!(path.is_file(), "listed fixture {name} is missing");
            assert_eq!(Layout::read(&path), expected, "{name}");
            assert!(listed.insert(name.to_owned()), "{name} is listed twice");
        }
        let present: std::collections::BTreeSet<String> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name != "expected.tsv")
            .collect();
        assert_eq!(present, listed, "fixtures present and listed differ");
        assert!(!listed.is_empty());
    }

    // Stand-in for a patched GtkSettings.
    mod fake {
        use std::cell::RefCell;
        use std::sync::OnceLock;

        use glib::prelude::*;
        use glib::subclass::prelude::*;

        #[derive(Default)]
        pub struct Settings {
            pub layout: RefCell<Option<String>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for Settings {
            const NAME: &'static str = "GnistTestSettings";
            type Type = super::SettingsObject;
        }

        impl ObjectImpl for Settings {
            fn properties() -> &'static [glib::ParamSpec] {
                static PROPS: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
                PROPS.get_or_init(|| {
                    vec![glib::ParamSpecString::builder("gtk-herda-layout").default_value(Some("native")).build()]
                })
            }

            fn set_property(&self, _id: usize, value: &glib::Value, _pspec: &glib::ParamSpec) {
                self.layout.replace(value.get().unwrap());
            }

            fn property(&self, _id: usize, _pspec: &glib::ParamSpec) -> glib::Value {
                self.layout.borrow().to_value()
            }

            fn constructed(&self) {
                self.parent_constructed();
                self.layout.replace(Some("native".into()));
            }
        }
    }

    glib::wrapper! {
        pub struct SettingsObject(ObjectSubclass<fake::Settings>);
    }

    #[test]
    fn property_is_set_only_on_change() {
        let settings: glib::Object = glib::Object::new::<SettingsObject>().upcast();
        let notified = std::rc::Rc::new(Cell::new(0));
        let seen = notified.clone();
        settings.connect_notify_local(Some(PROPERTY), move |_, _| seen.set(seen.get() + 1));

        assert_eq!(set_layout(&settings, Layout::Native), Publish::Unchanged);
        assert_eq!(set_layout(&settings, Layout::HerdaDesktop), Publish::Changed);
        assert_eq!(settings.property::<String>(PROPERTY), "herda-desktop");
        assert_eq!(set_layout(&settings, Layout::HerdaDesktop), Publish::Unchanged);
        assert_eq!(set_layout(&settings, Layout::Native), Publish::Changed);
        assert_eq!(notified.get(), 2);

        // Any other value the toolkit holds is replaced.
        settings.set_property(PROPERTY, "something-else");
        assert_eq!(set_layout(&settings, Layout::Native), Publish::Changed);
    }

    #[test]
    fn unpatched_toolkit_is_left_alone() {
        let settings = glib::Object::new::<glib::Object>();
        assert!(matches!(set_layout(&settings, Layout::HerdaDesktop), Publish::Unsupported(_)));
    }

    #[test]
    fn layout_path_is_glibs_user_config_dir() {
        // The toolkit reads g_get_user_config_dir()/herda/gtk-layout.ini.
        let expected = unsafe {
            std::ffi::CStr::from_ptr(glib::ffi::g_get_user_config_dir()).to_str().unwrap().to_owned()
        };
        assert_eq!(config_path(), Path::new(&expected).join("herda/gtk-layout.ini"));
    }

    #[test]
    fn nearest_existing_ancestor() {
        let base = std::env::temp_dir().join(format!("gnist-ancestor-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(nearest_existing(&base.join("config/herda")), base);
        std::fs::create_dir_all(base.join("config")).unwrap();
        assert_eq!(nearest_existing(&base.join("config/herda")), base.join("config"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
