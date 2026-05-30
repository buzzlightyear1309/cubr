// Phase 0 stubs: components/resource defined but not yet spawned (Phase 2).
#![allow(dead_code)]

use bevy::prelude::*;

/// Links a Bevy entity to its core cubie via the stable `home` id.
#[derive(Component)]
pub struct Cubie {
    pub home: IVec3,
}

/// A sticker quad on a cubie face, identified by its outward local normal.
#[derive(Component)]
pub struct Sticker {
    pub local_normal: IVec3,
}

/// Shared material handles: one per `StickerColor` + a dark body material.
/// Phase 2 fills in the real fields.
#[derive(Resource)]
#[allow(dead_code)]
pub struct CubeMaterials {
    // per-color + body handles
}
