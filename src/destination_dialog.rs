use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
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
        pub passwordentry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub savebutton: TemplateChild<gtk::Button>,
        pub is_edit_mode: Cell<bool>,
        pub on_save: RefCell<Option<Box<dyn Fn(String, String, String) + 'static>>>,
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
            self.handle_action();
        }

        #[template_callback]
        fn handle_cancelbutton_clicked(&self, _button: &gtk::Button) {
            self.obj().close();
        }

        #[template_callback]
        fn handle_savebutton_clicked(&self, _button: &gtk::Button) {
            self.handle_action();
        }

        fn handle_action(&self) {
            if self.is_edit_mode.get() {
                let name = self.nameentry.text().to_string();
                let hostname = self.hostnameentry.text().to_string();
                let username = self.usernameentry.text().to_string();
                if let Some(on_save) = self.on_save.borrow().as_ref() {
                    on_save(name, hostname, username);
                }
                self.obj().close();
            } else {
                self.connect_rdp();
            }
        }

        fn connect_rdp(&self) {
            self.obj()
                .activate_action("win.quick-connect", None)
                .expect("Quick connect action failed");
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
            self.obj().connect_map(|dialog| {
                dialog.imp().hostnameentry.grab_focus();
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
            self.imp().savebutton.set_label("Save");
            self.set_title("Edit Destination");
        } else {
            self.imp().savebutton.set_label("Connect");
            self.set_title("Add Destination");
        }
    }

    pub fn set_on_save(&self, callback: impl Fn(String, String, String) + 'static) {
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
}
