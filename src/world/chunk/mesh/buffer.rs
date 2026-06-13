use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::{DIRECTIONS, NORMALS, VERTEX_OFFSETS, face_brightness, get_ao_indices, uvs_for_rect};

#[derive(Default)]
pub(crate) struct MeshBufferBuilder {
    indices: Vec<u32>,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    uv1s: Vec<[f32; 2]>,
    colours: Vec<[f32; 4]>,
}

impl MeshBufferBuilder {
    pub(crate) fn with_face_capacity(face_count: usize) -> Self {
        Self {
            indices: Vec::with_capacity(face_count * 6),
            positions: Vec::with_capacity(face_count * 4),
            normals: Vec::with_capacity(face_count * 4),
            uvs: Vec::with_capacity(face_count * 4),
            uv1s: Vec::with_capacity(face_count * 4),
            colours: Vec::with_capacity(face_count * 4),
        }
    }

    pub(crate) fn push_face(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        side_index: usize,
        uv: Rect,
        color: Vec4,
        ao: [u8; 4],
        ao_brightness: [f32; 4],
    ) {
        self.indices
            .extend_from_slice(&get_ao_indices(self.positions.len() as u32, ao));

        for offset in VERTEX_OFFSETS[side_index] {
            self.positions.push([
                x as f32 + offset.x as f32,
                y as f32 + offset.y as f32,
                z as f32 + offset.z as f32,
            ]);
        }

        self.normals.extend_from_slice(&NORMALS[side_index]);
        self.uvs
            .extend_from_slice(&uvs_for_rect(Rect::new(0.0, 0.0, 1.0, 1.0)));
        let tile_offset = [uv.min.x, uv.min.y];
        self.uv1s.extend_from_slice(&[tile_offset; 4]);

        let face_light = face_brightness(DIRECTIONS[side_index]);
        self.colours.extend(ao.map(|ao| {
            let brightness = face_light * ao_brightness[ao as usize];
            [
                color.x * brightness,
                color.y * brightness,
                color.z * brightness,
                color.w,
            ]
        }));
    }

    pub(crate) fn push_merged_face(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        w: usize,
        h: usize,
        side_index: usize,
        uv: Rect,
        color: Vec4,
        ao: [u8; 4],
        ao_brightness: [f32; 4],
    ) {
        let base = self.positions.len() as u32;
        self.indices.extend_from_slice(&get_ao_indices(base, ao));

        match side_index {
            0 => {
                self.positions.push([x as f32, y as f32, (z + w) as f32]);
                self.positions.push([x as f32, y as f32, z as f32]);
                self.positions
                    .push([x as f32, (y + h) as f32, (z + w) as f32]);
                self.positions.push([x as f32, (y + h) as f32, z as f32]);
            }
            1 => {
                self.positions.push([(x + 1) as f32, y as f32, z as f32]);
                self.positions
                    .push([(x + 1) as f32, y as f32, (z + w) as f32]);
                self.positions
                    .push([(x + 1) as f32, (y + h) as f32, z as f32]);
                self.positions
                    .push([(x + 1) as f32, (y + h) as f32, (z + w) as f32]);
            }
            2 => {
                self.positions.push([x as f32, y as f32, (z + h) as f32]);
                self.positions
                    .push([(x + w) as f32, y as f32, (z + h) as f32]);
                self.positions.push([x as f32, y as f32, z as f32]);
                self.positions.push([(x + w) as f32, y as f32, z as f32]);
            }
            3 => {
                self.positions
                    .push([x as f32, (y + 1) as f32, (z + h) as f32]);
                self.positions.push([x as f32, (y + 1) as f32, z as f32]);
                self.positions
                    .push([(x + w) as f32, (y + 1) as f32, (z + h) as f32]);
                self.positions
                    .push([(x + w) as f32, (y + 1) as f32, z as f32]);
            }
            4 => {
                self.positions.push([x as f32, y as f32, z as f32]);
                self.positions.push([(x + w) as f32, y as f32, z as f32]);
                self.positions.push([x as f32, (y + h) as f32, z as f32]);
                self.positions
                    .push([(x + w) as f32, (y + h) as f32, z as f32]);
            }
            5 => {
                self.positions
                    .push([(x + w) as f32, y as f32, (z + 1) as f32]);
                self.positions.push([x as f32, y as f32, (z + 1) as f32]);
                self.positions
                    .push([(x + w) as f32, (y + h) as f32, (z + 1) as f32]);
                self.positions
                    .push([x as f32, (y + h) as f32, (z + 1) as f32]);
            }
            _ => unreachable!(),
        }

        self.normals.extend_from_slice(&NORMALS[side_index]);
        self.uvs
            .extend_from_slice(&uvs_for_rect(Rect::new(0.0, 0.0, w as f32, h as f32)));
        let tile_offset = [uv.min.x, uv.min.y];
        self.uv1s.extend_from_slice(&[tile_offset; 4]);

        let face_light = face_brightness(DIRECTIONS[side_index]);
        self.colours.extend(ao.map(|ao| {
            let brightness = face_light * ao_brightness[ao as usize];
            [
                color.x * brightness,
                color.y * brightness,
                color.z * brightness,
                color.w,
            ]
        }));
    }

    pub(crate) fn into_mesh(self) -> Option<Mesh> {
        if self.positions.is_empty() {
            return None;
        }

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );

        mesh.insert_indices(Indices::U32(self.indices));
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.uv1s);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colours);

        Some(mesh)
    }
}
