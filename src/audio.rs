use bevy::{
    audio::{AudioPlayer, AudioSource, PlaybackSettings, SpatialListener, Volume},
    prelude::*,
};
use bevy_settings::{ReflectSettingsGroup, SettingsGroup};

use crate::{
    item::ItemPickedUp,
    player::{
        Player,
        cam::MouseCam,
        interaction::{BlockEditCommitted, BlockEditKind},
    },
};

pub const BLOCK_BREAK_SOUND_PATHS: &[&str] = &["audio/block/rock_break.ogg"];
pub const BLOCK_PLACE_SOUND_PATHS: &[&str] = &["audio/block/small_rock_impact.ogg"];
pub const ITEM_PICKUP_SOUND_PATHS: &[&str] = &["audio/item/pickup.ogg"];

const SPATIAL_LISTENER_EAR_GAP: f32 = 0.2;

pub struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameAudioSettings>()
            .init_resource::<SoundBank>()
            .init_resource::<VariantCursor>()
            .add_message::<BlockEditCommitted>()
            .add_message::<ItemPickedUp>()
            .add_message::<PlaySound>()
            .add_systems(Update, attach_spatial_listener)
            .add_systems(
                Update,
                (
                    request_block_edit_sounds,
                    request_item_pickup_sounds,
                    play_requested_sounds,
                )
                    .chain(),
            );
    }
}

/// User-facing audio levels. These are registered with `bevy-settings` by the
/// application before its settings plugin is installed.
///
/// Each field is a linear gain in the inclusive range `0.0..=1.0`. Values are
/// sanitized at playback as a final guard against hand-edited settings files.
#[derive(Resource, SettingsGroup, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Resource, SettingsGroup, Default)]
pub struct GameAudioSettings {
    pub master_volume: f32,
    pub sound_effects_volume: f32,
}

impl Default for GameAudioSettings {
    fn default() -> Self {
        Self {
            master_volume: 1.0,
            sound_effects_volume: 0.8,
        }
    }
}

impl GameAudioSettings {
    fn sound_effect_gain(self) -> f32 {
        sanitized_gain(self.master_volume) * sanitized_gain(self.sound_effects_volume)
    }
}

fn sanitized_gain(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// A semantic sound identifier. Gameplay code asks for a cue rather than
/// depending on an asset path or Bevy audio component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundCue {
    BlockBreak,
    BlockPlace,
    ItemPickup,
}

impl SoundCue {
    const fn index(self) -> usize {
        match self {
            Self::BlockBreak => 0,
            Self::BlockPlace => 1,
            Self::ItemPickup => 2,
        }
    }

    const fn playback_speed(self) -> f32 {
        match self {
            // The CC0 source effects are longer, lower-pitched recordings than
            // the short, bright impacts expected for voxel interactions.
            Self::BlockBreak | Self::BlockPlace => 1.35,
            // Minecraft applies pitch at playback; the raw pop asset sounds
            // roughly an octave too low when played at Bevy's default 1.0x.
            Self::ItemPickup => 2.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SoundEmitter {
    Listener,
    World(Vec3),
}

/// Reusable request boundary for one-shot sounds. Future UI, mobs, and ambient
/// systems can publish these without knowing how sounds are loaded or played.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub struct PlaySound {
    pub cue: SoundCue,
    pub emitter: SoundEmitter,
}

impl PlaySound {
    pub const fn at(cue: SoundCue, position: Vec3) -> Self {
        Self {
            cue,
            emitter: SoundEmitter::World(position),
        }
    }

    pub const fn at_listener(cue: SoundCue) -> Self {
        Self {
            cue,
            emitter: SoundEmitter::Listener,
        }
    }
}

#[derive(Resource)]
struct SoundBank {
    block_break: Vec<Handle<AudioSource>>,
    block_place: Vec<Handle<AudioSource>>,
    item_pickup: Vec<Handle<AudioSource>>,
}

impl FromWorld for SoundBank {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self {
            block_break: load_sound_variants(asset_server, BLOCK_BREAK_SOUND_PATHS),
            block_place: load_sound_variants(asset_server, BLOCK_PLACE_SOUND_PATHS),
            item_pickup: load_sound_variants(asset_server, ITEM_PICKUP_SOUND_PATHS),
        }
    }
}

impl SoundBank {
    fn variants(&self, cue: SoundCue) -> &[Handle<AudioSource>] {
        match cue {
            SoundCue::BlockBreak => &self.block_break,
            SoundCue::BlockPlace => &self.block_place,
            SoundCue::ItemPickup => &self.item_pickup,
        }
    }
}

fn load_sound_variants(
    asset_server: &AssetServer,
    paths: &'static [&'static str],
) -> Vec<Handle<AudioSource>> {
    paths.iter().map(|&path| asset_server.load(path)).collect()
}

#[derive(Resource, Default)]
struct VariantCursor([usize; 3]);

impl VariantCursor {
    fn next(&mut self, cue: SoundCue, variant_count: usize) -> Option<usize> {
        if variant_count == 0 {
            return None;
        }

        let cursor = &mut self.0[cue.index()];
        let selected = *cursor % variant_count;
        *cursor = cursor.wrapping_add(1);
        Some(selected)
    }
}

fn attach_spatial_listener(
    mut commands: Commands,
    listeners: Query<(), With<SpatialListener>>,
    cameras: Query<Entity, (With<MouseCam>, Without<SpatialListener>)>,
) {
    if !listeners.is_empty() {
        return;
    }
    let Some(camera) = cameras.iter().next() else {
        return;
    };

    commands
        .entity(camera)
        .insert(SpatialListener::new(SPATIAL_LISTENER_EAR_GAP));
}

fn request_block_edit_sounds(
    mut edits: MessageReader<BlockEditCommitted>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for edit in edits.read() {
        let Some(cue) = cue_for_block_edit(edit) else {
            continue;
        };
        let position = edit.position.world().as_ivec3().as_vec3() + Vec3::splat(0.5);
        sounds.write(PlaySound::at(cue, position));
    }
}

fn request_item_pickup_sounds(
    mut pickups: MessageReader<ItemPickedUp>,
    players: Query<&Transform, With<Player>>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for pickup in pickups.read() {
        let Ok(transform) = players.get(pickup.player) else {
            continue;
        };
        sounds.write(PlaySound::at(SoundCue::ItemPickup, transform.translation));
    }
}

fn cue_for_block_edit(edit: &BlockEditCommitted) -> Option<SoundCue> {
    match edit.kind {
        BlockEditKind::Break if edit.delta.old.as_block().is_some() => Some(SoundCue::BlockBreak),
        BlockEditKind::Place if edit.delta.new.as_block().is_some() => Some(SoundCue::BlockPlace),
        _ => None,
    }
}

fn play_requested_sounds(
    mut commands: Commands,
    mut requests: MessageReader<PlaySound>,
    sounds: Res<SoundBank>,
    settings: Res<GameAudioSettings>,
    mut variants: ResMut<VariantCursor>,
) {
    let gain = settings.sound_effect_gain();
    if gain == 0.0 {
        requests.clear();
        return;
    }

    for request in requests.read().copied() {
        let available = sounds.variants(request.cue);
        let Some(index) = variants.next(request.cue, available.len()) else {
            warn!(?request.cue, "sound cue has no configured asset variants");
            continue;
        };

        let spatial = matches!(request.emitter, SoundEmitter::World(_));
        let playback = PlaybackSettings::DESPAWN
            .with_volume(Volume::Linear(gain))
            .with_speed(request.cue.playback_speed())
            .with_spatial(spatial);
        let mut entity = commands.spawn((AudioPlayer::new(available[index].clone()), playback));

        if let SoundEmitter::World(position) = request.emitter {
            entity.insert(Transform::from_translation(position));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bevy::audio::Decodable;

    use crate::{
        item::{Item, ItemStack},
        player::interaction::{BlockEditCommitted, BlockEditKind},
        world::chunk::{CellDelta, ChunkBlockPos, ChunkCell, WorldBlockPos},
    };

    use super::*;

    fn edit(kind: BlockEditKind, old: ChunkCell, new: ChunkCell) -> BlockEditCommitted {
        BlockEditCommitted {
            kind,
            position: ChunkBlockPos::default(),
            delta: CellDelta { old, new },
        }
    }

    #[test]
    fn block_edit_cues_follow_committed_material_changes() {
        assert_eq!(
            cue_for_block_edit(&edit(
                BlockEditKind::Break,
                Item::Stone.into(),
                ChunkCell::EMPTY,
            )),
            Some(SoundCue::BlockBreak)
        );
        assert_eq!(
            cue_for_block_edit(&edit(
                BlockEditKind::Place,
                ChunkCell::EMPTY,
                Item::Dirt.into(),
            )),
            Some(SoundCue::BlockPlace)
        );
        assert_eq!(
            cue_for_block_edit(&edit(
                BlockEditKind::Place,
                ChunkCell::EMPTY,
                ChunkCell::water_source(),
            )),
            None,
            "fluid placement needs its own material-appropriate cue"
        );
    }

    #[test]
    fn sound_effect_gain_sanitizes_persisted_settings() {
        assert_eq!(GameAudioSettings::default().sound_effect_gain(), 0.8);
        assert_eq!(
            GameAudioSettings {
                master_volume: 2.0,
                sound_effects_volume: 0.5,
            }
            .sound_effect_gain(),
            0.5
        );
        assert_eq!(
            GameAudioSettings {
                master_volume: f32::NAN,
                sound_effects_volume: 1.0,
            }
            .sound_effect_gain(),
            0.0
        );
    }

    #[test]
    fn configured_sound_assets_are_bundled_and_decodable() {
        let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        for path in BLOCK_BREAK_SOUND_PATHS
            .iter()
            .chain(BLOCK_PLACE_SOUND_PATHS)
            .chain(ITEM_PICKUP_SOUND_PATHS)
        {
            let bytes = std::fs::read(asset_root.join(path))
                .unwrap_or_else(|error| panic!("could not read audio asset {path}: {error}"));
            let source = AudioSource {
                bytes: bytes.into(),
            };
            assert!(
                source.decoder().next().is_some(),
                "audio asset did not decode any samples: {path}"
            );
        }
    }

    #[test]
    fn variant_selection_cycles_without_an_rng_dependency() {
        let mut cursor = VariantCursor::default();
        assert_eq!(cursor.next(SoundCue::BlockBreak, 3), Some(0));
        assert_eq!(cursor.next(SoundCue::BlockBreak, 3), Some(1));
        assert_eq!(cursor.next(SoundCue::BlockBreak, 3), Some(2));
        assert_eq!(cursor.next(SoundCue::BlockBreak, 3), Some(0));
        assert_eq!(cursor.next(SoundCue::BlockPlace, 0), None);
        assert_eq!(cursor.next(SoundCue::ItemPickup, 1), Some(0));
    }

    #[test]
    fn sound_cues_apply_intentional_playback_pitch() {
        assert_eq!(SoundCue::BlockBreak.playback_speed(), 1.35);
        assert_eq!(SoundCue::BlockPlace.playback_speed(), 1.35);
        assert_eq!(SoundCue::ItemPickup.playback_speed(), 2.0);
    }

    #[test]
    fn only_one_spatial_listener_is_attached() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, attach_spatial_listener);
        app.world_mut().spawn(MouseCam);
        app.world_mut().spawn(MouseCam);

        app.update();

        let listener_count = app
            .world_mut()
            .query_filtered::<Entity, With<SpatialListener>>()
            .iter(app.world())
            .count();
        assert_eq!(listener_count, 1);
    }

    #[test]
    fn committed_edits_schedule_one_spatial_sound_each_without_replaying() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<BlockEditCommitted>()
            .add_message::<PlaySound>()
            .insert_resource(SoundBank {
                block_break: vec![Handle::default()],
                block_place: vec![Handle::default()],
                item_pickup: vec![Handle::default()],
            })
            .insert_resource(GameAudioSettings::default())
            .init_resource::<VariantCursor>()
            .add_systems(
                Update,
                (request_block_edit_sounds, play_requested_sounds).chain(),
            );

        let edits = [
            BlockEditCommitted {
                kind: BlockEditKind::Break,
                position: WorldBlockPos::new(-2, 3, 4).split(),
                delta: CellDelta {
                    old: Item::Stone.into(),
                    new: ChunkCell::EMPTY,
                },
            },
            BlockEditCommitted {
                kind: BlockEditKind::Place,
                position: WorldBlockPos::new(8, -1, 6).split(),
                delta: CellDelta {
                    old: ChunkCell::EMPTY,
                    new: Item::Dirt.into(),
                },
            },
        ];
        for edit in edits {
            app.world_mut().write_message(edit);
        }

        app.update();

        let mut query = app
            .world_mut()
            .query_filtered::<(&Transform, &PlaybackSettings), With<AudioPlayer<AudioSource>>>();
        let emitters = query
            .iter(app.world())
            .map(|(transform, playback)| {
                assert!(playback.spatial);
                assert_eq!(playback.speed, 1.35);
                transform.translation
            })
            .collect::<Vec<_>>();
        assert_eq!(emitters.len(), 2);
        assert!(emitters.contains(&Vec3::new(-1.5, 3.5, 4.5)));
        assert!(emitters.contains(&Vec3::new(8.5, -0.5, 6.5)));

        app.update();
        assert_eq!(query.iter(app.world()).count(), 2);
    }

    #[test]
    fn item_pickup_schedules_sound_at_the_player() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<ItemPickedUp>()
            .add_message::<PlaySound>()
            .add_systems(Update, request_item_pickup_sounds);

        let player = app
            .world_mut()
            .spawn((Player::default(), Transform::from_xyz(3.0, 4.0, 5.0)))
            .id();
        let item = app.world_mut().spawn_empty().id();
        app.world_mut().write_message(ItemPickedUp {
            player,
            item,
            stack: ItemStack {
                item: Item::Dirt,
                count: 1,
            },
        });

        app.update();

        let sounds = app
            .world()
            .resource::<Messages<PlaySound>>()
            .iter_current_update_messages()
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(
            sounds,
            vec![PlaySound::at(
                SoundCue::ItemPickup,
                Vec3::new(3.0, 4.0, 5.0),
            )]
        );
    }
}
