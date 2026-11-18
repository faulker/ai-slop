//! Screen-space touch / thumb HUD geometry and hit tests.
//!
//! Pure `f32` rects only — no macroquad types — so layouts can be unit-tested.

/// Max seed length (matches the title text field).
pub const SEED_MAX: usize = 48;

/// Axis-aligned rectangle in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Creates a rect from origin and size.
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// True if `(px, py)` lies inside this rect (half-open on the max edges).
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    /// Center point of the rect.
    #[allow(dead_code)] // used by unit tests; handy for callers
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

/// Returns true if `c` may be appended to a seed string.
pub fn seed_char_allowed(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' '
}

/// Appends `c` when under the length cap and in the allowed set.
///
/// Returns whether the seed changed.
pub fn try_append_seed(seed: &mut String, c: char) -> bool {
    if seed.len() >= SEED_MAX || !seed_char_allowed(c) {
        return false;
    }
    seed.push(c);
    true
}

/// Race-control actions exposed as on-screen buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaceAction {
    Accelerate,
    Brake,
    SteerLeft,
    SteerRight,
    PipeLeft,
    PipeRight,
    Laser,
}

/// Held race-button snapshot (OR of all active pointers).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaceHolds {
    pub accelerate: bool,
    pub brake: bool,
    pub steer_left: bool,
    pub steer_right: bool,
    pub pipe_left: bool,
    pub pipe_right: bool,
    pub laser: bool,
}

/// Screen-space race thumb buttons.
#[derive(Clone, Debug)]
pub struct RaceButtons {
    pub gas: Rect,
    pub brake: Rect,
    pub left: Rect,
    pub right: Rect,
    pub pipe_l: Rect,
    pub pipe_r: Rect,
    pub laser: Rect,
}

impl RaceButtons {
    /// Iterates all race button rects with their action and short label.
    pub fn iter(&self) -> impl Iterator<Item = (RaceAction, Rect, &'static str)> + '_ {
        [
            (RaceAction::Accelerate, self.gas, "GAS"),
            (RaceAction::Brake, self.brake, "BRAKE"),
            (RaceAction::SteerLeft, self.left, "LEFT"),
            (RaceAction::SteerRight, self.right, "RIGHT"),
            (RaceAction::PipeLeft, self.pipe_l, "PIPE L"),
            (RaceAction::PipeRight, self.pipe_r, "PIPE R"),
            (RaceAction::Laser, self.laser, "LASER"),
        ]
        .into_iter()
    }
}

/// Lays out thumb-sized race controls along the bottom edges.
///
/// Prefers ~72px buttons; shrinks padding/gaps before shrinking buttons below 64.
pub fn layout_race_buttons(sw: f32, sh: f32) -> RaceButtons {
    let sw = sw.max(1.0);
    let sh = sh.max(1.0);
    let short = sw.min(sh);

    let mut pad = (short * 0.035).clamp(10.0, 28.0);
    let mut gap = (short * 0.025).clamp(10.0, 20.0);
    let mut btn = (short * 0.12).clamp(72.0, 100.0);

    // Vertical room for the tallest stack (right: 3 buttons).
    let stack_h = |btn: f32, gap: f32, pad: f32| pad + 3.0 * btn + 2.0 * gap;
    if stack_h(btn, gap, pad) > sh * 0.5 {
        let target = sh * 0.5;
        let excess = stack_h(btn, gap, pad) - target;
        let pad_cut = (pad - 6.0).min(excess * 0.4).max(0.0);
        pad -= pad_cut;
        let gap_cut = (gap - 6.0).min(excess * 0.3).max(0.0);
        gap -= gap_cut;
        if stack_h(btn, gap, pad) > target {
            btn = ((target - pad - 2.0 * gap) / 3.0).max(64.0);
        }
    }

    // Resolve horizontal overlap between center steer and side clusters.
    for _ in 0..8 {
        let left_cluster_r = pad + btn;
        let right_cluster_l = sw - pad - btn;
        let pair_w = 2.0 * btn + gap;
        let left_x = (sw - pair_w) * 0.5;
        let right_x = left_x + btn + gap;
        let overlaps_left = left_x < left_cluster_r + gap;
        let overlaps_right = right_x + btn > right_cluster_l - gap;
        if !overlaps_left && !overlaps_right {
            break;
        }
        if gap > 8.0 {
            gap = (gap - 2.0).max(8.0);
        } else if pad > 8.0 {
            pad = (pad - 2.0).max(8.0);
        } else if btn > 64.0 {
            btn = (btn - 4.0).max(64.0);
        } else {
            break;
        }
    }

    let bottom = sh - pad;
    let gas = Rect::new(pad, bottom - 2.0 * btn - gap, btn, btn);
    let brake = Rect::new(pad, bottom - btn, btn, btn);

    let pair_w = 2.0 * btn + gap;
    let left_x = (sw - pair_w) * 0.5;
    let steer_y = bottom - btn;
    let left = Rect::new(left_x, steer_y, btn, btn);
    let right = Rect::new(left_x + btn + gap, steer_y, btn, btn);

    let rx = sw - pad - btn;
    let pipe_l = Rect::new(rx, bottom - 3.0 * btn - 2.0 * gap, btn, btn);
    let pipe_r = Rect::new(rx, bottom - 2.0 * btn - gap, btn, btn);
    let laser = Rect::new(rx, bottom - btn, btn, btn);

    RaceButtons {
        gas,
        brake,
        left,
        right,
        pipe_l,
        pipe_r,
        laser,
    }
}

/// Returns the race action under `(x, y)`, if any.
pub fn race_action_at(layout: &RaceButtons, x: f32, y: f32) -> Option<RaceAction> {
    for (action, rect, _) in layout.iter() {
        if rect.contains(x, y) {
            return Some(action);
        }
    }
    None
}

/// ORs holds for every point that lands on a race button.
pub fn race_holds(layout: &RaceButtons, points: &[(f32, f32)]) -> RaceHolds {
    let mut held = RaceHolds::default();
    for &(x, y) in points {
        match race_action_at(layout, x, y) {
            Some(RaceAction::Accelerate) => held.accelerate = true,
            Some(RaceAction::Brake) => held.brake = true,
            Some(RaceAction::SteerLeft) => held.steer_left = true,
            Some(RaceAction::SteerRight) => held.steer_right = true,
            Some(RaceAction::PipeLeft) => held.pipe_left = true,
            Some(RaceAction::PipeRight) => held.pipe_right = true,
            Some(RaceAction::Laser) => held.laser = true,
            None => {}
        }
    }
    held
}

/// True on phone/tablet builds. Those use the OS keyboard for the seed.
///
/// Desktop builds type with the hardware keyboard and skip the touch chrome.
pub fn is_mobile_build() -> bool {
    cfg!(any(target_os = "android", target_os = "ios"))
}

/// Title-screen touch hits (edge-triggered by the caller).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleHit {
    Start,
    Edit,
    Keys,
}

/// Title seed label and nav buttons in screen space.
///
/// No character keys. Typing goes through the hardware keyboard on desktop
/// and the OS keyboard on mobile.
#[derive(Clone, Debug)]
pub struct TitleLayout {
    pub seed_label: Rect,
    pub start: Rect,
    pub edit: Rect,
    pub keys: Rect,
}

/// Lays out the seed label and START / EDIT / KEYS in the upper half.
///
/// The lower half stays clear so a native mobile keyboard can cover it.
pub fn layout_title(sw: f32, sh: f32) -> TitleLayout {
    let sw = sw.max(1.0);
    let sh = sh.max(1.0);
    let pad = (sw.min(sh) * 0.02).clamp(6.0, 16.0);
    let gap = (sw.min(sh) * 0.012).clamp(4.0, 10.0);

    let seed_h = (sh * 0.07).clamp(40.0, 64.0);
    let seed_label = Rect::new(pad, sh * 0.22, sw - 2.0 * pad, seed_h);

    let nav_h = (sh * 0.08).clamp(52.0, 76.0);
    let nav_y = seed_label.y + seed_label.h + gap;
    let nav_w = (sw - 2.0 * pad - 2.0 * gap) / 3.0;
    let start = Rect::new(pad, nav_y, nav_w, nav_h);
    let edit = Rect::new(pad + nav_w + gap, nav_y, nav_w, nav_h);
    let keys = Rect::new(pad + 2.0 * (nav_w + gap), nav_y, nav_w, nav_h);

    TitleLayout {
        seed_label,
        start,
        edit,
        keys,
    }
}

/// START / KEYS / EDIT inside the title frame (internal pixels, not the window).
///
/// Letter keys stay free for the seed. These buttons are what you click.
pub fn layout_title_menu(frame_w: f32, frame_h: f32) -> TitleLayout {
    let frame_w = frame_w.max(1.0);
    let frame_h = frame_h.max(1.0);
    let gap = (frame_w * 0.02).clamp(6.0, 10.0);
    let x0 = frame_w * 0.06;
    let w = (frame_w - 2.0 * x0 - 2.0 * gap) / 3.0;
    let h = (frame_h * 0.15).clamp(28.0, 36.0);
    let y = frame_h * 0.68;
    TitleLayout {
        seed_label: Rect::new(x0, frame_h * 0.42, frame_w - 2.0 * x0, h),
        start: Rect::new(x0, y, w, h),
        keys: Rect::new(x0 + w + gap, y, w, h),
        edit: Rect::new(x0 + 2.0 * (w + gap), y, w, h),
    }
}

/// Hit-tests one point against the title nav buttons.
pub fn title_hit_at(layout: &TitleLayout, x: f32, y: f32) -> Option<TitleHit> {
    if layout.start.contains(x, y) {
        return Some(TitleHit::Start);
    }
    if layout.edit.contains(x, y) {
        return Some(TitleHit::Edit);
    }
    if layout.keys.contains(x, y) {
        return Some(TitleHit::Keys);
    }
    None
}

/// Results-screen touch actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultsHit {
    Again,
    Title,
}

/// Results buttons in screen space.
#[derive(Clone, Copy, Debug)]
pub struct ResultsLayout {
    pub again: Rect,
    pub title: Rect,
}

/// Lays out AGAIN / TITLE near the bottom center.
pub fn layout_results(sw: f32, sh: f32) -> ResultsLayout {
    let sw = sw.max(1.0);
    let sh = sh.max(1.0);
    let short = sw.min(sh);
    let pad = (short * 0.04).clamp(12.0, 28.0);
    let gap = (short * 0.03).clamp(10.0, 20.0);
    let btn_h = (short * 0.12).clamp(64.0, 88.0);
    let btn_w = ((sw - 2.0 * pad - gap) * 0.5).clamp(120.0, 280.0);
    let y = sh - pad - btn_h;
    let total_w = 2.0 * btn_w + gap;
    let x0 = (sw - total_w) * 0.5;
    ResultsLayout {
        again: Rect::new(x0, y, btn_w, btn_h),
        title: Rect::new(x0 + btn_w + gap, y, btn_w, btn_h),
    }
}

/// Hit-tests results buttons.
pub fn results_hit_at(layout: &ResultsLayout, x: f32, y: f32) -> Option<ResultsHit> {
    if layout.again.contains(x, y) {
        Some(ResultsHit::Again)
    } else if layout.title.contains(x, y) {
        Some(ResultsHit::Title)
    } else {
        None
    }
}

/// Pause-screen touch actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseHit {
    Resume,
    Title,
}

/// Pause buttons in screen space.
#[derive(Clone, Copy, Debug)]
pub struct PauseLayout {
    pub resume: Rect,
    pub title: Rect,
}

/// Lays out RESUME / TITLE near the bottom center.
pub fn layout_pause(sw: f32, sh: f32) -> PauseLayout {
    let results = layout_results(sw, sh);
    PauseLayout {
        resume: results.again,
        title: results.title,
    }
}

/// Hit-tests pause buttons.
pub fn pause_hit_at(layout: &PauseLayout, x: f32, y: f32) -> Option<PauseHit> {
    if layout.resume.contains(x, y) {
        Some(PauseHit::Resume)
    } else if layout.title.contains(x, y) {
        Some(PauseHit::Title)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop_race() -> RaceButtons {
        layout_race_buttons(1280.0, 720.0)
    }

    #[test]
    fn gas_center_is_accelerate_not_brake() {
        let layout = desktop_race();
        let (x, y) = layout.gas.center();
        assert_eq!(race_action_at(&layout, x, y), Some(RaceAction::Accelerate));
        assert_ne!(race_action_at(&layout, x, y), Some(RaceAction::Brake));
        let holds = race_holds(&layout, &[(x, y)]);
        assert!(holds.accelerate);
        assert!(!holds.brake);
    }

    #[test]
    fn gap_between_gas_and_brake_is_empty() {
        let layout = desktop_race();
        let x = layout.gas.center().0;
        let y = layout.gas.y + layout.gas.h + (layout.brake.y - (layout.gas.y + layout.gas.h)) * 0.5;
        assert!(
            layout.brake.y > layout.gas.y + layout.gas.h,
            "expected a vertical gap between GAS and BRAKE"
        );
        assert_eq!(race_action_at(&layout, x, y), None);
    }

    #[test]
    fn no_two_buttons_claim_same_center() {
        let layout = desktop_race();
        let buttons: Vec<_> = layout.iter().collect();
        for (i, (a_action, a_rect, _)) in buttons.iter().enumerate() {
            let (cx, cy) = a_rect.center();
            for (j, (b_action, b_rect, _)) in buttons.iter().enumerate() {
                if i == j {
                    continue;
                }
                assert!(
                    !b_rect.contains(cx, cy),
                    "{a_action:?} center ({cx},{cy}) also inside {b_action:?}"
                );
            }
            assert_eq!(race_action_at(&layout, cx, cy), Some(*a_action));
        }
    }

    #[test]
    fn append_seed_respects_cap_and_charset() {
        let mut seed = String::new();
        assert!(try_append_seed(&mut seed, 'a'));
        assert_eq!(seed, "a");
        assert!(!try_append_seed(&mut seed, '!'));
        assert_eq!(seed, "a");
        assert!(!try_append_seed(&mut seed, '\n'));
        assert!(try_append_seed(&mut seed, '-'));
        assert!(try_append_seed(&mut seed, '_'));
        assert!(try_append_seed(&mut seed, ' '));
        assert!(try_append_seed(&mut seed, '9'));

        seed = "x".repeat(SEED_MAX);
        assert!(!try_append_seed(&mut seed, 'y'));
        assert_eq!(seed.len(), SEED_MAX);
    }

    #[test]
    fn title_start_hit() {
        let layout = layout_title(1280.0, 720.0);
        let (x, y) = layout.start.center();
        assert_eq!(title_hit_at(&layout, x, y), Some(TitleHit::Start));
        assert_eq!(
            title_hit_at(&layout, layout.edit.center().0, layout.edit.center().1),
            Some(TitleHit::Edit)
        );
        assert_eq!(
            title_hit_at(&layout, layout.keys.center().0, layout.keys.center().1),
            Some(TitleHit::Keys)
        );
    }

    #[test]
    fn title_menu_buttons_are_separate_clicks() {
        let layout = layout_title_menu(384.0, 216.0);
        assert_eq!(
            title_hit_at(&layout, layout.start.center().0, layout.start.center().1),
            Some(TitleHit::Start)
        );
        assert_eq!(
            title_hit_at(&layout, layout.keys.center().0, layout.keys.center().1),
            Some(TitleHit::Keys)
        );
        assert_eq!(
            title_hit_at(&layout, layout.edit.center().0, layout.edit.center().1),
            Some(TitleHit::Edit)
        );
        let gap_x = layout.start.x + layout.start.w + 1.0;
        let gap_y = layout.start.center().1;
        assert_eq!(title_hit_at(&layout, gap_x, gap_y), None);
        assert!(layout.start.y > layout.seed_label.y + layout.seed_label.h);
    }

    #[test]
    fn title_nav_stays_above_a_mobile_keyboard() {
        let layout = layout_title(390.0, 844.0);
        let bottom = layout.start.y + layout.start.h;
        assert!(
            bottom < 844.0 * 0.55,
            "nav bottom {bottom} would sit under the OS keyboard"
        );
    }

    #[test]
    fn results_and_pause_hits() {
        let r = layout_results(800.0, 600.0);
        assert_eq!(
            results_hit_at(&r, r.again.center().0, r.again.center().1),
            Some(ResultsHit::Again)
        );
        assert_eq!(
            results_hit_at(&r, r.title.center().0, r.title.center().1),
            Some(ResultsHit::Title)
        );
        let p = layout_pause(800.0, 600.0);
        assert_eq!(
            pause_hit_at(&p, p.resume.center().0, p.resume.center().1),
            Some(PauseHit::Resume)
        );
    }

    #[test]
    fn race_buttons_meet_min_size() {
        for &(sw, sh) in &[(1280.0, 720.0), (390.0, 844.0), (844.0, 390.0)] {
            let layout = layout_race_buttons(sw, sh);
            for (_, rect, label) in layout.iter() {
                assert!(
                    rect.w >= 64.0 - 0.01 && rect.h >= 64.0 - 0.01,
                    "{label} too small on {sw}x{sh}: {}x{}",
                    rect.w,
                    rect.h
                );
            }
        }
    }
}
