use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

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
    yaw: f32,
    pitch: f32,
    radius: f32,
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
const ZOOM_SENSITIVITY: f32 = 0.5;
/// Clamp pitch just shy of ±90° to avoid the look-at gimbal flip.
const PITCH_LIMIT: f32 = 1.54;
const RADIUS_MIN: f32 = 3.0;
const RADIUS_MAX: f32 = 20.0;

/// Translation for the current orbit state (camera position on the sphere).
fn orbit_translation(cam: &OrbitCamera) -> Vec3 {
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    cam.radius * Vec3::new(cp * cy, sp, cp * sy)
}

/// Spawn the single `Camera3d`, plus the ambient fill the temp camera carried
/// (Bevy 0.18 made `AmbientLight` a per-camera component).
fn spawn_camera(mut commands: Commands, orbit: Res<OrbitCamera>) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(orbit_translation(&orbit)).looking_at(Vec3::ZERO, Vec3::Y),
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

/// Left-drag updates yaw/pitch from accumulated mouse motion (clamped pitch).
fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    interactions: Query<&Interaction>,
    mut orbit: ResMut<OrbitCamera>,
) {
    if !buttons.pressed(MouseButton::Left) {
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
    // Drag right -> orbit right; drag up -> tilt up (feels like grabbing the cube).
    orbit.yaw -= delta.x * ORBIT_SENSITIVITY;
    orbit.pitch = (orbit.pitch + delta.y * ORBIT_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
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
        *transform =
            Transform::from_translation(orbit_translation(&orbit)).looking_at(Vec3::ZERO, Vec3::Y);
    }
}
