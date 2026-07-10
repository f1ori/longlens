use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::model::destination_object::ConnectionOptions;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/connection_options_dialog.ui")]
    pub struct LongLensConnectionOptionsDialog {
        #[template_child]
        pub clipboardswitch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub soundswitch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub forwardunicodeswitch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub inhibitsystemshortcutsswitch: TemplateChild<adw::SwitchRow>,
        pub on_save: RefCell<Option<Box<dyn Fn(ConnectionOptions) + 'static>>>,
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
                on_save(self.obj().connection_options());
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

    pub fn set_connection_options(&self, options: ConnectionOptions) {
        self.imp().clipboardswitch.set_active(options.clipboard_enabled);
        self.imp().soundswitch.set_active(options.sound_enabled);
        self.imp().forwardunicodeswitch.set_active(options.forward_unicode);
        self.imp()
            .inhibitsystemshortcutsswitch
            .set_active(options.inhibit_system_shortcuts);
    }

    pub fn connection_options(&self) -> ConnectionOptions {
        ConnectionOptions {
            clipboard_enabled: self.imp().clipboardswitch.is_active(),
            sound_enabled: self.imp().soundswitch.is_active(),
            forward_unicode: self.imp().forwardunicodeswitch.is_active(),
            inhibit_system_shortcuts: self.imp().inhibitsystemshortcutsswitch.is_active(),
        }
    }

    pub fn set_on_save(&self, callback: impl Fn(ConnectionOptions) + 'static) {
        *self.imp().on_save.borrow_mut() = Some(Box::new(callback));
    }
}
