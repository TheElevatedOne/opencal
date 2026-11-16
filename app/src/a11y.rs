//! Keep GTK's accessibility layer quiet when the session's AT-SPI registry is broken.
//!
//! When the accessibility bus is running but its registry can't be started (for example after
//! the bus was restarted while an old registry kept its systemd unit), GTK 4 prints
//! "Unable to register the application … org.a11y.atspi.Registry" for every window and popup.
//! Before GTK starts, try to start the registry the way GTK would; if that fails, set
//! `GTK_A11Y=none` for this process. Screen readers can't work in that state anyway.

use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::prelude::ToVariant;

const TIMEOUT_MS: i32 = 1500;

/// True when GTK's accessibility would fail with the "Unable to register" spam.
fn registry_broken() -> bool {
    let Ok(session) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    // No accessibility bus at all: GTK skips accessibility quietly.
    let Ok(reply) = session.call_sync(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        "org.a11y.Bus",
        "GetAddress",
        None,
        Some(glib::VariantTy::new("(s)").unwrap()),
        gio::DBusCallFlags::NO_AUTO_START,
        TIMEOUT_MS,
        gio::Cancellable::NONE,
    ) else {
        return false;
    };
    let Some((address,)) = reply.get::<(String,)>() else {
        return false;
    };
    let Ok(bus) = gio::DBusConnection::for_address_sync(
        &address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    ) else {
        return false;
    };
    let started = bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "StartServiceByName",
        Some(&("org.a11y.atspi.Registry", 0u32).to_variant()),
        Some(glib::VariantTy::new("(u)").unwrap()),
        gio::DBusCallFlags::NONE,
        TIMEOUT_MS,
        gio::Cancellable::NONE,
    );
    let _ = bus.close_sync(gio::Cancellable::NONE);
    started.is_err()
}

/// Call before GTK is initialised, while the process is still single-threaded.
pub fn quiet_if_broken() {
    if std::env::var_os("GTK_A11Y").is_some() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    // A D-Bus hang must not delay the start more than a moment.
    let worker = std::thread::spawn(move || {
        let _ = tx.send(registry_broken());
    });
    if rx
        .recv_timeout(Duration::from_millis(2000))
        .unwrap_or(false)
    {
        let _ = worker.join();
        // SAFETY: called at start-up before GTK starts; the only other thread has been joined.
        unsafe { std::env::set_var("GTK_A11Y", "none") };
    }
}
