//! Key bindings: the one place a game's logical buttons map to physical keys.

use spectrum::host::HostCtx;
use spectrum::keyboard;

use crate::Button;

/// Maps each logical [`Button`] to the physical key(s) that trigger it — the
/// single source of truth for input, shared by the host head ([`crate::install`])
/// and reusable by the agent env. Remappable: build a custom scheme with
/// [`Controls::set`] / [`Controls::bind`] and pass it to
/// [`crate::install_with_controls`]. The default is cursor keys **and** QAOP +
/// `0`/Space, so any common scheme works.
#[derive(Clone)]
pub struct Controls {
    bindings: Vec<(Button, char)>,
}

impl Default for Controls {
    fn default() -> Self {
        // Cursor key listed first per button, so `key_pos` prefers it.
        Controls {
            bindings: vec![
                (Button::Up, '7'),
                (Button::Up, 'q'),
                (Button::Down, '6'),
                (Button::Down, 'a'),
                (Button::Left, '5'),
                (Button::Left, 'o'),
                (Button::Right, '8'),
                (Button::Right, 'p'),
                (Button::Fire, '0'),
                (Button::Fire, ' '),
            ],
        }
    }
}

impl Controls {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a key for `button` (additive).
    pub fn bind(&mut self, button: Button, key: char) -> &mut Self {
        self.bindings.push((button, key));
        self
    }

    /// Replace all keys for `button` with `keys`.
    pub fn set(&mut self, button: Button, keys: &[char]) -> &mut Self {
        self.bindings.retain(|(b, _)| *b != button);
        for &k in keys {
            self.bindings.push((button, k));
        }
        self
    }

    /// The keys currently bound to `button`.
    pub fn keys_for(&self, button: Button) -> impl Iterator<Item = char> + '_ {
        self.bindings
            .iter()
            .filter(move |(b, _)| *b == button)
            .map(|(_, k)| *k)
    }

    /// The primary physical key for `button` (first binding) — the one the env
    /// presses to drive the button.
    pub fn key_pos(&self, button: Button) -> Option<keyboard::KeyPos> {
        self.bindings
            .iter()
            .find(|(b, _)| *b == button)
            .and_then(|(_, ch)| keyboard::key_for_char(*ch).map(|(p, _, _)| p))
    }

    /// Read the held-button bitset from the live keyboard (host side).
    pub(crate) fn read(&self, ctx: &HostCtx) -> u8 {
        let mut b = 0u8;
        for &(button, ch) in &self.bindings {
            if let Some((pos, _, _)) = keyboard::key_for_char(ch) {
                if ctx.key(pos) {
                    b |= button as u8;
                }
            }
        }
        b
    }
}
