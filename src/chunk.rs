use bevy::{math::IVec3, prelude::*};
use rand::prelude::*;
use strum::EnumCount;

use crate::{
    block::{BlockTextureMap, BlockType},
    mesh::{make_mesh, make_quad_groups},
    textures::{BlockStandardMaterial, TextureState},
};

pub const CHUNK_SIZE: usize = 16;
pub const CHUNK_ISIZE: isize = 16;

pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

#[derive(Component, Debug, Clone)]
pub struct Chunk {
    pub blocks: [[[BlockType; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
    pub position: IVec3,
}

impl Default for Chunk {
    fn default() -> Self {
        Self {
            blocks: [[[BlockType::Air; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
            position: IVec3::default(),
        }
    }
}

#[derive(Component)]
pub struct UpdateMesh;

#[derive(Bundle)]
pub struct ChunkBundle {
    pub spatial: SpatialBundle,
    pub chunk: Chunk,
}

pub struct ChunkPlugin;

impl Plugin for ChunkPlugin {
    fn build(&self, app: &mut App) {
        // app.add_systems(Update, spawn_mesh_naive);
        app.add_systems(
            Update,
            spawn_mesh_simple.run_if(in_state(TextureState::Finished)),
        );
    }
}

fn spawn_mesh_simple(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    block_material: Res<BlockStandardMaterial>,
    block_texture_map: Res<BlockTextureMap>,
    chunks: Query<(&Chunk, Entity), Or<(With<UpdateMesh>, Added<Chunk>)>>,
) {
    for (chunk, chunk_entity) in chunks.iter() {
        commands
            .entity(chunk_entity)
            .despawn_descendants()
            .remove::<UpdateMesh>();
        let result = make_quad_groups(chunk, block_texture_map.as_ref());

        let mesh = make_mesh(&result);

        commands
            .spawn(PbrBundle {
                mesh: meshes.add(mesh),
                material: block_material.clone(),
                ..default()
            })
            .set_parent(chunk_entity);
    }
}

pub fn generate_flat_chunk_data(position: IVec3) -> Chunk {
    let mut chunk = Chunk {
        position,
        ..default()
    };

    // Generate a flat floor of grass and glass blocks
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..=0 {
                chunk.blocks[x][z][y] = if (x + z) % 2 == 0 {
                    BlockType::Grass
                } else {
                    BlockType::Glass
                };
            }
        }
    }

    chunk
}

impl Chunk {
    pub fn get(&self, x: isize, y: isize, z: isize) -> Option<BlockType> {
        let outside = |a: isize| !(0..CHUNK_ISIZE).contains(&a);
        if outside(x) || outside(y) || outside(z) {
            return None;
        }

        Some(self.blocks[x as usize][z as usize][y as usize])
    }

    pub fn place_block(&mut self) -> Option<UpdateMesh> {
        let mut rng = rand::thread_rng();
        let mut get_range = || rng.gen_range(0..CHUNK_SIZE);
        let block = &mut self.blocks[get_range()][get_range()][get_range()];

        if *block == BlockType::Air {
            // Assumes Air = 0
            *block = BlockType::from_repr(rng.gen_range(1..BlockType::COUNT)).unwrap();
            return Some(UpdateMesh);
        }

        None
    }
}
