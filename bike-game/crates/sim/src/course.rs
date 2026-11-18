//! Course data and seeded procedural generator.
//!
//! [`course_from_seed`] is the generator. The signature stays stable; only the
//! body grows (segments, debris, pills, buildings, gangs, color/shape).
//! Spawn density can be scaled via [`GenConfig`] without changing the seed
//! string; weight `1.0` preserves the exact historical layout.

use crate::prng::Prng;

/// Spawn-density multipliers for the procedural generator.
///
/// Defaults are all `1.0`. At that value the generator consumes the same RNG
/// stream and places the same counts as the historical [`course_from_seed`].
#[derive(Clone, Debug, PartialEq)]
pub struct GenConfig {
    pub debris_weight: f32,
    pub pill_weight: f32,
    pub gang_weight: f32,
    /// 1 is the fewest rivals, 5 is every rolled gang. Default 5 matches
    /// [`course_from_seed`]. The title screen starts a run at an easier rating.
    pub hardness: u8,
}

/// Easiest hardness. One rival.
pub const HARDNESS_MIN: u8 = 1;
/// Full set of rolled gangs.
pub const HARDNESS_MAX: u8 = 5;
/// What a new race uses until the player changes it on the title screen.
pub const DEFAULT_PLAY_HARDNESS: u8 = 2;

impl Default for GenConfig {
    fn default() -> Self {
        Self {
            debris_weight: 1.0,
            pill_weight: 1.0,
            gang_weight: 1.0,
            hardness: HARDNESS_MAX,
        }
    }
}

/// Clamps a hardness rating into [`HARDNESS_MIN`]..=[`HARDNESS_MAX`].
pub fn clamp_hardness(hardness: u8) -> u8 {
    hardness.clamp(HARDNESS_MIN, HARDNESS_MAX)
}

/// Whether gang `gang_index` member `member_index` is placed at this hardness.
///
/// The generator still rolls every gang so the road stays on the same RNG stream.
/// Lower ratings simply keep fewer of those bikes.
pub fn hardness_keeps(
    gang_index: usize,
    member_index: usize,
    gang_n: usize,
    member_n: usize,
    hardness: u8,
) -> bool {
    let h = clamp_hardness(hardness);
    let gangs = match h {
        1 => 1,
        2 => 2,
        3 => 3,
        4 => gang_n.saturating_sub(1).max(1),
        _ => gang_n,
    };
    let members = match h {
        1 | 2 => 1,
        3 => 2,
        _ => member_n,
    };
    gang_index < gangs.min(gang_n) && member_index < members.min(member_n)
}

/// Scales a base spawn count by `weight`. Weight `1.0` returns `base` unchanged
/// (no float rounding) so the default path stays bit-identical.
fn scale_count(base: usize, weight: f32) -> usize {
    if weight == 1.0 {
        base
    } else {
        ((base as f32) * weight).round().max(0.0) as usize
    }
}

/// One stretch of road along the course.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// Length of this segment in meters.
    pub length: f32,
    /// Lateral curvature (positive curves right). Used for rendering and light x-drift.
    pub curvature: f32,
    /// Half-width of the drivable road in meters.
    pub half_width: f32,
}

/// Attack phase for a rival that drops behind and swings a pipe.
///
/// [`FlankPhase::Ride`] is also the idle state for bikes that never flank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlankPhase {
    /// Cruising at `cruise_speed`. A flanker waits here until the player closes.
    Ride,
    /// Slowing so the player passes and the bike slips behind.
    Drop,
    /// Accelerating from behind onto one side of the player.
    Chase,
    /// Pipe is out. Damage was applied when this phase started.
    Swing,
    /// Pulling ahead. `flank_timer` counts down before another attempt.
    Recover,
}

/// What kind of placed entity this is.
#[derive(Clone, Debug, PartialEq)]
pub enum EntityKind {
    Debris,
    Pill,
    Enemy {
        health: f32,
    },
    /// Shoulder facade. No collision, no damage. `shape` 0/1/2 → building_a/b/c.
    Building,
}

/// A placed object on the course at lateral `x` and distance `z`.
#[derive(Clone, Debug, PartialEq)]
pub struct Entity {
    pub z: f32,
    pub x: f32,
    pub kind: EntityKind,
    /// False after the entity is consumed or defeated.
    pub active: bool,
    /// Palette index (0..8). Gangs share one color. Editor may override later.
    pub color: u8,
    /// Silhouette / facade variant (0..2). Editor may override later.
    pub shape: u8,
    /// Forward speed along z in m/s. Non-enemies stay 0. Enemies ride forward;
    /// clamped at the finish line (not teleported past it).
    pub speed: f32,
    /// Lateral weave amplitude in centimeters (0 = no weave). Uses [`base_x`].
    pub weave_amp: i32,
    /// Lane center for weave (meters). Ignored when `weave_amp == 0`.
    pub base_x: f32,
    /// -1 pipes from the left, +1 from the right, 0 shoots past then brakes into you.
    ///
    /// Chosen from the gang slot without extra RNG so default layouts stay put.
    pub flank_side: i8,
    pub flank_phase: FlankPhase,
    /// Seconds left in [`FlankPhase::Swing`] or [`FlankPhase::Recover`].
    pub flank_timer: f32,
    /// Speed restored after a flank so the bike goes back to cruising.
    pub cruise_speed: f32,
}

impl Entity {
    /// Builds a static entity (speed 0, no weave) for tests and helpers.
    pub fn static_at(z: f32, x: f32, kind: EntityKind) -> Self {
        Self {
            z,
            x,
            kind,
            active: true,
            color: 0,
            shape: 0,
            speed: 0.0,
            weave_amp: 0,
            base_x: x,
            flank_side: 0,
            flank_phase: FlankPhase::Ride,
            flank_timer: 0.0,
            cruise_speed: 0.0,
        }
    }
}

/// Full course: segments, entities, and finish distance.
#[derive(Clone, Debug, PartialEq)]
pub struct Course {
    pub segments: Vec<Segment>,
    pub entities: Vec<Entity>,
    /// Distance to the finish line in meters (`z >= length` wins).
    pub length: f32,
}

impl Course {
    /// Road half-width at a given distance along the course.
    pub fn half_width_at(&self, z: f32) -> f32 {
        let mut cursor = 0.0_f32;
        for seg in &self.segments {
            let end = cursor + seg.length;
            if z < end || (z - end).abs() < f32::EPSILON {
                return seg.half_width;
            }
            cursor = end;
        }
        self.segments
            .last()
            .map(|s| s.half_width)
            .unwrap_or(4.0)
    }

    /// Curvature at a given distance (for rendering / mild drift).
    pub fn curvature_at(&self, z: f32) -> f32 {
        let mut cursor = 0.0_f32;
        for seg in &self.segments {
            let end = cursor + seg.length;
            if z < end || (z - end).abs() < f32::EPSILON {
                return seg.curvature;
            }
            cursor = end;
        }
        self.segments.last().map(|s| s.curvature).unwrap_or(0.0)
    }
}

/// Integer triangle wave in [-amp_cm, amp_cm], period 40 m, from z in meters.
///
/// No libm trig; pure integer ratios then convert once to meters.
pub fn triangle_weave_x(z: f32, amp_cm: i32, base_x: f32) -> f32 {
    if amp_cm <= 0 {
        return base_x;
    }
    let period_cm: i32 = 4000; // 40 m
    let z_cm = (z * 100.0) as i32;
    let phase = z_cm.rem_euclid(period_cm);
    let half = period_cm / 2;
    let raw = if phase < half {
        -amp_cm + (2 * amp_cm * phase) / half
    } else {
        amp_cm - (2 * amp_cm * (phase - half)) / half
    };
    base_x + (raw as f32) / 100.0
}

/// Builds a full procedural course from a seed string with default spawn weights.
///
/// Pure function of `seed`. Same seed always yields the same layout, gangs,
/// pills, debris, buildings, and color/shape ids on any machine (integer PRNG
/// only). Equal to [`course_from_seed_with`]`(seed, &GenConfig::default())`.
pub fn course_from_seed(seed: &str) -> Course {
    course_from_seed_with(seed, &GenConfig::default())
}

/// Builds a procedural course from `seed` with spawn-density weights.
///
/// When every weight is `1.0`, the result matches [`course_from_seed`] exactly
/// (same RNG consumption and entity counts). Non-default weights scale debris
/// scatter/cluster counts, pill count, and gang count via
/// `round(base * weight)` before placement.
pub fn course_from_seed_with(seed: &str, gen: &GenConfig) -> Course {
    let mut rng = Prng::from_seed_str(seed);

    // --- Segments: 6–14, total length ~450–1100 m ---
    let seg_count = 6 + rng.range_i32(0, 8) as usize;
    let target_len_cm = 45_000 + rng.range_i32(0, 65_000); // 450–1100 m in cm
    let mut segments = Vec::with_capacity(seg_count);
    let mut remaining_cm = target_len_cm;
    let mut total_len = 0.0_f32;

    for i in 0..seg_count {
        let left = seg_count - i;
        let seg_cm = if left == 1 {
            remaining_cm.max(4000)
        } else {
            let avg = remaining_cm / left as i32;
            let lo = (avg * 6 / 10).max(3000);
            let hi = (avg * 14 / 10).max(lo + 1);
            let pick = rng.range_i32(lo, hi).min(remaining_cm - 3000 * (left as i32 - 1));
            pick.max(3000)
        };
        remaining_cm -= seg_cm;

        // Roughly half straight, half curved.
        let curvature = if rng.range_i32(0, 1) == 0 {
            0.0
        } else {
            let sign = if rng.range_i32(0, 1) == 0 { -1.0 } else { 1.0 };
            let mag = rng.range_i32(2, 8) as f32 / 1000.0; // 0.002–0.008
            sign * mag
        };

        // Mostly ~4 m half-width; occasional narrow stretch.
        let half_width = if rng.range_i32(0, 5) == 0 {
            (rng.range_i32(250, 350) as f32) / 100.0
        } else {
            4.0
        };

        let length = (seg_cm as f32) / 100.0;
        total_len += length;
        segments.push(Segment {
            length,
            curvature,
            half_width,
        });
    }

    let mut entities = Vec::new();
    let playable_start = 35.0_f32;
    let playable_end = (total_len - 40.0).max(playable_start + 50.0);

    // --- Debris: scattered + a few tight clusters ---
    let scatter = 8 + rng.range_i32(0, 10) as usize;
    let scatter_n = scale_count(scatter, gen.debris_weight);
    for _ in 0..scatter_n {
        let z = rng.range_f32(playable_start, playable_end);
        let half = half_width_along(&segments, z);
        let x = rng.range_f32(-half * 0.85, half * 0.85);
        entities.push(Entity {
            z,
            x,
            kind: EntityKind::Debris,
            active: true,
            color: rng.range_i32(0, 7) as u8,
            shape: rng.range_i32(0, 2) as u8,
            speed: 0.0,
            weave_amp: 0,
            base_x: x,
            flank_side: 0,
            flank_phase: FlankPhase::Ride,
            flank_timer: 0.0,
            cruise_speed: 0.0,
        });
    }
    let clusters = 2 + rng.range_i32(0, 2) as usize;
    let clusters_n = scale_count(clusters, gen.debris_weight);
    for _ in 0..clusters_n {
        let cz = rng.range_f32(playable_start + 20.0, playable_end - 20.0);
        let half = half_width_along(&segments, cz);
        let n = 3 + rng.range_i32(0, 2) as usize;
        for _ in 0..n {
            let z = cz + rng.range_f32(-4.0, 4.0);
            let x = rng.range_f32(-half * 0.8, half * 0.8);
            entities.push(Entity {
                z: z.clamp(playable_start, playable_end),
                x,
                kind: EntityKind::Debris,
                active: true,
                color: rng.range_i32(0, 7) as u8,
                shape: rng.range_i32(0, 2) as u8,
                speed: 0.0,
                weave_amp: 0,
                base_x: x,
                flank_side: 0,
                flank_phase: FlankPhase::Ride,
                flank_timer: 0.0,
                cruise_speed: 0.0,
            });
        }
    }

    // Collect debris z for pill spacing (avoid stacking on debris).
    let debris_zs: Vec<f32> = entities
        .iter()
        .filter(|e| matches!(e.kind, EntityKind::Debris))
        .map(|e| e.z)
        .collect();

    // --- Pills: several on the road, between danger ---
    let pill_count = 5 + rng.range_i32(0, 6) as usize;
    let pill_n = scale_count(pill_count, gen.pill_weight);
    let mut pills_placed = 0;
    let mut attempts = 0;
    while pills_placed < pill_n && attempts < pill_n * 8 {
        attempts += 1;
        let z = rng.range_f32(playable_start + 10.0, playable_end);
        let too_close = debris_zs.iter().any(|&dz| (dz - z).abs() < 6.0);
        if too_close {
            continue;
        }
        let half = half_width_along(&segments, z);
        let x = rng.range_f32(-half * 0.75, half * 0.75);
        entities.push(Entity {
            z,
            x,
            kind: EntityKind::Pill,
            active: true,
            color: rng.range_i32(0, 7) as u8,
            shape: rng.range_i32(0, 2) as u8,
            speed: 0.0,
            weave_amp: 0,
            base_x: x,
            flank_side: 0,
            flank_phase: FlankPhase::Ride,
            flank_timer: 0.0,
            cruise_speed: 0.0,
        });
        pills_placed += 1;
    }

    // --- Buildings: both shoulders, outside the road ---
    // Half-width must use the final z (after jitter), or a wider segment can
    // leave the facade inside the pavement.
    let building_spacing_cm = 800 + rng.range_i32(0, 600); // 8–14 m
    let mut bz_cm = (playable_start * 100.0) as i32;
    let end_cm = (playable_end * 100.0) as i32;
    while bz_cm < end_cm {
        let z_base = (bz_cm as f32) / 100.0;
        for &side in &[-1.0_f32, 1.0] {
            // Occasional skip so the skyline is not a solid wall.
            if rng.range_i32(0, 4) == 0 {
                continue;
            }
            let z = (z_base + rng.range_f32(-1.5, 1.5)).clamp(playable_start, playable_end);
            let half = half_width_along(&segments, z);
            let gap_cm = 80 + rng.range_i32(0, 120); // 0.8–2.0 m past edge
            let gap = (gap_cm as f32) / 100.0;
            let x = side * (half + gap + rng.range_f32(0.0, 1.2));
            entities.push(Entity {
                z,
                x,
                kind: EntityKind::Building,
                active: true,
                color: rng.range_i32(0, 7) as u8,
                shape: rng.range_i32(0, 2) as u8,
                speed: 0.0,
                weave_amp: 0,
                base_x: x,
                flank_side: 0,
                flank_phase: FlankPhase::Ride,
                flank_timer: 0.0,
                cruise_speed: 0.0,
            });
        }
        bz_cm += building_spacing_cm + rng.range_i32(-200, 300);
    }

    // --- Gangs: 3–7 groups, each 2–4 bikes sharing color ---
    // Every pack starts behind the rider. Later packs sit further back so
    // they arrive in waves instead of waiting 100 m up the road.
    let gang_count = 3 + rng.range_i32(0, 4) as usize;
    let gang_n = scale_count(gang_count, gen.gang_weight);
    for g in 0..gang_n {
        let gang_z = -12.0 - (g as f32) * 40.0 + rng.range_f32(-6.0, 6.0);
        let half = half_width_along(&segments, gang_z.max(0.0));
        let members = 2 + rng.range_i32(0, 2) as usize;
        let color = rng.range_i32(0, 7) as u8;
        for m in 0..members {
            // Further members sit further behind the lead bike, so the pack stays behind z=0.
            let z = gang_z - (m as f32) * rng.range_f32(1.5, 3.5) + rng.range_f32(-1.0, 1.0);
            // Spread across the road; stay on pavement (not in buildings).
            let lane = if members == 1 {
                0.0
            } else {
                let span = half * 0.7;
                -span + (2.0 * span) * (m as f32) / ((members - 1) as f32)
            };
            let x = (lane + rng.range_f32(-0.4, 0.4)).clamp(-half * 0.85, half * 0.85);
            let hp = 20.0 + rng.range_f32(0.0, 25.0);
            // Speeds generally below player max (40); some closer to it.
            let speed_cm = 1800 + rng.range_i32(0, 1800); // 18–36 m/s
            let speed = (speed_cm as f32) / 100.0;
            let weave = if rng.range_i32(0, 3) == 0 {
                rng.range_i32(40, 90) // 0.4–0.9 m amp
            } else {
                0
            };
            // About one bike in three. Side follows the lane they already spawned in.
            // No extra RNG draw, so z/x/speed stay on the historical stream.
            let flank_side: i8 = if (m as i32 + color as i32) % 3 == 0 {
                if x < 0.0 { -1 } else { 1 }
            } else {
                0
            };
            // Roll shape even when this bike is skipped so a lower hardness
            // does not shift the RNG for the bikes that remain.
            let shape = rng.range_i32(0, 2) as u8;
            if !hardness_keeps(g, m, gang_n, members, gen.hardness) {
                continue;
            }
            entities.push(Entity {
                z,
                x,
                kind: EntityKind::Enemy { health: hp },
                active: true,
                color,
                shape,
                speed,
                weave_amp: weave,
                base_x: x,
                flank_side,
                flank_phase: FlankPhase::Chase,
                flank_timer: 0.0,
                cruise_speed: speed,
            });
        }
    }

    Course {
        segments,
        entities,
        length: total_len,
    }
}

/// Half-width at z while segments are still being built into a Course.
fn half_width_along(segments: &[Segment], z: f32) -> f32 {
    let mut cursor = 0.0_f32;
    for seg in segments {
        let end = cursor + seg.length;
        if z < end || (z - end).abs() < f32::EPSILON {
            return seg.half_width;
        }
        cursor = end;
    }
    segments.last().map(|s| s.half_width).unwrap_or(4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_identical_course() {
        let a = course_from_seed("night-city");
        let b = course_from_seed("night-city");
        assert_eq!(a, b);
    }

    #[test]
    fn empty_seed_stable() {
        let a = course_from_seed("");
        let b = course_from_seed("");
        assert_eq!(a, b);
        assert!(a.length > 0.0);
        assert!(!a.entities.is_empty());
    }

    #[test]
    fn different_seeds_differ_in_placement() {
        let a = course_from_seed("seed-a");
        let b = course_from_seed("seed-b");
        assert_ne!(a.entities, b.entities);
    }

    #[test]
    fn seeds_differ_in_segment_geometry() {
        let a = course_from_seed("geom-alpha");
        let b = course_from_seed("geom-beta");
        let seg_diff = a.segments.iter().zip(b.segments.iter()).any(|(sa, sb)| {
            (sa.length - sb.length).abs() > 0.01 || (sa.curvature - sb.curvature).abs() > 1e-6
        }) || a.segments.len() != b.segments.len()
            || (a.length - b.length).abs() > 0.01;
        assert!(
            seg_diff,
            "expected segment curvature or length to differ between seeds"
        );
    }

    #[test]
    fn seeds_differ_in_color_or_shape() {
        let a = course_from_seed("tint-one");
        let b = course_from_seed("tint-two");
        let differ = a
            .entities
            .iter()
            .zip(b.entities.iter())
            .any(|(ea, eb)| ea.color != eb.color || ea.shape != eb.shape)
            || a.entities.len() != b.entities.len();
        // Also compare any entity pair if lengths differ: look for at least one
        // color/shape that appears in one course's set differently.
        let a_ids: Vec<(u8, u8)> = a.entities.iter().map(|e| (e.color, e.shape)).collect();
        let b_ids: Vec<(u8, u8)> = b.entities.iter().map(|e| (e.color, e.shape)).collect();
        assert!(
            differ || a_ids != b_ids,
            "expected at least one color or shape id to differ"
        );
    }

    #[test]
    fn buildings_sit_outside_road() {
        let course = course_from_seed("city-shoulders");
        let buildings: Vec<_> = course
            .entities
            .iter()
            .filter(|e| matches!(e.kind, EntityKind::Building))
            .collect();
        assert!(!buildings.is_empty(), "expected buildings on the course");
        for b in buildings {
            let half = course.half_width_at(b.z);
            assert!(
                b.x.abs() > half,
                "building at z={} x={} should be outside half_width {}",
                b.z,
                b.x,
                half
            );
        }
    }

    #[test]
    fn gang_produces_more_than_one_enemy() {
        let course = course_from_seed("gang-check");
        let mut enemy_zs: Vec<f32> = course
            .entities
            .iter()
            .filter(|e| matches!(e.kind, EntityKind::Enemy { .. }))
            .map(|e| e.z)
            .collect();
        enemy_zs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(enemy_zs.len() >= 6, "expected multiple enemies from gangs");
        // At least one pair of enemies within a few meters (same gang spawn).
        let mut found_gang = false;
        for w in enemy_zs.windows(2) {
            if (w[1] - w[0]).abs() < 12.0 {
                found_gang = true;
                break;
            }
        }
        assert!(found_gang, "expected a gang clustered in z");
        // Shared color within a tight z cluster.
        let enemies: Vec<_> = course
            .entities
            .iter()
            .filter(|e| matches!(e.kind, EntityKind::Enemy { .. }))
            .collect();
        let mut shared = false;
        for i in 0..enemies.len() {
            for j in (i + 1)..enemies.len() {
                if (enemies[i].z - enemies[j].z).abs() < 12.0
                    && enemies[i].color == enemies[j].color
                    && (enemies[i].x - enemies[j].x).abs() > 0.3
                {
                    shared = true;
                }
            }
        }
        assert!(shared, "gang members should share color and differ in x");
    }

    fn enemy_count(course: &Course) -> usize {
        course
            .entities
            .iter()
            .filter(|e| matches!(e.kind, EntityKind::Enemy { .. }))
            .count()
    }

    #[test]
    fn harder_rating_adds_enemies_without_changing_the_road() {
        let seed = "neon";
        let at = |hardness: u8| {
            course_from_seed_with(
                seed,
                &GenConfig {
                    hardness,
                    ..GenConfig::default()
                },
            )
        };
        let easy = at(1);
        let mid = at(3);
        let hard = at(5);
        assert_eq!(enemy_count(&easy), 1, "hardness 1 is a single rival");
        assert!(enemy_count(&easy) < enemy_count(&mid));
        assert!(enemy_count(&mid) < enemy_count(&hard));
        assert_eq!(hard.segments, easy.segments);
        assert_eq!(hard.length, easy.length);
        let debris = |c: &Course| {
            c.entities
                .iter()
                .filter(|e| matches!(e.kind, EntityKind::Debris))
                .map(|e| (e.z.to_bits(), e.x.to_bits()))
                .collect::<Vec<_>>()
        };
        assert_eq!(debris(&easy), debris(&hard));
        assert_eq!(enemy_count(&hard), enemy_count(&course_from_seed(seed)));
    }

    #[test]
    fn some_gang_bikes_flank_and_some_do_not() {
        let course = course_from_seed("gang-check");
        let enemies: Vec<_> = course
            .entities
            .iter()
            .filter(|e| matches!(e.kind, EntityKind::Enemy { .. }))
            .collect();
        let flankers = enemies.iter().filter(|e| e.flank_side != 0).count();
        assert!(flankers >= 1, "expected at least one pipe flanker");
        assert!(flankers < enemies.len(), "not every bike should flank");
        for e in &enemies {
            assert!(e.z < 0.0, "gangs start behind the rider, z={}", e.z);
            assert!(e.flank_side >= -1 && e.flank_side <= 1);
            assert_eq!(e.flank_phase, FlankPhase::Chase);
            if e.flank_side != 0 {
                if e.x < 0.0 {
                    assert_eq!(e.flank_side, -1);
                } else {
                    assert_eq!(e.flank_side, 1);
                }
                assert!((e.cruise_speed - e.speed).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn course_length_and_segment_counts_in_range() {
        for seed in ["neon", "a", "long-run-99", ""] {
            let c = course_from_seed(seed);
            assert!(
                c.length >= 450.0 && c.length <= 1100.0,
                "length {} out of range for seed {seed}",
                c.length
            );
            assert!(
                c.segments.len() >= 6 && c.segments.len() <= 14,
                "segment count {} out of range",
                c.segments.len()
            );
        }
    }

    #[test]
    fn triangle_weave_stays_near_base() {
        let base = 1.0_f32;
        let amp = 50; // 0.5 m
        for i in 0..40 {
            let z = i as f32 * 2.0;
            let x = triangle_weave_x(z, amp, base);
            assert!((x - base).abs() <= 0.51);
        }
        assert_eq!(triangle_weave_x(10.0, 0, base), base);
    }

    #[test]
    fn buildings_outside_road_many_seeds() {
        for n in 0..200 {
            let seed = format!("probe-{n}");
            let course = course_from_seed(&seed);
            for b in course.entities.iter().filter(|e| matches!(e.kind, EntityKind::Building)) {
                let half = course.half_width_at(b.z);
                assert!(
                    b.x.abs() > half + 0.05,
                    "seed {seed} building z={} x={} half={}",
                    b.z, b.x, half
                );
            }
        }
    }

    #[test]
    fn default_gen_matches_course_from_seed() {
        for seed in ["neon", "night-city", "probe-31", ""] {
            let a = course_from_seed(seed);
            let b = course_from_seed_with(seed, &GenConfig::default());
            assert_eq!(a, b, "seed {seed:?} drifted under default GenConfig");
        }
    }

    fn count_kind(course: &Course, pred: impl Fn(&EntityKind) -> bool) -> usize {
        course
            .entities
            .iter()
            .filter(|e| pred(&e.kind))
            .count()
    }

    #[test]
    fn debris_weight_scales_count_for_neon() {
        let seed = "neon";
        let n0 = count_kind(
            &course_from_seed_with(
                seed,
                &GenConfig {
                    debris_weight: 0.0,
                    ..GenConfig::default()
                },
            ),
            |k| matches!(k, EntityKind::Debris),
        );
        let n1 = count_kind(
            &course_from_seed_with(seed, &GenConfig::default()),
            |k| matches!(k, EntityKind::Debris),
        );
        let n2 = count_kind(
            &course_from_seed_with(
                seed,
                &GenConfig {
                    debris_weight: 2.0,
                    ..GenConfig::default()
                },
            ),
            |k| matches!(k, EntityKind::Debris),
        );
        assert!(n0 < n1, "debris_weight 0 ({n0}) should be < weight 1 ({n1})");
        // Weight 2 doubles scatter+cluster counts; if a future cap saturates,
        // still require at least as many as weight 1.
        assert!(
            n2 >= n1,
            "debris_weight 2 ({n2}) should be >= weight 1 ({n1})"
        );
        if n2 > n1 {
            // Preferred strict ordering when uncapped.
            assert!(n0 < n1 && n1 < n2);
        }
    }

    #[test]
    fn gang_weight_zero_yields_fewer_enemies_for_neon() {
        let seed = "neon";
        let n0 = count_kind(
            &course_from_seed_with(
                seed,
                &GenConfig {
                    gang_weight: 0.0,
                    ..GenConfig::default()
                },
            ),
            |k| matches!(k, EntityKind::Enemy { .. }),
        );
        let n1 = count_kind(
            &course_from_seed_with(seed, &GenConfig::default()),
            |k| matches!(k, EntityKind::Enemy { .. }),
        );
        assert!(
            n0 < n1,
            "gang_weight 0 enemies ({n0}) should be < weight 1 ({n1})"
        );
    }
}
