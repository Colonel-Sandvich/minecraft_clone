use bevy::prelude::*;

use super::{Chunk, ChunkNeedsMeshRebuild};

pub(crate) const AO_BRIGHTNESS: [f32; 4] = [0.45, 0.65, 0.82, 1.0];
const AO_CONTRAST_BRIGHTNESS: [f32; 4] = [0.0, 0.25, 0.6, 1.0];

pub struct AmbientOcclusionPlugin;

impl Plugin for AmbientOcclusionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AmbientOcclusionSettings>()
            .register_type::<AmbientOcclusionSettings>()
            .register_type::<AmbientOcclusionMode>()
            .add_systems(Update, mark_chunk_meshes_dirty_on_ao_settings_change);
    }
}

#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub struct AmbientOcclusionSettings {
    pub mode: AmbientOcclusionMode,
}

impl Default for AmbientOcclusionSettings {
    fn default() -> Self {
        Self {
            mode: AmbientOcclusionMode::Normal,
        }
    }
}

impl AmbientOcclusionSettings {
    pub fn brightness_curve(&self) -> [f32; 4] {
        self.mode.brightness_curve()
    }

    pub fn cycle_mode(&mut self) {
        self.mode = self.mode.next();
    }
}

#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AmbientOcclusionMode {
    #[default]
    Normal,
    Contrast,
    Off,
}

impl AmbientOcclusionMode {
    pub fn next(self) -> Self {
        match self {
            Self::Normal => Self::Contrast,
            Self::Contrast => Self::Off,
            Self::Off => Self::Normal,
        }
    }

    pub(crate) fn brightness_curve(self) -> [f32; 4] {
        match self {
            Self::Normal => AO_BRIGHTNESS,
            Self::Contrast => AO_CONTRAST_BRIGHTNESS,
            Self::Off => [1.0; 4],
        }
    }
}

fn mark_chunk_meshes_dirty_on_ao_settings_change(
    mut commands: Commands,
    settings: Res<AmbientOcclusionSettings>,
    chunks: Query<Entity, With<Chunk>>,
) {
    if !settings.is_changed() {
        return;
    }

    for entity in chunks.iter() {
        commands.entity(entity).insert(ChunkNeedsMeshRebuild);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambient_occlusion_modes_cycle_and_expose_brightness_curves() {
        assert_eq!(
            AmbientOcclusionMode::Normal.next(),
            AmbientOcclusionMode::Contrast
        );
        assert_eq!(
            AmbientOcclusionMode::Contrast.next(),
            AmbientOcclusionMode::Off
        );
        assert_eq!(
            AmbientOcclusionMode::Off.next(),
            AmbientOcclusionMode::Normal
        );

        assert_eq!(
            AmbientOcclusionMode::Normal.brightness_curve(),
            AO_BRIGHTNESS
        );
        assert_eq!(
            AmbientOcclusionMode::Contrast.brightness_curve(),
            AO_CONTRAST_BRIGHTNESS
        );
        assert_eq!(AmbientOcclusionMode::Off.brightness_curve(), [1.0; 4]);
    }

    #[test]
    fn changing_ao_settings_marks_loaded_chunks_for_mesh_rebuild() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<AmbientOcclusionSettings>()
            .add_systems(Update, mark_chunk_meshes_dirty_on_ao_settings_change);
        let chunk_entity = app.world_mut().spawn(Chunk::default()).id();

        app.world_mut()
            .resource_mut::<AmbientOcclusionSettings>()
            .cycle_mode();
        app.update();

        assert_eq!(
            app.world().resource::<AmbientOcclusionSettings>().mode,
            AmbientOcclusionMode::Contrast
        );
        assert!(
            app.world()
                .get::<ChunkNeedsMeshRebuild>(chunk_entity)
                .is_some()
        );
    }
}
