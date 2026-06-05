use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use secrecy::SecretString;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/password_dialog.ui")]
    pub struct LongLensPasswordDialog {
        #[template_child]
        pub passwordentry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub passwordgroup: TemplateChild<adw::PreferencesGroup>,
        pub on_connect: RefCell<Option<Box<dyn Fn(SecretString) + 'static>>>,
    }

    impl std::fmt::Debug for LongLensPasswordDialog {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("LongLensPasswordDialog").finish()
        }
    }

    #[gtk::template_callbacks]
    impl LongLensPasswordDialog {
        #[template_callback]
        fn handle_cancelbutton_clicked(&self, _button: &gtk::Button) {
            self.obj().close();
        }

        #[template_callback]
        fn handle_connectbutton_clicked(&self, _button: &gtk::Button) {
            self.do_connect();
        }

        #[template_callback]
        fn handle_passwordentry_activated(&self, _entry: &adw::EntryRow) {
            self.do_connect();
        }

        fn do_connect(&self) {
            let password = SecretString::new(self.passwordentry.text().to_string());
            if let Some(on_connect) = self.on_connect.borrow().as_ref() {
                on_connect(password);
            }
            self.obj().close();
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LongLensPasswordDialog {
        const NAME: &'static str = "LongLensPasswordDialog";
        type Type = super::LongLensPasswordDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LongLensPasswordDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().connect_map(|dialog| {
                dialog.imp().passwordentry.grab_focus();
            });
        }
    }
    impl WidgetImpl for LongLensPasswordDialog {}
    impl AdwDialogImpl for LongLensPasswordDialog {}
}

glib::wrapper! {
    pub struct LongLensPasswordDialog(ObjectSubclass<imp::LongLensPasswordDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl LongLensPasswordDialog {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_destination_name(&self, name: &str) {
        self.imp().passwordgroup.set_description(Some(name));
    }

    pub fn set_on_connect(&self, callback: impl Fn(SecretString) + 'static) {
        *self.imp().on_connect.borrow_mut() = Some(Box::new(callback));
    }
}
