use std::collections::HashMap;

use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use crate::application::LongLensApplication;
use crate::model::destinations::Destinations;

const SEARCH_PROVIDER_XML: &str = r#"<node>
  <interface name="org.gnome.Shell.SearchProvider2">
    <method name="GetInitialResultSet">
      <arg type="as" name="terms" direction="in"/>
      <arg type="as" name="results" direction="out"/>
    </method>
    <method name="GetSubsearchResultSet">
      <arg type="as" name="previous_results" direction="in"/>
      <arg type="as" name="terms" direction="in"/>
      <arg type="as" name="results" direction="out"/>
    </method>
    <method name="GetResultMetas">
      <arg type="as" name="identifiers" direction="in"/>
      <arg type="aa{sv}" name="metas" direction="out"/>
    </method>
    <method name="ActivateResult">
      <arg type="s" name="identifier" direction="in"/>
      <arg type="as" name="terms" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
    <method name="LaunchSearch">
      <arg type="as" name="terms" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
  </interface>
</node>"#;

fn build_result_metas(
    destinations: &Destinations,
    identifiers: &[String],
) -> Vec<HashMap<String, glib::Variant>> {
    let items = destinations.items();
    identifiers
        .iter()
        .filter_map(|id| items.iter().find(|d| &d.uuid == id))
        .map(|dest| {
            let display_name = if dest.name.is_empty() {
                dest.hostname.clone()
            } else {
                dest.name.clone()
            };
            let description = format!("Connect to {}@{}", dest.username, dest.hostname);

            let mut meta: HashMap<String, glib::Variant> = HashMap::new();
            meta.insert("id".to_string(), dest.uuid.to_variant());
            meta.insert("name".to_string(), display_name.to_variant());
            meta.insert("description".to_string(), description.to_variant());
            meta.insert("gicon".to_string(), "network-server-symbolic".to_variant());
            meta
        })
        .collect()
}

pub fn register_search_provider(connection: &gio::DBusConnection, app: &LongLensApplication) {
    let node_info =
        gio::DBusNodeInfo::for_xml(SEARCH_PROVIDER_XML).expect("Valid search provider XML");
    let interface_info = node_info
        .lookup_interface("org.gnome.Shell.SearchProvider2")
        .expect("Interface defined in XML");

    let app_weak = app.downgrade();

    connection
        .register_object("/de/f1ori/longlens/SearchProvider", &interface_info)
        .method_call(
            move |_conn, _sender, _path, _iface, method, params, invocation| {
                let Some(app) = app_weak.upgrade() else {
                    return;
                };

                match method {
                    "GetInitialResultSet" => {
                        let terms: Vec<String> =
                            params.child_value(0).get().unwrap_or_default();
                        let results = app.destinations().search(&terms);
                        invocation.return_value(Some(&(results,).to_variant()));
                    }
                    "GetSubsearchResultSet" => {
                        let terms: Vec<String> =
                            params.child_value(1).get().unwrap_or_default();
                        let results = app.destinations().search(&terms);
                        invocation.return_value(Some(&(results,).to_variant()));
                    }
                    "GetResultMetas" => {
                        let identifiers: Vec<String> =
                            params.child_value(0).get().unwrap_or_default();
                        let metas = build_result_metas(&app.destinations(), &identifiers);
                        invocation.return_value(Some(&(metas,).to_variant()));
                    }
                    "ActivateResult" => {
                        let identifier: String =
                            params.child_value(0).get().unwrap_or_default();
                        app.imp().pending_connection.replace(Some(identifier));
                        app.activate();
                        invocation.return_value(Some(&().to_variant()));
                    }
                    "LaunchSearch" => {
                        app.activate();
                        invocation.return_value(Some(&().to_variant()));
                    }
                    _ => {
                        invocation.return_gerror(glib::Error::new(
                            gio::DBusError::UnknownMethod,
                            &format!("Unknown method: {method}"),
                        ));
                    }
                }
            },
        )
        .build()
        .expect("Failed to register search provider D-Bus object");
}
