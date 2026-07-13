/* rdp/key_handler.rs
 *
 * Copyright 2026 Florian Richter
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

use gtk::{glib, prelude::*};
use std::collections::{HashMap, HashSet};

pub(super) trait RemoteKeySender {
    fn send_key(&mut self, keycode: u16, pressed: bool);
    fn send_unicode_char(&mut self, ch: char, pressed: bool);
}

#[derive(Debug, Default)]
pub(super) struct KeyHandler {
    forward_unicode: bool,
    inhibit_system_shortcuts: bool,
    unicode_keys: HashMap<u16, char>,
    pressed_shortcut_modifier_keys: HashSet<u16>,
    filtered_system_shortcut_keys: HashSet<u16>,
}

impl KeyHandler {
    pub(super) fn set_forward_unicode(&mut self, enabled: bool) {
        self.forward_unicode = enabled;
    }

    pub(super) fn set_inhibit_system_shortcuts(&mut self, enabled: bool) {
        self.inhibit_system_shortcuts = enabled;
    }

    pub(super) fn update_system_shortcut_inhibition(
        &self,
        widget: &impl IsA<gtk::Widget>,
        active: bool,
    ) {
        crate::utils::set_shortcuts_inhibited(widget, active && self.inhibit_system_shortcuts);
    }

    pub(super) fn handle_key_pressed(
        &mut self,
        keyval: gtk::gdk::Key,
        keycode: u32,
        state: gtk::gdk::ModifierType,
        sender: &mut impl RemoteKeySender,
    ) -> glib::Propagation {
        let keycode = keycode as u16;
        if Self::is_shortcut_modifier(keyval) {
            self.pressed_shortcut_modifier_keys.insert(keycode);
        }

        // If system shortcuts are not inhibited, keep the local Super key local.
        if self.should_filter_system_shortcut_key(keyval) {
            self.filtered_system_shortcut_keys.insert(keycode);
            self.unicode_keys.remove(&keycode);
            return glib::Propagation::Stop;
        }

        // Unicode forwarding is only for plain text input. As soon as
        // modifiers are used, forward normal keycodes so remote shortcuts work.
        if let Some(ch) = self.unicode_char_without_modifiers(keyval, state) {
            self.unicode_keys.insert(keycode, ch);
            sender.send_unicode_char(ch, true);
        } else {
            self.unicode_keys.remove(&keycode);
            sender.send_key(keycode, true);
        }

        glib::Propagation::Stop
    }

    pub(super) fn handle_key_released(
        &mut self,
        keycode: u32,
        sender: &mut impl RemoteKeySender,
    ) {
        let keycode = keycode as u16;
        if self.filtered_system_shortcut_keys.remove(&keycode) {
            self.pressed_shortcut_modifier_keys.remove(&keycode);
            return;
        }

        if let Some(ch) = self.unicode_keys.remove(&keycode) {
            sender.send_unicode_char(ch, false);
        } else {
            sender.send_key(keycode, false);
        }
        self.pressed_shortcut_modifier_keys.remove(&keycode);
    }

    fn is_shortcut_modifier(keyval: gtk::gdk::Key) -> bool {
        matches!(
            keyval,
            gtk::gdk::Key::Control_L
                | gtk::gdk::Key::Control_R
                | gtk::gdk::Key::Alt_L
                | gtk::gdk::Key::Alt_R
                | gtk::gdk::Key::Super_L
                | gtk::gdk::Key::Super_R
                | gtk::gdk::Key::Hyper_L
                | gtk::gdk::Key::Hyper_R
                | gtk::gdk::Key::Meta_L
                | gtk::gdk::Key::Meta_R
        )
    }

    fn is_super_key(keyval: gtk::gdk::Key) -> bool {
        matches!(keyval, gtk::gdk::Key::Super_L | gtk::gdk::Key::Super_R)
    }

    fn should_filter_system_shortcut_key(&self, keyval: gtk::gdk::Key) -> bool {
        !self.inhibit_system_shortcuts && Self::is_super_key(keyval)
    }

    fn has_modifier(state: gtk::gdk::ModifierType) -> bool {
        state.intersects(
            gtk::gdk::ModifierType::SHIFT_MASK
                | gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK
                | gtk::gdk::ModifierType::HYPER_MASK
                | gtk::gdk::ModifierType::META_MASK,
        )
    }

    fn unicode_char_without_modifiers(
        &self,
        keyval: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
    ) -> Option<char> {
        if !self.forward_unicode
            || Self::has_modifier(state)
            || !self.pressed_shortcut_modifier_keys.is_empty()
        {
            return None;
        }

        keyval.to_unicode().filter(|ch| !ch.is_control())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum SentEvent {
        Key(u16, bool),
        Unicode(char, bool),
    }

    #[derive(Default)]
    struct TestSender {
        events: Vec<SentEvent>,
    }

    impl RemoteKeySender for TestSender {
        fn send_key(&mut self, keycode: u16, pressed: bool) {
            self.events.push(SentEvent::Key(keycode, pressed));
        }

        fn send_unicode_char(&mut self, ch: char, pressed: bool) {
            self.events.push(SentEvent::Unicode(ch, pressed));
        }
    }

    #[test]
    fn unicode_is_forwarded_without_modifiers() {
        let mut handler = KeyHandler::default();
        handler.set_forward_unicode(true);
        let mut sender = TestSender::default();

        handler.handle_key_pressed(
            gtk::gdk::Key::a,
            38,
            gtk::gdk::ModifierType::empty(),
            &mut sender,
        );
        handler.handle_key_released(38, &mut sender);

        assert_eq!(
            sender.events,
            vec![SentEvent::Unicode('a', true), SentEvent::Unicode('a', false)]
        );
    }

    #[test]
    fn unicode_with_modifiers_is_forwarded_as_keycodes() {
        let mut handler = KeyHandler::default();
        handler.set_forward_unicode(true);
        let mut sender = TestSender::default();

        handler.handle_key_pressed(
            gtk::gdk::Key::a,
            38,
            gtk::gdk::ModifierType::CONTROL_MASK,
            &mut sender,
        );
        handler.handle_key_released(38, &mut sender);

        assert_eq!(
            sender.events,
            vec![SentEvent::Key(38, true), SentEvent::Key(38, false)]
        );
    }

    #[test]
    fn super_is_filtered_when_system_shortcuts_are_not_inhibited() {
        let mut handler = KeyHandler::default();
        handler.set_inhibit_system_shortcuts(false);
        let mut sender = TestSender::default();

        handler.handle_key_pressed(
            gtk::gdk::Key::Super_L,
            133,
            gtk::gdk::ModifierType::empty(),
            &mut sender,
        );
        handler.handle_key_released(133, &mut sender);

        assert!(sender.events.is_empty());
    }
}
