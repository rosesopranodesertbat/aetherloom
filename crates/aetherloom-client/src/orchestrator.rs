use alloc::vec::Vec;

use aetherloom_core::ClientReplica;
use aetherloom_protocol::{PlayerCommand, ValidationError, AUTHORITATIVE_HZ};

use crate::MappedInput;

pub const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;
pub const MAX_PREDICTION_STEPS_PER_FRAME: u64 = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameCadence {
    /// Commands generated for every fixed prediction tick elapsed since the
    /// previous rendered frame.
    pub commands: Vec<PlayerCommand>,
    pub predicted_through_tick: Option<u64>,
    /// Numerator in `[0, NANOSECONDS_PER_SECOND)`. Dividing it by one billion
    /// gives the interpolation alpha without accumulating float drift.
    pub interpolation_numerator: u64,
    pub render_frame_index: u64,
}

impl FrameCadence {
    pub fn interpolation_alpha(&self) -> f32 {
        self.interpolation_numerator as f32 / NANOSECONDS_PER_SECOND as f32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrchestratorError {
    ClockOverflow,
    Command(ValidationError),
    /// The wall-clock gap was explicitly discarded and prediction is stopped
    /// until a fresh authoritative snapshot is used to rebase.
    PredictionBacklog { discarded_ticks: u64 },
    ResyncRequired,
    Suspended,
}

/// Variable-rate frame orchestration over fixed 128 Hz local prediction.
///
/// Time is accumulated as integer tick quanta (`nanoseconds * 128`), avoiding
/// the rounding drift of repeatedly adding `1.0 / 128.0` in floating point.
/// Normal frame variance never drops a tick. A gap beyond the bounded
/// prediction budget is discarded explicitly and requires snapshot rebasing;
/// the client never slows or bursts authoritative online time to hide it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientFrameOrchestrator {
    next_prediction_tick: u64,
    next_sequence: u32,
    accumulated_tick_quanta: u64,
    render_frame_index: u64,
    suspended: bool,
    resync_required: bool,
    look_yaw: u16,
    look_pitch: i16,
    pending_look_yaw_delta: i32,
    pending_look_pitch_delta: i32,
}

impl ClientFrameOrchestrator {
    pub const fn new(next_prediction_tick: u64, next_sequence: u32) -> Self {
        Self {
            next_prediction_tick,
            next_sequence: nonzero_sequence(next_sequence),
            accumulated_tick_quanta: 0,
            render_frame_index: 0,
            suspended: false,
            resync_required: false,
            look_yaw: 0,
            look_pitch: 0,
            pending_look_yaw_delta: 0,
            pending_look_pitch_delta: 0,
        }
    }

    pub const fn next_prediction_tick(&self) -> u64 {
        self.next_prediction_tick
    }

    pub const fn next_sequence(&self) -> u32 {
        self.next_sequence
    }

    pub const fn is_suspended(&self) -> bool {
        self.suspended
    }

    pub const fn requires_resync(&self) -> bool {
        self.resync_required
    }

    pub const fn look(&self) -> (u16, i16) {
        (self.look_yaw, self.look_pitch)
    }

    pub fn set_look(&mut self, yaw: u16, pitch: i16) {
        self.look_yaw = yaw;
        self.look_pitch = pitch.clamp(
            -aetherloom_protocol::MAX_LOOK_PITCH,
            aetherloom_protocol::MAX_LOOK_PITCH,
        );
        self.pending_look_yaw_delta = 0;
        self.pending_look_pitch_delta = 0;
    }

    pub fn suspend(&mut self) {
        self.suspended = true;
        self.accumulated_tick_quanta = 0;
        self.pending_look_yaw_delta = 0;
        self.pending_look_pitch_delta = 0;
    }

    /// Resume at an explicitly reconciled authoritative tick. Wall-clock time
    /// while suspended is never replayed as a burst of predicted input.
    pub fn resume_at(&mut self, next_prediction_tick: u64) {
        self.next_prediction_tick = next_prediction_tick;
        self.accumulated_tick_quanta = 0;
        self.suspended = false;
        self.resync_required = false;
        self.pending_look_yaw_delta = 0;
        self.pending_look_pitch_delta = 0;
    }

    pub fn rebase(&mut self, next_prediction_tick: u64, next_sequence: u32) {
        self.next_prediction_tick = next_prediction_tick;
        self.next_sequence = nonzero_sequence(next_sequence);
        self.accumulated_tick_quanta = 0;
        self.resync_required = false;
        self.pending_look_yaw_delta = 0;
        self.pending_look_pitch_delta = 0;
    }

    pub fn advance_frame(
        &mut self,
        elapsed_nanoseconds: u64,
        input: MappedInput,
        replica: &mut ClientReplica,
    ) -> Result<FrameCadence, OrchestratorError> {
        if self.suspended {
            return Err(OrchestratorError::Suspended);
        }
        if self.resync_required {
            return Err(OrchestratorError::ResyncRequired);
        }
        let added_quanta = elapsed_nanoseconds
            .checked_mul(AUTHORITATIVE_HZ as u64)
            .ok_or(OrchestratorError::ClockOverflow)?;
        let accumulated_tick_quanta = self
            .accumulated_tick_quanta
            .checked_add(added_quanta)
            .ok_or(OrchestratorError::ClockOverflow)?;

        let elapsed_ticks = accumulated_tick_quanta / NANOSECONDS_PER_SECOND;
        if elapsed_ticks > MAX_PREDICTION_STEPS_PER_FRAME {
            self.accumulated_tick_quanta = 0;
            self.pending_look_yaw_delta = 0;
            self.pending_look_pitch_delta = 0;
            self.resync_required = true;
            return Err(OrchestratorError::PredictionBacklog {
                discarded_ticks: elapsed_ticks,
            });
        }
        self.accumulated_tick_quanta = accumulated_tick_quanta;
        self.pending_look_yaw_delta = self
            .pending_look_yaw_delta
            .saturating_add(input.look_yaw_delta as i32);
        self.pending_look_pitch_delta = self
            .pending_look_pitch_delta
            .saturating_add(input.look_pitch_delta as i32);
        let command_capacity =
            usize::try_from(elapsed_ticks).map_err(|_| OrchestratorError::ClockOverflow)?;
        if elapsed_ticks != 0 {
            let (look_yaw, look_pitch) = advance_look(
                self.look_yaw,
                self.look_pitch,
                self.pending_look_yaw_delta,
                self.pending_look_pitch_delta,
                input.controller_yaw_per_tick,
                input.controller_pitch_per_tick,
            );
            PlayerCommand::new(
                self.next_prediction_tick,
                self.next_sequence,
                input.move_x,
                input.move_y,
                input.move_vertical,
                look_yaw,
                look_pitch,
                input.action_flags,
                input.requested_spell,
            )
            .map_err(OrchestratorError::Command)?;
        }
        self.accumulated_tick_quanta %= NANOSECONDS_PER_SECOND;
        let mut commands = Vec::with_capacity(command_capacity);
        for tick_index in 0..elapsed_ticks {
            let yaw_delta = if tick_index == 0 {
                self.pending_look_yaw_delta
            } else {
                0
            };
            let pitch_delta = if tick_index == 0 {
                self.pending_look_pitch_delta
            } else {
                0
            };
            (self.look_yaw, self.look_pitch) = advance_look(
                self.look_yaw,
                self.look_pitch,
                yaw_delta,
                pitch_delta,
                input.controller_yaw_per_tick,
                input.controller_pitch_per_tick,
            );
            let command = PlayerCommand::new(
                self.next_prediction_tick,
                self.next_sequence,
                input.move_x,
                input.move_y,
                input.move_vertical,
                self.look_yaw,
                self.look_pitch,
                input.action_flags,
                input.requested_spell,
            )
            .map_err(OrchestratorError::Command)?;
            replica.predict_local(command);
            commands.push(command);
            self.next_prediction_tick = self.next_prediction_tick.wrapping_add(1);
            self.next_sequence = nonzero_sequence(self.next_sequence.wrapping_add(1));
        }
        if elapsed_ticks != 0 {
            self.pending_look_yaw_delta = 0;
            self.pending_look_pitch_delta = 0;
        }

        self.render_frame_index = self.render_frame_index.wrapping_add(1);
        let predicted_through_tick = commands.last().map(PlayerCommand::target_tick);
        Ok(FrameCadence {
            commands,
            predicted_through_tick,
            interpolation_numerator: self.accumulated_tick_quanta,
            render_frame_index: self.render_frame_index,
        })
    }
}

const fn nonzero_sequence(sequence: u32) -> u32 {
    if sequence == 0 {
        1
    } else {
        sequence
    }
}

fn advance_look(
    yaw: u16,
    pitch: i16,
    yaw_delta: i32,
    pitch_delta: i32,
    controller_yaw_per_tick: i16,
    controller_pitch_per_tick: i16,
) -> (u16, i16) {
    let yaw_delta = yaw_delta.saturating_add(controller_yaw_per_tick as i32);
    let pitch_delta = pitch_delta.saturating_add(controller_pitch_per_tick as i32);
    let next_yaw = yaw.wrapping_add(yaw_delta as u16);
    let next_pitch = (pitch as i32)
        .saturating_add(pitch_delta)
        .clamp(
            -(aetherloom_protocol::MAX_LOOK_PITCH as i32),
            aetherloom_protocol::MAX_LOOK_PITCH as i32,
        ) as i16;
    (next_yaw, next_pitch)
}
