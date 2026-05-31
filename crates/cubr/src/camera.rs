use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

use crate::swipe::DragState;

/// Orbit camera: left-drag rotate (ignored over UI) + wheel zoom.
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OrbitCamera>()
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (
                    orbit_camera,
                    zoom_camera,
                    level_camera,
                    relevel_camera,
                    update_camera_transform,
                )
                    .chain(),
            );
    }
}

/// Quaternion orbit state, relative to the origin the camera looks at.
///
/// `rotation` is the camera's world orientation (camera-local axes -> world):
/// the camera looks down its local -Z, with local +X right and local +Y up.
/// Drags drive a re-basing turntable: horizontal orbit is azimuth about the
/// world cube-axis nearest camera-up (the "pole"), which switches as you tumble
/// so horizontal orbit never spins in place at the white/yellow faces.
#[derive(Resource)]
pub struct OrbitCamera {
    pub rotation: Quat,
    pub radius: f32,
}

/// The orbit view basis `(forward, right, up)` for a yaw/pitch (radius-free).
/// `forward` looks from the camera toward the origin; `right` is the level yaw
/// tangent. Single source of truth for the default view and the view-relative
/// tests.
pub fn basis_from_yaw_pitch(yaw: f32, pitch: f32) -> (Vec3, Vec3, Vec3) {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let forward = -Vec3::new(cp * cy, sp, cp * sy);
    let right = Vec3::new(sy, 0.0, -cy);
    let up = right.cross(forward);
    (forward, right, up)
}

impl OrbitCamera {
    /// Build an orientation from the legacy yaw/pitch basis so the default view
    /// is preserved exactly (white up, green front, red right).
    fn from_yaw_pitch(yaw: f32, pitch: f32, radius: f32) -> Self {
        let (forward, right, up) = basis_from_yaw_pitch(yaw, pitch);
        Self {
            rotation: Quat::from_mat3(&Mat3::from_cols(right, up, -forward)),
            radius,
        }
    }

    /// Orthonormal view basis (forward, right, up). Same signature as before.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        (
            self.rotation * Vec3::NEG_Z,
            self.rotation * Vec3::X,
            self.rotation * Vec3::Y,
        )
    }
}

impl Default for OrbitCamera {
    fn default() -> Self {
        // 45° around + 30° up: the (+X,+Y,+Z) octant — white up, green front,
        // red right (matches the old temp camera at (4,4,6)).
        Self::from_yaw_pitch(
            std::f32::consts::FRAC_PI_4,
            std::f32::consts::FRAC_PI_6,
            8.0,
        )
    }
}

/// Drag sensitivity (radians per pixel of mouse motion).
const ORBIT_SENSITIVITY: f32 = 0.008;
/// Zoom sensitivity (world units per scroll-delta unit).
const ZOOM_SENSITIVITY: f32 = 0.2;
const RADIUS_MIN: f32 = 3.0;
const RADIUS_MAX: f32 = 20.0;

/// The unit world axis (±X/±Y/±Z) most aligned with `v`.
fn snap_to_axis(v: Vec3) -> Vec3 {
    let axes = [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    crate::geom::best_by_dot(v, axes.map(|a| (a, a)))
}

/// Re-level the view so the horizon is level relative to `pole` (camera right ⟂
/// pole), preserving the view direction. No-op if looking ~along the pole.
fn leveled_to(rotation: Quat, pole: Vec3) -> Quat {
    let forward = rotation * Vec3::NEG_Z;
    let right = forward.cross(pole);
    if right.length_squared() < 1e-6 {
        return rotation;
    }
    let right = right.normalize();
    let up = right.cross(forward);
    Quat::from_mat3(&Mat3::from_cols(right, up, -forward))
}

/// Re-basing turntable drag: azimuth about the current pole (nearest world axis
/// to camera-up) + elevation about camera-right. Does NOT re-level — that is
/// damped smoothly by `relevel_step` each frame. While the view is leveled these
/// drags preserve level on their own, so this stays an anchored turntable.
fn turntable_drag(rotation: Quat, delta: Vec2, sens: f32) -> Quat {
    let pole = snap_to_axis(rotation * Vec3::Y);
    let right = rotation * Vec3::X;
    let yaw = Quat::from_axis_angle(pole, -delta.x * sens);
    let pitch = Quat::from_axis_angle(right, -delta.y * sens);
    (yaw * pitch * rotation).normalize()
}

/// One smoothing step toward level (right ⟂ current pole), by fraction `t` in
/// [0,1]. The leveled target shares the same forward, so this eases pure roll —
/// the horizon rotates smoothly to level rather than snapping. `t == 1` levels
/// fully; a stable, already-level view is a no-op.
fn relevel_step(rotation: Quat, t: f32) -> Quat {
    let pole = snap_to_axis(rotation * Vec3::Y);
    rotation.slerp(leveled_to(rotation, pole), t).normalize()
}

/// Build the camera `Transform` from the orbit state. The camera sits at
/// `-forward * radius` looking at the origin, oriented by `rotation`.
fn orbit_transform(orbit: &OrbitCamera) -> Transform {
    let (forward, _right, _up) = orbit.basis();
    Transform {
        translation: -forward * orbit.radius,
        rotation: orbit.rotation,
        ..default()
    }
}

/// Spawn the single `Camera3d`, plus the ambient fill the temp camera carried
/// (Bevy 0.18 made `AmbientLight` a per-camera component).
fn spawn_camera(mut commands: Commands, orbit: Res<OrbitCamera>) {
    commands.spawn((
        Camera3d::default(),
        orbit_transform(&orbit),
        AmbientLight {
            color: Color::WHITE,
            brightness: 350.0,
            ..default()
        },
    ));
}

/// True while the pointer is interacting with any UI node, so the orbit drag
/// can skip those frames and not spin the camera under buttons.
fn pointer_over_ui(interactions: &Query<&Interaction>) -> bool {
    interactions
        .iter()
        .any(|i| matches!(i, Interaction::Hovered | Interaction::Pressed))
}

/// Left-drag drives the re-basing turntable from accumulated mouse motion.
fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    interactions: Query<&Interaction>,
    drag: Option<Res<DragState>>,
    mut orbit: ResMut<OrbitCamera>,
) {
    if !buttons.pressed(MouseButton::Left) {
        return;
    }
    // A drag that began on the cube turns a layer (swipe), so don't orbit.
    if drag.is_some_and(|d| d.pressing_cube) {
        return;
    }
    // Don't orbit when the drag is happening over UI (button panel, etc.).
    if pointer_over_ui(&interactions) {
        return;
    }
    let delta = motion.delta;
    if delta == Vec2::ZERO {
        return;
    }
    // Re-basing turntable: azimuth about the world axis nearest camera-up and
    // elevation about camera-right. Re-leveling to the (possibly switched) pole
    // is handled separately by `relevel_camera`, which damps it smoothly so a
    // pole switch eases rather than snapping. Horizontal orbit stays an
    // anchored, level turntable in every orientation; the pole follows the
    // tumble so it never spins at the top/bottom faces. Press `L` to snap back
    // to an upright, world-level view.
    orbit.rotation = turntable_drag(orbit.rotation, delta, ORBIT_SENSITIVITY);
}

/// Mouse wheel adjusts the orbit radius (clamped).
fn zoom_camera(scroll: Res<AccumulatedMouseScroll>, mut orbit: ResMut<OrbitCamera>) {
    if scroll.delta.y == 0.0 {
        return;
    }
    // Scroll up (positive) -> zoom in (smaller radius).
    orbit.radius = (orbit.radius - scroll.delta.y * ZOOM_SENSITIVITY).clamp(RADIUS_MIN, RADIUS_MAX);
}

/// `L` snaps back to an upright, world-level view (re-level to the +Y pole)
/// while keeping the current view direction.
fn level_camera(keys: Res<ButtonInput<KeyCode>>, mut orbit: ResMut<OrbitCamera>) {
    if keys.just_pressed(KeyCode::KeyL) {
        orbit.rotation = leveled_to(orbit.rotation, Vec3::Y);
    }
}

/// Rate of the smooth re-level toward the current pole (higher = snappier).
const RELEVEL_RATE: f32 = 12.0;

/// Continuously eases the horizon to level relative to the current pole. Near-
/// zero work while the view is stable; smooths the twist when the pole switches.
fn relevel_camera(time: Res<Time>, mut orbit: ResMut<OrbitCamera>) {
    let t = 1.0 - (-RELEVEL_RATE * time.delta_secs()).exp();
    orbit.rotation = relevel_step(orbit.rotation, t);
}

/// Rebuild the camera transform from the orbit state each frame so orbit/zoom
/// take visible effect.
fn update_camera_transform(
    orbit: Res<OrbitCamera>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    if let Ok(mut transform) = camera.single_mut() {
        *transform = orbit_transform(&orbit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    const EPS: f32 = 1e-5;

    /// Assert a basis triple is orthonormal (unit length, mutually ⟂).
    fn assert_orthonormal(forward: Vec3, right: Vec3, up: Vec3) {
        assert!((forward.length() - 1.0).abs() < EPS, "forward len");
        assert!((right.length() - 1.0).abs() < EPS, "right len");
        assert!((up.length() - 1.0).abs() < EPS, "up len");
        assert!(forward.dot(right).abs() < EPS, "forward . right");
        assert!(forward.dot(up).abs() < EPS, "forward . up");
        assert!(right.dot(up).abs() < EPS, "right . up");
    }

    /// `snap_to_axis` returns the unit world axis most aligned with the input.
    #[test]
    fn snap_to_axis_picks_nearest() {
        assert_eq!(snap_to_axis(Vec3::new(0.1, 0.9, -0.2)), Vec3::Y);
        assert_eq!(snap_to_axis(Vec3::new(0.8, -0.1, 0.3)), Vec3::X);
        assert_eq!(snap_to_axis(Vec3::new(0.0, 0.1, -0.9)), Vec3::NEG_Z);
    }

    /// At the default view the basis matches the README scheme: white `U` up,
    /// green `F` toward the viewer (forward points into the +X+Z corner), red
    /// `R` on screen-right. This guards that the camera refactor preserved the
    /// default orientation exactly — other modules depend on it.
    #[test]
    fn default_view_orientation() {
        let (forward, right, up) = OrbitCamera::default().basis();

        // Up points roughly +Y (white on top). At the default 30° pitch the
        // up tilts back so up.y == cos(30°) ~= 0.866.
        assert!(up.y > 0.8, "up.y = {}", up.y);
        // Camera looks toward the +X+Z corner, so forward has negative X and Z.
        assert!(forward.dot(Vec3::Z) < 0.0, "forward.z = {}", forward.z);
        assert!(forward.dot(Vec3::X) < 0.0, "forward.x = {}", forward.x);
        // Red R face is on screen-right.
        assert!(right.dot(Vec3::X) > 0.0, "right.x = {}", right.x);
    }

    /// The basis is orthonormal for the default view AND a freely tumbled
    /// orientation. Quats are normalized, so this holds everywhere.
    #[test]
    fn basis_is_orthonormal() {
        let (f, r, u) = OrbitCamera::default().basis();
        assert_orthonormal(f, r, u);

        let tumbled = OrbitCamera {
            rotation: Quat::from_euler(EulerRot::YXZ, 1.0, 2.5, 0.3),
            radius: 8.0,
        };
        let (f, r, u) = tumbled.basis();
        assert_orthonormal(f, r, u);
    }

    /// Upright, the re-basing turntable is exactly the old level turntable: a
    /// pure horizontal drag is pure azimuth about +Y (stays level, elevation
    /// unchanged) and a pure vertical drag stays level too.
    #[test]
    fn upright_is_old_turntable() {
        let lvl = OrbitCamera::from_yaw_pitch(FRAC_PI_4, 0.0, 8.0);
        assert!(
            (lvl.rotation * Vec3::Y).y > 0.999,
            "level view up.y = {}",
            (lvl.rotation * Vec3::Y).y
        );

        let h = turntable_drag(lvl.rotation, Vec2::new(40.0, 0.0), 0.01);
        // Horizon stays level.
        assert!(
            (h * Vec3::X).y.abs() < 1e-4,
            "horizontal drag rolled the horizon: right.y = {}",
            (h * Vec3::X).y
        );
        // Pure azimuth: forward stays in the horizontal plane (no elevation).
        assert!(
            (h * Vec3::NEG_Z).y.abs() < 1e-4,
            "horizontal drag changed elevation: forward.y = {}",
            (h * Vec3::NEG_Z).y
        );

        let v = turntable_drag(lvl.rotation, Vec2::new(0.0, 40.0), 0.01);
        assert!(
            (v * Vec3::X).y.abs() < 1e-4,
            "vertical drag rolled the horizon: right.y = {}",
            (v * Vec3::X).y
        );
    }

    /// The core fix: from a straight-down view a horizontal drag ORBITS off the
    /// pole instead of spinning in place. A fixed-Y turntable would keep
    /// forward = -Y (spin); the re-based pole lets it orbit away.
    #[test]
    fn no_pole_spin_on_horizontal_drag() {
        let down = Quat::from_rotation_x(-FRAC_PI_2);
        assert!(
            (down * Vec3::NEG_Z - Vec3::NEG_Y).length() < EPS,
            "down forward = {:?}",
            down * Vec3::NEG_Z
        );

        let r = turntable_drag(down, Vec2::new(40.0, 0.0), 0.01);
        let f = r * Vec3::NEG_Z;
        let horiz = (f.x * f.x + f.z * f.z).sqrt();
        assert!(horiz > 0.1, "forward stayed vertical: f = {f:?}");
    }

    /// The pole re-bases to the world axis nearest camera-up: +Y at the default
    /// view, +Z for a view tumbled so camera-up points along +Z.
    #[test]
    fn pole_rebases_with_tumble() {
        assert_eq!(
            snap_to_axis(OrbitCamera::default().rotation * Vec3::Y),
            Vec3::Y
        );

        let r = Quat::from_rotation_x(FRAC_PI_2);
        assert!(
            (r * Vec3::Y - Vec3::Z).length() < EPS,
            "tumbled up = {:?}",
            r * Vec3::Y
        );
        assert_eq!(snap_to_axis(r * Vec3::Y), Vec3::Z);
    }

    /// `relevel_step(_, 1.0)` levels the horizon fully (camera-right ⟂ pole)
    /// while preserving the view direction — the easing target is pure roll.
    #[test]
    fn relevel_step_full_levels() {
        let lvl = OrbitCamera::from_yaw_pitch(FRAC_PI_4, 0.0, 8.0).rotation;
        assert!(
            (lvl * Vec3::Y).y > 0.999,
            "level up.y = {}",
            (lvl * Vec3::Y).y
        );
        let rolled = Quat::from_axis_angle(lvl * Vec3::NEG_Z, 0.4) * lvl;
        assert!(
            (rolled * Vec3::X).y.abs() > 0.05,
            "rolled right.y = {}",
            (rolled * Vec3::X).y
        );

        let r = relevel_step(rolled, 1.0);
        assert!(
            (r * Vec3::X).y.abs() < 1e-4,
            "not fully level: right.y = {}",
            (r * Vec3::X).y
        );
        // Forward preserved: the correction is pure roll.
        let dot = (r * Vec3::NEG_Z).dot(rolled * Vec3::NEG_Z);
        assert!(dot > 0.999, "forward changed: dot = {dot}");
    }

    /// `relevel_step(_, 0.3)` eases part-way: the horizon roll shrinks but does
    /// not reach level in one step — this is the smoothing.
    #[test]
    fn relevel_step_eases_partway() {
        let lvl = OrbitCamera::from_yaw_pitch(FRAC_PI_4, 0.0, 8.0).rotation;
        let rolled = Quat::from_axis_angle(lvl * Vec3::NEG_Z, 0.4) * lvl;
        let before = (rolled * Vec3::X).y.abs();

        let r = relevel_step(rolled, 0.3);
        let after = (r * Vec3::X).y.abs();
        assert!(
            after < before,
            "did not move toward level: {after} !< {before}"
        );
        assert!(after > 1e-4, "leveled fully in one step: right.y = {after}");
    }

    /// Repeated partial steps converge to a level horizon.
    #[test]
    fn relevel_step_converges() {
        let lvl = OrbitCamera::from_yaw_pitch(FRAC_PI_4, 0.0, 8.0).rotation;
        let mut r = Quat::from_axis_angle(lvl * Vec3::NEG_Z, 0.4) * lvl;
        for _ in 0..40 {
            r = relevel_step(r, 0.2);
        }
        assert!(
            (r * Vec3::X).y.abs() < 1e-3,
            "did not converge: right.y = {}",
            (r * Vec3::X).y
        );
    }

    /// A stable, already-level view is a no-op: forward preserved and the
    /// horizon stays level, so a steady view stays anchored (no drift).
    #[test]
    fn relevel_step_stable_view_is_noop() {
        let lvl = OrbitCamera::from_yaw_pitch(FRAC_PI_4, 0.0, 8.0).rotation;
        let r = relevel_step(lvl, 0.3);
        let dot = (r * Vec3::NEG_Z).dot(lvl * Vec3::NEG_Z);
        assert!(dot > 0.999, "forward changed: dot = {dot}");
        assert!(
            (r * Vec3::X).y.abs() < 1e-4,
            "stable view rolled: right.y = {}",
            (r * Vec3::X).y
        );
    }

    /// `leveled_to(_, +Y)` preserves the view direction and yields a level
    /// horizon on the already-level default view; near-degenerate input
    /// (looking ~along the pole) is returned unchanged.
    #[test]
    fn leveled_to_invariants() {
        let d = OrbitCamera::default().rotation;
        let l = leveled_to(d, Vec3::Y);
        // View direction preserved.
        let dot = (l * Vec3::NEG_Z).dot(d * Vec3::NEG_Z);
        assert!(dot > 0.999, "forward changed: dot = {dot}");
        // Level horizon.
        assert!(
            (l * Vec3::X).y.abs() < 1e-4,
            "default right.y = {}",
            (l * Vec3::X).y
        );

        // Near-degenerate: looking straight down (forward ∥ +Y) is a no-op.
        let down = Quat::from_rotation_x(-FRAC_PI_2);
        let n = leveled_to(down, Vec3::Y);
        let dot = (n * Vec3::NEG_Z).dot(down * Vec3::NEG_Z);
        assert!(dot > 0.999, "near-degenerate view was altered: dot = {dot}");
    }
}
