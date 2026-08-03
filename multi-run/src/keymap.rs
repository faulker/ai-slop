//! Pure key-mapping layer. DESIGN.md section 7 is the source of truth for
//! the binding table; this module turns one key event into at most one
//! `Action`, given the current view mode and modal state. No IO, no state
//! mutation, no drawing: `map` is a plain function of its three arguments.
//!
//! `app.rs` (not yet built) owns the actual `Action` handling and the richer
//! `Modal` enum sketched in DESIGN.md section 4 (queued compose lines,
//! confirm target, toast text, error log entries, ...). This module cannot
//! depend on `app.rs` (the dependency would run the wrong way), so it
//! defines its own minimal `ModalState`: just enough to route a key to the
//! right sub-table. `app.rs` must derive a `ModalState` from its `Modal` on
//! every keystroke and import this type as `crate::keymap::ModalState`
//! rather than redefining a parallel one. `ui/mod.rs` needs a different,
//! richer, render-only view of the same modal for drawing
//! (`overlay::Overlay`); `app.rs` derives that separately from the same
//! `Modal`. Neither of this module's or `ui/mod.rs`'s types is the source of
//! truth; `app.rs`'s `Modal` is.
//!
//! ## Known simplifications, given the `(ViewMode, ModalState, KeyEvent) ->
//! ## Option<Action>` signature specified in DESIGN.md section 7
//!
//! - **`gg`.** The signature carries no memory of the previous keystroke, so
//!   a two-key chord cannot be detected here. A single `g` maps directly to
//!   `Action::FocusFirst`. If true vim-style `gg` (distinct from a lone `g`)
//!   is wanted later, it needs a small key-history buffer in `app.rs` above
//!   this pure layer; this module cannot provide it without breaking
//!   purity or the given signature.
//! - **`G` in the list/detail detail pane.** DESIGN.md section 7 lists `G`
//!   twice: "focus the last job" (global) and, only when the detail pane is
//!   focused, "jump the detail pane back to the live bottom". This function
//!   has no `Pane`/`Focus` parameter (the canonical signature in section 7
//!   does not carry one either), so it cannot distinguish the two cases. It
//!   always emits `Action::FocusLast`; `app.rs`, which does know the current
//!   `Pane`, must reinterpret `FocusLast` as "scroll to bottom" when the
//!   view is `ListDetail` and the detail pane has focus.
//! - **`h`/`l` and Tab / Shift-Tab vs. pane switching.** In `ListDetail`,
//!   `h`/`l` and Tab / Shift-Tab move keyboard focus between the list and
//!   detail panes rather than between tiles. `h`/`l` always emit
//!   `FocusLeft`/`FocusRight`; Tab / Shift-Tab emit those same actions in
//!   `ListDetail` and `FocusNext`/`FocusPrev` in `Grid` (cycle tiles).
//!   `app.rs` reinterprets `FocusLeft`/`FocusRight` as a pane switch in
//!   list view.

use crate::ui::layout::ViewMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Scroll amount, in lines, for a half-page scroll (`Ctrl-u`/`Ctrl-d`). This
/// module has no pane height to compute an exact half page from (the
/// canonical `map` signature carries no geometry), so it uses this fixed
/// approximation. `app.rs` may clamp or rescale it against the actual
/// detail-pane height if it wants an exact half page.
const HALF_PAGE_SCROLL: u16 = 10;
/// Scroll amount, in lines, for a full-page scroll (`Ctrl-b`/`Ctrl-f`). See
/// `HALF_PAGE_SCROLL`.
const FULL_PAGE_SCROLL: u16 = 20;

/// Every action a key can produce. `app.rs` is the sole consumer: it applies
/// exactly one of these to `App` per keystroke. Mirrors the `Action` enum in
/// DESIGN.md section 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    FocusNext,
    FocusPrev,
    FocusLeft,
    FocusRight,
    FocusFirst,
    FocusLast,
    FocusIndex(usize),
    NextPage,
    PrevPage,
    ToggleView,
    SetView(ViewMode),
    Attach,
    /// Escalating stop: `C-c`, then `TERM`, then `KILL`.
    Stop,
    SendEof,
    /// Finished or vanished job only; `app.rs` gates the state check.
    Dismiss,
    DismissAllFinished,
    /// Any state, with confirmation.
    KillFocused,
    Rerun,
    ScrollUp(u16),
    ScrollDown(u16),
    ScrollBottom,
    OpenCompose,
    OpenHelp,
    OpenErrorLog,
    CloseModal,
    ComposeChar(char),
    ComposeBackspace,
    ComposeClear,
    ComposeSubmit,
    ComposeCommit,
    ConfirmYes,
    ConfirmNo,
    Quit,
    QuitKeepAll,
    QuitKillAll,
}

/// Which modal overlay, if any, currently owns the keyboard. A minimal,
/// mapping-only view of DESIGN.md section 4's richer `Modal` enum; see the
/// module doc comment for why this module cannot use `Modal` itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalState {
    /// No overlay open; the main grid/list-detail view has the keyboard.
    None,
    Help,
    /// The compose / add-command prompt. `line_empty` is whether the
    /// current (not-yet-committed) line is empty, which disambiguates
    /// `Enter` (commit vs. submit-all per DESIGN.md section 7).
    /// `launch_on_submit` mirrors `Modal::Compose`'s field of the same name
    /// and disambiguates `Esc` (cancel the mid-run prompt vs. quit the tool
    /// from the bare-launch compose screen).
    Compose {
        line_empty: bool,
        launch_on_submit: bool,
    },
    Confirm,
    ExitPrompt,
    /// Transient toast. Does not take the keyboard: `map` routes through
    /// the normal table so the toast never blocks ordinary use.
    Toast,
    ErrorLog,
}

/// Map one key event to at most one `Action`, given the current view mode
/// and modal state. Pure: no IO, no state mutation, no drawing. Modal maps
/// are checked first, per DESIGN.md section 7.
pub fn map(view: ViewMode, modal: &ModalState, key: KeyEvent) -> Option<Action> {
    // Only key-down (or repeat, which Unix terminals normally report as a
    // second Press) should produce an action; a Release must not.
    if key.kind == KeyEventKind::Release {
        return None;
    }
    match modal {
        ModalState::None => map_normal(view, key),
        ModalState::Toast => map_normal(view, key),
        ModalState::Help => map_help(key),
        ModalState::Compose {
            line_empty,
            launch_on_submit,
        } => map_compose(key, *line_empty, *launch_on_submit),
        ModalState::Confirm => map_confirm(key),
        ModalState::ExitPrompt => map_exit_prompt(key),
        ModalState::ErrorLog => map_error_log(key),
    }
}

/// True if `key` is a plain `Char(c)` with no Ctrl/Alt held (Shift alone, or
/// no modifiers, is fine: crossterm bakes Shift into the char's case).
fn is_plain_char(key: &KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c)
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
}

/// `Ctrl-<c>` (crossterm reports the base lowercase char plus the modifier).
fn is_ctrl_char(key: &KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Main grid/list-detail table: global keys, navigation, and session
/// actions from DESIGN.md section 7. Also used for `ModalState::Toast`,
/// which does not take the keyboard.
fn map_normal(view: ViewMode, key: KeyEvent) -> Option<Action> {
    use KeyCode::*;

    // Global.
    if is_plain_char(&key, '?') {
        return Some(Action::OpenHelp);
    }
    if is_plain_char(&key, 'v') {
        return Some(Action::ToggleView);
    }
    if is_plain_char(&key, '1') {
        return Some(Action::SetView(ViewMode::Grid));
    }
    if is_plain_char(&key, '2') {
        return Some(Action::SetView(ViewMode::ListDetail));
    }
    if is_plain_char(&key, 'n') {
        return Some(Action::OpenCompose);
    }
    if is_plain_char(&key, '!') {
        return Some(Action::OpenErrorLog);
    }
    if is_plain_char(&key, 'q') || is_ctrl_char(&key, 'c') {
        return Some(Action::Quit);
    }
    if key.code == Esc {
        // "Close the topmost overlay, else no-op." No overlay is open in
        // ModalState::None, so this is a no-op here.
        return None;
    }

    // Panel / tile switching: Tab and Shift-Tab. In list/detail these move
    // between the list and detail panes; in grid they cycle job focus.
    if key.code == Tab {
        return Some(match view {
            ViewMode::ListDetail => Action::FocusRight,
            ViewMode::Grid => Action::FocusNext,
        });
    }
    if key.code == BackTab {
        return Some(match view {
            ViewMode::ListDetail => Action::FocusLeft,
            ViewMode::Grid => Action::FocusPrev,
        });
    }

    // Navigation (vim keys). Arrow keys scroll instead (below).
    if is_plain_char(&key, 'j') {
        return Some(Action::FocusNext);
    }
    if is_plain_char(&key, 'k') {
        return Some(Action::FocusPrev);
    }
    if is_plain_char(&key, 'h') {
        return Some(Action::FocusLeft);
    }
    if is_plain_char(&key, 'l') {
        return Some(Action::FocusRight);
    }
    if is_plain_char(&key, 'g') {
        // Simplified `gg`; see the module doc comment.
        return Some(Action::FocusFirst);
    }
    if is_plain_char(&key, 'G') {
        return Some(Action::FocusLast);
    }
    if let Char(c) = key.code {
        if key.modifiers.contains(KeyModifiers::ALT) && c.is_ascii_digit() && c != '0' {
            let idx = (c as u8 - b'1') as usize;
            return Some(Action::FocusIndex(idx));
        }
    }
    if is_plain_char(&key, '[') {
        return Some(Action::PrevPage);
    }
    if is_plain_char(&key, ']') {
        return Some(Action::NextPage);
    }

    // Scroll the focused job's output. Arrows are line-at-a-time in both
    // views; Ctrl page keys remain list/detail shortcuts for larger jumps.
    if key.code == Up {
        return Some(Action::ScrollUp(1));
    }
    if key.code == Down {
        return Some(Action::ScrollDown(1));
    }
    if key.code == Left {
        return Some(Action::ScrollUp(1));
    }
    if key.code == Right {
        return Some(Action::ScrollDown(1));
    }
    if view == ViewMode::ListDetail {
        if is_ctrl_char(&key, 'u') {
            return Some(Action::ScrollUp(HALF_PAGE_SCROLL));
        }
        if is_ctrl_char(&key, 'd') {
            return Some(Action::ScrollDown(HALF_PAGE_SCROLL));
        }
        if is_ctrl_char(&key, 'b') {
            return Some(Action::ScrollUp(FULL_PAGE_SCROLL));
        }
        if is_ctrl_char(&key, 'f') {
            return Some(Action::ScrollDown(FULL_PAGE_SCROLL));
        }
    }

    // Session actions (operate on the focused job).
    if key.code == Enter || is_plain_char(&key, 'a') {
        return Some(Action::Attach);
    }
    if is_plain_char(&key, 's') {
        return Some(Action::Stop);
    }
    if is_plain_char(&key, 'e') {
        return Some(Action::SendEof);
    }
    if is_plain_char(&key, 'd') {
        return Some(Action::Dismiss);
    }
    if is_plain_char(&key, 'D') {
        return Some(Action::DismissAllFinished);
    }
    if is_plain_char(&key, 'X') {
        return Some(Action::KillFocused);
    }
    if is_plain_char(&key, 'r') {
        return Some(Action::Rerun);
    }

    None
}

/// The help overlay only accepts the keys that close it.
fn map_help(key: KeyEvent) -> Option<Action> {
    if key.code == KeyCode::Esc || is_plain_char(&key, '?') || is_plain_char(&key, 'q') {
        Some(Action::CloseModal)
    } else {
        None
    }
}

/// Compose / add-command prompt. Ordinary characters are literal input here,
/// not shortcuts: e.g. `q` types the letter `q` rather than quitting.
fn map_compose(key: KeyEvent, line_empty: bool, launch_on_submit: bool) -> Option<Action> {
    if is_ctrl_char(&key, 's') {
        return Some(Action::ComposeSubmit);
    }
    if is_ctrl_char(&key, 'u') {
        return Some(Action::ComposeClear);
    }
    match key.code {
        // Any other Ctrl combo is not a defined compose binding; do not
        // insert a control character as if it were literal text.
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::ComposeChar(c))
        }
        KeyCode::Backspace => Some(Action::ComposeBackspace),
        KeyCode::Enter => {
            // "Enter on an empty line" is the same as Ctrl-s (DESIGN.md
            // section 7).
            Some(if line_empty {
                Action::ComposeSubmit
            } else {
                Action::ComposeCommit
            })
        }
        KeyCode::Esc => Some(if launch_on_submit {
            Action::Quit
        } else {
            Action::CloseModal
        }),
        _ => None,
    }
}

/// Confirmation modal: `y`/`Enter` confirms, `n`/`Esc` cancels.
fn map_confirm(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Some(Action::ConfirmYes),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(Action::ConfirmNo),
        _ => None,
    }
}

/// Exit prompt: `k`/`x`/`Esc`, per DESIGN.md section 9. Deliberately not
/// `y`/`n`; "yes" is ambiguous when the question is "keep or kill".
fn map_exit_prompt(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('k') | KeyCode::Char('K') => Some(Action::QuitKeepAll),
        KeyCode::Char('x') | KeyCode::Char('X') => Some(Action::QuitKillAll),
        KeyCode::Esc => Some(Action::CloseModal),
        _ => None,
    }
}

/// Error log overlay: scroll with the same keys as the detail pane, `Esc`
/// closes it. Not specified in exact detail by DESIGN.md section 7 (which
/// only says `!` opens it); this is the smallest reasonable binding set.
fn map_error_log(key: KeyEvent) -> Option<Action> {
    if key.code == KeyCode::Esc {
        return Some(Action::CloseModal);
    }
    if is_plain_char(&key, 'j') || key.code == KeyCode::Down {
        return Some(Action::ScrollDown(1));
    }
    if is_plain_char(&key, 'k') || key.code == KeyCode::Up {
        return Some(Action::ScrollUp(1));
    }
    if is_ctrl_char(&key, 'd') {
        return Some(Action::ScrollDown(HALF_PAGE_SCROLL));
    }
    if is_ctrl_char(&key, 'u') {
        return Some(Action::ScrollUp(HALF_PAGE_SCROLL));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn release(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release)
    }

    const NONE: ModalState = ModalState::None;

    // --- Release events never map, in any modal --------------------------

    #[test]
    fn release_events_never_map() {
        for view in [ViewMode::Grid, ViewMode::ListDetail] {
            assert_eq!(map(view, &NONE, release(KeyCode::Char('j'))), None);
            assert_eq!(map(view, &ModalState::Help, release(KeyCode::Esc)), None);
        }
    }

    // --- Global bindings, both view modes ---------------------------------

    #[test]
    fn global_bindings_work_in_both_views() {
        for view in [ViewMode::Grid, ViewMode::ListDetail] {
            assert_eq!(
                map(view, &NONE, key(KeyCode::Char('?'))),
                Some(Action::OpenHelp)
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Char('v'))),
                Some(Action::ToggleView)
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Char('1'))),
                Some(Action::SetView(ViewMode::Grid))
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Char('2'))),
                Some(Action::SetView(ViewMode::ListDetail))
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Char('n'))),
                Some(Action::OpenCompose)
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Char('!'))),
                Some(Action::OpenErrorLog)
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Char('q'))),
                Some(Action::Quit)
            );
            assert_eq!(
                map(
                    view,
                    &NONE,
                    key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL)
                ),
                Some(Action::Quit)
            );
            assert_eq!(map(view, &NONE, key(KeyCode::Esc)), None);
        }
    }

    // --- Navigation ---------------------------------------------------

    #[test]
    fn navigation_bindings() {
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('j'))),
            Some(Action::FocusNext)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('k'))),
            Some(Action::FocusPrev)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('h'))),
            Some(Action::FocusLeft)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('l'))),
            Some(Action::FocusRight)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('g'))),
            Some(Action::FocusFirst)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('G'))),
            Some(Action::FocusLast)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('['))),
            Some(Action::PrevPage)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char(']'))),
            Some(Action::NextPage)
        );
    }

    #[test]
    fn tab_and_shift_tab_switch_panels() {
        // Grid: cycle job focus. List/detail: list <-> detail pane.
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Tab)),
            Some(Action::FocusNext)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::BackTab)),
            Some(Action::FocusPrev)
        );
        assert_eq!(
            map(ViewMode::ListDetail, &NONE, key(KeyCode::Tab)),
            Some(Action::FocusRight)
        );
        assert_eq!(
            map(ViewMode::ListDetail, &NONE, key(KeyCode::BackTab)),
            Some(Action::FocusLeft)
        );
    }

    #[test]
    fn h_l_bindings_are_the_same_action_regardless_of_view() {
        // app.rs reinterprets these as pane switches in ListDetail; keymap
        // itself emits the same Action either way (see module doc comment).
        for view in [ViewMode::Grid, ViewMode::ListDetail] {
            assert_eq!(
                map(view, &NONE, key(KeyCode::Char('h'))),
                Some(Action::FocusLeft)
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Char('l'))),
                Some(Action::FocusRight)
            );
        }
    }

    #[test]
    fn arrow_keys_scroll_in_both_views() {
        for view in [ViewMode::Grid, ViewMode::ListDetail] {
            assert_eq!(
                map(view, &NONE, key(KeyCode::Up)),
                Some(Action::ScrollUp(1))
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Down)),
                Some(Action::ScrollDown(1))
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Left)),
                Some(Action::ScrollUp(1))
            );
            assert_eq!(
                map(view, &NONE, key(KeyCode::Right)),
                Some(Action::ScrollDown(1))
            );
        }
    }

    #[test]
    fn alt_digit_focuses_index() {
        assert_eq!(
            map(
                ViewMode::Grid,
                &NONE,
                key_mod(KeyCode::Char('1'), KeyModifiers::ALT)
            ),
            Some(Action::FocusIndex(0))
        );
        assert_eq!(
            map(
                ViewMode::Grid,
                &NONE,
                key_mod(KeyCode::Char('9'), KeyModifiers::ALT)
            ),
            Some(Action::FocusIndex(8))
        );
        // Alt-0 is not a defined binding (there is no job 0).
        assert_eq!(
            map(
                ViewMode::Grid,
                &NONE,
                key_mod(KeyCode::Char('0'), KeyModifiers::ALT)
            ),
            None
        );
    }

    #[test]
    fn plain_digit_is_view_switch_not_focus_index() {
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('1'))),
            Some(Action::SetView(ViewMode::Grid))
        );
    }

    // --- Scrolling is list/detail only ------------------------------------

    #[test]
    fn scroll_bindings_only_in_list_detail() {
        let ctrl_u = key_mod(KeyCode::Char('u'), KeyModifiers::CONTROL);
        let ctrl_d = key_mod(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let ctrl_b = key_mod(KeyCode::Char('b'), KeyModifiers::CONTROL);
        let ctrl_f = key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL);

        assert_eq!(
            map(ViewMode::ListDetail, &NONE, ctrl_u),
            Some(Action::ScrollUp(HALF_PAGE_SCROLL))
        );
        assert_eq!(
            map(ViewMode::ListDetail, &NONE, ctrl_d),
            Some(Action::ScrollDown(HALF_PAGE_SCROLL))
        );
        assert_eq!(
            map(ViewMode::ListDetail, &NONE, ctrl_b),
            Some(Action::ScrollUp(FULL_PAGE_SCROLL))
        );
        assert_eq!(
            map(ViewMode::ListDetail, &NONE, ctrl_f),
            Some(Action::ScrollDown(FULL_PAGE_SCROLL))
        );

        assert_eq!(map(ViewMode::Grid, &NONE, ctrl_u), None);
        assert_eq!(map(ViewMode::Grid, &NONE, ctrl_d), None);
        assert_eq!(map(ViewMode::Grid, &NONE, ctrl_b), None);
        assert_eq!(map(ViewMode::Grid, &NONE, ctrl_f), None);
    }

    // --- Session actions ----------------------------------------------

    #[test]
    fn session_action_bindings() {
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Enter)),
            Some(Action::Attach)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('a'))),
            Some(Action::Attach)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('s'))),
            Some(Action::Stop)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('e'))),
            Some(Action::SendEof)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('d'))),
            Some(Action::Dismiss)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('D'))),
            Some(Action::DismissAllFinished)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('X'))),
            Some(Action::KillFocused)
        );
        assert_eq!(
            map(ViewMode::Grid, &NONE, key(KeyCode::Char('r'))),
            Some(Action::Rerun)
        );
    }

    // --- Keys that map to nothing ------------------------------------

    #[test]
    fn unbound_keys_map_to_nothing() {
        assert_eq!(map(ViewMode::Grid, &NONE, key(KeyCode::Char('z'))), None);
        assert_eq!(map(ViewMode::Grid, &NONE, key(KeyCode::F(5))), None);
        assert_eq!(map(ViewMode::Grid, &NONE, key(KeyCode::PageDown)), None);
    }

    // --- Modal: Toast is non-blocking --------------------------------

    #[test]
    fn toast_does_not_block_normal_keys() {
        assert_eq!(
            map(ViewMode::Grid, &ModalState::Toast, key(KeyCode::Char('j'))),
            Some(Action::FocusNext)
        );
    }

    // --- Modal: Help ----------------------------------------------------

    #[test]
    fn help_modal_only_accepts_close_keys() {
        let help = ModalState::Help;
        assert_eq!(
            map(ViewMode::Grid, &help, key(KeyCode::Esc)),
            Some(Action::CloseModal)
        );
        assert_eq!(
            map(ViewMode::Grid, &help, key(KeyCode::Char('?'))),
            Some(Action::CloseModal)
        );
        assert_eq!(
            map(ViewMode::Grid, &help, key(KeyCode::Char('q'))),
            Some(Action::CloseModal)
        );
        // Navigation is not live while help is open.
        assert_eq!(map(ViewMode::Grid, &help, key(KeyCode::Char('j'))), None);
    }

    // --- Modal: Compose --------------------------------------------------

    #[test]
    fn compose_ordinary_chars_are_literal_not_shortcuts() {
        // The team's own example: `q` must type a literal 'q', not quit.
        let compose = ModalState::Compose {
            line_empty: false,
            launch_on_submit: false,
        };
        assert_eq!(
            map(ViewMode::Grid, &compose, key(KeyCode::Char('q'))),
            Some(Action::ComposeChar('q'))
        );
        assert_eq!(
            map(ViewMode::Grid, &compose, key(KeyCode::Char('s'))),
            Some(Action::ComposeChar('s'))
        );
        assert_eq!(
            map(ViewMode::Grid, &compose, key(KeyCode::Char('j'))),
            Some(Action::ComposeChar('j'))
        );
    }

    #[test]
    fn compose_ctrl_s_submits_and_ctrl_u_clears() {
        let compose = ModalState::Compose {
            line_empty: false,
            launch_on_submit: false,
        };
        assert_eq!(
            map(
                ViewMode::Grid,
                &compose,
                key_mod(KeyCode::Char('s'), KeyModifiers::CONTROL)
            ),
            Some(Action::ComposeSubmit)
        );
        assert_eq!(
            map(
                ViewMode::Grid,
                &compose,
                key_mod(KeyCode::Char('u'), KeyModifiers::CONTROL)
            ),
            Some(Action::ComposeClear)
        );
    }

    #[test]
    fn compose_enter_commits_unless_line_is_empty() {
        let non_empty = ModalState::Compose {
            line_empty: false,
            launch_on_submit: false,
        };
        let empty = ModalState::Compose {
            line_empty: true,
            launch_on_submit: false,
        };
        assert_eq!(
            map(ViewMode::Grid, &non_empty, key(KeyCode::Enter)),
            Some(Action::ComposeCommit)
        );
        assert_eq!(
            map(ViewMode::Grid, &empty, key(KeyCode::Enter)),
            Some(Action::ComposeSubmit)
        );
    }

    #[test]
    fn compose_backspace_is_always_the_same_action() {
        // app.rs decides whether that means "delete a char" or "pop the
        // last queued command" based on its own line-empty state.
        let compose = ModalState::Compose {
            line_empty: true,
            launch_on_submit: false,
        };
        assert_eq!(
            map(ViewMode::Grid, &compose, key(KeyCode::Backspace)),
            Some(Action::ComposeBackspace)
        );
    }

    #[test]
    fn compose_esc_quits_on_bare_launch_else_cancels() {
        let bare_launch = ModalState::Compose {
            line_empty: true,
            launch_on_submit: true,
        };
        let mid_run = ModalState::Compose {
            line_empty: true,
            launch_on_submit: false,
        };
        assert_eq!(
            map(ViewMode::Grid, &bare_launch, key(KeyCode::Esc)),
            Some(Action::Quit)
        );
        assert_eq!(
            map(ViewMode::Grid, &mid_run, key(KeyCode::Esc)),
            Some(Action::CloseModal)
        );
    }

    #[test]
    fn compose_undefined_ctrl_combo_is_not_inserted_as_text() {
        let compose = ModalState::Compose {
            line_empty: false,
            launch_on_submit: false,
        };
        assert_eq!(
            map(
                ViewMode::Grid,
                &compose,
                key_mod(KeyCode::Char('x'), KeyModifiers::CONTROL)
            ),
            None
        );
    }

    // --- Modal: Confirm --------------------------------------------------

    #[test]
    fn confirm_modal_bindings() {
        let confirm = ModalState::Confirm;
        assert_eq!(
            map(ViewMode::Grid, &confirm, key(KeyCode::Char('y'))),
            Some(Action::ConfirmYes)
        );
        assert_eq!(
            map(ViewMode::Grid, &confirm, key(KeyCode::Enter)),
            Some(Action::ConfirmYes)
        );
        assert_eq!(
            map(ViewMode::Grid, &confirm, key(KeyCode::Char('n'))),
            Some(Action::ConfirmNo)
        );
        assert_eq!(
            map(ViewMode::Grid, &confirm, key(KeyCode::Esc)),
            Some(Action::ConfirmNo)
        );
        assert_eq!(map(ViewMode::Grid, &confirm, key(KeyCode::Char('z'))), None);
    }

    // --- Modal: ExitPrompt -------------------------------------------

    #[test]
    fn exit_prompt_bindings() {
        let exit_prompt = ModalState::ExitPrompt;
        assert_eq!(
            map(ViewMode::Grid, &exit_prompt, key(KeyCode::Char('k'))),
            Some(Action::QuitKeepAll)
        );
        assert_eq!(
            map(ViewMode::Grid, &exit_prompt, key(KeyCode::Char('x'))),
            Some(Action::QuitKillAll)
        );
        assert_eq!(
            map(ViewMode::Grid, &exit_prompt, key(KeyCode::Esc)),
            Some(Action::CloseModal)
        );
        // Exit prompt deliberately does not use y/n.
        assert_eq!(
            map(ViewMode::Grid, &exit_prompt, key(KeyCode::Char('y'))),
            None
        );
        assert_eq!(
            map(ViewMode::Grid, &exit_prompt, key(KeyCode::Char('n'))),
            None
        );
    }

    // --- Modal: ErrorLog --------------------------------------------

    #[test]
    fn error_log_modal_bindings() {
        let error_log = ModalState::ErrorLog;
        assert_eq!(
            map(ViewMode::Grid, &error_log, key(KeyCode::Esc)),
            Some(Action::CloseModal)
        );
        assert_eq!(
            map(ViewMode::Grid, &error_log, key(KeyCode::Char('j'))),
            Some(Action::ScrollDown(1))
        );
        assert_eq!(
            map(ViewMode::Grid, &error_log, key(KeyCode::Char('k'))),
            Some(Action::ScrollUp(1))
        );
        assert_eq!(
            map(
                ViewMode::Grid,
                &error_log,
                key_mod(KeyCode::Char('d'), KeyModifiers::CONTROL)
            ),
            Some(Action::ScrollDown(HALF_PAGE_SCROLL))
        );
        assert_eq!(
            map(
                ViewMode::Grid,
                &error_log,
                key_mod(KeyCode::Char('u'), KeyModifiers::CONTROL)
            ),
            Some(Action::ScrollUp(HALF_PAGE_SCROLL))
        );
    }
}
