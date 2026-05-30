// Phase 2: spawn the 26 cubie entities + their sticker children, build the
// shared materials, and keep the Bevy visuals synced to the pure `CubeCore`.

use bevy::prelude::*;

use crate::cube::core::CoreCubie;
use crate::cube::model::StickerColor;
use crate::cube::Cube;

/// World units between adjacent cubie centers. With spacing 1.0, a core `pos`
/// component in {-1,0,1} maps straight to a world translation.
const SPACING: f32 = 1.0;
/// Edge length of a cubie body (slightly under 1.0 so neighbours show a seam).
const CUBIE_SIZE: f32 = 0.95;
/// Edge length of a sticker quad on a face.
const STICKER_SIZE: f32 = 0.8;
/// Thickness of a sticker quad.
const STICKER_THICKNESS: f32 = 0.02;
/// How far a sticker's center sits from the cubie center along its local normal:
/// half the body plus half the sticker thickness, plus a hair to avoid z-fighting.
const STICKER_INSET: f32 = CUBIE_SIZE / 2.0 + STICKER_THICKNESS / 2.0 + 0.001;

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
/// The per-color handles are indexed by `StickerColor::ALL` order.
#[derive(Resource)]
pub struct CubeMaterials {
    /// One shared `StandardMaterial` per `StickerColor`, in `StickerColor::ALL` order.
    colors: [Handle<StandardMaterial>; 6],
    /// Dark near-black material for the cubie bodies.
    body: Handle<StandardMaterial>,
}

impl CubeMaterials {
    /// Handle of the shared material for a given sticker color.
    pub fn color(&self, c: StickerColor) -> Handle<StandardMaterial> {
        let idx = StickerColor::ALL
            .iter()
            .position(|&x| x == c)
            .expect("every StickerColor is in StickerColor::ALL");
        self.colors[idx].clone()
    }

    /// Handle of the shared dark body material.
    pub fn body(&self) -> Handle<StandardMaterial> {
        self.body.clone()
    }
}

impl FromWorld for CubeMaterials {
    fn from_world(world: &mut World) -> Self {
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        let colors = StickerColor::ALL.map(|c| {
            materials.add(StandardMaterial {
                base_color: c.to_render_color(),
                perceptual_roughness: 0.6,
                reflectance: 0.1,
                ..default()
            })
        });
        let body = materials.add(StandardMaterial {
            base_color: Color::srgb(0.03, 0.03, 0.03),
            perceptual_roughness: 0.8,
            reflectance: 0.05,
            ..default()
        });
        CubeMaterials { colors, body }
    }
}

/// Startup system: spawn the 26 cubie bodies, each with its visible sticker
/// children, reading the initial pose/colors from the core.
pub fn spawn_cubies(
    mut commands: Commands,
    cube: Res<Cube>,
    materials: Res<CubeMaterials>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let body_mesh = meshes.add(Cuboid::new(CUBIE_SIZE, CUBIE_SIZE, CUBIE_SIZE));
    let sticker_mesh = meshes.add(Cuboid::new(STICKER_SIZE, STICKER_SIZE, STICKER_THICKNESS));

    for cubie in cube.0.cubies() {
        commands
            .spawn((
                Cubie { home: cubie.home },
                Mesh3d(body_mesh.clone()),
                MeshMaterial3d(materials.body()),
                transform_for(cubie),
            ))
            .with_children(|parent| {
                for &(local_normal, color) in &cubie.stickers {
                    parent.spawn((
                        Sticker { local_normal },
                        Mesh3d(sticker_mesh.clone()),
                        MeshMaterial3d(materials.color(color)),
                        sticker_transform(local_normal),
                    ));
                }
            });
    }
}

/// Sync system (runs on `Update`, gated on `CoreChanged`, and once right after
/// spawn): copy every cubie's pose and every sticker's color from the core onto
/// the Bevy entities, restoring the integer-grid invariant.
pub fn sync_visuals(
    cube: Res<Cube>,
    mut cubies: Query<(&Cubie, &mut Transform)>,
    mut stickers: Query<(&Sticker, &ChildOf, &mut MeshMaterial3d<StandardMaterial>)>,
    cubie_homes: Query<&Cubie>,
    materials: Res<CubeMaterials>,
) {
    // Pose: snap each cubie body onto its core pose.
    for (cubie, mut transform) in &mut cubies {
        if let Some(core) = find_core(&cube, cubie.home) {
            *transform = transform_for(core);
        }
    }

    // Color: repaint each sticker from the matching core sticker (by parent home
    // + local outward normal).
    for (sticker, child_of, mut material) in &mut stickers {
        let Ok(parent) = cubie_homes.get(child_of.parent()) else {
            continue;
        };
        let Some(core) = find_core(&cube, parent.home) else {
            continue;
        };
        if let Some(&(_, color)) = core
            .stickers
            .iter()
            .find(|(local, _)| *local == sticker.local_normal)
        {
            *material = MeshMaterial3d(materials.color(color));
        }
    }
}

/// Find the core cubie with the given `home`.
fn find_core(cube: &Cube, home: IVec3) -> Option<&CoreCubie> {
    cube.0.cubies().iter().find(|c| c.home == home)
}

/// World transform for a cubie body from its core pose.
fn transform_for(cubie: &CoreCubie) -> Transform {
    let rotation = Quat::from_mat3(&Mat3::from_cols(
        cubie.orient[0].as_vec3(),
        cubie.orient[1].as_vec3(),
        cubie.orient[2].as_vec3(),
    ));
    Transform {
        translation: cubie.pos.as_vec3() * SPACING,
        rotation,
        ..default()
    }
}

/// Local transform for a sticker quad: sit just outside the body face along the
/// local normal, with the quad's thin axis (+Z of the mesh) pointing outward.
fn sticker_transform(local_normal: IVec3) -> Transform {
    let normal = local_normal.as_vec3();
    let translation = normal * STICKER_INSET;
    // Rotate the quad's local +Z onto the outward normal. `Quat::from_rotation_arc`
    // handles the antiparallel case (e.g. the -Z back face) with a stable axis.
    let rotation = Quat::from_rotation_arc(Vec3::Z, normal.normalize());
    Transform {
        translation,
        rotation,
        ..default()
    }
}
