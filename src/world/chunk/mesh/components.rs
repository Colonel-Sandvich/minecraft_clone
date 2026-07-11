use std::sync::Arc;

use bevy::prelude::*;

use crate::block::BlockMaterialLayer;

use super::PackedFace;

/// Packed CPU mesh output waiting to be uploaded by the render world.
#[derive(Component, Clone)]
pub struct ChunkMeshFaces {
    faces: Vec<PackedFace>,
}

impl ChunkMeshFaces {
    pub fn new(faces: Vec<PackedFace>) -> Self {
        Self { faces }
    }

    pub fn as_slice(&self) -> &[PackedFace] {
        &self.faces
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.faces.capacity()
    }
}

/// Renderable material layer belonging to a chunk mesh.
#[derive(Component, Clone)]
pub struct ChunkMeshLayer {
    material_layer: BlockMaterialLayer,
    face_count: u32,
    origin: Vec3,
}

impl ChunkMeshLayer {
    pub fn new(material_layer: BlockMaterialLayer, origin: Vec3, faces: &ChunkMeshFaces) -> Self {
        Self {
            material_layer,
            face_count: face_count(faces),
            origin,
        }
    }

    pub fn update(
        &mut self,
        material_layer: BlockMaterialLayer,
        origin: Vec3,
        faces: &ChunkMeshFaces,
    ) {
        self.material_layer = material_layer;
        self.face_count = face_count(faces);
        self.origin = origin;
    }

    pub fn material_layer(&self) -> BlockMaterialLayer {
        self.material_layer
    }

    pub fn face_count(&self) -> u32 {
        self.face_count
    }

    pub fn origin(&self) -> Vec3 {
        self.origin
    }
}

fn face_count(faces: &ChunkMeshFaces) -> u32 {
    u32::try_from(faces.len()).expect("chunk mesh face count must fit in u32")
}

/// Shared padded light data used to shade a chunk mesh layer.
#[derive(Component, Clone)]
pub struct ChunkMeshLight {
    data: Arc<[u32]>,
}

impl ChunkMeshLight {
    pub fn new(data: Arc<[u32]>) -> Self {
        Self { data }
    }

    pub fn data(&self) -> &[u32] {
        &self.data
    }

    pub fn shared_data(&self) -> Arc<[u32]> {
        Arc::clone(&self.data)
    }

    pub fn replace(&mut self, data: Arc<[u32]>) {
        self.data = data;
    }

    pub(crate) fn data_key(&self) -> SharedLightDataKey {
        SharedLightDataKey {
            ptr: self.data.as_ptr() as usize,
            len: self.data.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SharedLightDataKey {
    pub(crate) ptr: usize,
    pub(crate) len: usize,
}
