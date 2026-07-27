use alloc::vec::Vec;

use crate::codec::{Reader, Writer};
use crate::{PlayerId, ProtocolError, ValidationError, MAX_PLAYERS};

pub const COMMAND_REDUNDANCY: usize = 3;
pub const MAX_MOVE_AXIS: i16 = 2_047;
pub const MAX_LOOK_PITCH: i16 = 16_384;
pub const MAX_SPELL_ID: u8 = 31;

pub const ACTION_PRIMARY: u16 = 1 << 0;
pub const ACTION_SECONDARY: u16 = 1 << 1;
pub const ACTION_CAST: u16 = 1 << 2;
pub const ACTION_DASH: u16 = 1 << 3;
pub const ACTION_JUMP: u16 = 1 << 4;
pub const ACTION_INTERACT: u16 = 1 << 5;
pub const ACTION_EXTRACT: u16 = 1 << 6;
pub const ALLOWED_ACTION_FLAGS: u16 = ACTION_PRIMARY
    | ACTION_SECONDARY
    | ACTION_CAST
    | ACTION_DASH
    | ACTION_JUMP
    | ACTION_INTERACT
    | ACTION_EXTRACT;

/// A deterministic, compact input sample for one authoritative tick.
///
/// Movement axes use signed 12-bit precision in an `i16`. Yaw spans the full
/// `u16` circle. Pitch is clamped to half that range. The constructor prevents
/// invalid values from entering normal application code; decoding validates
/// untrusted values again. Sequence zero is reserved for "no input
/// acknowledged" in snapshot messages, and wraparound therefore advances from
/// `u32::MAX` to one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerCommand {
    target_tick: u64,
    sequence: u32,
    move_x: i16,
    move_y: i16,
    look_yaw: u16,
    look_pitch: i16,
    action_flags: u16,
    requested_spell: Option<u8>,
}

impl PlayerCommand {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        target_tick: u64,
        sequence: u32,
        move_x: i16,
        move_y: i16,
        look_yaw: u16,
        look_pitch: i16,
        action_flags: u16,
        requested_spell: Option<u8>,
    ) -> Result<Self, ValidationError> {
        let command = Self {
            target_tick,
            sequence,
            move_x,
            move_y,
            look_yaw,
            look_pitch,
            action_flags,
            requested_spell,
        };
        command.validate()?;
        Ok(command)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.sequence == 0 {
            return Err(ValidationError::ZeroCommandSequence);
        }
        validate_axis("move_x", self.move_x)?;
        validate_axis("move_y", self.move_y)?;
        if !(-MAX_LOOK_PITCH..=MAX_LOOK_PITCH).contains(&self.look_pitch) {
            return Err(ValidationError::LookPitchOutOfRange(self.look_pitch));
        }
        let unsupported = self.action_flags & !ALLOWED_ACTION_FLAGS;
        if unsupported != 0 {
            return Err(ValidationError::UnsupportedActionFlags(unsupported));
        }
        if let Some(spell) = self.requested_spell {
            if spell > MAX_SPELL_ID {
                return Err(ValidationError::SpellOutOfRange(spell));
            }
        }
        Ok(())
    }

    pub const fn target_tick(&self) -> u64 {
        self.target_tick
    }

    pub const fn sequence(&self) -> u32 {
        self.sequence
    }

    pub const fn move_x(&self) -> i16 {
        self.move_x
    }

    pub const fn move_y(&self) -> i16 {
        self.move_y
    }

    pub const fn look_yaw(&self) -> u16 {
        self.look_yaw
    }

    pub const fn look_pitch(&self) -> i16 {
        self.look_pitch
    }

    pub const fn action_flags(&self) -> u16 {
        self.action_flags
    }

    pub const fn requested_spell(&self) -> Option<u8> {
        self.requested_spell
    }

    pub(crate) fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.u64(self.target_tick);
        writer.u32(self.sequence);
        writer.i16(self.move_x);
        writer.i16(self.move_y);
        writer.u16(self.look_yaw);
        writer.i16(self.look_pitch);
        writer.u16(self.action_flags);
        writer.u8(self.requested_spell.unwrap_or(u8::MAX));
        Ok(())
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let target_tick = reader.u64()?;
        let sequence = reader.u32()?;
        let move_x = reader.i16()?;
        let move_y = reader.i16()?;
        let look_yaw = reader.u16()?;
        let look_pitch = reader.i16()?;
        let action_flags = reader.u16()?;
        let requested_spell = match reader.u8()? {
            u8::MAX => None,
            spell => Some(spell),
        };
        Ok(Self::new(
            target_tick,
            sequence,
            move_x,
            move_y,
            look_yaw,
            look_pitch,
            action_flags,
            requested_spell,
        )?)
    }
}

fn validate_axis(axis: &'static str, value: i16) -> Result<(), ValidationError> {
    if (-MAX_MOVE_AXIS..=MAX_MOVE_AXIS).contains(&value) {
        Ok(())
    } else {
        Err(ValidationError::MoveAxisOutOfRange { axis, value })
    }
}

/// One client's current command plus up to two preceding commands.
///
/// The redundancy allows a receiver to recover isolated datagram loss without
/// waiting for a reliable retransmission. Commands are always newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputBatch {
    pub commands: Vec<PlayerCommand>,
}

impl InputBatch {
    pub fn new(commands: Vec<PlayerCommand>) -> Result<Self, ValidationError> {
        let batch = Self { commands };
        batch.validate()?;
        Ok(batch)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.commands.is_empty() {
            return Err(ValidationError::EmptyCommandBatch);
        }
        if self.commands.len() > COMMAND_REDUNDANCY {
            return Err(ValidationError::TooManyCommands(self.commands.len()));
        }
        for command in &self.commands {
            command.validate()?;
        }
        for (index, newer) in self.commands.iter().enumerate() {
            for older in self.commands.iter().skip(index + 1) {
                if newer.sequence == older.sequence {
                    return Err(ValidationError::DuplicateCommandSequence(newer.sequence));
                }
                if newer.target_tick == older.target_tick {
                    return Err(ValidationError::DuplicateCommandTick(newer.target_tick));
                }
            }
        }
        for pair in self.commands.windows(2) {
            if !sequence_is_newer(pair[0].sequence, pair[1].sequence)
                || pair[0].target_tick <= pair[1].target_tick
            {
                return Err(ValidationError::CommandsNotNewestFirst);
            }
        }
        Ok(())
    }

    pub fn newest(&self) -> &PlayerCommand {
        &self.commands[0]
    }

    pub(crate) fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.u8(
            u8::try_from(self.commands.len())
                .map_err(|_| ProtocolError::NumericOverflow("input command count"))?,
        );
        for command in &self.commands {
            command.encode(writer)?;
        }
        Ok(())
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let count = reader.u8()? as usize;
        if count == 0 {
            return Err(ValidationError::EmptyCommandBatch.into());
        }
        if count > COMMAND_REDUNDANCY {
            return Err(ValidationError::TooManyCommands(count).into());
        }
        let mut commands = Vec::with_capacity(count);
        for _ in 0..count {
            commands.push(PlayerCommand::decode(reader)?);
        }
        Ok(Self::new(commands)?)
    }
}

fn sequence_is_newer(received: u32, previous: u32) -> bool {
    let distance = received.wrapping_sub(previous);
    distance != 0 && distance < (1_u32 << 31)
}

/// Authoritative commands selected for one tick, indexed by stable player ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSet {
    tick: u64,
    slots: [Option<PlayerCommand>; MAX_PLAYERS],
    len: usize,
}

impl CommandSet {
    pub const fn new(tick: u64) -> Self {
        Self {
            tick,
            slots: [None; MAX_PLAYERS],
            len: 0,
        }
    }

    pub const fn tick(&self) -> u64 {
        self.tick
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn insert(
        &mut self,
        player_id: PlayerId,
        command: PlayerCommand,
    ) -> Result<(), ValidationError> {
        command.validate()?;
        if command.target_tick != self.tick {
            return Err(ValidationError::CommandTickMismatch {
                expected: self.tick,
                actual: command.target_tick,
            });
        }
        let slot = &mut self.slots[player_id.index()];
        if slot.is_some() {
            return Err(ValidationError::DuplicatePlayer(player_id.get()));
        }
        *slot = Some(command);
        self.len += 1;
        Ok(())
    }

    pub fn get(&self, player_id: PlayerId) -> Option<&PlayerCommand> {
        self.slots[player_id.index()].as_ref()
    }

    pub fn remove(&mut self, player_id: PlayerId) -> Option<PlayerCommand> {
        let value = self.slots[player_id.index()].take();
        if value.is_some() {
            self.len -= 1;
        }
        value
    }

    /// Iterates in ascending player-ID order for deterministic resolution.
    pub fn iter(&self) -> impl Iterator<Item = (PlayerId, &PlayerCommand)> {
        self.slots.iter().enumerate().filter_map(|(index, command)| {
            command
                .as_ref()
                .and_then(|command| PlayerId::new(index as u16).ok().map(|id| (id, command)))
        })
    }
}
