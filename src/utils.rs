use std::path::PathBuf;

use gtk::glib;
use gtk::prelude::*;

use crate::APP_ID;

// ANCHOR: data_path
pub fn data_path() -> PathBuf {
    let mut path = glib::user_data_dir();
    path.push(APP_ID);
    std::fs::create_dir_all(&path).expect("Could not create directory.");
    path.push("data.json");
    path
}

/// Inhibits or restores the system keyboard shortcuts (e.g. Super, Alt-Tab) for
/// the toplevel that `widget` belongs to. No-op if the widget is not yet rooted
/// on a toplevel surface.
pub fn set_shortcuts_inhibited(widget: &impl IsA<gtk::Widget>, inhibited: bool) {
    let Some(toplevel) = widget
        .root()
        .and_then(|r| r.surface())
        .and_then(|s| s.downcast::<gdk::Toplevel>().ok())
    else {
        return;
    };
    if inhibited {
        toplevel.inhibit_system_shortcuts(None::<gdk::Event>);
    } else {
        toplevel.restore_system_shortcuts();
    }
}
