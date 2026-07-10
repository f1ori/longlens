use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/connection_options_dialog.ui")]
    pub struct LongLensConnectionOptionsDialog {
        #[template_child]
        pub clipboardswitch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub soundswitch: TemplateChild<adw::SwitchRow>,
        pub on_save: RefCell<Option<Box<dyn Fn(bool, bool) + 'static>>>,
    }

    impl std::fmt::Debug for LongLensConnectionOptionsDialog {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("LongLensConnectionOptionsDialog").finish()
        }
    }

    #[gtk::template_callbacks]
    impl LongLensConnectionOptionsDialog {
        #[template_callback]
        fn handle_cancelbutton_clicked(&self, _button: &gtk::Button) {
            self.obj().close();
        }

        #[template_callback]
        fn handle_savebutton_clicked(&self, _button: &gtk::Button) {
            if let Some(on_save) = self.on_save.borrow().as_ref() {
                on_save(
                    self.clipboardswitch.is_active(),
                    self.soundswitch.is_active(),
                );
            }
            self.obj().close();
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LongLensConnectionOptionsDialog {
        const NAME: &'static str = "LongLensConnectionOptionsDialog";
        type Type = super::LongLensConnectionOptionsDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LongLensConnectionOptionsDialog {}
    impl WidgetImpl for LongLensConnectionOptionsDialog {}
    impl AdwDialogImpl for LongLensConnectionOptionsDialog {}
}

glib::wrapper! {
    pub struct LongLensConnectionOptionsDialog(ObjectSubclass<imp::LongLensConnectionOptionsDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl LongLensConnectionOptionsDialog {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_clipboard_enabled(&self, enabled: bool) {
        self.imp().clipboardswitch.set_active(enabled);
    }

    pub fn set_sound_enabled(&self, enabled: bool) {
        self.imp().soundswitch.set_active(enabled);
    }

    pub fn set_on_save(&self, callback: impl Fn(bool, bool) + 'static) {
        *self.imp().on_save.borrow_mut() = Some(Box::new(callback));
    }
}
