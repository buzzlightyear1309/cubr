use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

use crate::swipe::DragState;

/// Orbit camera: left-drag rotate (ignored over UI) + wheel zoom.
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OrbitCamera>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, (orbit_camera, zoom_camera, update_camera_transform));
    }
}

/// Spherical orbit state, all relative to the origin the camera looks at.
///
/// `yaw` is measured in the XZ plane from +X toward +Z; `pitch` lifts the
/// camera above the XZ plane. With the README coords (+X right, +Y up, +Z
/// front), a positive yaw/pitch in the (+X, +Y, +Z) octant shows white `U` on
/// top, green `F` toward the viewer, and red `R` on the right.
#[derive(Resource)]
pub struct OrbitCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub radius: f32,
}

impl OrbitCamera {
    /// Orthonormal view basis (forward, right, up). `forward` is the look
    /// direction (camera -> origin). `right` is the yaw tangent in the XZ
    /// plane (independent of pitch -> the horizon never rolls). `up` follows
    /// the orientation continuously over the poles.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = (-orbit_translation(self)).normalize();
        let right = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
        let up = right.cross(forward);
        (forward, right, up)
    }
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            // 45° around + 30° up: the (+X,+Y,+Z) octant — white up, green
            // front, red right (matches the old temp camera at (4,4,6)).
            yaw: std::f32::consts::FRAC_PI_4,
            pitch: std::f32::consts::FRAC_PI_6,
            radius: 8.0,
        }
    }
}

/// Drag sensitivity (radians per pixel of mouse motion).
const ORBIT_SENSITIVITY: f32 = 0.008;
/// Zoom sensitivity (world units per scroll-delta unit).
const ZOOM_SENSITIVITY: f32 = 0.2;
const RADIUS_MIN: f32 = 3.0;
const RADIUS_MAX: f32 = 20.0;

/// Translation for the current orbit state (camera position on the sphere).
fn orbit_translation(cam: &OrbitCamera) -> Vec3 {
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    cam.radius * Vec3::new(cp * cy, sp, cp * sy)
}

/// Build the camera `Transform` from the orbit basis. Uses the manually-built
/// orthonormal basis (not `looking_at`) so the camera tumbles continuously over
/// the poles without gimbal-flipping and the horizon never rolls sideways.
fn orbit_transform(orbit: &OrbitCamera) -> Transform {
    // The camera looks down its local -Z, so feed -forward as the third column.
    let (forward, right, up) = orbit.basis();
    let rotation = Quat::from_mat3(&Mat3::from_cols(right, up, -forward));
    Transform {
        translation: orbit_translation(orbit),
        rotation,
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

/// Left-drag updates yaw/pitch from accumulated mouse motion (pitch unbounded).
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
    // Horizontal drag orbits left/right; vertical drag tilts up/down. Yaw sign is
    // chosen so the cube tracks the drag direction (feels like grabbing it).
    // Pitch is unbounded: the basis() math keeps the horizon level and lets the
    // camera tumble continuously over the poles.
    orbit.yaw += delta.x * ORBIT_SENSITIVITY;
    orbit.pitch += delta.y * ORBIT_SENSITIVITY;
}

/// Mouse wheel adjusts the orbit radius (clamped).
fn zoom_camera(scroll: Res<AccumulatedMouseScroll>, mut orbit: ResMut<OrbitCamera>) {
    if scroll.delta.y == 0.0 {
        return;
    }
    // Scroll up (positive) -> zoom in (smaller radius).
    orbit.radius = (orbit.radius - scroll.delta.y * ZOOM_SENSITIVITY).clamp(RADIUS_MIN, RADIUS_MAX);
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
    use std::f32::consts::{FRAC_PI_4, FRAC_PI_6};

    const EPS: f32 = 1e-5;

    /// At the default view the basis matches the README scheme: white `U` up,
    /// green `F` toward the viewer (forward points into the +X+Z corner), red
    /// `R` on screen-right.
    #[test]
    fn default_view_orientation() {
        let cam = OrbitCamera {
            yaw: FRAC_PI_4,
            pitch: FRAC_PI_6,
            radius: 8.0,
        };
        let (forward, right, up) = cam.basis();

        // Up points roughly +Y (white on top). At the default 30° pitch the
        // turntable up tilts back so up.y == cos(30°) ~= 0.866.
        assert!(up.y > 0.8, "up.y = {}", up.y);
        // Camera looks toward the +X+Z corner, so forward has negative X and Z.
        assert!(forward.dot(Vec3::Z) < 0.0, "forward.z = {}", forward.z);
        assert!(forward.dot(Vec3::X) < 0.0, "forward.x = {}", forward.x);
        // Red R face is on screen-right.
        assert!(right.dot(Vec3::X) > 0.0, "right.x = {}", right.x);
    }

    /// The horizon stays level (no roll) at ANY pitch, including past vertical.
    #[test]
    fn horizon_level_at_any_pitch() {
        for &pitch in &[-3.0_f32, -1.0, 0.0, 1.0, 2.0, 3.0] {
            let cam = OrbitCamera {
                yaw: FRAC_PI_4,
                pitch,
                radius: 8.0,
            };
            let (_forward, right, _up) = cam.basis();
            assert_eq!(right.y, 0.0, "right.y nonzero at pitch {pitch}");
        }
    }

    /// The basis is orthonormal even at a past-the-pole angle.
    #[test]
    fn basis_is_orthonormal() {
        let cam = OrbitCamera {
            yaw: 1.0,
            pitch: 2.5, // past the pole
            radius: 8.0,
        };
        let (forward, right, up) = cam.basis();

        // Each axis is unit length.
        assert!((forward.length() - 1.0).abs() < EPS, "forward len");
        assert!((right.length() - 1.0).abs() < EPS, "right len");
        assert!((up.length() - 1.0).abs() < EPS, "up len");

        // Mutually perpendicular.
        assert!(forward.dot(right).abs() < EPS, "forward . right");
        assert!(forward.dot(up).abs() < EPS, "forward . up");
        assert!(right.dot(up).abs() < EPS, "right . up");
    }
}
