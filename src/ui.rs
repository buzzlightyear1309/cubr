use bevy::prelude::*;

use crate::camera::OrbitCamera;
use crate::cube::model::{Move, Turn};
use crate::cube::MoveQueue;
use crate::view_relative::{relative_move, RelFace};

/// Native bevy_ui panel feeding the shared `MoveQueue`, with a header toggle
/// between two control schemes:
///
/// - **Standard** (default): the original 18 absolute-move buttons, laid out as
///   6 rows (one per face, in `Move::ALL` order: U D L R F B), each row a
///   `[X] [X'] [X2]` triple. Each carries its `Move` via [`MoveButton`].
/// - **Beginner**: 18 view-relative buttons (6 relative faces × CW/CCW/180°)
///   with full-word labels. Each carries a [`RelMoveButton`] and, on
///   press, resolves to an absolute `Move` against the current camera basis via
///   [`relative_move`].
///
/// Both schemes push onto the same [`MoveQueue`] (the queue Phase 4 drains) on
/// the `Interaction::Pressed` transition. Only one panel is visible at a time;
/// the hidden one (`Display::None`) receives no pointer interaction, so the two
/// handlers never both fire.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControlScheme>()
            .add_systems(Startup, spawn_panel)
            .add_systems(
                Update,
                (
                    handle_buttons,
                    handle_relative_buttons,
                    handle_scheme_toggle,
                    update_panel_visibility.run_if(resource_changed::<ControlScheme>),
                ),
            );
    }
}

/// Which control scheme is currently active. Default is `Standard`.
#[derive(Resource, Default, PartialEq, Eq, Clone, Copy)]
enum ControlScheme {
    #[default]
    Standard,
    Beginner,
}

/// A header toggle button; remembers which scheme it selects.
#[derive(Component, Clone, Copy)]
struct SchemeToggle(ControlScheme);

/// Marker on the absolute-grid (Standard) container.
#[derive(Component)]
struct StandardPanel;

/// Marker on the relative-grid (Beginner) container.
#[derive(Component)]
struct BeginnerPanel;

/// Marks a Standard button and remembers which absolute move it enqueues.
#[derive(Component, Clone, Copy)]
struct MoveButton(Move);

/// Marks a Beginner button: a view-relative face + turn, resolved to an absolute
/// `Move` against the camera basis at press time (not a fixed `Move`).
#[derive(Component, Clone, Copy)]
struct RelMoveButton {
    rel: RelFace,
    turn: Turn,
}

// --- Styling ------------------------------------------------------------------

/// Subtle semi-transparent dark panel background.
const PANEL_BG: Color = Color::srgba(0.10, 0.10, 0.12, 0.85);
/// Button colors for the three interaction states.
const BTN_NORMAL: Color = Color::srgb(0.18, 0.18, 0.22);
const BTN_HOVER: Color = Color::srgb(0.28, 0.28, 0.34);
const BTN_PRESSED: Color = Color::srgb(0.40, 0.55, 0.85);
/// Thin button border + label color.
const BTN_BORDER: Color = Color::srgb(0.32, 0.32, 0.40);
const LABEL_COLOR: Color = Color::srgb(0.92, 0.92, 0.95);

const BUTTON_WIDTH: f32 = 52.0;
const BUTTON_HEIGHT: f32 = 32.0;
const LABEL_FONT_SIZE: f32 = 16.0;

/// Wider button for the Beginner panel so full-word labels like
/// "Front 180" / "Right CCW" fit on one line.
const BEGINNER_BUTTON_WIDTH: f32 = 112.0;
/// Header toggle button width — wide enough for "Standard" / "Beginner".
const TOGGLE_WIDTH: f32 = 84.0;

/// The six relative faces, in the row order the Beginner panel lays them out.
const REL_FACES: [RelFace; 6] = [
    RelFace::Front,
    RelFace::Back,
    RelFace::Up,
    RelFace::Down,
    RelFace::Left,
    RelFace::Right,
];

/// Spawn the docked panel: a header scheme toggle plus the Standard and Beginner
/// grids (only one shown at a time).
fn spawn_panel(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                top: Val::Px(12.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(6.0),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            // Gives the panel background an `Interaction` so the camera's
            // `pointer_over_ui` guard ignores drags on the padding/gaps (only
            // `Button` nodes get an `Interaction` automatically).
            Interaction::default(),
        ))
        .with_children(|panel| {
            // 1. Header row: the two scheme-toggle buttons.
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|header| {
                    spawn_toggle(header, ControlScheme::Standard, "Standard");
                    spawn_toggle(header, ControlScheme::Beginner, "Beginner");
                });

            // 2. Standard grid (visible by default): the existing 6×3 absolute
            //    buttons. `Move::ALL` is already grouped U,U',U2, D,..., B,B',B2.
            panel
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(6.0),
                        display: Display::Flex,
                        ..default()
                    },
                    StandardPanel,
                    Interaction::default(),
                ))
                .with_children(|grid| {
                    for row in Move::ALL.chunks(3) {
                        grid.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(6.0),
                            ..default()
                        })
                        .with_children(|row_node| {
                            for &mv in row {
                                spawn_button(row_node, mv);
                            }
                        });
                    }
                });

            // 3. Beginner grid (hidden by default): 6 rows of relative buttons,
            //    one per relative face, each `[CW] [CCW] [180°]`.
            panel
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(6.0),
                        display: Display::None,
                        ..default()
                    },
                    BeginnerPanel,
                    Interaction::default(),
                ))
                .with_children(|grid| {
                    for &rel in &REL_FACES {
                        grid.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(6.0),
                            ..default()
                        })
                        .with_children(|row_node| {
                            for turn in [Turn::Cw, Turn::Ccw, Turn::Double] {
                                spawn_relative_button(row_node, rel, turn);
                            }
                        });
                    }
                });
        });
}

/// Spawn one header toggle button labeled with its scheme name.
fn spawn_toggle(parent: &mut ChildSpawnerCommands, scheme: ControlScheme, label: &str) {
    parent
        .spawn((
            Button,
            SchemeToggle(scheme),
            Node {
                width: Val::Px(TOGGLE_WIDTH),
                height: Val::Px(BUTTON_HEIGHT),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(BTN_NORMAL),
            BorderColor::all(BTN_BORDER),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(label),
                TextFont {
                    font_size: LABEL_FONT_SIZE,
                    ..default()
                },
                TextColor(LABEL_COLOR),
            ));
        });
}

/// Spawn one absolute-move button (a `Button` + `Node` + colors) with a centered
/// text label child.
fn spawn_button(parent: &mut ChildSpawnerCommands, mv: Move) {
    parent
        .spawn((
            Button,
            MoveButton(mv),
            Node {
                width: Val::Px(BUTTON_WIDTH),
                height: Val::Px(BUTTON_HEIGHT),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(BTN_NORMAL),
            BorderColor::all(BTN_BORDER),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(mv.notation()),
                TextFont {
                    font_size: LABEL_FONT_SIZE,
                    ..default()
                },
                TextColor(LABEL_COLOR),
            ));
        });
}

/// Spawn one view-relative button (wider, to fit the full-word label on one line).
fn spawn_relative_button(parent: &mut ChildSpawnerCommands, rel: RelFace, turn: Turn) {
    parent
        .spawn((
            Button,
            RelMoveButton { rel, turn },
            Node {
                width: Val::Px(BEGINNER_BUTTON_WIDTH),
                height: Val::Px(BUTTON_HEIGHT),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(BTN_NORMAL),
            BorderColor::all(BTN_BORDER),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(rel_label(rel, turn)),
                // Never wrap: the stock Bevy font has no rotation-arrow glyphs, so
                // turns are spelled out (CW/CCW/180°) and must stay on one line.
                TextLayout::new_with_no_wrap(),
                TextFont {
                    font_size: LABEL_FONT_SIZE,
                    ..default()
                },
                TextColor(LABEL_COLOR),
            ));
        });
}

/// Full-word name for a relative face.
fn rel_word(rel: RelFace) -> &'static str {
    match rel {
        RelFace::Front => "Front",
        RelFace::Back => "Back",
        RelFace::Up => "Up",
        RelFace::Down => "Down",
        RelFace::Left => "Left",
        RelFace::Right => "Right",
    }
}

/// Beginner button label: full word + spelled-out turn (CW, CCW, 180). The stock
/// Bevy font ships a minimal glyph set — rotation arrows (↻/↺) and even the
/// degree sign render as tofu — so labels stay plain ASCII: turn direction is
/// text and the half turn is "180" (no ° symbol).
fn rel_label(rel: RelFace, turn: Turn) -> String {
    let word = rel_word(rel);
    match turn {
        Turn::Cw => format!("{word} CW"),
        Turn::Ccw => format!("{word} CCW"),
        Turn::Double => format!("{word} 180"),
    }
}

/// React to Standard button interactions: enqueue the absolute move on the press
/// transition and give visual feedback for the three states. `Changed<Interaction>`
/// fires once per transition, so a click enqueues exactly one move.
fn handle_buttons(
    mut interactions: Query<
        (&Interaction, &MoveButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
    mut queue: ResMut<MoveQueue>,
) {
    for (interaction, button, mut bg) in &mut interactions {
        match *interaction {
            Interaction::Pressed => {
                queue.0.push_back(button.0);
                bg.0 = BTN_PRESSED;
            }
            Interaction::Hovered => bg.0 = BTN_HOVER,
            Interaction::None => bg.0 = BTN_NORMAL,
        }
    }
}

/// React to Beginner button interactions: resolve the view-relative face+turn to
/// an absolute move against the current camera basis and enqueue it on press.
/// Same per-state color feedback as `handle_buttons`.
fn handle_relative_buttons(
    mut interactions: Query<
        (&Interaction, &RelMoveButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
    orbit: Res<OrbitCamera>,
    mut queue: ResMut<MoveQueue>,
) {
    for (interaction, button, mut bg) in &mut interactions {
        match *interaction {
            Interaction::Pressed => {
                queue
                    .0
                    .push_back(relative_move(orbit.basis(), button.rel, button.turn));
                bg.0 = BTN_PRESSED;
            }
            Interaction::Hovered => bg.0 = BTN_HOVER,
            Interaction::None => bg.0 = BTN_NORMAL,
        }
    }
}

/// On press, switch the active control scheme to whatever the toggle selects.
fn handle_scheme_toggle(
    interactions: Query<(&Interaction, &SchemeToggle), Changed<Interaction>>,
    mut scheme: ResMut<ControlScheme>,
) {
    for (interaction, toggle) in &interactions {
        if *interaction == Interaction::Pressed {
            *scheme = toggle.0;
        }
    }
}

/// Reflect the active scheme into the UI: show the matching grid, hide the other,
/// and highlight the active toggle. Runs whenever `ControlScheme` changes —
/// including the first frame after init, so the initial state lands correctly.
fn update_panel_visibility(
    scheme: Res<ControlScheme>,
    mut standard: Query<&mut Node, (With<StandardPanel>, Without<BeginnerPanel>)>,
    mut beginner: Query<&mut Node, (With<BeginnerPanel>, Without<StandardPanel>)>,
    mut toggles: Query<(&SchemeToggle, &mut BackgroundColor)>,
) {
    let standard_active = *scheme == ControlScheme::Standard;
    for mut node in &mut standard {
        node.display = if standard_active {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut node in &mut beginner {
        node.display = if standard_active {
            Display::None
        } else {
            Display::Flex
        };
    }
    for (toggle, mut bg) in &mut toggles {
        bg.0 = if toggle.0 == *scheme {
            BTN_PRESSED
        } else {
            BTN_NORMAL
        };
    }
}
