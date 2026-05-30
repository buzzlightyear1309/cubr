use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::*;

mod api;
mod camera;
mod cube;
mod geom;
mod solve_ui;
mod solver;
mod swipe;
mod ui;
mod view_relative;

use api::ApiPlugin;
use camera::CameraPlugin;
use cube::CubePlugin;
use solve_ui::SolverPlugin;
use swipe::SwipePlugin;
use ui::UiPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "cubr".to_string(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((
            CubePlugin,
            CameraPlugin,
            UiPlugin,
            ApiPlugin,
            MeshPickingPlugin,
            SwipePlugin,
            SolverPlugin,
        ))
        .run();
}
