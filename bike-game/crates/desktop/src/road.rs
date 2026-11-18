//! Chase-camera road projection and the night-highway palette.
//!
//! Strips are nearest-first. Index 0 sits on the bottom edge of the screen
//! (the camera) and the last strip sits on the horizon. Gameplay `x` is in
//! meters; a 4 m half-width road fills `bottom_half_px` at the camera.

/// RGBA color, kept free of the renderer so projection tests stay pure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

/// One depth sample of the road, nearest-first.
#[derive(Clone, Copy, Debug)]
pub struct RoadStrip {
    pub y: f32,
    pub mid: f32,
    pub half: f32,
    /// World distance of this sample (meters).
    pub z: f32,
    pub index: i32,
}

/// Projection settings for the race view.
#[derive(Clone, Copy, Debug)]
pub struct ProjConfig {
    pub screen_w: f32,
    pub screen_h: f32,
    pub horizon_y: f32,
    /// Camera depth. Larger values keep the road wide farther ahead.
    pub cam_depth: f32,
    pub seg_len: f32,
    pub segments: usize,
    /// Pixel half-width of a 4 m road at the camera.
    pub bottom_half_px: f32,
    /// Scales the generator's small curvature values into a visible bend.
    pub curve_response: f32,
}

impl ProjConfig {
    /// Settings used by the 384×216 race view, scaled to `screen_w` × `screen_h`.
    pub fn game(screen_w: f32, screen_h: f32) -> Self {
        Self {
            screen_w,
            screen_h,
            horizon_y: screen_h * 0.36,
            cam_depth: 14.0,
            seg_len: 4.0,
            segments: 64,
            bottom_half_px: screen_w * 0.33,
            curve_response: 0.65,
        }
    }
}

/// How far behind the rider the chase camera sits, in meters.
///
/// That gap is the road you see below the bike, so a rival coming from behind
/// travels up the screen instead of hiding under the player sprite.
pub const CAMERA_BACK_M: f32 = 12.0;

/// Projects the road from the camera forward.
///
/// `world_half_at` and `curvature_at` are sampled in meters along `z`.
/// Strip 0 is the road under the camera (bottom of the screen, widest).
pub fn project_strips(
    cam_z: f32,
    cam_x: f32,
    world_half_at: impl Fn(f32) -> f32,
    curvature_at: impl Fn(f32) -> f32,
    cfg: &ProjConfig,
) -> Vec<RoadStrip> {
    let mut strips = Vec::with_capacity(cfg.segments);
    let mut heading = 0.0_f32;
    let mut lateral = 0.0_f32;
    let px_per_m_bottom = cfg.bottom_half_px / 4.0;

    for n in 0..cfg.segments {
        let z_ahead = n as f32 * cfg.seg_len;
        let z = cam_z + z_ahead;
        let scale = cfg.cam_depth / (cfg.cam_depth + z_ahead);
        let world_half = world_half_at(z).max(0.5);
        let half = cfg.bottom_half_px * (world_half / 4.0) * scale;
        let mid = cfg.screen_w * 0.5 + (lateral - cam_x) * px_per_m_bottom * scale;
        let y = cfg.horizon_y + scale * (cfg.screen_h - cfg.horizon_y);
        strips.push(RoadStrip {
            y,
            mid,
            half,
            z,
            index: n as i32,
        });

        let curv = curvature_at(z);
        heading += curv * cfg.curve_response * cfg.seg_len;
        lateral += heading * cfg.seg_len;
    }
    strips
}

/// Chase view: camera sits [`CAMERA_BACK_M`] behind the rider, and the rider stays centered.
///
/// Strips between the camera and the rider are the road behind the bike. Rivals
/// there draw lower on the screen and rise as they catch up.
pub fn project_chase(
    player_z: f32,
    player_x: f32,
    world_half_at: impl Fn(f32) -> f32 + Copy,
    curvature_at: impl Fn(f32) -> f32,
    cfg: &ProjConfig,
) -> Vec<RoadStrip> {
    let cam_z = player_z - CAMERA_BACK_M;
    let mut strips = project_strips(cam_z, player_x, world_half_at, curvature_at, cfg);
    let Some(strip) = strips
        .iter()
        .find(|s| player_z >= s.z && player_z < s.z + cfg.seg_len)
        .copied()
    else {
        return strips;
    };
    let half_road = world_half_at(strip.z).max(0.5);
    let nx = (player_x / half_road).clamp(-2.2, 2.2);
    let sx = strip.mid + nx * strip.half;
    let shift = cfg.screen_w * 0.5 - sx;
    for s in &mut strips {
        s.mid += shift;
    }
    strips
}

/// Outside edge of the red/white curb, as a multiple of the asphalt half-width.
pub const RUMBLE_OUTER: f32 = 1.14;

/// Two lane-divider positions on a 3-lane road, as fractions of half-width.
pub const LANE_DIVIDERS: [f32; 2] = [-1.0 / 3.0, 1.0 / 3.0];

/// One screen sample of a forward laser along the player's lane.
#[derive(Clone, Copy, Debug)]
pub struct BeamSample {
    pub x: f32,
    pub y: f32,
    /// Half-width of the beam in pixels at this depth.
    pub half: f32,
    /// World distance of this sample (meters).
    ///
    /// Drawing uses `y` and `half`. Tests read `z` to check the bolt stays in range.
    #[allow(dead_code)]
    pub z: f32,
}

/// Drawn bolt half-width in meters. Narrower than the hit test, so the shot
/// looks like a laser and does not cover the bikes in the lane.
pub const LASER_VISUAL_HALF_M: f32 = 0.16;

/// Pixel half-width of a beam that is `half_m` meters wide on this strip.
pub fn beam_half_px(strip: &RoadStrip, half_m: f32, road_half: f32) -> f32 {
    let road_half = road_half.max(0.5);
    (half_m.abs() / road_half * strip.half).clamp(1.0, 5.0)
}

/// Projects a laser straight up the screen from the bike nose.
///
/// `x` stays on `origin_x` even when the road bends. `y` and width still
/// shrink with distance so the bolt reaches toward the horizon. Samples stay
/// inside `range_m` and in front of the nose (smaller `y` is toward the horizon).
pub fn project_laser_beam(
    nose_z: f32,
    range_m: f32,
    half_m: f32,
    origin_x: f32,
    origin_y: f32,
    road_half_at: impl Fn(f32) -> f32,
    cfg: &ProjConfig,
) -> Vec<BeamSample> {
    let cam_z = nose_z - CAMERA_BACK_M;
    let strips = project_strips(cam_z, 0.0, &road_half_at, |_| 0.0, cfg);
    let mut ahead = Vec::new();
    for strip in &strips {
        let z_ahead = strip.z - nose_z;
        if z_ahead <= 0.5 || z_ahead > range_m {
            continue;
        }
        // Larger y is closer to the camera. Skip samples still behind the nose.
        if strip.y >= origin_y - 1.0 {
            continue;
        }
        let road_half = road_half_at(strip.z);
        ahead.push(BeamSample {
            x: origin_x,
            y: strip.y,
            half: beam_half_px(strip, half_m, road_half),
            z: strip.z,
        });
    }

    if ahead.is_empty() {
        ahead.push(BeamSample {
            x: origin_x,
            y: origin_y - 40.0,
            half: 2.5,
            z: cam_z + range_m.max(0.0),
        });
    }

    let muzzle_half = ahead[0].half;
    let mut samples = Vec::with_capacity(ahead.len() + 1);
    samples.push(BeamSample {
        x: origin_x,
        y: origin_y,
        half: muzzle_half,
        z: nose_z,
    });
    samples.extend(ahead);
    samples
}

/// True on every other strip, so lane paint reads as dashes.
pub fn lane_dash(index: i32) -> bool {
    index % 2 == 0
}

/// Sky color from the top of the screen (`t` = 0) to the horizon (`t` = 1).
///
/// Stays night-dark. The horizon picks up a little city glow so the skyline reads.
pub fn sky(t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    lerp_rgb((8, 6, 20), (36, 16, 48), t, 255)
}

pub fn mountain_far() -> Rgba {
    Rgba(18, 14, 32, 255)
}

pub fn mountain_near() -> Rgba {
    Rgba(12, 10, 22, 255)
}

pub fn city(index: i32) -> Rgba {
    match index % 3 {
        0 => Rgba(16, 18, 36, 255),
        1 => Rgba(22, 14, 36, 255),
        _ => Rgba(14, 22, 36, 255),
    }
}

/// Neon speck in a distant window. `which` picks pink or cyan.
pub fn window(which: i32) -> Rgba {
    if which % 2 == 0 {
        Rgba(255, 64, 112, 255)
    } else {
        Rgba(64, 220, 220, 255)
    }
}

pub fn grass(index: i32) -> Rgba {
    if index % 2 == 0 {
        Rgba(22, 40, 32, 255)
    } else {
        Rgba(12, 26, 20, 255)
    }
}

pub fn asphalt(index: i32) -> Rgba {
    if index % 2 == 0 {
        Rgba(38, 40, 50, 255)
    } else {
        Rgba(26, 28, 36, 255)
    }
}

/// Red and pale curb stripes. Neighboring strips flip so the curb scrolls.
pub fn rumble(index: i32) -> Rgba {
    if index % 2 == 0 {
        Rgba(196, 32, 48, 255)
    } else {
        Rgba(210, 206, 196, 255)
    }
}

pub fn lane_mark() -> Rgba {
    Rgba(220, 214, 180, 255)
}

pub fn rail() -> Rgba {
    Rgba(150, 156, 168, 255)
}

pub fn rail_post() -> Rgba {
    Rgba(28, 30, 38, 255)
}

pub fn tree_crown() -> Rgba {
    Rgba(16, 48, 32, 255)
}

pub fn tree_trunk() -> Rgba {
    Rgba(48, 32, 28, 255)
}

pub fn lamp_glow() -> Rgba {
    Rgba(255, 72, 128, 255)
}

/// Screen spot for a bike that is behind the chase camera, rising into view from the bottom.
#[derive(Clone, Copy, Debug)]
pub struct TrailingPlacement {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

/// Places a rival that is `behind_m` meters behind the camera.
///
/// `rel_x` is the bike's world x minus the camera x. Returns `None` when the
/// bike is ahead of the camera or too far back to show beside the rider.
pub fn trailing_placement(
    behind_m: f32,
    rel_x: f32,
    road_half: f32,
    screen_w: f32,
    screen_h: f32,
    bottom_half_px: f32,
) -> Option<TrailingPlacement> {
    if behind_m <= 0.0 || behind_m > 14.0 {
        return None;
    }
    let t = 1.0 - behind_m / 14.0;
    let road_half = road_half.max(0.5);
    let nx = (rel_x / road_half).clamp(-1.5, 1.5);
    Some(TrailingPlacement {
        x: screen_w * 0.5 + nx * bottom_half_px,
        // Larger y is lower on the screen. Far bikes start below the edge and rise.
        // Scale tops out at 1, the same size as the player bike.
        y: screen_h + 18.0 - t * 40.0,
        scale: 0.55 + t * 0.45,
    })
}

fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32, alpha: u8) -> Rgba {
    let ch = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Rgba(ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(_z: f32) -> f32 {
        4.0
    }

    fn straight(_z: f32) -> f32 {
        0.0
    }

    #[test]
    fn nearest_strip_is_widest_and_lowest() {
        let cfg = ProjConfig::game(384.0, 216.0);
        let strips = project_strips(0.0, 0.0, flat, straight, &cfg);
        let near = strips.first().unwrap();
        let far = strips.last().unwrap();
        assert!(near.y > far.y, "camera strip should sit below the horizon");
        assert!(near.half > far.half * 4.0);
        assert!((near.y - cfg.screen_h).abs() < 0.01);
        assert!(far.y < cfg.horizon_y + 20.0);
    }

    #[test]
    fn centered_camera_on_a_straight_keeps_the_road_centered() {
        let cfg = ProjConfig::game(384.0, 216.0);
        let strips = project_strips(10.0, 0.0, flat, straight, &cfg);
        for s in &strips {
            assert!(
                (s.mid - cfg.screen_w * 0.5).abs() < 0.01,
                "mid {} drifted",
                s.mid
            );
        }
    }

    #[test]
    fn player_offset_slides_the_road_the_other_way() {
        let cfg = ProjConfig::game(384.0, 216.0);
        let strips = project_strips(0.0, 2.0, flat, straight, &cfg);
        let near = &strips[0];
        // 2 m to the right puts the road center left of the screen center.
        assert!(near.mid < cfg.screen_w * 0.5 - 20.0);
    }

    #[test]
    fn laser_on_a_straight_road_stays_in_front_of_the_bike() {
        let cfg = ProjConfig::game(384.0, 216.0);
        // Nose sits where the bike sprite tops out on the 216-tall view.
        let nose_y = 130.0;
        let samples = project_laser_beam(0.0, 120.0, 1.8, cfg.screen_w * 0.5, nose_y, flat, &cfg);
        assert!(samples.len() >= 2);
        assert!((samples[0].x - cfg.screen_w * 0.5).abs() < 0.01);
        assert!((samples[0].y - nose_y).abs() < 0.01);
        for s in &samples {
            assert!(
                (s.x - cfg.screen_w * 0.5).abs() < 1.0,
                "beam drifted off the bike to {}",
                s.x
            );
            assert!(s.z <= 120.0 + 0.01);
        }
        assert!(samples.last().unwrap().y < nose_y - 15.0);
        assert!(samples.last().unwrap().z > 80.0);
    }

    #[test]
    fn chase_lateral_matches_the_road_bend() {
        let cfg = ProjConfig::game(384.0, 216.0);
        assert!((cfg.seg_len - bike_sim::CHASE_SEG_LEN).abs() < 1e-6);
        assert!((cfg.curve_response - bike_sim::CHASE_CURVE_RESPONSE).abs() < 1e-6);
        let strips = project_strips(10.0, 0.0, flat, |_| 0.006, &cfg);
        let strip = &strips[12];
        let lateral = bike_sim::chase_lateral(10.0, strip.z, |_| 0.006);
        let z_ahead = strip.z - 10.0;
        let scale = cfg.cam_depth / (cfg.cam_depth + z_ahead);
        let px_per_m = cfg.bottom_half_px / 4.0;
        let expected_mid = cfg.screen_w * 0.5 + lateral * px_per_m * scale;
        assert!(
            (strip.mid - expected_mid).abs() < 0.05,
            "mid {} vs lateral prediction {}",
            strip.mid,
            expected_mid
        );
    }

    #[test]
    fn laser_stays_straight_when_the_road_bends() {
        let cfg = ProjConfig::game(384.0, 216.0);
        let nose_y = 130.0;
        let samples = project_laser_beam(0.0, 120.0, 1.8, cfg.screen_w * 0.5, nose_y, flat, &cfg);
        assert!(samples.len() >= 2);
        for s in &samples {
            assert!(
                (s.x - cfg.screen_w * 0.5).abs() < 0.01,
                "straight bolt drifted to {}",
                s.x
            );
        }
        assert!(samples.last().unwrap().y < nose_y);

        // The road itself still bends. The bolt does not follow it.
        let strips = project_strips(0.0, 0.0, flat, |_| 0.006, &cfg);
        assert!(strips[24].mid > strips[0].mid + 15.0);
    }

    #[test]
    fn laser_bolt_stays_narrower_than_a_bike() {
        let cfg = ProjConfig::game(384.0, 216.0);
        let samples = project_laser_beam(
            0.0,
            120.0,
            LASER_VISUAL_HALF_M,
            cfg.screen_w * 0.5,
            130.0,
            flat,
            &cfg,
        );
        let widest = samples.iter().fold(0.0_f32, |w, s| w.max(s.half));
        assert!(
            widest <= 5.0,
            "beam half {widest}px would cover the lane"
        );
    }

    #[test]
    fn positive_curvature_bends_the_road_right_ahead() {
        let cfg = ProjConfig::game(384.0, 216.0);
        let strips = project_strips(0.0, 0.0, flat, |_| 0.006, &cfg);
        let near = strips[0].mid;
        let ahead = strips[24].mid;
        assert!(
            ahead > near + 15.0,
            "expected a visible right bend, near {near} ahead {ahead}"
        );
    }

    #[test]
    fn a_gang_down_the_road_sits_on_a_subpixel_band() {
        let cfg = ProjConfig::game(384.0, 216.0);
        let strips = project_strips(0.0, 0.0, flat, straight, &cfg);
        let z = 140.0;
        let idx = strips
            .iter()
            .position(|s| z >= s.z && z < s.z + cfg.seg_len)
            .expect("140m is inside the chase view");
        let band = strips[idx].y - strips[idx + 1].y;
        assert!(
            band < 1.0,
            "band {band}px is under a pixel, so sprites cannot use the road-quad cutoff"
        );
    }

    #[test]
    fn lane_dividers_sit_inside_the_asphalt_and_rumble_outside() {
        let half = 40.0_f32;
        for f in LANE_DIVIDERS {
            assert!(f.abs() < 1.0);
            assert!(f.abs() * half < half);
        }
        assert!(RUMBLE_OUTER > 1.0);
        assert!(half * RUMBLE_OUTER > half);
    }

    #[test]
    fn rumble_and_grass_alternate() {
        assert_ne!(rumble(0), rumble(1));
        assert_ne!(grass(0), grass(1));
        assert_ne!(asphalt(0), asphalt(1));
        assert!(lane_dash(0));
        assert!(!lane_dash(1));
    }

    #[test]
    fn sky_stays_dark_and_lifts_at_the_horizon() {
        let top = sky(0.0);
        let horizon = sky(1.0);
        assert!(top.0 < 24 && top.1 < 24 && top.2 < 40);
        assert!(horizon.0 > top.0, "horizon should pick up city glow");
        assert!(horizon.0 < 80 && horizon.1 < 48 && horizon.2 < 80);
    }

    #[test]
    fn a_bike_behind_the_rider_sits_lower_on_the_chase_road() {
        let cfg = ProjConfig::game(384.0, 216.0);
        let player_z = 40.0;
        let strips = project_chase(player_z, 0.0, flat, straight, &cfg);
        let rider = strips
            .iter()
            .find(|s| player_z >= s.z && player_z < s.z + cfg.seg_len)
            .expect("rider is on the chase road");
        let behind_z = player_z - 6.0;
        let behind = strips
            .iter()
            .find(|s| behind_z >= s.z && behind_z < s.z + cfg.seg_len)
            .expect("a bike 6m back is still in front of the camera");
        assert!(
            behind.y > rider.y + 10.0,
            "behind {} should be lower than the rider {}",
            behind.y,
            rider.y
        );
        assert!((rider.mid - cfg.screen_w * 0.5).abs() < 1.0);
        assert!(rider.y < cfg.screen_h - 30.0, "rider is not glued to the bottom edge");
    }

    #[test]
    fn chase_camera_keeps_the_rider_centered_in_a_bend() {
        let cfg = ProjConfig::game(384.0, 216.0);
        let player_z = 30.0;
        let player_x = 1.5;
        let strips = project_chase(player_z, player_x, flat, |_| 0.01, &cfg);
        let rider = strips
            .iter()
            .find(|s| player_z >= s.z && player_z < s.z + cfg.seg_len)
            .unwrap();
        let sx = rider.mid + (player_x / 4.0) * rider.half;
        assert!(
            (sx - cfg.screen_w * 0.5).abs() < 1.5,
            "rider screen x {sx}"
        );
    }

    #[test]
    fn trailing_bike_rises_from_the_bottom_on_its_side() {
        let far = trailing_placement(12.0, 2.2, 4.0, 384.0, 216.0, 120.0).unwrap();
        let near = trailing_placement(1.0, 2.2, 4.0, 384.0, 216.0, 120.0).unwrap();
        assert!(near.y < far.y, "closer bikes sit higher, coming up from behind");
        assert!(near.scale > far.scale);
        assert!(near.x > 384.0 * 0.5);
        let left = trailing_placement(4.0, -2.0, 4.0, 384.0, 216.0, 120.0).unwrap();
        assert!(left.x < 384.0 * 0.5);
        assert!(trailing_placement(0.0, 1.0, 4.0, 384.0, 216.0, 120.0).is_none());
        assert!(trailing_placement(20.0, 1.0, 4.0, 384.0, 216.0, 120.0).is_none());
    }
}
