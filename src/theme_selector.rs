use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use crate::config::APP_ID;

mod imp {
    use super::*;

    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/theme_selector.ui")]
    pub struct LlThemeSelector {
        #[template_child]
        pub system: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub light: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub dark: TemplateChild<gtk::CheckButton>,
        pub settings: gio::Settings,
    }

    impl Default for LlThemeSelector {
        fn default() -> Self {
            Self {
                system: Default::default(),
                light: Default::default(),
                dark: Default::default(),
                settings: gio::Settings::new(APP_ID),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LlThemeSelector {
        const NAME: &'static str = "LlThemeSelector";
        type Type = super::LlThemeSelector;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LlThemeSelector {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_settings();
        }
    }

    #[gtk::template_callbacks]
    impl LlThemeSelector {
        #[template_callback]
        fn on_option_selected(&self) {
            let scheme = if self.light.is_active() {
                "light"
            } else if self.dark.is_active() {
                "dark"
            } else if self.system.is_active() {
                "default"
            } else {
                return;
            };

            let _ = self
                .obj()
                .activate_action("app.color-scheme", Some(&scheme.to_variant()));
        }
    }

    impl WidgetImpl for LlThemeSelector {}
    impl BoxImpl for LlThemeSelector {}
}

glib::wrapper! {
    pub struct LlThemeSelector(ObjectSubclass<imp::LlThemeSelector>)
        @extends gtk::Widget, gtk::Box,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl LlThemeSelector {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup_settings(&self) {
        self.imp().settings.connect_changed(
            Some("color-scheme"),
            clone!(
                #[weak(rename_to = this)]
                self,
                move |_, _| {
                    this.refresh_from_setting();
                }
            ),
        );
        self.refresh_from_setting();
    }

    fn refresh_from_setting(&self) {
        let imp = self.imp();
        match imp.settings.string("color-scheme").as_str() {
            "light" => imp.light.set_active(true),
            "dark" => imp.dark.set_active(true),
            _ => imp.system.set_active(true),
        }
    }
}

impl Default for LlThemeSelector {
    fn default() -> Self {
        Self::new()
    }
}
