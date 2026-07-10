/* window.rs
 *
 * Copyright 2025 Florian Richter
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use secrecy::SecretString;

use std::cell::{Cell, RefCell};

use crate::connection_options_dialog::LongLensConnectionOptionsDialog;
use crate::model::destination_object::DestinationObject;
use crate::rdp::{RdpState, RdpWidget};
use crate::theme_selector::LlThemeSelector;
use crate::destinations_page::LlDestinationPage;
use crate::fullscreen_bar::LlFullscreenBar;
use crate::password_dialog::LongLensPasswordDialog;

fn stack_page(state: RdpState, n_destinations: u32) -> &'static str {
    if state == RdpState::Connected {
        "rdppage"
    } else if state == RdpState::Connecting {
        "connectingpage"
    } else if n_destinations == 0 {
        "emptypage"
    } else {
        "destinationspage"
    }
}

fn parse_domain_port(input: &str) -> (String, u16) {
    let mut parts = input.splitn(2, ':');
    let domain = parts.next().unwrap_or("").to_string();
    let port = parts
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3389);
    (domain, port)
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/window.ui")]
    pub struct LongLensWindow {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub destinations_page: TemplateChild<LlDestinationPage>,
        #[template_child]
        pub headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub disconnectbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub adddestinationbutton: TemplateChild<adw::SplitButton>,
        #[template_child]
        pub primarymenubutton: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub fullscreenbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub connectionoptionsbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub fullscreen_bar: TemplateChild<LlFullscreenBar>,
        #[template_child]
        pub rdpwidget: TemplateChild<RdpWidget>,
        pub connection_display_title: RefCell<String>,
        pub connection_destination_uuid: RefCell<Option<String>>,
        pub inhibit_system_shortcuts: Cell<bool>,
    }
    #[gtk::template_callbacks]
    impl LongLensWindow {
        #[template_callback]
        fn handle_disconnectbutton_clicked(&self, _button: &gtk::Button) {
            self.rdpwidget.disconnect();
        }

        #[template_callback]
        fn handle_adddestinationbutton_clicked(&self, _button: &gtk::Widget) {
            self.destinations_page.show_add_dialog();
        }

        #[template_callback]
        fn handle_connection_failed(&self, reason: String) {
            let dialog = adw::AlertDialog::new(Some(&gettext("Connection error")), Some(&reason));
            dialog.add_response("close", &gettext("Close"));
            dialog.present(Some(&*self.obj()));
        }

        #[template_callback]
        fn handle_fullscreenbutton_clicked(&self, _button: &gtk::Button) {
            self.obj().fullscreen();
        }

        #[template_callback]
        fn handle_connectionoptionsbutton_clicked(&self, _button: &gtk::Button) {
            self.obj().show_connection_options_dialog();
        }

        /// Installs a capture-phase key controller on the window that forwards
        /// every key to the RDP widget while its surface holds the focus.
        ///
        /// Running in the capture phase on the toplevel lets it preempt GTK's
        /// own keyboard handling — accelerators, mnemonics and the F10 primary
        /// menu — which would otherwise swallow keys like F10 before they could
        /// reach the remote session. Gating on the RDP widget's focus keeps the
        /// local UI fully keyboard-navigable whenever the pointer leaves the
        /// remote surface (which drops its focus).
        fn setup_key_grab(&self) {
            let controller = gtk::EventControllerKey::new();
            controller.set_propagation_phase(gtk::PropagationPhase::Capture);
            controller.connect_key_pressed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_controller, _keyval, keycode, _state| {
                    if !window.rdpwidget.has_focus() {
                        return glib::Propagation::Proceed;
                    }
                    window.rdpwidget.send_key(keycode as u16, true);
                    glib::Propagation::Stop
                }
            ));
            controller.connect_key_released(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_controller, _keyval, keycode, _state| {
                    if window.rdpwidget.has_focus() {
                        window.rdpwidget.send_key(keycode as u16, false);
                    }
                }
            ));
            self.obj().add_controller(controller);
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LongLensWindow {
        const NAME: &'static str = "LongLensWindow";
        type Type = super::LongLensWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LongLensWindow {
        fn constructed(&self) {
            self.parent_constructed();

            self.rdpwidget
                .bind_property::<gtk::Button>("state", self.disconnectbutton.as_ref(), "visible")
                .transform_to(|_binding, value: glib::Value| {
                    let state = value.get::<RdpState>().unwrap_or_default();
                    Some(state == RdpState::Connected || state == RdpState::Connecting)
                })
                .sync_create()
                .build();
            self.rdpwidget
                .bind_property::<adw::SplitButton>("state", self.adddestinationbutton.as_ref(), "visible")
                .transform_to(|_binding, value: glib::Value| {
                    let state = value.get::<RdpState>().unwrap_or_default();
                    Some(state != RdpState::Connected && state != RdpState::Connecting)
                })
                .sync_create()
                .build();
            self.rdpwidget
                .bind_property::<gtk::Button>("state", self.fullscreenbutton.as_ref(), "visible")
                .transform_to(|_binding, value: glib::Value| {
                    let state = value.get::<RdpState>().unwrap_or_default();
                    Some(state == RdpState::Connected)
                })
                .sync_create()
                .build();
            self.rdpwidget
                .bind_property::<gtk::Button>("state", self.connectionoptionsbutton.as_ref(), "visible")
                .transform_to(|_binding, value: glib::Value| {
                    let state = value.get::<RdpState>().unwrap_or_default();
                    Some(state == RdpState::Connected)
                })
                .sync_create()
                .build();
            self.obj()
                .bind_property::<LlFullscreenBar>("title", self.fullscreen_bar.as_ref(), "title")
                .sync_create()
                .build();

            // Hide the headerbar and reveal the auto-hiding control bar while
            // fullscreen; restore the headerbar when leaving fullscreen.
            self.obj().connect_fullscreened_notify(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |obj| {
                    let fullscreen = obj.is_fullscreen();
                    window.headerbar.set_visible(!fullscreen);
                    if fullscreen {
                        window.fullscreen_bar.reveal();
                    } else {
                        window.fullscreen_bar.hide();
                    }
                }
            ));

            self.rdpwidget.connect_state_notify(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |widget| {
                    let obj = window.obj();
                    let n = window.destinations_page.destinations().n_items();
                    window.stack.set_visible_child_name(stack_page(widget.state(), n));
                    let state = widget.state();
                    if state == RdpState::Connected || state == RdpState::Connecting {
                        let display_title = window.connection_display_title.borrow().clone();
                        obj.set_title(Some(&display_title));
                        crate::utils::set_shortcuts_inhibited(
                            &*obj,
                            state == RdpState::Connected && window.inhibit_system_shortcuts.get(),
                        );
                    } else {
                        window.connection_destination_uuid.borrow_mut().take();
                        obj.set_title(Some(&gettext("Long Lens")));
                        crate::utils::set_shortcuts_inhibited(&*obj, false);
                        if obj.is_fullscreen() {
                            obj.unfullscreen();
                        }
                    }
                }
            ));
            let model = self.destinations_page.destinations();
            self.stack.set_visible_child_name(stack_page(RdpState::default(), model.n_items()));
            model.connect_items_changed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |model, _, _, _| {
                    let state = window.rdpwidget.state();
                    window.stack.set_visible_child_name(stack_page(state, model.n_items()));
                }
            ));
            self.obj().setup_actions();
            self.obj().populate_menu();
            self.setup_key_grab();
        }
    }
    impl WidgetImpl for LongLensWindow {}
    impl WindowImpl for LongLensWindow {
        fn close_request(&self) -> glib::Propagation {
            let state = self.rdpwidget.state();
            if state != RdpState::Connected && state != RdpState::Connecting {
                return self.parent_close_request();
            }

            let dialog = adw::AlertDialog::new(
                Some(&gettext("Disconnect?")),
                Some(&gettext("You are currently connected to a remote session. Do you want to disconnect and close?")),
            );
            dialog.add_response("cancel", &gettext("Cancel"));
            dialog.add_response("disconnect", &gettext("Disconnect"));
            dialog.set_response_appearance("disconnect", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("disconnect"));
            dialog.set_close_response("cancel");

            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = window)]
                self,
                async move {
                    let response = dialog.choose_future(Some(&*window.obj())).await;
                    if response == "disconnect" {
                        window.rdpwidget.disconnect();
                        window.obj().destroy();
                    }
                }
            ));

            glib::Propagation::Stop
        }
    }
    impl ApplicationWindowImpl for LongLensWindow {}
    impl AdwApplicationWindowImpl for LongLensWindow {}
}

glib::wrapper! {
    pub struct LongLensWindow(ObjectSubclass<imp::LongLensWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl LongLensWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    fn populate_menu(&self) {
        if let Some(popover) = self
            .imp()
            .primarymenubutton
            .popover()
            .and_downcast::<gtk::PopoverMenu>()
        {
            let theme_selector = LlThemeSelector::new();
            popover.add_child(&theme_selector, "theme");
        }
    }

    fn setup_actions(&self) {
        let action_connect = gio::ActionEntry::builder("connect")
            .parameter_type(Some(glib::VariantTy::new("s").unwrap()))
            .activate(move |window: &Self, _action, parameter| {
                let uuid: String = parameter.unwrap().get().unwrap();
                let destinations = window.imp().destinations_page.destinations();
                let Some(dest) = destinations
                    .iter::<DestinationObject>()
                    .filter_map(|r| r.ok())
                    .find(|d| d.uuid() == uuid)
                else {
                    return;
                };
                let hostname = dest.hostname();
                let username = dest.username();
                let display_title = dest.property::<String>("display-title");
                let clipboard_enabled = dest.clipboard_enabled();
                let sound_enabled = dest.sound_enabled();
                let inhibit_system_shortcuts = dest.inhibit_system_shortcuts();
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    window,
                    async move {
                        match crate::secrets::get_password(&uuid).await {
                            Some(password) => {
                                window.start_connection(uuid, hostname, username, password, display_title, clipboard_enabled, sound_enabled, inhibit_system_shortcuts);
                            }
                            None => {
                                let dialog = LongLensPasswordDialog::new();
                                dialog.set_hostname(&hostname);
                                dialog.set_username(&username);
                                dialog.set_on_connect(glib::clone!(
                                    #[weak]
                                    window,
                                    move |password| {
                                        window.start_connection(uuid.clone(), hostname.clone(), username.clone(), password, display_title.clone(), clipboard_enabled, sound_enabled, inhibit_system_shortcuts);
                                    }
                                ));
                                dialog.present(Some(&window));
                            }
                        }
                    }
                ));
            })
            .build();

        let action_add_from_rdp = gio::ActionEntry::builder("add-from-rdp-file")
            .activate(move |window: &Self, _action, _parameter| {
                let filter = gtk::FileFilter::new();
                filter.set_name(Some(&gettext("RDP Files")));
                filter.add_pattern("*.rdp");
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);

                let dialog = gtk::FileDialog::builder()
                    .title(gettext("Open .rdp File"))
                    .filters(&filters)
                    .default_filter(&filter)
                    .build();
                dialog.open(
                    Some(window),
                    gio::Cancellable::NONE,
                    glib::clone!(
                        #[weak]
                        window,
                        move |result| {
                            let Ok(file) = result else {
                                return;
                            };
                            match file.path().as_deref().and_then(crate::rdp_file::parse_file) {
                                Some(conn) => window.show_add_from_rdp(conn),
                                None => window.show_rdp_file_error(),
                            }
                        }
                    ),
                );
            })
            .build();

        self.add_action_entries([action_connect, action_add_from_rdp]);
    }

    /// Open the Add Destination dialog pre-filled from a parsed `.rdp` file.
    pub fn show_add_from_rdp(&self, conn: crate::rdp_file::RdpConnection) {
        self.imp()
            .destinations_page
            .show_add_dialog_with(conn.name, conn.hostname, conn.username);
    }

    fn show_rdp_file_error(&self) {
        let dialog = adw::AlertDialog::new(
            Some(&gettext("Could not open .rdp file")),
            Some(&gettext("The file could not be read or contains no valid connection.")),
        );
        dialog.add_response("close", &gettext("Close"));
        dialog.present(Some(self));
    }

    fn show_connection_options_dialog(&self) {
        let Some(uuid) = self.imp().connection_destination_uuid.borrow().clone() else {
            return;
        };
        let destinations = self.imp().destinations_page.destinations();
        let Some(dest) = destinations
            .iter::<DestinationObject>()
            .filter_map(|r| r.ok())
            .find(|d| d.uuid() == uuid)
        else {
            return;
        };

        let dialog = LongLensConnectionOptionsDialog::new();
        dialog.set_clipboard_enabled(dest.clipboard_enabled());
        dialog.set_sound_enabled(dest.sound_enabled());
        dialog.set_inhibit_system_shortcuts(dest.inhibit_system_shortcuts());
        dialog.set_on_save(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            dest,
            move |clipboard_enabled, sound_enabled, inhibit_system_shortcuts| {
                let uuid = dest.uuid();
                dest.set_clipboard_enabled(clipboard_enabled);
                dest.set_sound_enabled(sound_enabled);
                dest.set_inhibit_system_shortcuts(inhibit_system_shortcuts);
                window.imp().inhibit_system_shortcuts.set(inhibit_system_shortcuts);
                window.imp().rdpwidget.set_clipboard_enabled(clipboard_enabled);
                window.imp().rdpwidget.set_inhibit_system_shortcuts(inhibit_system_shortcuts);
                window.imp().destinations_page.store().update(
                    &uuid,
                    dest.name(),
                    dest.hostname(),
                    dest.username(),
                    clipboard_enabled,
                    sound_enabled,
                    inhibit_system_shortcuts,
                );
            }
        ));
        dialog.present(Some(self));
    }

    fn start_connection(
        &self,
        uuid: String,
        hostname: String,
        username: String,
        password: SecretString,
        display_title: String,
        clipboard_enabled: bool,
        sound_enabled: bool,
        inhibit_system_shortcuts: bool,
    ) {
        *self.imp().connection_destination_uuid.borrow_mut() = Some(uuid);
        *self.imp().connection_display_title.borrow_mut() = display_title;
        self.imp().inhibit_system_shortcuts.set(inhibit_system_shortcuts);
        self.imp().rdpwidget.set_inhibit_system_shortcuts(inhibit_system_shortcuts);
        let (server, port) = parse_domain_port(&hostname);
        let w = self.imp().stack.width();
        let h = self.imp().stack.height();
        let width = if w > 0 { w as u16 } else { 1280 };
        let height = if h > 0 { h as u16 } else { 800 };
        self.imp().rdpwidget
            .connect_to_server(server, port, username, password, width, height, clipboard_enabled, sound_enabled);
    }
}
