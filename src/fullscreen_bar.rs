/*
 * Copyright 2025 Florian Richter
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use std::cell::Cell;
use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/de/f1ori/longlens/ui/fullscreen_bar.ui")]
    #[properties(wrapper_type = super::LlFullscreenBar)]
    pub struct LlFullscreenBar {
        #[template_child]
        pub revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub bar: TemplateChild<gtk::Box>,
        #[template_child]
        pub title: TemplateChild<gtk::Label>,
        #[property(name = "title", get = Self::title_text, set = Self::set_title_text, type = String)]
        _title: (),
        pub hide_timer: RefCell<Option<glib::SourceId>>,
        pub bar_motion: RefCell<Option<gtk::EventControllerMotion>>,
        pub edge_installed: Cell<bool>,
    }

    #[gtk::template_callbacks]
    impl LlFullscreenBar {
        #[template_callback]
        fn handle_leave_clicked(&self, _button: &gtk::Button) {
            if let Some(window) = self.obj().root().and_downcast::<gtk::Window>() {
                window.unfullscreen();
            }
        }
    }

    impl LlFullscreenBar {
        fn title_text(&self) -> String {
            self.title.text().to_string()
        }

        fn set_title_text(&self, value: String) {
            self.title.set_text(&value);
        }

        /// Reveals the auto-hiding bar and (re)starts the hide timer.
        fn reveal_bar(&self) {
            self.revealer.set_reveal_child(true);
            self.schedule_hide();
        }

        /// Cancels any pending hide and schedules the bar to hide after a delay.
        fn schedule_hide(&self) {
            self.cancel_hide();
            let source = glib::timeout_add_local_once(
                std::time::Duration::from_secs(3),
                glib::clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        imp.hide_timer.borrow_mut().take();
                        // Don't hide while the pointer is still hovering the bar;
                        // re-arm instead. This also covers the case where the bar
                        // was revealed under a motionless pointer (edge trigger),
                        // which never emits an `enter` event.
                        let hovered = imp
                            .bar_motion
                            .borrow()
                            .as_ref()
                            .is_some_and(|c| c.contains_pointer());
                        if hovered {
                            imp.schedule_hide();
                        } else {
                            imp.revealer.set_reveal_child(false);
                        }
                    }
                ),
            );
            *self.hide_timer.borrow_mut() = Some(source);
        }

        /// Cancels a pending hide timer, if any.
        fn cancel_hide(&self) {
            if let Some(source) = self.hide_timer.borrow_mut().take() {
                source.remove();
            }
        }

        /// Reveals the bar and restarts the auto-hide timer.
        pub(super) fn reveal(&self) {
            self.reveal_bar();
        }

        /// Immediately hides the bar and cancels any pending hide.
        pub(super) fn hide(&self) {
            self.cancel_hide();
            self.revealer.set_reveal_child(false);
        }

        /// Installs the top-edge motion controller on the window so the bar can
        /// be re-revealed when the pointer hits the top-center edge. Done lazily
        /// on `map`, when the window root is available.
        fn install_edge_controller(&self) {
            if self.edge_installed.get() {
                return;
            }
            let Some(window) = self.obj().root().and_downcast::<gtk::Window>() else {
                return;
            };
            let edge_motion = gtk::EventControllerMotion::new();
            edge_motion.set_propagation_phase(gtk::PropagationPhase::Capture);
            edge_motion.connect_motion(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller, x, y| {
                    let Some(window) = imp.obj().root().and_downcast::<gtk::Window>() else {
                        return;
                    };
                    if !window.is_fullscreen() {
                        return;
                    }
                    let width = window.width() as f64;
                    if y <= 5.0 && (x - width / 2.0).abs() <= 150.0 {
                        imp.reveal_bar();
                    }
                }
            ));
            window.add_controller(edge_motion);
            self.edge_installed.set(true);
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LlFullscreenBar {
        const NAME: &'static str = "LlFullscreenBar";
        type Type = super::LlFullscreenBar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LlFullscreenBar {
        fn constructed(&self) {
            self.parent_constructed();

            // Keep the bar visible while the pointer hovers it.
            let bar_motion = gtk::EventControllerMotion::new();
            bar_motion.connect_enter(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller, _x, _y| {
                    imp.cancel_hide();
                }
            ));
            bar_motion.connect_leave(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller| {
                    imp.schedule_hide();
                }
            ));
            self.bar.add_controller(bar_motion.clone());
            *self.bar_motion.borrow_mut() = Some(bar_motion);
        }
    }

    impl WidgetImpl for LlFullscreenBar {
        fn map(&self) {
            self.parent_map();
            self.install_edge_controller();
        }
    }

    impl BinImpl for LlFullscreenBar {}
}

glib::wrapper! {
    pub struct LlFullscreenBar(ObjectSubclass<imp::LlFullscreenBar>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl LlFullscreenBar {
    /// Reveals the bar and (re)starts the auto-hide timer.
    pub fn reveal(&self) {
        self.imp().reveal();
    }

    /// Immediately hides the bar and cancels any pending hide.
    pub fn hide(&self) {
        self.imp().hide();
    }
}
