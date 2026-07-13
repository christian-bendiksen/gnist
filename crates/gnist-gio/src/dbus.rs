use std::cell::RefCell;

use crate::{config::Mode, ox_log, style};

struct Subscription {
    connection: gio::DBusConnection,
    id: gio::SignalSubscriptionId,
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
            let id = connection.signal_subscribe(
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

            SUBSCRIPTION.with(|state| {
                state.replace(Some(Subscription { connection, id }));
            });
            ox_log!("subscribed to D-Bus signal");
        }
        Err(error) => ox_log!("D-Bus connect failed: {error}"),
    });
}

pub fn unsubscribe() {
    SUBSCRIPTION.with(|state| {
        if let Some(subscription) = state.borrow_mut().take() {
            subscription.connection.signal_unsubscribe(subscription.id);
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
}
