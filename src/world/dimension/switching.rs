use avian3d::prelude::{Collider, Position};
use bevy::{input::InputSystems, prelude::*};

use crate::{
    game_state::GameState,
    mob::controller::{CharacterController, Grounded, Velocity},
    player::{
        Player, PlayerDimension,
        control::KeyBindings,
        interaction::{BlockInteractionRequest, BlockInteractionSystems, CurrentBlockTarget},
    },
    world::chunk::{
        Chunk, ChunkColliderRuntime, ChunkColumn, ChunkContentCounts, ChunkHeightmap,
        ChunkNeedsSave, ChunkPos, ChunkPosition, collider::column_colliders_ready,
    },
};

use super::{
    Active, ChunkSaveTasks, DesiredColumnView, Dimension, DimensionCatalog, DimensionStreamingSet,
    persistence::{SaveSnapshotContext, capture_dimension_save_snapshots},
};

pub(super) fn install(app: &mut App) {
    app.add_systems(
        PreUpdate,
        begin_dimension_switch
            .after(InputSystems)
            .run_if(in_state(GameState::Playing)),
    )
    .add_systems(
        FixedUpdate,
        suppress_transition_interactions
            .before(BlockInteractionSystems::EmitRequests)
            .run_if(in_state(GameState::Playing)),
    )
    .add_systems(
        Update,
        finish_dimension_switch
            .after(DimensionStreamingSet)
            .run_if(in_state(GameState::Playing)),
    );
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct AwaitingDimensionReady {
    target_root: Entity,
    target_id: crate::world::DimensionId,
    center: ChunkColumn,
    restore_character_controller: bool,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn begin_dimension_switch(
    mut commands: Commands,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    bindings: Option<Res<KeyBindings>>,
    catalog: Res<DimensionCatalog>,
    mut save_tasks: ResMut<ChunkSaveTasks>,
    mut dimensions: Query<(Entity, &mut Dimension, &mut DesiredColumnView, Has<Active>)>,
    chunks: Query<(
        &ChunkPosition,
        &Chunk,
        &ChunkHeightmap,
        Option<&ChunkNeedsSave>,
    )>,
    player: Option<
        Single<
            (
                Entity,
                &mut PlayerDimension,
                &mut Transform,
                &mut Position,
                &mut Velocity,
                Has<CharacterController>,
                Has<AwaitingDimensionReady>,
            ),
            With<Player>,
        >,
    >,
    current_target: Option<ResMut<CurrentBlockTarget>>,
    requests: Option<ResMut<Messages<BlockInteractionRequest>>>,
) {
    let (Some(keys), Some(bindings), Some(player)) = (keys, bindings, player) else {
        return;
    };
    let (
        player_entity,
        mut player_dimension,
        mut player_transform,
        mut player_position,
        mut velocity,
        had_character_controller,
        transitioning,
    ) = player.into_inner();
    if transitioning || !keys.just_pressed(bindings.switch_dimension) {
        return;
    }

    let roots = dimensions
        .iter_mut()
        .map(|(entity, dimension, _, active)| (entity, dimension.id(), active))
        .collect::<Vec<_>>();
    let active_roots = roots
        .iter()
        .filter(|(_, _, active)| *active)
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(
        active_roots.len(),
        1,
        "dimension switching requires exactly one active root"
    );
    let (outgoing_root, outgoing_id, _) = active_roots[0];
    assert_eq!(
        player_dimension.id(),
        outgoing_id,
        "player membership must match the active dimension before switching"
    );

    let target_id = catalog
        .next_id(outgoing_id)
        .expect("active dimension must belong to the world catalog");
    let target_root = roots
        .iter()
        .find_map(|&(entity, id, _)| (id == target_id).then_some(entity))
        .expect("target catalog dimension must retain a persistent runtime root");
    let target_arrival = *catalog
        .get(target_id)
        .expect("cycled dimension must retain its definition");
    let target_arrival = target_arrival.arrival();
    let center = ChunkColumn::from(ChunkPos::containing_translation(target_arrival));

    let (captured, evicted) = {
        let (_, mut outgoing, mut outgoing_view, active) = dimensions
            .get_mut(outgoing_root)
            .expect("active dimension root must remain queryable");
        assert!(active);
        let captured = capture_dimension_save_snapshots(
            &mut save_tasks,
            &outgoing,
            SaveSnapshotContext::Detached,
            outgoing_root,
            &chunks,
        );
        let evicted = outgoing.drain_streamed_columns();
        *outgoing_view = DesiredColumnView::default();
        (captured, evicted)
    };

    for column in &evicted {
        commands.entity(column.incarnation).despawn();
    }
    commands
        .entity(outgoing_root)
        .remove::<Active>()
        .insert(Visibility::Hidden);
    commands
        .entity(target_root)
        .insert((Active, Visibility::Inherited));

    **velocity = Vec3::ZERO;
    player_position.0 = target_arrival;
    player_transform.translation = target_arrival;
    *player_dimension = PlayerDimension::new(target_id);
    commands
        .entity(player_entity)
        .remove::<(CharacterController, Grounded)>()
        .insert(AwaitingDimensionReady {
            target_root,
            target_id,
            center,
            restore_character_controller: had_character_controller,
        });

    if let Some(mut current_target) = current_target {
        current_target.0 = None;
    }
    if let Some(mut requests) = requests {
        requests.clear();
    }

    info!(
        from = %outgoing_id,
        to = %target_id,
        captured_saves = captured,
        evicted_columns = evicted.len(),
        "Switching dimensions"
    );
}

fn suppress_transition_interactions(
    transitioning_players: Query<(), (With<Player>, With<AwaitingDimensionReady>)>,
    current_target: Option<ResMut<CurrentBlockTarget>>,
    requests: Option<ResMut<Messages<BlockInteractionRequest>>>,
) {
    if transitioning_players.is_empty() {
        return;
    }
    if let Some(mut current_target) = current_target {
        current_target.0 = None;
    }
    if let Some(mut requests) = requests {
        requests.clear();
    }
}

#[allow(clippy::type_complexity)]
fn finish_dimension_switch(
    mut commands: Commands,
    player: Option<Single<(Entity, &AwaitingDimensionReady, &mut Velocity), With<Player>>>,
    active_dimension: Single<(Entity, &Dimension), With<Active>>,
    chunks: Query<(&ChunkPosition, &ChunkContentCounts, Option<&Children>)>,
    colliders: Query<(), With<Collider>>,
    collider_runtime: Option<Res<ChunkColliderRuntime>>,
) {
    let Some(player) = player else { return };
    let (player_entity, transition, mut velocity) = player.into_inner();
    let (active_root, dimension) = active_dimension.into_inner();
    if active_root != transition.target_root || dimension.id() != transition.target_id {
        return;
    }

    let Some(center_chunks) = dimension.complete_loaded_column(transition.center) else {
        return;
    };
    if !center_chunks
        .iter()
        .all(|(position, entity)| dimension.published_chunk_entity(*position) == Some(*entity))
    {
        return;
    }

    let colliders_enabled = collider_runtime
        .as_deref()
        .is_none_or(|runtime| runtime.enabled());
    if colliders_enabled
        && !column_colliders_ready(dimension, transition.center, &chunks, &colliders)
    {
        return;
    }

    **velocity = Vec3::ZERO;
    let mut player = commands.entity(player_entity);
    player.remove::<AwaitingDimensionReady>();
    if transition.restore_character_controller {
        player.insert(CharacterController);
    }
    info!(dimension = %transition.target_id, "Dimension switch ready");
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::{
        block::BlockType,
        player::interaction::{BlockInteractionKind, BlockTarget},
        world::{
            WorldMetadata,
            chunk::{ChunkBlockPos, ChunkPerfCounters, LocalBlockPos},
            dimension::{
                ChunkTaskPool, ColumnActivationBudget, ColumnLightBudget, ColumnLoadBudget,
                ColumnStagingBudget, ViewDistance,
                light::{cancel_inactive_dimension_light_tasks, rebuild_chunk_light},
                persistence::{ChunkSaveBudget, finish_chunk_save_tasks, start_chunk_save_tasks},
                streaming::{
                    finish_column_loads, maintain_column_residency, publish_lit_columns,
                    refresh_desired_column_view, start_column_loads,
                },
            },
            storage::{ChunkRepository, InMemoryChunkStore},
        },
    };

    #[derive(Resource)]
    struct TestRoots {
        overworld: Entity,
        grass: Entity,
        glass: Entity,
        player: Entity,
    }

    fn switch_app() -> App {
        let metadata = WorldMetadata::with_seed(7).with_height_chunks(1).unwrap();
        let catalog = DimensionCatalog::for_world(&metadata);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(catalog.clone())
            .init_resource::<ChunkSaveTasks>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<KeyBindings>()
            .init_resource::<Messages<BlockInteractionRequest>>()
            .insert_resource(CurrentBlockTarget(Some(test_target())))
            .add_systems(
                Update,
                (begin_dimension_switch, finish_dimension_switch).chain(),
            );

        let overworld = Dimension::spawn_in_world(
            app.world_mut(),
            &catalog,
            crate::world::DimensionId::OVERWORLD,
        );
        let grass = Dimension::spawn_in_world(
            app.world_mut(),
            &catalog,
            crate::world::DimensionId::GRASS_FLOOR,
        );
        let glass = Dimension::spawn_in_world(
            app.world_mut(),
            &catalog,
            crate::world::DimensionId::CENTER_GLASS_PLATFORM,
        );
        app.world_mut().entity_mut(overworld).insert(Active);
        app.world_mut().entity_mut(grass).insert(Visibility::Hidden);
        app.world_mut().entity_mut(glass).insert(Visibility::Hidden);

        let arrival = catalog
            .get(crate::world::DimensionId::OVERWORLD)
            .unwrap()
            .arrival();
        let player = app
            .world_mut()
            .spawn((
                Player::default(),
                PlayerDimension::new(crate::world::DimensionId::OVERWORLD),
                Position::new(arrival),
                Transform::from_translation(arrival),
                CharacterController,
            ))
            .id();
        app.insert_resource(TestRoots {
            overworld,
            grass,
            glass,
            player,
        });
        app
    }

    fn streamed_switch_app() -> App {
        let metadata = WorldMetadata::with_seed(7).with_height_chunks(1).unwrap();
        let catalog = DimensionCatalog::for_world(&metadata);
        let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata.clone()));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata)
            .insert_resource(catalog.clone())
            .insert_resource(repository)
            .insert_resource(ChunkTaskPool::new_for_test())
            .insert_resource(ChunkSaveBudget(0))
            .init_resource::<ChunkSaveTasks>()
            .insert_resource(ColumnLoadBudget(9))
            .insert_resource(ColumnStagingBudget(9))
            .insert_resource(ColumnActivationBudget(9))
            .insert_resource(ColumnLightBudget(100))
            .insert_resource(ViewDistance::new(1))
            .insert_resource(ChunkColliderRuntime::new(false))
            .init_resource::<ChunkPerfCounters>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<KeyBindings>()
            .add_systems(PreUpdate, begin_dimension_switch)
            .add_systems(
                Update,
                (
                    cancel_inactive_dimension_light_tasks,
                    refresh_desired_column_view,
                    maintain_column_residency,
                    finish_column_loads,
                    rebuild_chunk_light,
                    publish_lit_columns,
                    start_column_loads,
                    finish_dimension_switch,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                (finish_chunk_save_tasks, start_chunk_save_tasks).chain(),
            );

        let overworld = Dimension::spawn_in_world(
            app.world_mut(),
            &catalog,
            crate::world::DimensionId::OVERWORLD,
        );
        let grass = Dimension::spawn_in_world(
            app.world_mut(),
            &catalog,
            crate::world::DimensionId::GRASS_FLOOR,
        );
        let glass = Dimension::spawn_in_world(
            app.world_mut(),
            &catalog,
            crate::world::DimensionId::CENTER_GLASS_PLATFORM,
        );
        app.world_mut().entity_mut(overworld).insert(Active);
        app.world_mut().entity_mut(grass).insert(Visibility::Hidden);
        app.world_mut().entity_mut(glass).insert(Visibility::Hidden);

        let arrival = catalog
            .get(crate::world::DimensionId::OVERWORLD)
            .unwrap()
            .arrival();
        let player = app
            .world_mut()
            .spawn((
                Player::default(),
                PlayerDimension::new(crate::world::DimensionId::OVERWORLD),
                Position::new(arrival),
                Transform::from_translation(arrival),
                CharacterController,
            ))
            .id();
        app.insert_resource(TestRoots {
            overworld,
            grass,
            glass,
            player,
        });
        app
    }

    fn update_until(app: &mut App, mut predicate: impl FnMut(&World) -> bool) {
        for _ in 0..1_000 {
            app.update();
            if predicate(app.world()) {
                return;
            }
            thread::yield_now();
        }
        panic!("condition was not met after 1,000 updates");
    }

    fn trigger_dimension_switch(app: &mut App) {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.reset(KeyCode::F6);
        keys.press(KeyCode::F6);
        drop(keys);
        app.update();
    }

    fn test_target() -> BlockTarget {
        let position = ChunkBlockPos::new(ChunkPos::ZERO, LocalBlockPos::ZERO);
        BlockTarget {
            hit_block: position,
            adjacent_block: position,
        }
    }

    fn request() -> BlockInteractionRequest {
        BlockInteractionRequest {
            kind: BlockInteractionKind::Break,
            target: test_target(),
        }
    }

    fn active_root(world: &mut World) -> Entity {
        world
            .query_filtered::<Entity, (With<Dimension>, With<Active>)>()
            .single(world)
            .unwrap()
    }

    fn publish_solid_center(app: &mut App, root: Entity) -> Entity {
        let position = ChunkPos::ZERO;
        let entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(position),
                ChunkContentCounts {
                    solid: 1,
                    ..default()
                },
            ))
            .id();
        app.world_mut()
            .get_mut::<Dimension>(root)
            .unwrap()
            .register_published_chunk(position, entity);
        entity
    }

    #[test]
    fn cycle_swaps_roots_teleports_and_freezes_the_player() {
        let mut app = switch_app();
        let roots = app.world().resource::<TestRoots>();
        let (overworld, grass, player) = (roots.overworld, roots.grass, roots.player);
        app.world_mut()
            .resource_mut::<Messages<BlockInteractionRequest>>()
            .write(request());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F6);

        app.update();

        assert_eq!(active_root(app.world_mut()), grass);
        assert_eq!(
            app.world().get::<Visibility>(overworld),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            app.world().get::<Visibility>(grass),
            Some(&Visibility::Inherited)
        );
        assert_eq!(
            app.world().get::<PlayerDimension>(player).unwrap().id(),
            crate::world::DimensionId::GRASS_FLOOR
        );
        let arrival = app
            .world()
            .resource::<DimensionCatalog>()
            .get(crate::world::DimensionId::GRASS_FLOOR)
            .unwrap()
            .arrival();
        assert_eq!(app.world().get::<Position>(player).unwrap().0, arrival);
        assert_eq!(
            app.world().get::<Transform>(player).unwrap().translation,
            arrival
        );
        assert!(app.world().get::<AwaitingDimensionReady>(player).is_some());
        assert!(app.world().get::<CharacterController>(player).is_none());
        assert_eq!(app.world().resource::<CurrentBlockTarget>().0, None);
        assert!(
            app.world()
                .resource::<Messages<BlockInteractionRequest>>()
                .is_empty()
        );

        app.update();
        assert_eq!(active_root(app.world_mut()), grass);
    }

    #[test]
    fn world_only_apps_leave_switching_dormant_without_player_input_resources() {
        let metadata = WorldMetadata::with_seed(7);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DimensionCatalog::for_world(&metadata))
            .init_resource::<ChunkSaveTasks>()
            .add_systems(Update, begin_dimension_switch);

        app.update();
    }

    #[test]
    fn readiness_waits_for_the_exact_target_collider_then_restores_control() {
        let mut app = switch_app();
        let roots = app.world().resource::<TestRoots>();
        let (grass, player) = (roots.grass, roots.player);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F6);
        app.update();

        let chunk = publish_solid_center(&mut app, grass);
        **app.world_mut().get_mut::<Velocity>(player).unwrap() = Vec3::X;
        app.update();
        assert!(app.world().get::<AwaitingDimensionReady>(player).is_some());
        assert!(app.world().get::<CharacterController>(player).is_none());

        app.world_mut()
            .spawn((ChildOf(chunk), Collider::cuboid(1.0, 1.0, 1.0)));
        app.update();

        assert!(app.world().get::<AwaitingDimensionReady>(player).is_none());
        assert!(app.world().get::<CharacterController>(player).is_some());
        assert_eq!(**app.world().get::<Velocity>(player).unwrap(), Vec3::ZERO);
    }

    #[test]
    fn collider_disabled_runtime_resumes_after_center_publication() {
        let mut app = switch_app();
        app.insert_resource(ChunkColliderRuntime::new(false));
        let roots = app.world().resource::<TestRoots>();
        let (grass, player) = (roots.grass, roots.player);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F6);
        app.update();

        let chunk = publish_solid_center(&mut app, grass);
        app.update();

        assert!(app.world().get::<Children>(chunk).is_none());
        assert!(app.world().get::<AwaitingDimensionReady>(player).is_none());
        assert!(app.world().get::<CharacterController>(player).is_some());
    }

    #[test]
    fn streamed_full_cycle_persists_teardown_and_reloads_the_original_dimension() {
        let mut app = streamed_switch_app();
        let roots = app.world().resource::<TestRoots>();
        let (overworld, grass, player) = (roots.overworld, roots.grass, roots.player);
        let center = ChunkColumn::new(0, 0);
        update_until(&mut app, |world| {
            world
                .get::<Dimension>(overworld)
                .unwrap()
                .contains_published_chunk(center.chunk(0))
        });

        let (incarnation, chunk_entity) = {
            let dimension = app.world().get::<Dimension>(overworld).unwrap();
            (
                dimension.column_incarnation(center).unwrap(),
                dimension.loaded_chunk_entity(center.chunk(0)).unwrap(),
            )
        };
        let counts = {
            let mut chunk = app.world_mut().get_mut::<Chunk>(chunk_entity).unwrap();
            chunk.set_cell_xyz(0, 15, 0, BlockType::Glowstone.into());
            chunk.compute_content_counts()
        };
        app.world_mut()
            .entity_mut(chunk_entity)
            .insert((counts, ChunkNeedsSave));
        trigger_dimension_switch(&mut app);

        let outgoing = app.world().get::<Dimension>(overworld).unwrap();
        assert_eq!(outgoing.loaded_chunk_count(), 0);
        assert_eq!(outgoing.published_chunk_count(), 0);
        assert!(outgoing.stream().is_empty());
        assert!(app.world().get_entity(incarnation).is_err());
        assert_eq!(active_root(app.world_mut()), grass);
        assert!(
            app.world()
                .resource::<ChunkSaveTasks>()
                .has_uncommitted_dimension(crate::world::DimensionId::OVERWORLD)
        );
        assert!(app.world().get::<AwaitingDimensionReady>(player).is_some());

        app.world_mut().resource_mut::<ChunkSaveBudget>().0 = 2;
        update_until(&mut app, |world| {
            world
                .get::<Dimension>(grass)
                .unwrap()
                .contains_published_chunk(center.chunk(0))
                && world.get::<AwaitingDimensionReady>(player).is_none()
                && !world
                    .resource::<ChunkSaveTasks>()
                    .has_uncommitted_dimension(crate::world::DimensionId::OVERWORLD)
        });

        assert!(app.world().get::<CharacterController>(player).is_some());
        let target_chunk = app
            .world()
            .get::<Dimension>(grass)
            .unwrap()
            .published_chunk_entity(center.chunk(0))
            .unwrap();
        assert_eq!(
            app.world()
                .get::<Chunk>(target_chunk)
                .unwrap()
                .cell_xyz(0, 0, 0),
            BlockType::Grass.into()
        );

        trigger_dimension_switch(&mut app);
        let glass = app.world().resource::<TestRoots>().glass;
        update_until(&mut app, |world| {
            world
                .get::<Dimension>(glass)
                .unwrap()
                .contains_published_chunk(center.chunk(0))
                && world.get::<AwaitingDimensionReady>(player).is_none()
        });

        trigger_dimension_switch(&mut app);
        update_until(&mut app, |world| {
            world
                .get::<Dimension>(overworld)
                .unwrap()
                .contains_published_chunk(center.chunk(0))
                && world.get::<AwaitingDimensionReady>(player).is_none()
        });

        assert_eq!(active_root(app.world_mut()), overworld);
        let reloaded = app
            .world()
            .get::<Dimension>(overworld)
            .unwrap()
            .published_chunk_entity(center.chunk(0))
            .unwrap();
        assert_eq!(
            app.world()
                .get::<Chunk>(reloaded)
                .unwrap()
                .cell_xyz(0, 15, 0),
            BlockType::Glowstone.into()
        );
    }

    #[test]
    fn readiness_cannot_complete_against_a_different_root_incarnation() {
        let mut app = switch_app();
        let roots = app.world().resource::<TestRoots>();
        let (grass, glass, player) = (roots.grass, roots.glass, roots.player);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F6);
        app.update();

        let chunk = publish_solid_center(&mut app, grass);
        app.world_mut()
            .spawn((ChildOf(chunk), Collider::cuboid(1.0, 1.0, 1.0)));
        app.world_mut()
            .get_mut::<AwaitingDimensionReady>(player)
            .unwrap()
            .target_root = glass;

        app.update();

        assert!(app.world().get::<AwaitingDimensionReady>(player).is_some());
        assert!(app.world().get::<CharacterController>(player).is_none());
    }
}
