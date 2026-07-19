mod render;

use std::time::Duration;

use avian3d::prelude::*;
use bevy::{mesh::MeshTag, prelude::*};

use crate::{
    block::{BlockTextureMap, render_id_for_block},
    input::GameActionSystems,
    player::{PLAYER_HEIGHT, Player, cam::gameplay_input_active, spawn::EYELINE},
    textures::BlockTextures,
    world::{ITEM_COLLISION_LAYERS, PICKUP_SENSOR_COLLISION_LAYERS},
};

use super::ItemStack;
use render::{DroppedItemRenderAssets, prepare_dropped_item_render_assets};

const ITEM_DROP_FORWARD_OFFSET: f32 = 0.75;
const ITEM_DROP_SPEED: f32 = 6.0;
const ITEM_PICKUP_DELAY: Duration = Duration::from_secs(2);

#[derive(Component)]
#[require(
    Sensor,
    Collider = Collider::cuboid(2.8, 2.8, 2.8),
    CollisionLayers = PICKUP_SENSOR_COLLISION_LAYERS,
    CollidingEntities
)]
pub struct PlayerPickupSensor;

pub struct DroppedItemPlugin;

impl Plugin for DroppedItemPlugin {
    fn build(&self, app: &mut App) {
        render::install(app);
        app.add_message::<DropItemRequest>()
            .add_message::<ItemPickedUp>()
            .add_observer(on_player_spawn)
            .add_systems(
                Update,
                prepare_dropped_item_render_assets.run_if(
                    not(resource_exists::<DroppedItemRenderAssets>)
                        .and_then(resource_exists::<BlockTextures>)
                        .and_then(resource_exists::<BlockTextureMap>),
                ),
            )
            .add_systems(
                FixedUpdate,
                on_drop_item
                    .in_set(GameActionSystems)
                    .run_if(gameplay_input_active)
                    .run_if(resource_exists::<DroppedItemRenderAssets>),
            )
            .add_systems(FixedPreUpdate, tick_item_pickup_delays)
            .add_systems(
                FixedPostUpdate,
                pick_up_eligible_items.after(PhysicsSystems::Last),
            );
    }
}

#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct DropItemRequest {
    pub stack: ItemStack,
    pub look_direction: Vec3,
}

/// A completed transfer from a dropped item entity to a player.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemPickedUp {
    pub player: Entity,
    pub item: Entity,
    pub stack: ItemStack,
}

fn on_drop_item(
    mut requests: MessageReader<DropItemRequest>,
    player_q: Single<&Transform, With<Player>>,
    item_render_assets: Res<DroppedItemRenderAssets>,
    mut commands: Commands,
) {
    let eye_position = player_q.translation + Vec3::Y * (PLAYER_HEIGHT / 2.0 - EYELINE);

    for request in requests.read().copied() {
        let look_direction = request.look_direction.normalize_or_zero();
        if look_direction == Vec3::ZERO {
            continue;
        }
        let position = eye_position + look_direction * ITEM_DROP_FORWARD_OFFSET;

        commands.spawn((
            DroppedItem,
            request.stack,
            ItemPickupDelay(Timer::new(ITEM_PICKUP_DELAY, TimerMode::Once)),
            Transform::from_translation(position),
            LinearVelocity(look_direction * ITEM_DROP_SPEED),
            Mesh3d(item_render_assets.cube_mesh.clone()),
            MeshMaterial3d(item_render_assets.material_for(request.stack)),
            MeshTag(u32::from(render_id_for_block(request.stack.item))),
        ));
    }
}

fn on_player_spawn(add: On<Add, Player>, mut commands: Commands) {
    let sensor = commands.spawn(PlayerPickupSensor).id();
    commands.entity(add.entity).add_child(sensor);
}

fn tick_item_pickup_delays(
    time: Res<Time<Fixed>>,
    mut delayed_items: Query<(Entity, &mut ItemPickupDelay)>,
    mut commands: Commands,
) {
    for (item, mut pickup_delay) in &mut delayed_items {
        if pickup_delay.0.tick(time.delta()).just_finished() {
            commands.entity(item).remove::<ItemPickupDelay>();
        }
    }
}

fn pick_up_eligible_items(
    pickup_sensors: Query<(&CollidingEntities, &ChildOf), With<PlayerPickupSensor>>,
    eligible_items: Query<(Entity, &ItemStack), Without<ItemPickupDelay>>,
    mut pickups: MessageWriter<ItemPickedUp>,
    mut commands: Commands,
) {
    for (item, stack) in &eligible_items {
        let player = pickup_sensors
            .iter()
            .filter_map(|(colliding_entities, sensor_owner)| {
                colliding_entities
                    .contains(&item)
                    .then_some(sensor_owner.parent())
            })
            .min_by_key(|entity| entity.to_bits());

        if let Some(player) = player {
            pick_up_item(player, item, *stack, &mut pickups, &mut commands);
        }
    }
}

fn pick_up_item(
    player: Entity,
    item: Entity,
    stack: ItemStack,
    pickups: &mut MessageWriter<ItemPickedUp>,
    commands: &mut Commands,
) {
    pickups.write(ItemPickedUp {
        player,
        item,
        stack,
    });
    commands.entity(item).despawn();
}

#[derive(Component)]
struct ItemPickupDelay(Timer);

#[derive(Component)]
#[require(
    RigidBody::Dynamic,
    Collider::cuboid(0.25, 0.25, 0.25),
    CollisionLayers = ITEM_COLLISION_LAYERS,
    LockedAxes = LockedAxes::ROTATION_LOCKED,
    SweptCcd = SweptCcd::LINEAR,
    TransformInterpolation,
    Mesh3d
)]
struct DroppedItem;

#[cfg(test)]
mod tests {
    use crate::{block::render_id_for_block, item::Item};

    use super::*;

    #[test]
    fn drop_request_spawns_an_item_stack() {
        let mut app = App::new();
        app.add_message::<DropItemRequest>()
            .insert_resource(DroppedItemRenderAssets::test_handles())
            .add_systems(FixedUpdate, on_drop_item);
        app.world_mut().spawn((
            Player::default(),
            Transform::from_translation(vec3(10.0, 20.0, 30.0)),
        ));
        app.world_mut()
            .resource_mut::<Messages<DropItemRequest>>()
            .write(DropItemRequest {
                stack: ItemStack::one(Item::Dirt),
                look_direction: Vec3::NEG_Z,
            });

        app.world_mut().run_schedule(FixedUpdate);

        let mut items = app.world_mut().query::<(
            &ItemStack,
            &ItemPickupDelay,
            &CollisionLayers,
            &Transform,
            &LinearVelocity,
            &MeshTag,
            &LockedAxes,
            &SweptCcd,
            &TransformInterpolation,
        )>();
        let (
            stack,
            pickup_delay,
            collision_layers,
            transform,
            velocity,
            mesh_tag,
            locked_axes,
            swept_ccd,
            _interpolation,
        ) = items.single(app.world()).unwrap();
        assert_eq!(stack.item, Item::Dirt);
        assert_eq!(stack.count, 1);
        assert_eq!(pickup_delay.0.duration(), ITEM_PICKUP_DELAY);
        assert!(!pickup_delay.0.is_finished());
        assert_eq!(*collision_layers, ITEM_COLLISION_LAYERS);
        assert_eq!(transform.translation, vec3(10.0, 20.8, 29.25));
        assert_eq!(velocity.0, Vec3::NEG_Z * ITEM_DROP_SPEED);
        assert_eq!(**mesh_tag, u32::from(render_id_for_block(Item::Dirt)));
        assert!(locked_axes.is_rotation_locked());
        assert!(!locked_axes.is_translation_locked());
        assert_eq!(*swept_ccd, SweptCcd::LINEAR);
    }

    #[test]
    fn pickup_delay_expires_independently_per_item_on_fixed_ticks() {
        let mut fixed_time = Time::<Fixed>::from_hz(20.0);
        fixed_time.advance_by(Duration::from_millis(50));

        let mut app = App::new();
        app.add_message::<ItemPickedUp>()
            .insert_resource(fixed_time)
            .add_systems(FixedPreUpdate, tick_item_pickup_delays);

        let expired_item = app
            .world_mut()
            .spawn((
                ItemStack {
                    item: Item::Dirt,
                    count: 1,
                },
                ItemPickupDelay(Timer::new(Duration::from_millis(50), TimerMode::Once)),
            ))
            .id();
        let waiting_item = app
            .world_mut()
            .spawn((
                ItemStack {
                    item: Item::Stone,
                    count: 1,
                },
                ItemPickupDelay(Timer::new(Duration::from_millis(100), TimerMode::Once)),
            ))
            .id();

        app.world_mut().run_schedule(FixedPreUpdate);

        assert!(app.world().get_entity(expired_item).is_ok());
        assert!(app.world().get::<ItemPickupDelay>(expired_item).is_none());
        let waiting_delay = app.world().get::<ItemPickupDelay>(waiting_item).unwrap();
        assert_eq!(waiting_delay.0.elapsed(), Duration::from_millis(50));
    }

    #[test]
    fn item_already_inside_pickup_sensor_is_picked_up_when_delay_expires() {
        let mut fixed_time = Time::<Fixed>::from_hz(20.0);
        fixed_time.advance_by(Duration::from_millis(50));

        let mut app = App::new();
        app.add_message::<ItemPickedUp>()
            .insert_resource(fixed_time)
            .add_systems(FixedPreUpdate, tick_item_pickup_delays)
            .add_systems(FixedPostUpdate, pick_up_eligible_items);

        let stack = ItemStack {
            item: Item::Dirt,
            count: 3,
        };
        let item = app
            .world_mut()
            .spawn((
                stack,
                ItemPickupDelay(Timer::new(Duration::from_millis(50), TimerMode::Once)),
            ))
            .id();
        let player = app.world_mut().spawn(Player::default()).id();
        let mut colliding_entities = CollidingEntities::default();
        colliding_entities.insert(item);
        let sensor = app
            .world_mut()
            .spawn((PlayerPickupSensor, colliding_entities))
            .id();
        app.world_mut().entity_mut(player).add_child(sensor);

        app.world_mut().run_schedule(FixedPreUpdate);
        app.world_mut().run_schedule(FixedPostUpdate);

        assert!(app.world().get_entity(item).is_err());
        let pickups = app
            .world()
            .resource::<Messages<ItemPickedUp>>()
            .iter_current_update_messages()
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(
            pickups,
            vec![ItemPickedUp {
                player,
                item,
                stack,
            }]
        );
    }

    #[test]
    fn delayed_item_inside_pickup_sensor_is_not_picked_up() {
        let mut app = App::new();
        app.add_message::<ItemPickedUp>()
            .add_systems(FixedPostUpdate, pick_up_eligible_items);

        let item = app
            .world_mut()
            .spawn((
                ItemStack {
                    item: Item::Dirt,
                    count: 1,
                },
                ItemPickupDelay(Timer::new(ITEM_PICKUP_DELAY, TimerMode::Once)),
            ))
            .id();
        let player = app.world_mut().spawn(Player::default()).id();
        let mut colliding_entities = CollidingEntities::default();
        colliding_entities.insert(item);
        let sensor = app
            .world_mut()
            .spawn((PlayerPickupSensor, colliding_entities))
            .id();
        app.world_mut().entity_mut(player).add_child(sensor);

        app.world_mut().run_schedule(FixedPostUpdate);

        assert!(app.world().get_entity(item).is_ok());
        assert!(
            app.world()
                .resource::<Messages<ItemPickedUp>>()
                .iter_current_update_messages()
                .next()
                .is_none()
        );
    }

    #[test]
    fn pickup_sensor_uses_explicit_pickup_collision_layer() {
        let mut app = App::new();
        let sensor = app.world_mut().spawn(PlayerPickupSensor).id();

        assert_eq!(
            *app.world().get::<CollisionLayers>(sensor).unwrap(),
            PICKUP_SENSOR_COLLISION_LAYERS
        );
        assert!(ITEM_COLLISION_LAYERS.interacts_with(PICKUP_SENSOR_COLLISION_LAYERS));
    }
}
