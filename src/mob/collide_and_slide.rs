use avian3d::prelude::*;
use bevy::prelude::*;

use super::controller::{CharacterController, Grounded, Velocity};
use crate::world::WORLD_LAYER;

#[derive(Component, Clone)]
pub struct CollideAndSlideConfig {
    /// Maximum amount of bounces before we early exit.
    pub max_bounces: usize,
    /// How much space to leave inbetween walls when sliding/colliding.
    pub skin: f32,
    pub ignore_origin_penetration: bool,
    pub autostep: f32,
}

impl Default for CollideAndSlideConfig {
    fn default() -> Self {
        Self {
            max_bounces: 4,
            skin: 0.015,
            ignore_origin_penetration: true,
            autostep: 0.6,
        }
    }
}

pub fn move_character_controllers(
    mut commands: Commands,
    mut params: ParamSet<(
        Query<
            (
                Entity,
                &Velocity,
                Option<&Collider>,
                &Position,
                &Rotation,
                &CollideAndSlideConfig,
                Has<Grounded>,
            ),
            With<CharacterController>,
        >,
        SpatialQuery,
        Query<(&mut Velocity, &mut Position), With<CharacterController>>,
    )>,
    children_q: Query<&Children>,
    time: Res<Time<Fixed>>,
) {
    // SpatialQuery reads Position internally, so stage results before mutating Position.
    let inputs = params
        .p0()
        .iter()
        .map(
            |(
                entity,
                velocity,
                collider,
                position,
                rotation,
                collide_and_slide_config,
                was_grounded,
            )| {
                (
                    entity,
                    velocity.0,
                    collider.cloned(),
                    position.0,
                    rotation.0,
                    collide_and_slide_config.clone(),
                    was_grounded,
                )
            },
        )
        .collect::<Vec<_>>();

    let mut outputs = Vec::with_capacity(inputs.len());
    {
        let spatial_query = params.p1();
        for (
            entity,
            mut velocity,
            collider,
            position,
            rotation,
            collide_and_slide_config,
            was_grounded,
        ) in inputs
        {
            let excluded_entities = children_q
                .get(entity)
                .ok()
                .into_iter()
                .flat_map(|children| children.iter())
                .chain(std::iter::once(entity));
            let (new_position, is_grounded) = if let Some(collider) = collider {
                collide_and_slide(
                    &spatial_query,
                    &collider,
                    position,
                    rotation,
                    &mut velocity,
                    &SpatialQueryFilter::from_mask(WORLD_LAYER)
                        .with_excluded_entities(excluded_entities),
                    &collide_and_slide_config,
                    &time,
                )
            } else {
                (position + velocity * time.delta_secs(), false)
            };

            outputs.push((entity, velocity, new_position, is_grounded, was_grounded));
        }
    }

    let mut query = params.p2();
    for (entity, new_velocity, new_position, is_grounded, was_grounded) in outputs {
        let Ok((mut velocity, mut position)) = query.get_mut(entity) else {
            continue;
        };

        velocity.0 = new_velocity;
        position.0 = new_position;

        if is_grounded && !was_grounded {
            commands.entity(entity).insert(Grounded);
        }

        if !is_grounded && was_grounded {
            commands.entity(entity).remove::<Grounded>();
        }
    }
}

fn collide_and_slide(
    spatial_query: &SpatialQuery,
    collider: &Collider,
    position: Vec3,
    rotation: Quat,
    velocity: &mut Vec3,
    filter: &SpatialQueryFilter,
    config: &CollideAndSlideConfig,
    time: &Res<Time<Fixed>>,
) -> (Vec3, bool) {
    let mut vdt = *velocity * time.delta_secs();
    let intended_xz = vdt.with_y(0.0);
    let original_position = position;
    let original_velocity = *velocity;

    let mut grounded = false;
    let mut had_horizontal_collision = false;
    let mut normal_velocity = *velocity;
    let mut position = position;

    for _ in 0..config.max_bounces {
        if vdt.abs_diff_eq(Vec3::ZERO, 0.0001) {
            break;
        }

        let y_target_distance = Vec3::Y * vdt.y;
        let (distance, hit_data, direction) = shapecast(
            spatial_query,
            collider,
            rotation,
            filter,
            config,
            position,
            y_target_distance,
        );

        let y_impact = hit_data.is_some();
        if y_impact {
            normal_velocity.y = 0.0;
        }
        grounded = grounded || (y_impact && y_target_distance.y < 0.0);

        position += direction * distance;
        vdt -= direction * distance;
        if y_impact {
            vdt.y = 0.0;
        }

        let xz_target_distance = vdt.with_y(0.0);
        let (distance, hit_data, direction) = shapecast(
            spatial_query,
            collider,
            rotation,
            filter,
            config,
            position,
            xz_target_distance,
        );

        position += direction * distance;
        vdt -= direction * distance;

        if let Some(hit_data) = hit_data {
            had_horizontal_collision = true;
            vdt = vdt.reject_from_normalized(hit_data.normal1);
            normal_velocity = normal_velocity.reject_from_normalized(hit_data.normal1);
        }
    }

    let normal_position = position;

    if grounded
        && had_horizontal_collision
        && config.autostep > 0.0
        && let Some((autostep_position, autostep_velocity)) = try_autostep(
            spatial_query,
            collider,
            rotation,
            filter,
            config,
            original_position,
            original_velocity,
            intended_xz,
            normal_position,
        )
    {
        *velocity = autostep_velocity;
        return (autostep_position, true);
    }

    *velocity = normal_velocity;
    (normal_position, grounded)
}

fn try_autostep(
    spatial_query: &SpatialQuery,
    collider: &Collider,
    rotation: Quat,
    filter: &SpatialQueryFilter,
    config: &CollideAndSlideConfig,
    original_position: Vec3,
    original_velocity: Vec3,
    intended_xz: Vec3,
    normal_position: Vec3,
) -> Option<(Vec3, Vec3)> {
    if intended_xz.abs_diff_eq(Vec3::ZERO, 0.0001) {
        return None;
    }

    let mut position = original_position;
    let mut velocity = original_velocity;
    let mut autostep_config = config.clone();
    autostep_config.ignore_origin_penetration = false;

    let (up_distance, _, _) = shapecast(
        spatial_query,
        collider,
        rotation,
        filter,
        &autostep_config,
        position,
        Vec3::Y * config.autostep,
    );
    if up_distance <= 0.0001 {
        return None;
    }
    position += Vec3::Y * up_distance;

    let (xz_distance, xz_hit, xz_direction) = shapecast(
        spatial_query,
        collider,
        rotation,
        filter,
        &autostep_config,
        position,
        intended_xz,
    );
    position += xz_direction * xz_distance;
    velocity.y = 0.0;
    if let Some(hit) = xz_hit {
        velocity = velocity.reject_from_normalized(hit.normal1);
    }

    let (down_distance, down_hit, _) = shapecast(
        spatial_query,
        collider,
        rotation,
        filter,
        &autostep_config,
        position,
        Vec3::NEG_Y * up_distance,
    );
    down_hit?;
    position += Vec3::NEG_Y * down_distance;
    velocity.y = 0.0;

    let stepped_height = position.y - original_position.y;
    if stepped_height <= config.skin || stepped_height > config.autostep + config.skin {
        return None;
    }

    let normal_xz_dist = (normal_position - original_position).xz().length_squared();
    let autostep_xz_dist = (position - original_position).xz().length_squared();

    if autostep_xz_dist > normal_xz_dist + 0.000001 {
        Some((position, velocity))
    } else {
        None
    }
}

fn shapecast(
    spatial_query: &SpatialQuery,
    collider: &Collider,
    rotation: Quat,
    filter: &SpatialQueryFilter,
    config: &CollideAndSlideConfig,
    position: Vec3,
    target_distance: Vec3,
) -> (f32, Option<ShapeHitData>, Vec3) {
    let Ok(direction) = Dir3::new(target_distance) else {
        return (0.0, None, Vec3::ZERO);
    };

    let (distance, hit_data) = if let Some(hit_data) = spatial_query.cast_shape(
        collider,
        position,
        rotation,
        direction,
        &ShapeCastConfig {
            max_distance: target_distance.length(),
            ignore_origin_penetration: config.ignore_origin_penetration,
            ..default()
        },
        filter,
    ) {
        let distance = (hit_data.distance - config.skin).max(0.0);
        (distance, Some(hit_data))
    } else {
        (target_distance.length(), None)
    };

    (distance, hit_data, direction.as_vec3())
}
