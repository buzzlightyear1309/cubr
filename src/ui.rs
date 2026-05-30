use bevy::prelude::*;

use crate::cube::model::Move;
use crate::cube::MoveQueue;

/// Native bevy_ui panel of 18 move buttons -> MoveQueue.
///
/// A root panel is docked on the left as an absolutely-positioned `Node`; inside
/// it sit 6 rows (one per face, in `Move::ALL` order: U D L R F B), each row a
/// `[X] [X'] [X2]` triple. Each button carries its `Move` via [`MoveButton`];
/// the press-handler pushes that move onto [`MoveQueue`] (the same queue Phase 4
/// drains) on the `Interaction::Pressed` transition.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_panel)
            .add_systems(Update, handle_buttons);
    }
}

/// Marks a UI button and remembers which move it enqueues.
#[derive(Component, Clone, Copy)]
struct MoveButton(Move);

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

/// Spawn the docked panel and its 18 labeled buttons.
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
        ))
        .with_children(|panel| {
            // `Move::ALL` is already grouped U,U',U2, D,..., B,B',B2 — chunk into
            // rows of 3 so each row holds one face's three turns.
            for row in Move::ALL.chunks(3) {
                panel
                    .spawn(Node {
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
}

/// Spawn one move button (a `Button` + `Node` + colors) with a centered text label child.
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

/// React to button interactions: enqueue the move on the press transition and
/// give visual feedback for the three states. `Changed<Interaction>` fires once
/// per transition, so a click enqueues exactly one move.
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
