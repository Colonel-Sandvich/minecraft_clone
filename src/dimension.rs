use std::ops::Mul;

use bevy::{platform::collections::HashMap, prelude::*};

use crate::{
    chunk::{CHUNK_ISIZE, CHUNK_SIZE, Chunk, util::generate_full_chunk_data},
    game_state::{GameState, Playing},
    player::Player,
};

#[derive(Default, Component)]
pub struct Dimension {
    pub chunks: HashMap<IVec3, Entity>,
}

pub struct DimensionPlugin;

impl Plugin for DimensionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(GameState::GenWorld),
            (
                setup,
                gen_chunks_in_view,
                |mut game_state: ResMut<NextState<GameState>>| game_state.set(GameState::Playing),
            )
                .chain(),
        );

        app.add_systems(Update, (gen_chunks_in_view).in_set(Playing));
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Dimension::default(),
        Transform::default(),
        Visibility::default(),
        Active,
    ));
}

#[derive(Component)]
pub struct Active;

pub const VIEW_DISTANCE: i32 = 3;
pub const DIMENSION_HEIGHT_IN_SUB_CHUNKS: usize = 5;

fn gen_chunks_in_view(
    mut commands: Commands,
    dimension: Single<(&mut Dimension, Entity), With<Active>>,
    maybe_player_q: Option<Single<&Transform, With<Player>>>,
) {
    let centre = maybe_player_q.map_or(Transform::default(), |q| **q);

    let centre_in_chunk_coords = (centre.translation / CHUNK_ISIZE as f32).with_y(0.0);

    let (mut dim, dimension_entity) = dimension.into_inner();

    for pos in CuboidIterator::new(2 * VIEW_DISTANCE + 1, DIMENSION_HEIGHT_IN_SUB_CHUNKS as i32)
        .map(|p| p - ivec3(VIEW_DISTANCE, 0, VIEW_DISTANCE))
        .filter(|p| p.x * p.x + p.z * p.z <= VIEW_DISTANCE * VIEW_DISTANCE)
        .map(|p| (p.as_vec3() + centre_in_chunk_coords).floor().as_ivec3())
    {
        if dim.chunks.contains_key(&pos) {
            continue;
        }

        let chunk = if pos.y == 0 {
            generate_full_chunk_data()
        } else {
            Chunk::default()
        };

        let chunk_entity = commands
            .spawn((
                ChildOf(dimension_entity),
                chunk,
                Transform::from_translation(pos.as_vec3().mul(CHUNK_SIZE as f32)),
                Visibility::default(),
            ))
            .id();

        dim.chunks.insert(pos, chunk_entity);
    }
}

impl CuboidIterator {
    /// Iterate around centre
    pub fn new(length: i32, height: i32) -> Self {
        Self {
            length,
            height,
            x: 0,
            y: 0,
            z: 0,
        }
    }
}

/// Iterate over a cuboid (x,y,z) -> (x + length, y + height, z + length) in y,x,z order
#[derive(Default)]
pub struct CuboidIterator {
    x: i32,
    y: i32,
    z: i32,
    length: i32,
    height: i32,
}

impl Iterator for CuboidIterator {
    type Item = IVec3;

    fn next(&mut self) -> Option<Self::Item> {
        if self.z >= self.length {
            return None;
        }

        let pos = ivec3(self.x, self.y, self.z);

        self.y += 1;
        if self.y >= self.height {
            self.y = 0;
            self.x += 1;
            if self.x >= self.length {
                self.x = 0;
                self.z += 1;
            }
        }

        Some(pos)
    }
}

/// Iterate over a cuboid (x,y,z) -> (x + length, y + height, z + length) in y,x,z order
#[derive(Default)]
pub struct CircularIterator {
    x: i32,
    y: i32,
    z: i32,
    radius: i32,
    height: i32,
}

impl Iterator for CircularIterator {
    type Item = IVec3;

    fn next(&mut self) -> Option<Self::Item> {
        if self.z >= self.radius {
            return None;
        }

        let pos = ivec3(self.x, self.y, self.z);

        self.y += 1;
        if self.y >= self.height {
            self.y = 0;
            self.x += 1;
            if self.x >= self.radius {
                self.x = 0;
                self.z += 1;
            }
        }

        Some(pos)
    }
}
