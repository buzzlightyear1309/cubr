// Phase 0 stubs: the contract is defined but not yet consumed by any phase.
#![allow(dead_code)]

use crate::cube::model::{CubeState, Move, StickerColor};
use bevy::math::IVec3;

/// 26 cubies (the hidden core at (0,0,0) is omitted). Integer rotation math only.
pub struct CubeCore {
    // private: Vec<CoreCubie>, 26 entries
    cubies: Vec<CoreCubie>,
}

impl CubeCore {
    pub fn solved() -> Self {
        // Phase 1 builds the real 26-cubie solved core.
        CubeCore { cubies: Vec::new() }
    }

    /// Apply a move as an integer permutation+reorientation of the affected layer.
    /// Quarter turns applied quarter_turns_cw times. Pure geometry — colors ride along.
    pub fn apply(&mut self, _m: Move) {
        // Phase 1 implements the integer rotation.
    }

    /// Repaint to an arbitrary state for POST /state: reset all cubies to home pose,
    /// then assign each visible sticker the given color. Represents impossible states fine.
    pub fn paint(&mut self, _state: &CubeState) {
        // Phase 1 implements the repaint.
    }

    /// Read the current facelets in README per-face orientation (row-major, the index
    /// layout and per-face viewing rules in README "Per-face viewing orientation").
    pub fn to_state(&self) -> CubeState {
        // Phase 1 reads the real facelets; placeholder keeps the crate compiling.
        CubeState::solved()
    }

    /// For the renderer: snapshot of each cubie's current pose + visible stickers, so the
    /// Bevy layer can build/sync entities. `home` identifies the entity across moves.
    pub fn cubies(&self) -> &[CoreCubie] {
        &self.cubies
    }

    /// Indices into cubies() that lie in the layer this move turns (the 9 moving pieces).
    pub fn layer(&self, _m: Move) -> Vec<usize> {
        // Phase 1 computes the real layer membership.
        Vec::new()
    }
}

/// Read-only view the renderer consumes.
#[allow(dead_code)]
pub struct CoreCubie {
    pub home: IVec3,        // solved position; stable id for the entity
    pub pos: IVec3,         // current grid position, components in {-1,0,1}
    pub orient: [IVec3; 3], // integer rotation matrix columns (local->world basis)
    // visible stickers: which outward local face shows which color
    pub stickers: Vec<(IVec3 /*local outward normal*/, StickerColor)>,
}
