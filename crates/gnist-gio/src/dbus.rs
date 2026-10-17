use std::cell::RefCell;

use crate::{config::Mode, layout, ox_log, style};

struct Subscription {
    connection: gio::DBusConnection,
    ids: Vec<gio::SignalSubscriptionId>,
}

thread_local! {
    static SUBSCRIPTION: RefCell<Option<Subscription>> = const { RefCell::new(None) };
}

pub fn subscribe() {
    if SUBSCRIPTION.with(|state| state.borrow().is_some()) {
        return;
    }

    gio::bus_get(gio::BusType::Session, gio::Cancellable::NONE, |result| match result {
        Ok(connection) => {
            let appearance = connection.signal_subscribe(
                None::<&str>,
                Some("org.gnist.Appearance1"),
                Some("Changed"),
                Some("/org/gnist/Appearance1"),
                None::<&str>,
                gio::DBusSignalFlags::NONE,
                |_connection, _sender, _path, _interface, _signal, parameters| {
                    handle_changed(parameters);
                },
            );
            // The layout signal only says "look again": the file stays the
            // authority, so a missed or reordered signal still converges.
            let layout = connection.signal_subscribe(
                None::<&str>,
                Some("org.gnist.Layout1"),
                Some("Changed"),
                Some("/org/gnist/Layout1"),
                None::<&str>,
                gio::DBusSignalFlags::NONE,
                |_connection, _sender, _path, _interface, _signal, _parameters| {
                    layout::refresh("signal");
                },
            );

            SUBSCRIPTION.with(|state| {
                state.replace(Some(Subscription { connection, ids: vec![appearance, layout] }));
            });
            ox_log!("subscribed to D-Bus signals");
        }
        Err(error) => ox_log!("D-Bus connect failed: {error}"),
    });
}

pub fn unsubscribe() {
    SUBSCRIPTION.with(|state| {
        if let Some(subscription) = state.borrow_mut().take() {
            for id in subscription.ids {
                subscription.connection.signal_unsubscribe(id);
            }
        }
    });
}

fn handle_changed(parameters: &glib::Variant) {
    let Some((revision, css_path, mode)) = parameters.get::<(u64, String, String)>() else {
        ox_log!("ignored malformed Appearance1.Changed signal");
        return;
    };

    let mode = Mode::parse(&mode);
    ox_log!("signal revision={revision} path={css_path} mode={mode}");

    style::update_and_reload(&css_path, mode);
    // A theme change is also a chance to recover a layout change whose file
    // event or signal this process missed.
    layout::refresh("appearance");
}
