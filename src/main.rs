use bevy::prelude::*;

mod api;
mod camera;
mod cube;
mod ui;
mod view_relative;

use api::ApiPlugin;
use camera::CameraPlugin;
use cube::CubePlugin;
use ui::UiPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Cube".to_string(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((CubePlugin, CameraPlugin, UiPlugin, ApiPlugin))
        .run();
}
