use avian3d::prelude::*;
use bevy::prelude::*;

use super::controller::{CharacterController, Grounded, Velocity};

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

pub fn mov_system(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &mut Velocity,
            &Collider,
            &mut Transform,
            &CollideAndSlideConfig,
            Has<Grounded>,
        ),
        With<CharacterController>,
    >,
    spatial_query: SpatialQuery,
    time: Res<Time>,
) {
    for (entity, mut velocity, collider, mut transform, collide_and_slide_config, is_grounded) in
        &mut query
    {
        let (position, grounded) = mov(
            &spatial_query.query_pipeline,
            collider,
            transform.translation,
            transform.rotation,
            &mut velocity,
            &SpatialQueryFilter::from_excluded_entities([entity]),
            collide_and_slide_config,
            &time,
        );

        transform.translation = position;

        if grounded && !is_grounded {
            commands.entity(entity).insert(Grounded);
        }

        if !grounded && is_grounded {
            commands.entity(entity).remove::<Grounded>();
        }
    }
}

fn mov(
    pipeline: &SpatialQueryPipeline,
    collider: &Collider,
    // Origin position.
    position: Vec3,
    // Rotation of the collider.
    rotation: Quat,
    velocity: &mut Vec3,
    // Entities/colliders to ignore in this colliding.
    // Should add the entity related to this collider to this.
    filter: &SpatialQueryFilter,
    config: &CollideAndSlideConfig,
    time: &Res<Time>,
) -> (Vec3, bool) {
    // move down y
    // if hit ground set state to grounded
    // move xz
    // mark first attempt
    // then allow for autostep to kick in
    // go back to original pos and move up by up to autostep
    // move xz
    // move back down by up to autostep
    // Instead of lowering the bounding box by a maximum of 0.6m,
    // the game now lowers it until it reaches the player's height minus their vertical speed.
    // finally compare this position to first attempt
    // move to whichever has largest distance to start pos
    // let original_position = position.clone();

    let mut vdt = *velocity * time.delta_secs();

    let mut grounded = false;

    let mut position = position;
    for _ in 0..config.max_bounces {
        if vdt.abs_diff_eq(Vec3::ZERO, 0.0001) {
            break;
        }

        let y_target_distance = vec3(0.0, vdt.y, 0.0);

        let (distance, hit_data, direction) = shapecast(
            pipeline,
            collider,
            rotation,
            filter,
            &config,
            position,
            y_target_distance,
        );

        let y_impact = hit_data.is_some();

        if y_impact {
            vdt.y = 0.0;
            velocity.y = 0.0;
        }

        grounded = grounded || (y_impact && y_target_distance.y < 0.0);

        position += direction * distance;
        vdt -= direction * distance;

        let xz_target_distance = vec3(vdt.x, 0.0, vdt.z);

        let (distance, hit_data, direction) = shapecast(
            pipeline,
            collider,
            rotation,
            filter,
            &config,
            position,
            xz_target_distance,
        );

        // let first_position = position + xz_target_distance.normalize_or_zero() * distance;
        position += direction * distance;
        vdt -= direction * distance;

        if let Some(hit_data) = hit_data {
            // Only redirect when autostep fails??
            vdt = vdt.reject_from_normalized(hit_data.normal1);
            *velocity = velocity.reject_from_normalized(hit_data.normal1);
        }
    }

    (position, grounded)

    // let position = if grounded && hit_data.is_some() {
    //     try_autostep(
    //         pipeline,
    //         collider,
    //         rotation,
    //         filter,
    //         config,
    //         original_position,
    //         position,
    //         xz_target_distance,
    //         first_position,
    //     )
    // } else {
    //     first_position
    // };

    // (position, grounded)
}

// fn try_autostep(
//     pipeline: &SpatialQueryPipeline,
//     collider: &Collider,
//     rotation: Quat,
//     filter: &SpatialQueryFilter,
//     config: &CollideAndSlideConfig,
//     original_position: Vec3,
//     mut position: Vec3,
//     xz_target_distance: Vec3,
//     first_position: Vec3,
// ) -> Vec3 {
//     let autostep_up = Vec3::Y * config.autostep;

//     let (distance, _, direction) = shapecast(
//         pipeline,
//         collider,
//         rotation,
//         filter,
//         &config,
//         position,
//         autostep_up,
//     );
//     position += direction * distance;

//     let (distance, _, direction) = shapecast(
//         pipeline,
//         collider,
//         rotation,
//         filter,
//         &config,
//         position,
//         xz_target_distance,
//     );
//     position += direction * distance;

//     let autostep_down = Vec3::NEG_Y * config.autostep;

//     let (distance, _, direction) = shapecast(
//         pipeline,
//         collider,
//         rotation,
//         filter,
//         &config,
//         position,
//         autostep_down,
//     );
//     position += direction * distance;

//     let first_distance = (original_position - first_position).length();
//     let autostep_distance = (original_position - position).length();

//     if first_distance > 0.0 || autostep_distance > 0.0 {
//         dbg!("First distance: {}", first_distance);
//         dbg!("Autostep distance: {}", autostep_distance);
//     }

//     if first_distance > autostep_distance {
//         return first_position;
//     } else {
//         return position;
//     }
// }

fn shapecast(
    pipeline: &SpatialQueryPipeline,
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

    let (distance, hit_data) = if let Some(hit_data) = pipeline.cast_shape(
        collider,
        position,
        rotation,
        direction,
        &ShapeCastConfig {
            max_distance: target_distance.length(),
            ignore_origin_penetration: config.ignore_origin_penetration,
            // target_distance: config.skin,
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
