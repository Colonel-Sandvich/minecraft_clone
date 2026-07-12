use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bevy::{
    platform::collections::HashMap,
    prelude::Entity,
    tasks::{Task, futures::check_ready},
};

use crate::world::chunk::{
    Chunk, ChunkColumn, ChunkHeightmap, ChunkLight, ChunkPos, ChunkRevision,
    light::RebuiltChunkLight, mesh::ChunkMeshLight,
};

use super::{
    ChunkTaskPool,
    streaming::{ColumnLightRevision, LightPatchTicket},
};

pub(crate) struct OwnedLightPatchInput {
    height_chunks: usize,
    chunks: Vec<OwnedLightCalculationChunk>,
}

impl OwnedLightPatchInput {
    pub(crate) fn new(height_chunks: usize, chunks: Vec<OwnedLightCalculationChunk>) -> Self {
        Self {
            height_chunks,
            chunks,
        }
    }
}

pub(crate) struct OwnedLightCalculationChunk {
    pub(crate) position: ChunkPos,
    pub(crate) chunk: Chunk,
    pub(crate) commit_baseline: Option<LightCommitBaseline>,
}

pub(crate) struct LightCommitBaseline {
    pub(crate) light: ChunkLight,
    pub(crate) heightmap: ChunkHeightmap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LightColumnInputStamp {
    pub(crate) column: ChunkColumn,
    pub(crate) incarnation: Entity,
    pub(crate) commit_light_revision: Option<ColumnLightRevision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LightChunkInputStamp {
    pub(crate) position: ChunkPos,
    pub(crate) entity: Entity,
    pub(crate) content_revision: ChunkRevision,
}

pub(crate) struct LightPatchTaskRequest {
    pub(crate) ticket: LightPatchTicket,
    pub(crate) commit_columns: Vec<ChunkColumn>,
    pub(crate) column_inputs: Vec<LightColumnInputStamp>,
    pub(crate) chunk_inputs: Vec<LightChunkInputStamp>,
    pub(crate) input: OwnedLightPatchInput,
}

pub(crate) struct PreparedLightPayload {
    pub(crate) position: ChunkPos,
    pub(crate) data: Arc<[u32]>,
}

pub(crate) struct SolvedLightPatch {
    pub(crate) rebuilt: Vec<RebuiltChunkLight>,
    pub(crate) prepared: Vec<PreparedLightPayload>,
    pub(crate) elapsed: Duration,
    pub(crate) solve_elapsed: Duration,
    pub(crate) prepare_elapsed: Duration,
    pub(crate) queue_elapsed: Duration,
}

struct ActiveLightPatchTask {
    ticket: LightPatchTicket,
    commit_columns: Vec<ChunkColumn>,
    column_inputs: Vec<LightColumnInputStamp>,
    chunk_inputs: Vec<LightChunkInputStamp>,
    started: Instant,
    task: Task<SolvedLightPatch>,
    cancel_requested: bool,
}

pub(crate) struct FinishedLightPatchTask {
    pub(crate) ticket: LightPatchTicket,
    pub(crate) commit_columns: Vec<ChunkColumn>,
    pub(crate) column_inputs: Vec<LightColumnInputStamp>,
    pub(crate) chunk_inputs: Vec<LightChunkInputStamp>,
    pub(crate) latency: Duration,
    pub(crate) cancel_requested: bool,
    pub(crate) result: SolvedLightPatch,
}

#[derive(Default)]
pub(crate) struct DimensionLightTasks {
    active: Option<ActiveLightPatchTask>,
    cancelled_since_last_take: usize,
}

impl DimensionLightTasks {
    pub(crate) fn is_idle(&self) -> bool {
        self.active.is_none()
    }

    pub(crate) fn active_ticket(&self) -> Option<LightPatchTicket> {
        self.active.as_ref().map(|active| active.ticket)
    }

    pub(crate) fn active_depends_on(&self, column: ChunkColumn) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active
                .column_inputs
                .iter()
                .any(|input| input.column == column)
        })
    }

    pub(crate) fn start(&mut self, task_pool: &ChunkTaskPool, request: LightPatchTaskRequest) {
        assert!(
            self.active.is_none(),
            "only one light patch may run per dimension"
        );
        let LightPatchTaskRequest {
            ticket,
            commit_columns,
            column_inputs,
            chunk_inputs,
            input,
        } = request;
        let started = Instant::now();
        let task = task_pool.spawn(async move {
            let queue_elapsed = started.elapsed();
            solve_light_patch(input, queue_elapsed)
        });
        self.active = Some(ActiveLightPatchTask {
            ticket,
            commit_columns,
            column_inputs,
            chunk_inputs,
            started,
            task,
            cancel_requested: false,
        });
    }

    pub(crate) fn cancel(&mut self, ticket: LightPatchTicket) -> bool {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.ticket == ticket)
        else {
            return false;
        };
        if active.cancel_requested {
            return false;
        }
        active.cancel_requested = true;
        self.cancelled_since_last_take += 1;
        true
    }

    pub(crate) fn take_cancelled_count(&mut self) -> usize {
        std::mem::take(&mut self.cancelled_since_last_take)
    }

    pub(crate) fn take_ready(&mut self) -> Option<FinishedLightPatchTask> {
        let active = self.active.as_mut()?;
        let result = check_ready(&mut active.task)?;
        let active = self
            .active
            .take()
            .expect("ready light task must remain active until collected");
        Some(FinishedLightPatchTask {
            ticket: active.ticket,
            commit_columns: active.commit_columns,
            column_inputs: active.column_inputs,
            chunk_inputs: active.chunk_inputs,
            latency: active.started.elapsed(),
            cancel_requested: active.cancel_requested,
            result,
        })
    }
}

fn solve_light_patch(input: OwnedLightPatchInput, queue_elapsed: Duration) -> SolvedLightPatch {
    let started = Instant::now();
    let mut region = crate::world::chunk::light::ChunkLightRegion::new(input.height_chunks);
    for chunk in &input.chunks {
        region.insert_calculation_chunk(chunk.position, &chunk.chunk);
        if let Some(baseline) = &chunk.commit_baseline {
            region.mark_commit_target(chunk.position, &baseline.light, &baseline.heightmap);
        }
    }

    let solve_started = Instant::now();
    let solved = region.solve();
    let solve_elapsed = solve_started.elapsed();
    let prepare_started = Instant::now();
    let prepared = {
        let solved_lights = solved.lights().collect::<HashMap<_, _>>();
        input
            .chunks
            .iter()
            .filter(|chunk| chunk.commit_baseline.is_some())
            .map(|chunk| PreparedLightPayload {
                position: chunk.position,
                data: Arc::from(ChunkMeshLight::build_padded_data(
                    chunk.position,
                    &solved_lights,
                )),
            })
            .collect()
    };
    let prepare_elapsed = prepare_started.elapsed();
    let rebuilt = solved.into_committed();

    SolvedLightPatch {
        rebuilt,
        prepared,
        elapsed: started.elapsed(),
        solve_elapsed,
        prepare_elapsed,
        queue_elapsed,
    }
}
