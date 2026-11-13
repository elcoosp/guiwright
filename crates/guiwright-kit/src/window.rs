//! The kit's window-handle surface (§16 v0.4 "targets per window", the
//! embedded half of M6): [`crate::App::windows`] enumerates the
//! adapter's window targets and [`crate::App::window`] aims a
//! borrow-and-act handle at one target. Locators born from the handle
//! resolve only inside that window's subtree — the same semantics the
//! wire's `Window::Use` gives a remote connection, on the embedded
//! surface, with no new session or wire code.
//!
//! ```text
//! let preview = app.window(&w2)?;                      // aim (validated now)
//! preview.get_by_role(Role::Button, "Save").click()?;  // scoped to Preview
//! expect(preview.get_by_text("Preview text")).to_be_visible()?; // scoped
//! ```
//!
//! The tree is the source of truth (§16 v0.4): a handle that has left the
//! tree (a closed window) fails at aim time with the typed
//! `UnknownWindow` error even if adapter metadata still lists it.

use guiwright_core::selector::{Selector, TextMatch};
use guiwright_driver::adapter::FrameworkAdapter;

use accesskit::Role;

use crate::app::{App, Locator};
use crate::input::{KeyboardHandle, MouseHandle};

/// One aimed window target (§16 v0.4): a borrow-and-act view of the app
/// narrowed to a single window subtree. The DSL mirrors [`App`]'s — the
/// same `get_by_*` vocabulary, born window-scoped — plus the target's
/// metadata and the pre-pinned input handles.
pub struct WindowHandle<'a, A: FrameworkAdapter> {
    pub(crate) app: &'a mut App<A>,
    pub(crate) handle: String,
    pub(crate) title: String,
    pub(crate) focused: bool,
}

impl<A: FrameworkAdapter> std::fmt::Debug for WindowHandle<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowHandle")
            .field("handle", &self.handle)
            .field("title", &self.title)
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

impl<A: FrameworkAdapter> WindowHandle<'_, A> {
    /// The target's handle — the AccessKit subtree `TreeId` for grafted
    /// (multi-window) adapters, `"main"` for single-window ones.
    pub fn handle(&self) -> &str {
        &self.handle
    }

    /// The adapter's title for the target (the metadata row; falls back
    /// to the handle itself when the adapter did not script one).
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether the adapter reports this window focused.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// `window.get_by_role(role, name)` — the [`App`] locator vocabulary,
    /// resolving only inside this window's subtree.
    pub fn get_by_role(&mut self, role: Role, name: impl Into<TextMatch>) -> Locator<'_, A> {
        self.locator(Selector::role_named(role, name))
    }

    /// `window.get_by_label(...)` — accessible label, this window only.
    pub fn get_by_label(&mut self, name: impl Into<TextMatch>) -> Locator<'_, A> {
        self.locator(Selector::label(name))
    }

    /// `window.get_by_text(...)` — this window only.
    pub fn get_by_text(&mut self, text: impl Into<TextMatch>) -> Locator<'_, A> {
        self.locator(Selector::text(text))
    }

    /// `window.get_by_test_id(...)` (§8.1 O(1) hash), this window only.
    pub fn get_by_test_id(&mut self, id: impl Into<String>) -> Locator<'_, A> {
        self.locator(Selector::test_id(id))
    }

    /// Unnamed role locator — every widget of the role in this window;
    /// strict mode applies within the subtree (§8.2).
    pub fn role_locator(&mut self, role: Role) -> Locator<'_, A> {
        self.locator(Selector::role(role))
    }

    /// `window.get_by_value(...)` — this window only.
    pub fn get_by_value(&mut self, value: impl Into<TextMatch>) -> Locator<'_, A> {
        self.locator(Selector::value(value))
    }

    /// The raw-selector form (§8.1 `locator`): a locator for an
    /// already-built [`Selector`], scoped to this window.
    pub fn locator(&mut self, selector: Selector) -> Locator<'_, A> {
        Locator {
            app: self.app,
            selector,
            window: Some(self.handle.clone()),
        }
    }

    /// The window's raw-input surface, pre-pinned (§9): every injection
    /// lands in this window's pipeline without an explicit
    /// [`MouseHandle::in_window`].
    pub fn mouse(&mut self) -> MouseHandle<'_, A> {
        MouseHandle {
            app: self.app,
            window: Some(self.handle.clone()),
        }
    }

    /// The window's keyboard surface, pre-pinned (§9).
    pub fn keyboard(&mut self) -> KeyboardHandle<'_, A> {
        KeyboardHandle {
            app: self.app,
            window: Some(self.handle.clone()),
        }
    }
}
