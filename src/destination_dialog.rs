use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::gio;
use gtk::glib;

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
        pub savebutton: TemplateChild<adw::SplitButton>,
        pub is_edit_mode: Cell<bool>,
        pub on_save: RefCell<Option<Box<dyn Fn(String, String, String, String, bool) + 'static>>>,
        pub on_delete: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub on_connect: RefCell<Option<Box<dyn Fn(String, String, String, String, bool) + 'static>>>,
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
        fn handle_savebutton_clicked(&self, _button: &adw::SplitButton) {
            self.handle_action();
        }

        fn save_only(&self) {
            let name = self.nameentry.text().to_string();
            let hostname = self.hostnameentry.text().to_string();
            let username = self.usernameentry.text().to_string();
            let password = self.passwordentry.text().to_string();
            let remember = self.rememberpasswordswitch.is_active();
            if self.is_edit_mode.get() {
                if let Some(on_save) = self.on_save.borrow().as_ref() {
                    on_save(name, hostname, username, password, remember);
                }
            } else if let Some(on_connect) = self.on_connect.borrow().as_ref() {
                on_connect(name, hostname, username, password, remember);
            }
            self.obj().close();
        }

        fn handle_action(&self) {
            if self.is_edit_mode.get() {
                let name = self.nameentry.text().to_string();
                let hostname = self.hostnameentry.text().to_string();
                let username = self.usernameentry.text().to_string();
                let password = self.passwordentry.text().to_string();
                let remember = self.rememberpasswordswitch.is_active();
                if let Some(on_save) = self.on_save.borrow().as_ref() {
                    on_save(name, hostname, username, password, remember);
                }
            }
            self.connect_rdp();
        }

        fn connect_rdp(&self) {
            let hostname = self.hostnameentry.text().to_string();
            let username = self.usernameentry.text().to_string();
            let password = self.passwordentry.text().to_string();
            let remember = self.rememberpasswordswitch.is_active();
            if let Some(on_connect) = self.on_connect.borrow().as_ref() {
                on_connect(
                    self.nameentry.text().to_string(),
                    hostname.clone(),
                    username.clone(),
                    password.clone(),
                    remember,
                );
            }
            let variant = (hostname, username, password).to_variant();
            self.obj()
                .activate_action("win.connect", Some(&variant))
                .expect("win.connect action failed");
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

            let action_group = gio::SimpleActionGroup::new();
            let save_only = gio::SimpleAction::new("save-only", None);
            let obj = self.obj().clone();
            save_only.connect_activate(move |_, _| obj.imp().save_only());
            action_group.add_action(&save_only);
            self.obj().insert_action_group("dialog", Some(&action_group));

            self.savebutton.set_sensitive(false);
            self.hostnameentry.connect_notify_local(
                Some("text"),
                glib::clone!(
                    #[weak(rename_to = dialog)]
                    self,
                    move |entry, _| {
                        dialog.savebutton.set_sensitive(!entry.text().is_empty());
                    }
                ),
            );

            self.rememberpasswordswitch.connect_active_notify(glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |switch| {
                    if !switch.is_active() {
                        dialog.passwordentry.set_text("");
                    }
                }
            ));

            self.obj().connect_map(|dialog| {
                dialog.imp().hostnameentry.grab_focus();
                let available = crate::secrets::is_available();
                dialog.imp().rememberpasswordswitch.set_sensitive(available);
                if !available {
                    dialog.imp().rememberpasswordswitch.set_active(false);
                }
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
            self.set_title("Edit Destination");
        } else {
            self.set_title("Add Destination");
        }
    }

    pub fn set_on_delete(&self, callback: impl Fn() + 'static) {
        *self.imp().on_delete.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_connect(
        &self,
        callback: impl Fn(String, String, String, String, bool) + 'static,
    ) {
        *self.imp().on_connect.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_save(
        &self,
        callback: impl Fn(String, String, String, String, bool) + 'static,
    ) {
        *self.imp().on_save.borrow_mut() = Some(Box::new(callback));
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

    pub fn password(&self) -> String {
        self.imp().passwordentry.text().to_string()
    }

    pub fn remember_password(&self) -> bool {
        self.imp().rememberpasswordswitch.is_active()
    }
}
