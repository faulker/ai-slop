//! Fixed-timestep run state: player, course, and win/loss rules.

use crate::course::{
    course_from_seed_with, triangle_weave_x, Course, Entity, EntityKind, FlankPhase, GenConfig,
};
use crate::input::Input;

/// Tunable constants for the simulation. Kept in one place for clarity.
#[derive(Clone, Debug)]
pub struct SimConfig {
    /// Maximum on-road speed (m/s).
    pub max_speed: f32,
    /// Minimum speed while moving; brake can reach this. Documented: brake floors at 0.
    pub min_speed: f32,
    /// Acceleration while holding accelerate (m/s^2).
    pub accel: f32,
    /// Deceleration while holding brake (m/s^2).
    pub brake: f32,
    /// Coast drag when neither accel nor brake (m/s^2).
    pub drag: f32,
    /// Steer rate (m/s of lateral motion at full steer).
    pub steer_rate: f32,
    /// How far past the road edge the bike may go (meters beyond half-width).
    pub offroad_margin: f32,
    /// Max speed multiplier while off-road.
    pub offroad_speed_factor: f32,
    /// Health lost per second while off-road.
    pub offroad_dps: f32,
    /// Health lost on debris hit.
    pub debris_damage: f32,
    /// Health lost on enemy contact (once per contact window; see docs).
    pub contact_damage: f32,
    /// Overlap radius for pickups / debris / contact (meters in x/z plane approximation).
    pub overlap_radius: f32,
    /// Boost duration granted by a pill (seconds).
    pub boost_duration: f32,
    /// Speed cap multiplier while boosted.
    pub boost_speed_factor: f32,
    /// Acceleration multiplier while boosted.
    pub boost_accel_factor: f32,
    /// Pipe melee reach along z (meters ahead/behind).
    pub pipe_z_range: f32,
    /// Pipe melee reach along x (meters from player toward that flank).
    pub pipe_x_range: f32,
    /// Damage dealt by a successful pipe hit.
    pub pipe_damage: f32,
    /// Score for defeating an enemy with pipe.
    pub pipe_score: i32,
    /// Cooldown after swinging a pipe (seconds).
    pub pipe_cooldown: f32,
    /// Laser max distance ahead (meters along z).
    ///
    /// The chase bike covers the near road, so this has to reach riders
    /// that are actually visible past the sprite.
    pub laser_range: f32,
    /// Laser half-width in x (the lane directly in front of the bike).
    pub laser_half_width: f32,
    /// Damage dealt by laser.
    pub laser_damage: f32,
    /// Score for defeating an enemy with laser.
    pub laser_score: i32,
    /// Health lost when a flanker's pipe connects from the side.
    pub enemy_pipe_damage: f32,
    /// How far behind the player a flanker slips before chasing (meters).
    pub flank_behind: f32,
    /// Lateral gap when a flanker pulls alongside (meters).
    pub flank_gap: f32,
    /// Minimum seconds between laser shots.
    pub laser_cooldown: f32,
    /// Full laser energy. A shot spends [`SimConfig::laser_charge_cost`].
    pub laser_charge_max: f32,
    /// Energy spent each time the laser fires.
    pub laser_charge_cost: f32,
    /// Energy restored per second while racing.
    pub laser_charge_regen: f32,
    /// Starting player health.
    pub max_health: f32,
    /// Starting speed.
    pub start_speed: f32,
    /// Mild lateral drift from road curvature (scale * curvature * speed * dt).
    pub curvature_drift: f32,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            max_speed: 40.0,
            min_speed: 0.0,
            accel: 18.0,
            brake: 28.0,
            drag: 4.0,
            steer_rate: 8.0,
            offroad_margin: 2.5,
            offroad_speed_factor: 0.55,
            offroad_dps: 8.0,
            debris_damage: 12.0,
            contact_damage: 10.0,
            overlap_radius: 1.2,
            boost_duration: 3.0,
            boost_speed_factor: 1.35,
            boost_accel_factor: 1.4,
            pipe_z_range: 3.0,
            pipe_x_range: 3.5,
            pipe_damage: 40.0,
            pipe_score: 100,
            pipe_cooldown: 0.45,
            laser_range: 120.0,
            laser_half_width: 2.8,
            laser_damage: 25.0,
            laser_score: 150,
            enemy_pipe_damage: 18.0,
            flank_behind: 9.0,
            flank_gap: 2.2,
            laser_cooldown: 0.4,
            laser_charge_max: 100.0,
            laser_charge_cost: 34.0,
            laser_charge_regen: 22.0,
            max_health: 100.0,
            start_speed: 12.0,
            curvature_drift: 25.0,
        }
    }
}

/// Outcome of the current run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Racing,
    Won,
    Lost,
}

/// Player bike state.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    /// Lateral position; 0 is road center, negative is left.
    pub x: f32,
    /// Distance along the course in meters.
    pub z: f32,
    /// Speed in m/s.
    pub speed: f32,
    pub health: f32,
    pub score: i32,
    /// Seconds of boost remaining.
    pub boost_remaining: f32,
    pub pipe_cooldown: f32,
    pub laser_cooldown: f32,
    /// Energy left in the laser. Starts full. Shots spend it; time refills it.
    pub laser_charge: f32,
}

/// Step length of the chase-view road, in meters. Matches `ProjConfig::game`.
pub const CHASE_SEG_LEN: f32 = 4.0;
/// How strongly curvature bends the chase view. Matches `ProjConfig::game`.
pub const CHASE_CURVE_RESPONSE: f32 = 0.65;

/// Meters the road centerline has slid between `cam_z` and `z` in the chase view.
///
/// A straight shot up the screen hits world x = camera x minus this value.
/// Strip 0 (the camera) has no slide. Later strips add heading, then lateral,
/// the same way the road renderer does.
pub fn chase_lateral(cam_z: f32, z: f32, curvature_at: impl Fn(f32) -> f32) -> f32 {
    let ahead = z - cam_z;
    if ahead <= 0.0 {
        return 0.0;
    }
    let steps = (ahead / CHASE_SEG_LEN).floor() as i32;
    let mut heading = 0.0_f32;
    let mut lateral = 0.0_f32;
    for n in 0..steps {
        let sample_z = cam_z + n as f32 * CHASE_SEG_LEN;
        heading += curvature_at(sample_z) * CHASE_CURVE_RESPONSE * CHASE_SEG_LEN;
        lateral += heading * CHASE_SEG_LEN;
    }
    lateral
}

/// How long a hit spark or kill burst stays on the road, in seconds.
pub const EXPLOSION_LIFE: f32 = 0.42;

/// A laser impact. `power` near 1 is a kill burst; smaller is a hit spark.
#[derive(Clone, Debug)]
pub struct Explosion {
    pub x: f32,
    pub z: f32,
    pub age: f32,
    pub power: f32,
}

/// One race: course + player + status, driven by [`Level::step`].
#[derive(Clone, Debug)]
pub struct Level {
    pub seed: String,
    pub course: Course,
    pub player: Player,
    pub status: RunStatus,
    pub config: SimConfig,
    /// Spawn weights used to build `course` (preserved across restart).
    pub gen: GenConfig,
    /// Hit sparks and kill bursts, newest last.
    pub explosions: Vec<Explosion>,
    /// Enemy indices that already applied contact damage this overlap streak.
    contact_latched: Vec<usize>,
}

impl Level {
    /// Starts a new run from `seed` via [`course_from_seed`].
    pub fn new(seed: &str) -> Self {
        Self::with_config(seed, SimConfig::default())
    }

    /// Starts a new run with custom sim config and default spawn weights.
    pub fn with_config(seed: &str, config: SimConfig) -> Self {
        Self::with_gen_and_config(seed, &GenConfig::default(), config)
    }

    /// Starts a run from `seed` with spawn weights and sim config (desktop tuning).
    pub fn with_gen_and_config(seed: &str, gen: &GenConfig, config: SimConfig) -> Self {
        let course = course_from_seed_with(seed, gen);
        let player = Player {
            x: 0.0,
            z: 0.0,
            speed: config.start_speed,
            health: config.max_health,
            score: 0,
            boost_remaining: 0.0,
            pipe_cooldown: 0.0,
            laser_cooldown: 0.0,
            laser_charge: config.laser_charge_max,
        };
        Self {
            seed: seed.to_string(),
            course,
            player,
            status: RunStatus::Racing,
            config,
            gen: gen.clone(),
            explosions: Vec::new(),
            contact_latched: Vec::new(),
        }
    }

    /// Resets player state and reloads the course from the same seed and gen.
    pub fn restart(&mut self) {
        let seed = self.seed.clone();
        let config = self.config.clone();
        let gen = self.gen.clone();
        *self = Self::with_gen_and_config(&seed, &gen, config);
    }

    /// Advances the simulation by `dt` seconds with the given input.
    ///
    /// No-op if the run already ended.
    pub fn step(&mut self, input: &Input, dt: f32) {
        if self.status != RunStatus::Racing || dt <= 0.0 {
            return;
        }

        self.tick_cooldowns(dt);
        self.tick_explosions(dt);
        self.apply_speed(input, dt);
        self.apply_steer(input, dt);
        self.apply_offroad(dt);
        self.advance_enemies(dt);
        self.resolve_pickups_and_debris();
        self.resolve_enemy_contact();
        self.resolve_pipe(input);
        self.resolve_laser(input);
        self.advance_z(dt);
        self.check_end_conditions();
    }

    /// Moves active enemies forward by `speed * dt` along z.
    ///
    /// Enemies are never teleported. At the finish line their z is clamped and
    /// they are deactivated (despawned) so they do not sit forever past the end.
    /// Optional weave updates `x` from an integer triangle wave on `z`.
    /// Flankers that are dropping or chasing skip weave and use [`update_flankers`].
    fn advance_enemies(&mut self, dt: f32) {
        self.update_enemy_ai(dt);
        let finish = self.course.length;
        for ent in &mut self.course.entities {
            if !ent.active {
                continue;
            }
            if !matches!(ent.kind, EntityKind::Enemy { .. }) {
                continue;
            }
            let scripted = ent.flank_phase != FlankPhase::Ride;
            if ent.speed <= 0.0 && !scripted {
                continue;
            }
            ent.z += ent.speed * dt;
            if !scripted && ent.weave_amp > 0 {
                ent.x = triangle_weave_x(ent.z, ent.weave_amp, ent.base_x);
            }
            if ent.z >= finish {
                ent.z = finish;
                ent.active = false;
            }
        }
    }

    /// Every rival starts behind the player.
    ///
    /// `flank_side != 0` pulls up on that side and swings a pipe. The others
    /// shoot past, then brake and cut into the player's lane for a hit.
    fn update_enemy_ai(&mut self, dt: f32) {
        for i in 0..self.course.entities.len() {
            if !self.course.entities[i].active {
                continue;
            }
            if !matches!(self.course.entities[i].kind, EntityKind::Enemy { .. }) {
                continue;
            }
            if self.course.entities[i].flank_side != 0 {
                self.tick_piper(i, dt);
            } else {
                self.tick_overtaker(i, dt);
            }
        }
    }

    /// Comes up from behind on one side and swings a pipe once per pass.
    fn tick_piper(&mut self, i: usize, dt: f32) {
        const SWING_SECS: f32 = 0.4;

        let px = self.player.x;
        let pz = self.player.z;
        let pspeed = self.player.speed;
        let pipe_dmg = self.config.enemy_pipe_damage;
        let behind_need = self.config.flank_behind;
        let gap = self.config.flank_gap;
        let max_speed = self.config.max_speed;

        let z = self.course.entities[i].z;
        let limit = self.course.half_width_at(z) * 0.82;
        let side = self.course.entities[i].flank_side as f32;
        let target_x = (px + side * gap).clamp(-limit, limit);
        let mut piped = false;

        {
            let ent = &mut self.course.entities[i];
            match ent.flank_phase {
                FlankPhase::Ride => {
                    let ahead = ent.z - pz;
                    if ahead <= 6.0 {
                        ent.flank_phase = FlankPhase::Chase;
                        ent.flank_timer = 0.0;
                    } else if ahead < 22.0 {
                        ent.flank_phase = FlankPhase::Drop;
                        ent.flank_timer = 0.0;
                    }
                }
                FlankPhase::Drop => {
                    ent.speed = if pspeed < 6.0 {
                        pspeed
                    } else {
                        (pspeed - 12.0).max(4.0)
                    };
                    Self::steer_toward(ent, target_x, 5.0, dt);
                    ent.flank_timer += dt;
                    let behind = pz - ent.z;
                    if behind >= behind_need {
                        ent.flank_phase = FlankPhase::Chase;
                        ent.flank_timer = 0.0;
                    } else if ent.flank_timer > 8.0 && behind < 0.0 {
                        ent.speed = ent.cruise_speed;
                        ent.flank_phase = FlankPhase::Ride;
                        ent.flank_timer = 0.0;
                    }
                }
                FlankPhase::Chase => {
                    ent.speed = (pspeed + 14.0).clamp(18.0, max_speed + 10.0);
                    Self::steer_toward(ent, target_x, 8.0, dt);
                    ent.flank_timer += dt;
                    let behind = pz - ent.z;
                    let on_side = (ent.x - px) * side > 0.7;
                    let alongside = behind < 1.4 && behind > -1.6;
                    if alongside && on_side {
                        ent.flank_phase = FlankPhase::Swing;
                        ent.flank_timer = SWING_SECS;
                        piped = true;
                    } else if behind < -4.0 || ent.flank_timer > 6.0 {
                        ent.flank_phase = FlankPhase::Recover;
                        ent.flank_timer = 0.0;
                    }
                }
                FlankPhase::Swing => {
                    ent.speed = pspeed.max(6.0);
                    Self::steer_toward(ent, target_x, 10.0, dt);
                    ent.flank_timer -= dt;
                    if ent.flank_timer <= 0.0 {
                        ent.flank_phase = FlankPhase::Recover;
                        ent.flank_timer = 0.0;
                    }
                }
                FlankPhase::Recover => {
                    // Fall back behind the player and line up another pass.
                    ent.speed = (pspeed - 8.0).max(4.0);
                    Self::steer_toward(ent, target_x, 5.0, dt);
                    ent.flank_timer += dt;
                    if pz - ent.z >= behind_need || ent.flank_timer > 4.0 {
                        ent.flank_phase = FlankPhase::Chase;
                        ent.flank_timer = 0.0;
                    }
                }
            }
        }

        if piped {
            self.player.health -= pipe_dmg;
        }
    }

    /// Blasts past the player, then brakes and cuts into their lane.
    fn tick_overtaker(&mut self, i: usize, dt: f32) {
        const PASS_AHEAD: f32 = 9.0;

        let px = self.player.x;
        let pz = self.player.z;
        let pspeed = self.player.speed;
        let max_speed = self.config.max_speed;
        let z = self.course.entities[i].z;
        let limit = self.course.half_width_at(z) * 0.82;

        let overlap = self.config.overlap_radius;
        let ent = &mut self.course.entities[i];
        match ent.flank_phase {
            FlankPhase::Ride => {}
            FlankPhase::Chase | FlankPhase::Swing => {
                ent.flank_phase = FlankPhase::Chase;
                ent.speed = (pspeed + 16.0).clamp(22.0, max_speed + 14.0);
                let lane = ent.base_x.clamp(-limit, limit);
                Self::steer_toward(ent, lane, 7.0, dt);
                if ent.z - pz >= PASS_AHEAD {
                    ent.flank_phase = FlankPhase::Drop;
                    ent.flank_timer = 0.0;
                }
            }
            FlankPhase::Drop => {
                let ahead = ent.z - pz;
                // Once the player is past, drive again. Sitting at speed 0 back there
                // only slides with the rider and never moves up the road.
                if ahead <= 0.0 {
                    ent.flank_phase = FlankPhase::Chase;
                    ent.flank_timer = 0.0;
                    ent.speed = (pspeed + 16.0).clamp(22.0, max_speed + 14.0);
                } else {
                    // Hold just in front so the player arrives into contact.
                    // Contact is checked before the player moves, so a bike still
                    // rolling forward slips out of the radius.
                    if ahead < 3.0 {
                        ent.speed = 0.0;
                    } else {
                        ent.speed = (pspeed - 10.0).max(6.0);
                    }
                    let into = px.clamp(-limit, limit);
                    Self::steer_toward(ent, into, 9.0, dt);
                    ent.flank_timer += dt;
                    let hitting = (ent.z - pz).hypot(ent.x - px) <= overlap;
                    let fell_back = pz - ent.z > 8.0;
                    if (hitting && ent.flank_timer > 0.7) || ent.flank_timer > 3.5 || fell_back {
                        ent.flank_phase = FlankPhase::Recover;
                        ent.flank_timer = 1.6;
                        if hitting {
                            ent.speed = 0.0;
                        }
                    }
                }
            }
            FlankPhase::Recover => {
                ent.speed = (pspeed + 14.0).clamp(20.0, max_speed + 10.0);
                ent.flank_timer -= dt;
                if ent.flank_timer <= 0.0 {
                    ent.flank_phase = FlankPhase::Chase;
                    ent.flank_timer = 0.0;
                }
            }
        }
    }

    /// Slides `ent.x` toward `target_x` at `rate` meters per second.
    fn steer_toward(ent: &mut Entity, target_x: f32, rate: f32, dt: f32) {
        let dx = target_x - ent.x;
        let step = rate * dt;
        if dx.abs() <= step {
            ent.x = target_x;
        } else if dx > 0.0 {
            ent.x += step;
        } else {
            ent.x -= step;
        }
    }

    fn tick_cooldowns(&mut self, dt: f32) {
        self.player.pipe_cooldown = (self.player.pipe_cooldown - dt).max(0.0);
        self.player.laser_cooldown = (self.player.laser_cooldown - dt).max(0.0);
        self.player.boost_remaining = (self.player.boost_remaining - dt).max(0.0);
        let max_charge = self.config.laser_charge_max;
        self.player.laser_charge =
            (self.player.laser_charge + self.config.laser_charge_regen * dt).min(max_charge);
    }

    fn apply_speed(&mut self, input: &Input, dt: f32) {
        let cfg = &self.config;
        let boosted = self.player.boost_remaining > 0.0;
        let half = self.course.half_width_at(self.player.z);
        let offroad = self.player.x.abs() > half;

        let mut max_speed = cfg.max_speed;
        if boosted {
            max_speed *= cfg.boost_speed_factor;
        }
        if offroad {
            max_speed *= cfg.offroad_speed_factor;
        }

        let mut accel = cfg.accel;
        if boosted {
            accel *= cfg.boost_accel_factor;
        }

        if input.accelerate {
            self.player.speed += accel * dt;
        } else if input.brake {
            // Brake decreases speed down to min_speed (default 0).
            self.player.speed -= cfg.brake * dt;
        } else {
            self.player.speed -= cfg.drag * dt;
        }

        self.player.speed = self.player.speed.clamp(cfg.min_speed, max_speed);
    }

    fn apply_steer(&mut self, input: &Input, dt: f32) {
        let cfg = &self.config;
        let mut dx = 0.0_f32;
        if input.steer_left {
            dx -= cfg.steer_rate * dt;
        }
        if input.steer_right {
            dx += cfg.steer_rate * dt;
        }

        // Mild curvature drift so curves tug the bike.
        let curv = self.course.curvature_at(self.player.z);
        dx += curv * self.player.speed * cfg.curvature_drift * dt;

        self.player.x += dx;

        let half = self.course.half_width_at(self.player.z);
        let limit = half + cfg.offroad_margin;
        self.player.x = self.player.x.clamp(-limit, limit);
    }

    fn apply_offroad(&mut self, dt: f32) {
        let half = self.course.half_width_at(self.player.z);
        if self.player.x.abs() > half {
            self.player.health -= self.config.offroad_dps * dt;
        }
    }

    fn advance_z(&mut self, dt: f32) {
        self.player.z += self.player.speed * dt;
    }

    /// Debris chips health once then is consumed. Pills grant boost then are consumed.
    fn resolve_pickups_and_debris(&mut self) {
        let radius = self.config.overlap_radius;
        let debris_dmg = self.config.debris_damage;
        let boost_dur = self.config.boost_duration;
        let px = self.player.x;
        let pz = self.player.z;

        for ent in &mut self.course.entities {
            if !ent.active {
                continue;
            }
            let dx = px - ent.x;
            let dz = pz - ent.z;
            if dx * dx + dz * dz > radius * radius {
                continue;
            }
            match &ent.kind {
                EntityKind::Debris => {
                    self.player.health -= debris_dmg;
                    ent.active = false;
                }
                EntityKind::Pill => {
                    self.player.boost_remaining = boost_dur;
                    ent.active = false;
                }
                EntityKind::Enemy { .. } | EntityKind::Building => {}
            }
        }
    }

    /// Enemy contact chips player health. Enemies are not removed on mere contact;
    /// damage is latched per overlap streak so it does not tick every frame.
    fn resolve_enemy_contact(&mut self) {
        let radius = self.config.overlap_radius;
        let dmg = self.config.contact_damage;
        let px = self.player.x;
        let pz = self.player.z;

        let mut still_touching = Vec::new();
        for (i, ent) in self.course.entities.iter().enumerate() {
            if !ent.active {
                continue;
            }
            if !matches!(ent.kind, EntityKind::Enemy { .. }) {
                continue;
            }
            let dx = px - ent.x;
            let dz = pz - ent.z;
            if dx * dx + dz * dz > radius * radius {
                continue;
            }
            still_touching.push(i);
            if !self.contact_latched.contains(&i) {
                self.player.health -= dmg;
            }
        }
        self.contact_latched = still_touching;
    }

    fn resolve_pipe(&mut self, input: &Input) {
        if self.player.pipe_cooldown > 0.0 {
            return;
        }
        let left = input.pipe_left;
        let right = input.pipe_right;
        if !left && !right {
            return;
        }

        let cfg = &self.config;
        let px = self.player.x;
        let pz = self.player.z;
        let mut hit_index: Option<usize> = None;
        let mut best_dist = f32::MAX;

        for (i, ent) in self.course.entities.iter().enumerate() {
            if !ent.active {
                continue;
            }
            let EntityKind::Enemy { .. } = ent.kind else {
                continue;
            };
            let dz = (ent.z - pz).abs();
            if dz > cfg.pipe_z_range {
                continue;
            }
            let dx = ent.x - px;
            let on_left = left && dx < 0.0 && dx.abs() <= cfg.pipe_x_range;
            let on_right = right && dx > 0.0 && dx.abs() <= cfg.pipe_x_range;
            if !(on_left || on_right) {
                continue;
            }
            let dist = dx.abs() + dz;
            if dist < best_dist {
                best_dist = dist;
                hit_index = Some(i);
            }
        }

        self.player.pipe_cooldown = cfg.pipe_cooldown;

        if let Some(i) = hit_index {
            self.damage_enemy(i, cfg.pipe_damage, cfg.pipe_score);
        }
    }

    fn resolve_laser(&mut self, input: &Input) {
        if !input.laser || self.player.laser_cooldown > 0.0 {
            return;
        }
        if self.player.laser_charge < self.config.laser_charge_cost {
            return;
        }

        let cfg_range = self.config.laser_range;
        let cfg_half = self.config.laser_half_width;
        let px = self.player.x;
        let pz = self.player.z;
        let targets: Vec<(usize, f32, f32)> = self
            .course
            .entities
            .iter()
            .enumerate()
            .filter_map(|(i, ent)| {
                if !ent.active || !matches!(ent.kind, EntityKind::Enemy { .. }) {
                    return None;
                }
                Some((i, ent.x, ent.z))
            })
            .collect();
        let mut hit_index: Option<usize> = None;
        let mut best_z = f32::MAX;

        for (i, x, z) in targets {
            // Straight ahead of the bike, not along the bent lane. The chase
            // view slides the road sideways, so subtract that slide.
            let dz = z - pz;
            if dz <= 0.0 || dz > cfg_range {
                continue;
            }
            let aim_x = px - chase_lateral(pz, z, |sample| self.course.curvature_at(sample));
            if (x - aim_x).abs() > cfg_half {
                continue;
            }
            if z < best_z {
                best_z = z;
                hit_index = Some(i);
            }
        }

        self.player.laser_charge = (self.player.laser_charge - self.config.laser_charge_cost).max(0.0);
        self.player.laser_cooldown = self.config.laser_cooldown;

        if let Some(i) = hit_index {
            let damage = self.config.laser_damage;
            let score = self.config.laser_score;
            self.damage_enemy(i, damage, score);
        }
    }

    fn damage_enemy(&mut self, index: usize, damage: f32, score_on_kill: i32) {
        let Some(ent) = self.course.entities.get_mut(index) else {
            return;
        };
        if !ent.active {
            return;
        }
        let (x, z, killed) = if let EntityKind::Enemy { health } = &mut ent.kind {
            *health -= damage;
            let killed = *health <= 0.0;
            if killed {
                ent.active = false;
            }
            (ent.x, ent.z, killed)
        } else {
            return;
        };
        // A spark on every hit, a full burst when health is gone.
        self.explosions.push(Explosion {
            x,
            z,
            age: 0.0,
            power: if killed { 1.0 } else { 0.4 },
        });
        if killed {
            self.player.score += score_on_kill;
        }
    }

    fn tick_explosions(&mut self, dt: f32) {
        for boom in &mut self.explosions {
            boom.age += dt;
        }
        self.explosions.retain(|boom| boom.age < EXPLOSION_LIFE);
    }

    fn check_end_conditions(&mut self) {
        if self.player.health <= 0.0 {
            self.player.health = 0.0;
            self.status = RunStatus::Lost;
            return;
        }
        if self.player.z >= self.course.length {
            self.player.z = self.course.length;
            self.status = RunStatus::Won;
        }
    }

    /// True when the player is currently off the paved road.
    pub fn is_offroad(&self) -> bool {
        self.player.x.abs() > self.course.half_width_at(self.player.z)
    }

    /// Meters remaining until the finish line.
    pub fn distance_to_finish(&self) -> f32 {
        (self.course.length - self.player.z).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::course::{course_from_seed, Entity, FlankPhase, Segment};

    fn empty_input() -> Input {
        Input::default()
    }

    fn accel_input() -> Input {
        Input {
            accelerate: true,
            ..Input::default()
        }
    }

    fn brake_input() -> Input {
        Input {
            brake: true,
            ..Input::default()
        }
    }

    #[test]
    fn accelerate_clamps_to_max() {
        let mut level = Level::with_config(
            "clamp-accel",
            SimConfig {
                curvature_drift: 0.0,
                drag: 0.0,
                start_speed: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.segments = vec![crate::course::Segment {
            length: 10_000.0,
            curvature: 0.0,
            half_width: 4.0,
        }];
        level.course.length = 10_000.0;
        level.course.entities.clear();
        level.player.x = 0.0;
        for _ in 0..200 {
            level.step(&accel_input(), 0.05);
        }
        assert!(
            (level.player.speed - level.config.max_speed).abs() < 0.01,
            "speed {} vs max {}",
            level.player.speed,
            level.config.max_speed
        );
    }

    #[test]
    fn brake_clamps_to_min() {
        let mut level = Level::new("clamp-brake");
        level.player.speed = 20.0;
        for _ in 0..200 {
            level.step(&brake_input(), 0.05);
        }
        assert!((level.player.speed - level.config.min_speed).abs() < 0.01);
        assert!(level.player.speed >= 0.0);
    }

    #[test]
    fn steer_clamps_offroad_margin() {
        let mut level = Level::new("clamp-steer");
        let half = level.course.half_width_at(0.0);
        let limit = half + level.config.offroad_margin;
        let input = Input {
            steer_left: true,
            ..Input::default()
        };
        for _ in 0..500 {
            level.step(&input, 0.05);
        }
        assert!(level.player.x >= -limit - 0.01);
        assert!(level.player.x <= limit + 0.01);
        assert!(level.player.x.abs() > half);
    }

    #[test]
    fn debris_chips_health_once() {
        let mut level = Level::with_config(
            "debris-once",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.entities.clear();
        level.course.entities.push(Entity::static_at(0.0, 0.0, EntityKind::Debris));
        let before = level.player.health;
        level.step(&empty_input(), 0.016);
        let after_first = level.player.health;
        assert!((before - after_first - level.config.debris_damage).abs() < 0.01);
        assert!(!level.course.entities[0].active);
        level.step(&empty_input(), 0.016);
        assert!((level.player.health - after_first).abs() < 0.01);
    }

    #[test]
    fn pill_consumed_and_boost_applied() {
        let mut level = Level::with_config(
            "pill-boost",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.entities.clear();
        level.course.entities.push(Entity::static_at(0.0, 0.0, EntityKind::Pill));
        level.step(&empty_input(), 0.016);
        assert!(!level.course.entities[0].active);
        assert!(level.player.boost_remaining > 0.0);
    }

    #[test]
    fn laser_damages_ahead_not_beside_or_behind() {
        let mut level = Level::with_config(
            "laser-aim",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                laser_cooldown: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.entities.clear();
        level.course.entities.push(Entity::static_at(
            20.0,
            0.0,
            EntityKind::Enemy { health: 50.0 },
        ));
        level.course.entities.push(Entity::static_at(
            20.0,
            5.0,
            EntityKind::Enemy { health: 50.0 },
        ));
        level.course.entities.push(Entity::static_at(
            -5.0,
            0.0,
            EntityKind::Enemy { health: 50.0 },
        ));

        let input = Input {
            laser: true,
            ..Input::default()
        };
        level.step(&input, 0.016);

        match &level.course.entities[0].kind {
            EntityKind::Enemy { health } => {
                assert!((*health - (50.0 - level.config.laser_damage)).abs() < 0.01);
            }
            _ => panic!("expected enemy"),
        }
        match &level.course.entities[1].kind {
            EntityKind::Enemy { health } => assert!((*health - 50.0).abs() < 0.01),
            _ => panic!("expected enemy"),
        }
        match &level.course.entities[2].kind {
            EntityKind::Enemy { health } => assert!((*health - 50.0).abs() < 0.01),
            _ => panic!("expected enemy"),
        }
    }

    #[test]
    fn laser_reaches_a_rider_visible_ahead_of_the_bike() {
        let mut level = Level::with_config(
            "laser-range",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                laser_cooldown: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.entities.clear();
        // Past the old 45 m cap, still inside the lane in front of the bike.
        level.course.entities.push(Entity::static_at(
            80.0,
            1.2,
            EntityKind::Enemy { health: 50.0 },
        ));
        level.course.entities.push(Entity::static_at(
            200.0,
            0.0,
            EntityKind::Enemy { health: 50.0 },
        ));

        level.step(
            &Input {
                laser: true,
                ..Input::default()
            },
            0.016,
        );

        match &level.course.entities[0].kind {
            EntityKind::Enemy { health } => {
                assert!((*health - (50.0 - level.config.laser_damage)).abs() < 0.01);
            }
            _ => panic!("expected enemy"),
        }
        match &level.course.entities[1].kind {
            EntityKind::Enemy { health } => assert!((*health - 50.0).abs() < 0.01),
            _ => panic!("expected enemy"),
        }
    }

    #[test]
    fn laser_charge_blocks_shots_until_it_refills() {
        let mut level = Level::with_config(
            "laser-charge",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                laser_cooldown: 0.0,
                laser_charge_max: 100.0,
                laser_charge_cost: 40.0,
                laser_charge_regen: 0.0,
                laser_damage: 10.0,
                ..SimConfig::default()
            },
        );
        level.course.entities.clear();
        level.course.entities.push(Entity::static_at(
            30.0,
            0.0,
            EntityKind::Enemy { health: 100.0 },
        ));
        let input = Input {
            laser: true,
            ..Input::default()
        };
        level.step(&input, 0.016);
        level.step(&input, 0.016);
        match &level.course.entities[0].kind {
            EntityKind::Enemy { health } => assert!((health - 80.0).abs() < 0.01),
            _ => panic!("expected enemy"),
        }
        assert!(level.player.laser_charge < 40.0);
        level.step(&input, 0.016);
        match &level.course.entities[0].kind {
            EntityKind::Enemy { health } => assert!((health - 80.0).abs() < 0.01),
            _ => panic!("expected enemy"),
        }
        level.config.laser_charge_regen = 50.0;
        level.step(&Input::default(), 1.0);
        assert!(level.player.laser_charge >= 40.0);
        level.step(&input, 0.016);
        match &level.course.entities[0].kind {
            EntityKind::Enemy { health } => assert!((health - 70.0).abs() < 0.01),
            _ => panic!("expected enemy"),
        }
    }

    #[test]
    fn laser_shoots_straight_not_along_the_bend() {
        let mut level = Level::with_config(
            "laser-straight",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                curvature_drift: 0.0,
                laser_cooldown: 0.0,
                laser_charge_cost: 10.0,
                ..SimConfig::default()
            },
        );
        level.course.segments.clear();
        level.course.segments.push(Segment {
            length: 400.0,
            curvature: 0.006,
            half_width: 4.0,
        });
        level.course.length = 400.0;
        level.course.entities.clear();
        let z = 40.0;
        let lateral = chase_lateral(0.0, z, |_| 0.006);
        assert!(lateral > level.config.laser_half_width + 0.5);
        // Same world x as the bike: that rider sits on the bent lane, off the bolt.
        level.course.entities.push(Entity::static_at(
            z,
            0.0,
            EntityKind::Enemy { health: 50.0 },
        ));
        // On the straight line up the screen.
        level.course.entities.push(Entity::static_at(
            z,
            -lateral,
            EntityKind::Enemy { health: 50.0 },
        ));
        level.step(
            &Input {
                laser: true,
                ..Input::default()
            },
            0.016,
        );
        match &level.course.entities[0].kind {
            EntityKind::Enemy { health } => assert!((*health - 50.0).abs() < 0.01),
            _ => panic!("expected enemy"),
        }
        match &level.course.entities[1].kind {
            EntityKind::Enemy { health } => {
                assert!((*health - (50.0 - level.config.laser_damage)).abs() < 0.01);
            }
            _ => panic!("expected enemy"),
        }
    }

    #[test]
    fn laser_blows_up_an_enemy_after_enough_damage() {
        let mut level = Level::with_config(
            "laser-boom",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                laser_cooldown: 0.0,
                laser_charge_cost: 10.0,
                laser_damage: 25.0,
                ..SimConfig::default()
            },
        );
        level.course.segments.clear();
        level.course.segments.push(Segment {
            length: 400.0,
            curvature: 0.0,
            half_width: 4.0,
        });
        level.course.length = 400.0;
        level.course.entities.clear();
        // Toughest stock rival is 45. Two shots of 25 cross that.
        level.course.entities.push(Entity::static_at(
            30.0,
            0.0,
            EntityKind::Enemy { health: 45.0 },
        ));
        let shot = Input {
            laser: true,
            ..Input::default()
        };
        level.step(&shot, 0.016);
        assert!(level.course.entities[0].active);
        assert_eq!(level.explosions.len(), 1);
        assert!(level.explosions[0].power < 0.8);
        level.step(&shot, 0.5);
        assert!(!level.course.entities[0].active);
        assert!(level.explosions.iter().any(|boom| boom.power >= 1.0));
        assert_eq!(level.player.score, level.config.laser_score);
    }

    #[test]
    fn pipe_hits_correct_side_only() {
        let mut level = Level::with_config(
            "pipe-side",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                pipe_cooldown: 0.0,
                pipe_damage: 100.0,
                ..SimConfig::default()
            },
        );
        level.course.entities.clear();
        level.course.entities.push(Entity::static_at(
            1.0,
            -2.0,
            EntityKind::Enemy { health: 50.0 },
        ));
        level.course.entities.push(Entity::static_at(
            1.0,
            2.0,
            EntityKind::Enemy { health: 50.0 },
        ));

        let left = Input {
            pipe_left: true,
            ..Input::default()
        };
        level.step(&left, 0.016);
        assert!(!level.course.entities[0].active);
        assert!(level.course.entities[1].active);
        assert_eq!(level.player.score, level.config.pipe_score);

        // Cooldown was set; clear it to swing the other way.
        level.player.pipe_cooldown = 0.0;
        let right = Input {
            pipe_right: true,
            ..Input::default()
        };
        level.step(&right, 0.016);
        assert!(!level.course.entities[1].active);
    }

    #[test]
    fn health_depletion_ends_as_loss() {
        let mut level = Level::with_config(
            "loss",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.entities.clear();
        level.course.entities.push(Entity::static_at(0.0, 0.0, EntityKind::Debris));
        level.player.health = 5.0;
        level.step(&empty_input(), 0.016);
        assert_eq!(level.status, RunStatus::Lost);
        assert!(level.player.health <= 0.0);
    }

    #[test]
    fn reaching_finish_ends_as_win() {
        let mut level = Level::with_config(
            "win",
            SimConfig {
                start_speed: 50.0,
                max_speed: 50.0,
                drag: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.length = 10.0;
        level.player.z = 0.0;
        level.player.health = 100.0;
        // Drive past finish.
        for _ in 0..20 {
            level.step(&accel_input(), 0.05);
            if level.status != RunStatus::Racing {
                break;
            }
        }
        assert_eq!(level.status, RunStatus::Won);
    }

    #[test]
    fn restart_reloads_same_seed() {
        let mut level = Level::new("restart-me");
        level.player.score = 999;
        level.player.z = 50.0;
        level.player.health = 10.0;
        level.restart();
        assert_eq!(level.player.score, 0);
        assert_eq!(level.player.z, 0.0);
        assert_eq!(level.player.health, level.config.max_health);
        assert_eq!(level.status, RunStatus::Racing);
        assert_eq!(level.course, course_from_seed("restart-me"));
    }

    #[test]
    fn enemy_with_speed_moves_forward() {
        let mut level = Level::with_config(
            "enemy-motion",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                curvature_drift: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.segments = vec![crate::course::Segment {
            length: 10_000.0,
            curvature: 0.0,
            half_width: 4.0,
        }];
        level.course.length = 10_000.0;
        level.course.entities.clear();
        let mut enemy = Entity::static_at(50.0, 1.0, EntityKind::Enemy { health: 40.0 });
        enemy.speed = 20.0;
        level.course.entities.push(enemy);
        let z0 = level.course.entities[0].z;
        level.step(&empty_input(), 0.5);
        let z1 = level.course.entities[0].z;
        assert!(
            (z1 - z0 - 10.0).abs() < 0.01,
            "expected +10 m from 20 m/s * 0.5 s, got {} -> {}",
            z0,
            z1
        );
        assert!(level.course.entities[0].active);
    }

    #[test]
    fn building_contact_does_not_change_health() {
        let mut level = Level::with_config(
            "building-safe",
            SimConfig {
                start_speed: 0.0,
                drag: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.entities.clear();
        level.course.entities.push(Entity::static_at(0.0, 0.0, EntityKind::Building));
        let before = level.player.health;
        level.step(&empty_input(), 0.016);
        assert!((level.player.health - before).abs() < 0.01);
        assert!(level.course.entities[0].active);
    }

    #[test]
    fn same_seed_same_inputs_same_state_including_enemies() {
        let seed = "det-run-42";
        let input = Input {
            accelerate: true,
            steer_left: true,
            ..Input::default()
        };
        let run = |seed: &str| {
            let mut level = Level::with_config(
                seed,
                SimConfig {
                    curvature_drift: 0.0,
                    ..SimConfig::default()
                },
            );
            for _ in 0..90 {
                level.step(&input, 1.0 / 60.0);
            }
            (
                level.player.x,
                level.player.z,
                level.player.health,
                level.player.score,
                level
                    .course
                    .entities
                    .iter()
                    .filter(|e| matches!(e.kind, EntityKind::Enemy { .. }))
                    .map(|e| (e.z, e.x, e.active))
                    .collect::<Vec<_>>(),
            )
        };
        let a = run(seed);
        let b = run(seed);
        assert_eq!(a.3, b.3, "score");
        assert!((a.0 - b.0).abs() < 1e-5, "player x");
        assert!((a.1 - b.1).abs() < 1e-5, "player z");
        assert!((a.2 - b.2).abs() < 1e-5, "health");
        assert_eq!(a.4.len(), b.4.len());
        for (ea, eb) in a.4.iter().zip(b.4.iter()) {
            assert!((ea.0 - eb.0).abs() < 1e-5, "enemy z");
            assert!((ea.1 - eb.1).abs() < 1e-5, "enemy x");
            assert_eq!(ea.2, eb.2, "enemy active");
        }
    }

    /// Straight road, player holding a steady speed, one flanker just ahead.
    fn flank_course(side: i8, enemy_x: f32) -> Level {
        let mut level = Level::with_config(
            "flank-pipe",
            SimConfig {
                start_speed: 20.0,
                max_speed: 20.0,
                drag: 0.0,
                accel: 0.0,
                curvature_drift: 0.0,
                offroad_dps: 0.0,
                contact_damage: 0.0,
                ..SimConfig::default()
            },
        );
        level.course.segments = vec![Segment {
            length: 10_000.0,
            curvature: 0.0,
            half_width: 4.0,
        }];
        level.course.length = 10_000.0;
        level.course.entities.clear();
        level.player.x = 0.0;
        level.player.z = 100.0;
        level.player.speed = 20.0;
        let mut enemy = Entity::static_at(112.0, enemy_x, EntityKind::Enemy { health: 80.0 });
        enemy.speed = 18.0;
        enemy.cruise_speed = 18.0;
        enemy.flank_side = side;
        level.course.entities.push(enemy);
        level
    }

    #[test]
    fn flanker_drops_behind_then_pipes_from_the_side() {
        let mut level = flank_course(1, 1.5);
        let before = level.player.health;
        let mut saw_behind = false;
        let mut piped = false;
        for _ in 0..(60 * 8) {
            level.step(&accel_input(), 1.0 / 60.0);
            let enemy = &level.course.entities[0];
            if enemy.z < level.player.z - 3.0 {
                saw_behind = true;
            }
            if level.player.health < before - 0.5 {
                assert_eq!(enemy.flank_phase, FlankPhase::Swing);
                assert!(
                    enemy.x > 1.0,
                    "pipe should connect from the right, x={}",
                    enemy.x
                );
                piped = true;
                break;
            }
        }
        assert!(saw_behind, "flanker should fall behind the player");
        assert!(piped, "flanker should connect a pipe");
        let lost = before - level.player.health;
        assert!(
            (lost - level.config.enemy_pipe_damage).abs() < 0.01,
            "expected one pipe hit, lost {lost}"
        );

        let after = level.player.health;
        for _ in 0..20 {
            level.step(&accel_input(), 1.0 / 60.0);
        }
        assert!(
            (level.player.health - after).abs() < 0.01,
            "swing should not tick damage"
        );
    }

    #[test]
    fn left_flanker_pipes_from_the_left() {
        let mut level = flank_course(-1, -1.5);
        let before = level.player.health;
        for _ in 0..(60 * 8) {
            level.step(&accel_input(), 1.0 / 60.0);
            if level.player.health < before - 0.5 {
                assert!(level.course.entities[0].x < -1.0);
                return;
            }
        }
        panic!("left flanker never connected");
    }

    #[test]
    fn overtaker_shoots_ahead_then_brakes_into_the_player() {
        let mut level = flank_course(0, 2.4);
        level.config.contact_damage = 10.0;
        level.course.entities[0].base_x = 2.4;
        level.course.entities[0].z = 88.0;
        level.course.entities[0].flank_phase = FlankPhase::Chase;
        let before = level.player.health;
        let mut shot_ahead = false;
        let mut braked = false;
        for _ in 0..(60 * 8) {
            level.step(&accel_input(), 1.0 / 60.0);
            let enemy = &level.course.entities[0];
            if enemy.z > level.player.z + 6.0 {
                shot_ahead = true;
            }
            if enemy.flank_phase == FlankPhase::Drop && enemy.speed + 1.0 < level.player.speed {
                braked = true;
            }
            if level.player.health < before - 0.5 {
                assert!(shot_ahead, "should pass the player before the hit");
                assert!(braked, "should be braking when the hit lands");
                let lost = before - level.player.health;
                assert!(
                    (lost - level.config.contact_damage).abs() < 0.01,
                    "expected one contact hit, lost {lost}"
                );
                return;
            }
        }
        panic!("overtaker never connected, ahead={shot_ahead} braked={braked}");
    }

    #[test]
    fn overtaker_drives_forward_again_after_the_player_passes() {
        let mut level = flank_course(0, 0.0);
        level.course.entities[0].z = level.player.z - 2.0;
        level.course.entities[0].x = 0.0;
        level.course.entities[0].flank_phase = FlankPhase::Drop;
        level.course.entities[0].speed = 0.0;
        let z0 = level.course.entities[0].z;
        level.step(&accel_input(), 1.0 / 60.0);
        let enemy = &level.course.entities[0];
        assert_eq!(enemy.flank_phase, FlankPhase::Chase);
        assert!(enemy.speed > level.player.speed);
        assert!(enemy.z > z0, "should roll forward, not stay parked");
    }
}
