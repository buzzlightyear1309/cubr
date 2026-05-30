//! View-relative move mapping: turn a face named relative to the current camera
//! view into one of the 18 absolute `Move`s. Pure (no Bevy systems) so it is
//! unit-tested directly. Consumed by the Beginner panel in `ui.rs` (next unit).

use bevy::math::Vec3;

use crate::cube::model::{Face, Move, Turn};

/// A cube face named relative to where the camera is currently looking.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RelFace {
    Front,
    Back,
    Up,
    Down,
    Left,
    Right,
}

/// Resolve a view-relative face + turn into an absolute `Move`.
///
/// `basis` is `OrbitCamera::basis()` = `(forward, right, up)` (forward is the
/// look direction camera->origin). The relative face picks a world direction;
/// we resolve it to the absolute `Face` whose outward normal aligns best with
/// that direction. The turn (CW/CCW/180 "looking at that face from outside") is
/// already viewpoint-independent, so it passes through unchanged.
pub fn relative_move(basis: (Vec3, Vec3, Vec3), rel: RelFace, turn: Turn) -> Move {
    let (forward, right, up) = basis;
    let dir = match rel {
        RelFace::Front => -forward,
        RelFace::Back => forward,
        RelFace::Up => up,
        RelFace::Down => -up,
        RelFace::Right => right,
        RelFace::Left => -right,
    };
    Move {
        face: resolve_face(dir),
        turn,
    }
}

/// Face priority for resolving 45°-corner ties deterministically. Chosen so the
/// default view (yaw π/4, pitch π/6) resolves Front->F, Up->U, Right->R.
const FACE_PRIORITY: [Face; 6] = [Face::F, Face::U, Face::R, Face::B, Face::D, Face::L];

/// The absolute face whose outward normal best aligns with `dir`; ties broken by
/// FACE_PRIORITY order (strict `>` keeps the earlier-listed face).
fn resolve_face(dir: Vec3) -> Face {
    crate::geom::best_by_dot(dir, FACE_PRIORITY.map(|f| (f, f.normal().as_vec3())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::basis_from_yaw_pitch;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, FRAC_PI_6};

    /// At the default view (yaw π/4, pitch π/6) the relative faces resolve to
    /// the README scheme: Front->F, Up->U, Right->R (and the opposites Back->B,
    /// Down->D).
    #[test]
    fn default_view_resolves_to_readme_scheme() {
        let basis = basis_from_yaw_pitch(FRAC_PI_4, FRAC_PI_6);
        assert_eq!(
            relative_move(basis, RelFace::Front, Turn::Cw).face,
            Face::F
        );
        assert_eq!(relative_move(basis, RelFace::Up, Turn::Cw).face, Face::U);
        assert_eq!(relative_move(basis, RelFace::Right, Turn::Cw).face, Face::R);
        assert_eq!(relative_move(basis, RelFace::Back, Turn::Cw).face, Face::B);
        assert_eq!(relative_move(basis, RelFace::Down, Turn::Cw).face, Face::D);
    }

    /// Looking from +X (yaw 0, pitch 0) — a clean non-degenerate view. The
    /// camera faces -X, so screen-front is the L face's outward direction... no:
    /// forward = -pos = (-1,0,0), so Front = -forward = +X = R, etc.
    #[test]
    fn looking_from_plus_x() {
        let basis = basis_from_yaw_pitch(0.0, 0.0);
        assert_eq!(
            relative_move(basis, RelFace::Front, Turn::Cw).face,
            Face::R
        );
        assert_eq!(relative_move(basis, RelFace::Right, Turn::Cw).face, Face::B);
        assert_eq!(relative_move(basis, RelFace::Up, Turn::Cw).face, Face::U);
        assert_eq!(relative_move(basis, RelFace::Left, Turn::Cw).face, Face::F);
        assert_eq!(relative_move(basis, RelFace::Back, Turn::Cw).face, Face::L);
        assert_eq!(relative_move(basis, RelFace::Down, Turn::Cw).face, Face::D);
    }

    /// A +90° yaw from the default remaps which absolute face is "Right": it is
    /// no longer R.
    #[test]
    fn yaw_quarter_turn_remaps_right() {
        let basis = basis_from_yaw_pitch(FRAC_PI_4 + FRAC_PI_2, FRAC_PI_6);
        assert_ne!(relative_move(basis, RelFace::Right, Turn::Cw).face, Face::R);
    }

    /// The turn passes through unchanged regardless of the relative face.
    #[test]
    fn turn_passes_through() {
        let basis = basis_from_yaw_pitch(FRAC_PI_4, FRAC_PI_6);
        assert_eq!(
            relative_move(basis, RelFace::Front, Turn::Cw).turn,
            Turn::Cw
        );
        assert_eq!(
            relative_move(basis, RelFace::Right, Turn::Ccw).turn,
            Turn::Ccw
        );
        assert_eq!(
            relative_move(basis, RelFace::Up, Turn::Double).turn,
            Turn::Double
        );
    }
}
