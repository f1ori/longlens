use std::cell::RefCell;

use glib::Object;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use serde::{Deserialize, Serialize};


#[derive(Default, Clone, Serialize, Deserialize)]
pub struct DestinationData {
    pub hostname: String,
    pub username: String,
}

mod imp {
    use super::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::DestinationObject)]
    pub struct DestinationObject {
        #[property(name = "hostname", get, set, type = String, member = hostname)]
        #[property(name = "username", get, set, type = String, member = username)]
        pub data: RefCell<DestinationData>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DestinationObject {
        const NAME: &'static str = "FernsichtRdpDestinationObject";
        type Type = super::DestinationObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for DestinationObject {}
}

glib::wrapper! {
    pub struct DestinationObject(ObjectSubclass<imp::DestinationObject>);
}

impl DestinationObject {
    pub fn new(hostname: String, username: String) -> Self {
        Object::builder()
            .property("hostname", hostname)
            .property("username", username)
            .build()
    }

    pub fn destination_data(&self) -> DestinationData {
        self.imp().data.borrow().clone()
    }

    pub fn from_destination_data(destination_data: DestinationData) -> Self {
        Self::new(destination_data.hostname, destination_data.username)
    }
}

