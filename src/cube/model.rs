// Phase 0 stubs: the contract is defined but not yet consumed by any phase.
// Real consumers arrive in Phases 1+; this avoids pre-poisoning `clippy -D warnings`.
#![allow(dead_code)]

use bevy::prelude::*; // for IVec3, Color
use serde::{Deserialize, Serialize};

/// Sticker color. Serializes to its single-letter name ("W","Y","R","O","B","G").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum StickerColor {
    W,
    Y,
    R,
    O,
    B,
    G,
}

impl StickerColor {
    /// Render color (sRGB). White/Yellow/Red/Orange/Blue/Green tuned for a clean look.
    pub fn to_render_color(self) -> Color {
        // Phase 1 supplies the tuned palette; placeholder keeps the crate compiling.
        Color::WHITE
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Face {
    U,
    D,
    L,
    R,
    F,
    B,
}

impl Face {
    pub const ALL: [Face; 6] = [Face::U, Face::D, Face::L, Face::R, Face::F, Face::B];

    /// Outward normal in world space: U=+Y, D=-Y, R=+X, L=-X, F=+Z, B=-Z  (see README coords).
    pub fn normal(self) -> IVec3 {
        // Phase 1 implements the real mapping.
        IVec3::ZERO
    }

    /// Solved color: U=W, D=Y, F=G, B=B, R=R, L=O  (see README face table).
    pub fn solved_color(self) -> StickerColor {
        // Phase 1 implements the real mapping.
        StickerColor::W
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Turn {
    Cw,
    Ccw,
    Double,
} // (none), ', 2

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Move {
    pub face: Face,
    pub turn: Turn,
}

impl Move {
    /// All 18 standard moves, in README order: U U' U2 D D' D2 L L' L2 R R' R2 F F' F2 B B' B2.
    pub const ALL: [Move; 18] = [
        Move {
            face: Face::U,
            turn: Turn::Cw,
        },
        Move {
            face: Face::U,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::U,
            turn: Turn::Double,
        },
        Move {
            face: Face::D,
            turn: Turn::Cw,
        },
        Move {
            face: Face::D,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::D,
            turn: Turn::Double,
        },
        Move {
            face: Face::L,
            turn: Turn::Cw,
        },
        Move {
            face: Face::L,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::L,
            turn: Turn::Double,
        },
        Move {
            face: Face::R,
            turn: Turn::Cw,
        },
        Move {
            face: Face::R,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::R,
            turn: Turn::Double,
        },
        Move {
            face: Face::F,
            turn: Turn::Cw,
        },
        Move {
            face: Face::F,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::F,
            turn: Turn::Double,
        },
        Move {
            face: Face::B,
            turn: Turn::Cw,
        },
        Move {
            face: Face::B,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::B,
            turn: Turn::Double,
        },
    ];

    /// Parse one of the 18 notation strings; None for anything else.
    pub fn parse(_s: &str) -> Option<Move> {
        // Phase 1 implements the real parser.
        None
    }

    /// Notation string, e.g. "R", "R'", "R2".
    pub fn notation(self) -> String {
        // Phase 1 implements the real notation.
        String::new()
    }

    /// Rotation axis = self.face.normal().
    pub fn axis(self) -> IVec3 {
        self.face.normal()
    }

    /// Number of clockwise (looking at the face from outside) quarter-turns: Cw=1, Ccw=3, Double=2.
    pub fn quarter_turns_cw(self) -> u8 {
        // Phase 1 implements the real mapping.
        0
    }
}

/// The exact JSON shape of POST /state (see README "Cube-state JSON contract").
/// Field order is irrelevant to serde; keys are U R F D L B.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct CubeState {
    pub U: [StickerColor; 9],
    pub R: [StickerColor; 9],
    pub F: [StickerColor; 9],
    pub D: [StickerColor; 9],
    pub L: [StickerColor; 9],
    pub B: [StickerColor; 9],
}

impl CubeState {
    /// All 9 of each face = that face's solved color.
    pub fn solved() -> Self {
        // Phase 1 implements the real solved state from Face::solved_color.
        CubeState {
            U: [StickerColor::W; 9],
            R: [StickerColor::R; 9],
            F: [StickerColor::G; 9],
            D: [StickerColor::Y; 9],
            L: [StickerColor::O; 9],
            B: [StickerColor::B; 9],
        }
    }

    pub fn face(&self, f: Face) -> &[StickerColor; 9] {
        match f {
            Face::U => &self.U,
            Face::D => &self.D,
            Face::L => &self.L,
            Face::R => &self.R,
            Face::F => &self.F,
            Face::B => &self.B,
        }
    }

    /// Non-fatal sanity check per README "Validation notes": 6 faces × 9, each color ×9.
    /// Returns warnings; never rejects (impossible states are allowed).
    pub fn sanity_warnings(&self) -> Vec<String> {
        // Phase 1 implements the real checks.
        Vec::new()
    }
}
