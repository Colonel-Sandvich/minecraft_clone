use std::fmt;

use bevy::prelude::{Resource, Vec3};

use super::{
    chunk::{ChunkColumn, ChunkPos},
    generation::{WorldHeight, WorldMetadata, terrain_height},
};

const ARRIVAL_XZ: f32 = 8.0;
// Matches the current player spawn calculation without making world
// definitions depend on the player module.
const ARRIVAL_BODY_HEIGHT: f32 = 1.8;
const ARRIVAL_PADDING: f32 = 2.0;

/// Stable logical identity for a dimension.
///
/// This is deliberately separate from the Bevy entity that owns a resident
/// dimension incarnation. Entity generations protect runtime work, while this
/// value qualifies durable data and survives unloading.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DimensionId(u32);

impl DimensionId {
    pub const OVERWORLD: Self = Self(0);
    pub const GRASS_FLOOR: Self = Self(1);
    pub const CENTER_GLASS_PLATFORM: Self = Self(2);

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for DimensionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A dimension-qualified chunk coordinate used at durable and asynchronous
/// ownership boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkAddress {
    dimension: DimensionId,
    position: ChunkPos,
}

impl ChunkAddress {
    pub const fn new(dimension: DimensionId, position: ChunkPos) -> Self {
        Self {
            dimension,
            position,
        }
    }

    pub const fn dimension(self) -> DimensionId {
        self.dimension
    }

    pub const fn position(self) -> ChunkPos {
        self.position
    }

    pub const fn column(self) -> ColumnAddress {
        ColumnAddress::new(self.dimension, self.position.column())
    }
}

/// A dimension-qualified XZ column coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnAddress {
    dimension: DimensionId,
    position: ChunkColumn,
}

impl ColumnAddress {
    pub const fn new(dimension: DimensionId, position: ChunkColumn) -> Self {
        Self {
            dimension,
            position,
        }
    }

    pub const fn dimension(self) -> DimensionId {
        self.dimension
    }

    pub const fn column(self) -> ChunkColumn {
        self.position
    }

    pub const fn chunk(self, y: i32) -> ChunkAddress {
        ChunkAddress::new(self.dimension, self.position.chunk(y))
    }
}

impl From<ChunkAddress> for ColumnAddress {
    fn from(address: ChunkAddress) -> Self {
        address.column()
    }
}

/// A stable generator family and version.
///
/// New terrain behavior must be introduced as a new variant instead of
/// changing an existing profile in place. Persisted dimensions can therefore
/// keep requesting the generator that originally created them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeneratorProfile {
    OverworldV1,
    GrassFloorV1,
    CenterGlassPlatformV1,
}

impl GeneratorProfile {
    /// Stable, human-readable generator family used at persistence boundaries.
    pub const fn family(self) -> &'static str {
        match self {
            Self::OverworldV1 => "overworld",
            Self::GrassFloorV1 => "grass_floor",
            Self::CenterGlassPlatformV1 => "center_glass_platform",
        }
    }

    pub const fn version(self) -> u32 {
        match self {
            Self::OverworldV1 | Self::GrassFloorV1 | Self::CenterGlassPlatformV1 => 1,
        }
    }
}

/// Immutable configuration for one logical dimension.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DimensionDefinition {
    id: DimensionId,
    height: WorldHeight,
    generator: GeneratorProfile,
    arrival: Vec3,
}

impl DimensionDefinition {
    pub const fn new(
        id: DimensionId,
        height: WorldHeight,
        generator: GeneratorProfile,
        arrival: Vec3,
    ) -> Self {
        Self {
            id,
            height,
            generator,
            arrival,
        }
    }

    pub const fn id(self) -> DimensionId {
        self.id
    }

    pub const fn height(self) -> WorldHeight {
        self.height
    }

    pub const fn generator(self) -> GeneratorProfile {
        self.generator
    }

    pub const fn arrival(self) -> Vec3 {
        self.arrival
    }
}

const fn dimension_ids_are_unique(definitions: &[DimensionDefinition]) -> bool {
    let mut left = 0;
    while left < definitions.len() {
        let mut right = left + 1;
        while right < definitions.len() {
            if definitions[left].id.get() == definitions[right].id.get() {
                return false;
            }
            right += 1;
        }
        left += 1;
    }
    true
}

/// Read-only catalog of the dimensions built into this world format.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct DimensionCatalog {
    definitions: [DimensionDefinition; 3],
}

impl DimensionCatalog {
    /// Builds the built-in definitions for one world's seed and validated
    /// height. The resulting value is safe to capture in asynchronous work.
    pub fn for_world(metadata: &WorldMetadata) -> Self {
        let height = metadata.height();
        let overworld_surface = terrain_height(metadata, ARRIVAL_XZ as i32, ARRIVAL_XZ as i32);
        let definitions = [
            DimensionDefinition::new(
                DimensionId::OVERWORLD,
                height,
                GeneratorProfile::OverworldV1,
                Vec3::new(
                    ARRIVAL_XZ,
                    overworld_surface as f32 + ARRIVAL_BODY_HEIGHT + ARRIVAL_PADDING,
                    ARRIVAL_XZ,
                ),
            ),
            DimensionDefinition::new(
                DimensionId::GRASS_FLOOR,
                height,
                GeneratorProfile::GrassFloorV1,
                Vec3::new(
                    ARRIVAL_XZ,
                    ARRIVAL_BODY_HEIGHT + ARRIVAL_PADDING,
                    ARRIVAL_XZ,
                ),
            ),
            DimensionDefinition::new(
                DimensionId::CENTER_GLASS_PLATFORM,
                height,
                GeneratorProfile::CenterGlassPlatformV1,
                Vec3::new(
                    ARRIVAL_XZ,
                    ARRIVAL_BODY_HEIGHT + ARRIVAL_PADDING,
                    ARRIVAL_XZ,
                ),
            ),
        ];
        assert!(
            dimension_ids_are_unique(&definitions),
            "built-in dimension IDs must be unique"
        );
        Self { definitions }
    }

    pub const fn definitions(&self) -> &[DimensionDefinition] {
        &self.definitions
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &DimensionDefinition> {
        self.definitions.iter()
    }

    pub fn get(&self, id: DimensionId) -> Option<&DimensionDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.id == id)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn builtin_catalog_has_unique_stable_ids() {
        let catalog = DimensionCatalog::for_world(&WorldMetadata::default());
        let ids = catalog
            .iter()
            .map(|definition| definition.id())
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            [
                DimensionId::new(0),
                DimensionId::new(1),
                DimensionId::new(2)
            ]
        );
        assert_eq!(ids.iter().copied().collect::<HashSet<_>>().len(), ids.len());
        for id in ids {
            assert_eq!(catalog.get(id).map(|definition| definition.id()), Some(id));
        }
        assert!(catalog.get(DimensionId::new(u32::MAX)).is_none());
    }

    #[test]
    fn generator_profiles_expose_stable_family_versions() {
        let catalog = DimensionCatalog::for_world(&WorldMetadata::default());
        let profiles = catalog
            .iter()
            .map(|definition| definition.generator())
            .collect::<Vec<_>>();
        let identities = profiles
            .iter()
            .map(|profile| (profile.family(), profile.version()))
            .collect::<HashSet<_>>();

        assert_eq!(identities.len(), profiles.len());
        assert!(profiles.iter().all(|profile| profile.version() == 1));
    }

    #[test]
    fn catalog_uses_the_world_height_and_seeded_overworld_arrival() {
        let metadata = WorldMetadata::with_seed(42).with_height_chunks(3).unwrap();
        let catalog = DimensionCatalog::for_world(&metadata);

        assert!(
            catalog
                .iter()
                .all(|definition| definition.height() == metadata.height())
        );
        let overworld = catalog.get(DimensionId::OVERWORLD).unwrap();
        assert_eq!(
            overworld.arrival().y,
            terrain_height(&metadata, 8, 8) as f32 + ARRIVAL_BODY_HEIGHT + ARRIVAL_PADDING
        );

        for definition in catalog.iter() {
            let arrival = definition.arrival();
            assert!(arrival.x.is_finite() && arrival.y.is_finite() && arrival.z.is_finite());
            assert!((0.0..definition.height().blocks() as f32).contains(&arrival.y));
        }
    }
}
