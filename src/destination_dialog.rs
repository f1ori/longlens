use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::glib;
use secrecy::SecretString;

use crate::connection_options_dialog::LongLensConnectionOptionsDialog;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/destination_dialog.ui")]
    pub struct LongLensDestinationDialog {
        #[template_child]
        pub nameentry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub hostnameentry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub usernameentry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub rememberpasswordswitch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub passwordentry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub saveandconnectbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub saveonlybutton: TemplateChild<gtk::Button>,
        pub is_edit_mode: Cell<bool>,
        pub clipboard_enabled: Cell<bool>,
        pub sound_enabled: Cell<bool>,
        pub inhibit_system_shortcuts: Cell<bool>,
        pub on_save: RefCell<Option<Box<dyn Fn(String, String, String, SecretString, bool, bool, bool, bool) -> Option<String> + 'static>>>,
        pub on_delete: RefCell<Option<Box<dyn Fn() + 'static>>>,
    }

    impl std::fmt::Debug for LongLensDestinationDialog {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("LongLensDestinationDialog").finish()
        }
    }

    #[gtk::template_callbacks]
    impl LongLensDestinationDialog {
        #[template_callback]
        fn handle_entries_activated(&self, _entry: &adw::EntryRow) {
            if !self.hostnameentry.text().is_empty() {
                self.handle_action();
            }
        }

        #[template_callback]
        fn handle_cancelbutton_clicked(&self, _button: &gtk::Button) {
            self.obj().close();
        }

        #[template_callback]
        fn handle_saveandconnectbutton_clicked(&self, _button: &gtk::Button) {
            self.handle_action();
        }

        #[template_callback]
        fn handle_saveonlybutton_clicked(&self, _button: &gtk::Button) {
            self.save_only();
        }

        #[template_callback]
        fn handle_optionsrow_activated(&self, _row: &adw::ActionRow) {
            self.show_options_dialog();
        }

        #[template_callback]
        fn handle_optionsbutton_clicked(&self, _button: &gtk::Button) {
            self.show_options_dialog();
        }

        fn show_options_dialog(&self) {
            let dialog = LongLensConnectionOptionsDialog::new();
            dialog.set_clipboard_enabled(self.clipboard_enabled.get());
            dialog.set_sound_enabled(self.sound_enabled.get());
            dialog.set_inhibit_system_shortcuts(self.inhibit_system_shortcuts.get());
            dialog.set_on_save(glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |clipboard_enabled, sound_enabled, inhibit_system_shortcuts| {
                    this.clipboard_enabled.set(clipboard_enabled);
                    this.sound_enabled.set(sound_enabled);
                    this.inhibit_system_shortcuts.set(inhibit_system_shortcuts);
                }
            ));
            dialog.present(Some(&*self.obj()));
        }

        fn form_values(&self) -> (String, String, String, SecretString, bool, bool, bool, bool) {
            (
                self.nameentry.text().to_string(),
                self.hostnameentry.text().to_string(),
                self.usernameentry.text().to_string(),
                SecretString::new(self.passwordentry.text().to_string().into()),
                self.rememberpasswordswitch.is_active(),
                self.clipboard_enabled.get(),
                self.sound_enabled.get(),
                self.inhibit_system_shortcuts.get(),
            )
        }

        fn save_only(&self) {
            let (name, hostname, username, password, remember, clipboard_enabled, sound_enabled, inhibit_system_shortcuts) = self.form_values();
            if let Some(on_save) = self.on_save.borrow().as_ref() {
                on_save(name, hostname, username, password, remember, clipboard_enabled, sound_enabled, inhibit_system_shortcuts);
            }
            self.obj().close();
        }

        fn handle_action(&self) {
            let (name, hostname, username, password, remember, clipboard_enabled, sound_enabled, inhibit_system_shortcuts) = self.form_values();
            if let Some(on_save) = self.on_save.borrow().as_ref() {
                if let Some(uuid) = on_save(name, hostname, username, password, remember, clipboard_enabled, sound_enabled, inhibit_system_shortcuts) {
                    self.obj()
                        .activate_action("win.connect", Some(&uuid.to_variant()))
                        .expect("win.connect action failed");
                }
            }
            self.obj().close();
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LongLensDestinationDialog {
        const NAME: &'static str = "LongLensDestinationDialog";
        type Type = super::LongLensDestinationDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LongLensDestinationDialog {
        fn constructed(&self) {
            self.parent_constructed();

            self.clipboard_enabled.set(true);
            self.sound_enabled.set(true);
            self.inhibit_system_shortcuts.set(true);
            self.saveandconnectbutton.set_sensitive(false);
            self.saveonlybutton.set_sensitive(false);
            self.hostnameentry.connect_notify_local(
                Some("text"),
                glib::clone!(
                    #[weak(rename_to = dialog)]
                    self,
                    move |entry, _| {
                        let sensitive = !entry.text().is_empty();
                        dialog.saveandconnectbutton.set_sensitive(sensitive);
                        dialog.saveonlybutton.set_sensitive(sensitive);
                    }
                ),
            );

            self.rememberpasswordswitch.connect_active_notify(glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |switch| {
                    let active = switch.is_active();
                    dialog.passwordentry.set_sensitive(active);
                    if !active {
                        dialog.passwordentry.set_text("");
                    }
                }
            ));

            self.obj().connect_map(|dialog| {
                dialog.imp().hostnameentry.grab_focus();
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    dialog,
                    async move {
                        let available = crate::secrets::is_available().await;
                        dialog.imp().rememberpasswordswitch.set_sensitive(available);
                        if !available {
                            dialog.imp().rememberpasswordswitch.set_active(false);
                        }
                        dialog.imp().passwordentry.set_sensitive(dialog.imp().rememberpasswordswitch.is_active());
                    }
                ));
            });
        }
    }
    impl WidgetImpl for LongLensDestinationDialog {}
    impl AdwDialogImpl for LongLensDestinationDialog {}
}

glib::wrapper! {
    pub struct LongLensDestinationDialog(ObjectSubclass<imp::LongLensDestinationDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl LongLensDestinationDialog {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_edit_mode(&self, edit_mode: bool) {
        self.imp().is_edit_mode.set(edit_mode);
        if edit_mode {
            self.set_title(&gettext("Edit Destination"));
        } else {
            self.set_title(&gettext("Add Destination"));
        }
    }

    pub fn set_on_save(
        &self,
        callback: impl Fn(String, String, String, SecretString, bool, bool, bool, bool) -> Option<String> + 'static,
    ) {
        *self.imp().on_save.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_delete(&self, callback: impl Fn() + 'static) {
        *self.imp().on_delete.borrow_mut() = Some(Box::new(callback));
    }

    pub fn name(&self) -> String {
        self.imp().nameentry.text().to_string()
    }

    pub fn hostname(&self) -> String {
        self.imp().hostnameentry.text().to_string()
    }

    pub fn username(&self) -> String {
        self.imp().usernameentry.text().to_string()
    }

    pub fn password(&self) -> SecretString {
        SecretString::new(self.imp().passwordentry.text().to_string().into())
    }

    pub fn remember_password(&self) -> bool {
        self.imp().rememberpasswordswitch.is_active()
    }

    pub fn set_clipboard_enabled(&self, enabled: bool) {
        self.imp().clipboard_enabled.set(enabled);
    }

    pub fn set_sound_enabled(&self, enabled: bool) {
        self.imp().sound_enabled.set(enabled);
    }

    pub fn set_inhibit_system_shortcuts(&self, enabled: bool) {
        self.imp().inhibit_system_shortcuts.set(enabled);
    }

    pub fn clipboard_enabled(&self) -> bool {
        self.imp().clipboard_enabled.get()
    }

    pub fn sound_enabled(&self) -> bool {
        self.imp().sound_enabled.get()
    }
}
