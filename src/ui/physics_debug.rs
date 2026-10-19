//! `F3+B` toggles Avian's physics debug gizmos.
//!
//! The renderer is kept query-focused by default: voxel chunk colliders span
//! thousands of blocks, so drawing every collider wireframe is both noisy and
//! expensive. Ray/shape casts, their hit points and normals, point projections,
//! and shape-intersection hits are the useful parts for movement and block
//! targeting bugs.

use avian3d::prelude::PhysicsGizmos;
use bevy::{
    gizmos::config::{GizmoConfig, GizmoConfigStore},
    prelude::*,
};

use crate::input::ModifierCombo;

// Debug chords stay local, like F3+G/F3+L. `KeyBindings` is the gameplay
// input surface, not the developer-tool surface.
const PHYSICS_DEBUG_KEY: KeyCode = KeyCode::KeyB;

pub(super) struct PhysicsDebugTogglePlugin;

impl Plugin for PhysicsDebugTogglePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhysicsDebugVisible>()
            .init_resource::<ModifierCombo>()
            .insert_gizmo_config(
                query_only_gizmos(),
                GizmoConfig {
                    enabled: false,
                    ..default()
                },
            )
            .add_systems(
                Update,
                toggle_physics_debug.before(super::debug::toggle_debug),
            );
    }
}

#[derive(Resource, Default)]
struct PhysicsDebugVisible(bool);

/// Returns a deliberately query-only gizmo configuration.
///
/// In particular, `collider_color`, `aabb_color`, `axis_lengths`, contacts,
/// joints, islands, and collider-tree nodes stay disabled.
fn query_only_gizmos() -> PhysicsGizmos {
    PhysicsGizmos {
        raycast_color: Some(Color::srgb(1.0, 0.25, 0.25)),
        raycast_point_color: Some(Color::srgb(1.0, 0.9, 0.25)),
        raycast_normal_color: Some(Color::srgb(1.0, 0.4, 0.7)),
        shapecast_color: Some(Color::srgb(0.25, 0.9, 1.0)),
        shapecast_shape_color: Some(Color::srgb(0.4, 0.6, 1.0)),
        shapecast_point_color: Some(Color::srgb(1.0, 0.9, 0.25)),
        shapecast_normal_color: Some(Color::srgb(1.0, 0.4, 0.7)),
        point_projection_color: Some(Color::WHITE),
        point_projection_origin_color: Some(Color::WHITE),
        point_projection_point_color: Some(Color::srgb(1.0, 0.9, 0.25)),
        shape_intersection_shape_color: Some(Color::WHITE),
        shape_intersection_color: Some(Color::srgb(1.0, 0.9, 0.25)),
        ..PhysicsGizmos::none()
    }
}

fn toggle_physics_debug(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut combo: ResMut<ModifierCombo>,
    mut visible: ResMut<PhysicsDebugVisible>,
    mut store: ResMut<GizmoConfigStore>,
) {
    if !keyboard.just_pressed(PHYSICS_DEBUG_KEY) || !keyboard.pressed(KeyCode::F3) {
        return;
    }

    combo.mark_combo();
    visible.0 = !visible.0;
    store.config_mut::<PhysicsGizmos>().0.enabled = visible.0;
}

#[cfg(test)]
mod tests {
    use super::super::debug::{DebugVisible, toggle_debug};
    use super::*;

    fn physics_debug_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(ButtonInput::<KeyCode>::default())
            .add_plugins(PhysicsDebugTogglePlugin);
        app
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
    }

    fn release(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(key);
    }

    fn config_enabled(app: &App) -> bool {
        app.world()
            .resource::<GizmoConfigStore>()
            .get_config::<PhysicsGizmos>()
            .unwrap()
            .0
            .enabled
    }

    #[test]
    fn default_physics_gizmos_are_query_only_and_disabled() {
        let app = physics_debug_app();
        let (config, gizmos) = app
            .world()
            .resource::<GizmoConfigStore>()
            .get_config::<PhysicsGizmos>()
            .unwrap();

        assert!(!config.enabled);
        assert!(gizmos.raycast_color.is_some());
        assert!(gizmos.shapecast_color.is_some());
        assert!(gizmos.collider_color.is_none());
        assert!(gizmos.aabb_color.is_none());
        assert!(gizmos.axis_lengths.is_none());
        assert!(gizmos.contact_point_color.is_none());
        assert!(gizmos.joint_anchor_color.is_none());
        assert!(gizmos.collider_tree_color.is_none());
    }

    #[test]
    fn f3_b_toggles_the_physics_gizmos_and_consumes_the_chord() {
        let mut app = physics_debug_app();
        press(&mut app, KeyCode::F3);
        press(&mut app, PHYSICS_DEBUG_KEY);
        app.world_mut().run_schedule(Update);

        assert!(app.world().resource::<PhysicsDebugVisible>().0);
        assert!(config_enabled(&app));

        release(&mut app, KeyCode::F3);
        release(&mut app, PHYSICS_DEBUG_KEY);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        assert!(
            !app.world_mut().resource_mut::<ModifierCombo>().check_solo(),
            "F3+B must suppress the solo F3 debug-overlay toggle"
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        press(&mut app, KeyCode::F3);
        press(&mut app, PHYSICS_DEBUG_KEY);
        app.world_mut().run_schedule(Update);
        assert!(!app.world().resource::<PhysicsDebugVisible>().0);
        assert!(!config_enabled(&app));
    }

    #[test]
    fn adding_avian_debug_plugin_preserves_the_disabled_query_config() {
        let mut app = physics_debug_app();
        app.add_plugins(avian3d::prelude::PhysicsDebugPlugin);

        let (config, gizmos) = app
            .world()
            .resource::<GizmoConfigStore>()
            .get_config::<PhysicsGizmos>()
            .unwrap();
        assert!(!config.enabled);
        assert!(gizmos.collider_color.is_none());
        assert!(gizmos.aabb_color.is_none());
    }

    #[test]
    fn f3_b_suppresses_the_text_overlay_toggle() {
        let mut app = physics_debug_app();
        app.init_resource::<DebugVisible>()
            .add_systems(Update, toggle_debug);

        press(&mut app, KeyCode::F3);
        press(&mut app, PHYSICS_DEBUG_KEY);
        app.world_mut().run_schedule(Update);
        assert!(app.world().resource::<PhysicsDebugVisible>().0);

        release(&mut app, KeyCode::F3);
        app.world_mut().run_schedule(Update);

        assert!(
            !app.world().resource::<DebugVisible>().0,
            "releasing F3 after F3+B must not toggle the text overlay"
        );
    }

    #[test]
    fn b_alone_does_nothing() {
        let mut app = physics_debug_app();
        press(&mut app, PHYSICS_DEBUG_KEY);
        app.world_mut().run_schedule(Update);

        assert!(!app.world().resource::<PhysicsDebugVisible>().0);
        assert!(!config_enabled(&app));
    }
}
