use std::cell::RefCell;

use glib::Object;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;


#[derive(Default, Clone, Serialize, Deserialize)]
pub struct DestinationData {
    #[serde(default = "new_uuid")]
    pub uuid: String,
    #[serde(default)]
    pub name: String,
    pub hostname: String,
    pub username: String,
}

fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

mod imp {
    use super::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::DestinationObject)]
    pub struct DestinationObject {
        #[property(name = "uuid", get, set, type = String, member = uuid)]
        #[property(name = "name", get, set, type = String, member = name)]
        #[property(name = "hostname", get, set, type = String, member = hostname)]
        #[property(name = "username", get, set, type = String, member = username)]
        pub data: RefCell<DestinationData>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DestinationObject {
        const NAME: &'static str = "LongLensDestinationObject";
        type Type = super::DestinationObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for DestinationObject {}
}

glib::wrapper! {
    pub struct DestinationObject(ObjectSubclass<imp::DestinationObject>);
}

impl DestinationObject {
    pub fn new(name: String, hostname: String, username: String) -> Self {
        Object::builder()
            .property("uuid", Uuid::new_v4().to_string())
            .property("name", name)
            .property("hostname", hostname)
            .property("username", username)
            .build()
    }

    pub fn destination_data(&self) -> DestinationData {
        self.imp().data.borrow().clone()
    }

    pub fn from_destination_data(data: DestinationData) -> Self {
        Object::builder()
            .property("uuid", &data.uuid)
            .property("name", &data.name)
            .property("hostname", &data.hostname)
            .property("username", &data.username)
            .build()
    }
}
