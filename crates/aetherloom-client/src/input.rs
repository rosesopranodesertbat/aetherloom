use alloc::vec::Vec;

use aetherloom_protocol::{
    ACTION_CAST, ACTION_DASH, ACTION_EXTRACT, ACTION_INTERACT, ACTION_JUMP, ACTION_PRIMARY,
    ACTION_SECONDARY, MAX_MOVE_AXIS,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDeviceMode {
    KeyboardMouse,
    ControllerOnly,
    Mixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyboardMouseInput {
    pub move_left: bool,
    pub move_right: bool,
    pub move_forward: bool,
    pub move_backward: bool,
    pub move_up: bool,
    pub move_down: bool,
    pub look_yaw_delta: i16,
    pub look_pitch_delta: i16,
    pub primary: bool,
    pub secondary: bool,
    pub cast: bool,
    pub dash: bool,
    pub jump: bool,
    pub interact: bool,
    pub extract: bool,
    pub requested_spell: Option<u8>,
    pub navigate_up: bool,
    pub navigate_down: bool,
    pub navigate_left: bool,
    pub navigate_right: bool,
    pub accept: bool,
    pub cancel: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControllerButtons(u16);

impl ControllerButtons {
    pub const PRIMARY: Self = Self(1 << 0);
    pub const SECONDARY: Self = Self(1 << 1);
    pub const CAST: Self = Self(1 << 2);
    pub const DASH: Self = Self(1 << 3);
    pub const JUMP: Self = Self(1 << 4);
    pub const INTERACT: Self = Self(1 << 5);
    pub const EXTRACT: Self = Self(1 << 6);
    pub const DPAD_UP: Self = Self(1 << 7);
    pub const DPAD_DOWN: Self = Self(1 << 8);
    pub const DPAD_LEFT: Self = Self(1 << 9);
    pub const DPAD_RIGHT: Self = Self(1 << 10);
    pub const ACCEPT: Self = Self(1 << 11);
    pub const CANCEL: Self = Self(1 << 12);
    const KNOWN_BITS: u16 = Self::PRIMARY.0
        | Self::SECONDARY.0
        | Self::CAST.0
        | Self::DASH.0
        | Self::JUMP.0
        | Self::INTERACT.0
        | Self::EXTRACT.0
        | Self::DPAD_UP.0
        | Self::DPAD_DOWN.0
        | Self::DPAD_LEFT.0
        | Self::DPAD_RIGHT.0
        | Self::ACCEPT.0
        | Self::CANCEL.0;

    pub const fn from_bits_truncate(bits: u16) -> Self {
        Self(bits & Self::KNOWN_BITS)
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, button: Self) -> bool {
        self.0 & button.0 != 0
    }

    pub const fn bits(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControllerInput {
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    /// Continuous flight lift axis in `-1.0..=1.0`.
    pub vertical_axis: f32,
    pub buttons: ControllerButtons,
    pub requested_spell: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MappedInput {
    pub move_x: i16,
    pub move_y: i16,
    pub move_vertical: i16,
    /// One-shot pointer/mouse delta accumulated until the next prediction
    /// tick. It is applied once even if one render frame produces many ticks.
    pub look_yaw_delta: i16,
    pub look_pitch_delta: i16,
    /// Held-stick angular velocity, quantized as protocol angle units per
    /// authoritative tick. The orchestrator applies it once for every 128 Hz
    /// prediction tick, never once per rendered frame.
    pub controller_yaw_per_tick: i16,
    pub controller_pitch_per_tick: i16,
    pub action_flags: u16,
    pub requested_spell: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationEvent {
    Up,
    Down,
    Left,
    Right,
    Accept,
    Cancel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavigationEvents {
    pub events: Vec<NavigationEvent>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NavigationState {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    accept: bool,
    cancel: bool,
}

/// Maps platform input into protocol-ready, quantized commands and provides
/// edge-triggered UI navigation that works without a pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct InputMapper {
    mode: InputDeviceMode,
    movement_deadzone: f32,
    navigation_threshold: f32,
    controller_look_units_per_tick: i16,
    previous_navigation: NavigationState,
}

impl InputMapper {
    pub fn new(mode: InputDeviceMode, movement_deadzone: f32) -> Self {
        Self {
            mode,
            movement_deadzone: sanitize_deadzone(movement_deadzone),
            navigation_threshold: 0.65,
            controller_look_units_per_tick: 384,
            previous_navigation: NavigationState::default(),
        }
    }

    pub const fn mode(&self) -> InputDeviceMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: InputDeviceMode) {
        self.mode = mode;
        self.previous_navigation = NavigationState::default();
    }

    pub fn map(
        &mut self,
        keyboard_mouse: Option<KeyboardMouseInput>,
        controller: Option<ControllerInput>,
    ) -> (MappedInput, NavigationEvents) {
        let keyboard_enabled = self.mode != InputDeviceMode::ControllerOnly;
        let controller_enabled = self.mode != InputDeviceMode::KeyboardMouse;
        let keyboard = keyboard_mouse.filter(|_| keyboard_enabled);
        let controller = controller.filter(|_| controller_enabled);

        let keyboard_move = keyboard.map(keyboard_move).unwrap_or([0.0, 0.0]);
        let controller_move = controller
            .map(|input| {
                [
                    apply_deadzone(input.left_stick[0], self.movement_deadzone),
                    apply_deadzone(-input.left_stick[1], self.movement_deadzone),
                ]
            })
            .unwrap_or([0.0, 0.0]);
        let keyboard_vertical = keyboard.map(keyboard_vertical).unwrap_or(0.0);
        let controller_vertical = controller
            .map(|input| apply_deadzone(input.vertical_axis, self.movement_deadzone))
            .unwrap_or(0.0);
        let movement = match self.mode {
            InputDeviceMode::KeyboardMouse => keyboard_move,
            InputDeviceMode::ControllerOnly => controller_move,
            InputDeviceMode::Mixed => [
                choose_greater_magnitude(keyboard_move[0], controller_move[0]),
                choose_greater_magnitude(keyboard_move[1], controller_move[1]),
            ],
        };
        let vertical = match self.mode {
            InputDeviceMode::KeyboardMouse => keyboard_vertical,
            InputDeviceMode::ControllerOnly => controller_vertical,
            InputDeviceMode::Mixed => {
                choose_greater_magnitude(keyboard_vertical, controller_vertical)
            }
        };

        let keyboard_look = keyboard
            .map(|input| [input.look_yaw_delta, input.look_pitch_delta])
            .unwrap_or([0, 0]);
        let controller_look_per_tick = controller
            .map(|input| {
                [
                    quantize_look_axis(
                        input.right_stick[0],
                        self.movement_deadzone,
                        self.controller_look_units_per_tick,
                    ),
                    quantize_look_axis(
                        -input.right_stick[1],
                        self.movement_deadzone,
                        self.controller_look_units_per_tick,
                    ),
                ]
            })
            .unwrap_or([0, 0]);

        let mut flags = 0;
        let mut requested_spell = None;
        if let Some(input) = keyboard {
            flags |= keyboard_action_flags(input);
            requested_spell = input.requested_spell;
        }
        if let Some(input) = controller {
            flags |= controller_action_flags(input.buttons);
            if input.requested_spell.is_some() {
                requested_spell = input.requested_spell;
            }
        }

        let current_navigation =
            navigation_state(keyboard, controller, self.navigation_threshold);
        let navigation = navigation_edges(self.previous_navigation, current_navigation);
        self.previous_navigation = current_navigation;

        (
            MappedInput {
                move_x: quantize_movement(movement[0]),
                move_y: quantize_movement(movement[1]),
                move_vertical: quantize_movement(vertical),
                look_yaw_delta: keyboard_look[0],
                look_pitch_delta: keyboard_look[1],
                controller_yaw_per_tick: controller_look_per_tick[0],
                controller_pitch_per_tick: controller_look_per_tick[1],
                action_flags: flags,
                requested_spell,
            },
            navigation,
        )
    }
}

fn sanitize_deadzone(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 0.95)
    } else {
        0.2
    }
}

fn apply_deadzone(value: f32, deadzone: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    let clamped = value.clamp(-1.0, 1.0);
    let magnitude = clamped.abs();
    if magnitude <= deadzone {
        0.0
    } else {
        let scaled = (magnitude - deadzone) / (1.0 - deadzone);
        if clamped < 0.0 {
            -scaled
        } else {
            scaled
        }
    }
}

fn choose_greater_magnitude(left: f32, right: f32) -> f32 {
    if left.abs() >= right.abs() {
        left
    } else {
        right
    }
}

fn quantize_movement(value: f32) -> i16 {
    let scaled = value.clamp(-1.0, 1.0) * MAX_MOVE_AXIS as f32;
    scaled as i16
}

fn quantize_look_axis(value: f32, deadzone: f32, maximum: i16) -> i16 {
    (apply_deadzone(value, deadzone) * maximum as f32) as i16
}

fn keyboard_move(input: KeyboardMouseInput) -> [f32; 2] {
    let horizontal = i8::from(input.move_right) - i8::from(input.move_left);
    let vertical = i8::from(input.move_forward) - i8::from(input.move_backward);
    [horizontal as f32, vertical as f32]
}

fn keyboard_vertical(input: KeyboardMouseInput) -> f32 {
    let vertical = i8::from(input.move_up) - i8::from(input.move_down);
    vertical as f32
}

fn keyboard_action_flags(input: KeyboardMouseInput) -> u16 {
    bool_flag(input.primary, ACTION_PRIMARY)
        | bool_flag(input.secondary, ACTION_SECONDARY)
        | bool_flag(input.cast, ACTION_CAST)
        | bool_flag(input.dash, ACTION_DASH)
        | bool_flag(input.jump, ACTION_JUMP)
        | bool_flag(input.interact, ACTION_INTERACT)
        | bool_flag(input.extract, ACTION_EXTRACT)
}

fn controller_action_flags(buttons: ControllerButtons) -> u16 {
    bool_flag(buttons.contains(ControllerButtons::PRIMARY), ACTION_PRIMARY)
        | bool_flag(
            buttons.contains(ControllerButtons::SECONDARY),
            ACTION_SECONDARY,
        )
        | bool_flag(buttons.contains(ControllerButtons::CAST), ACTION_CAST)
        | bool_flag(buttons.contains(ControllerButtons::DASH), ACTION_DASH)
        | bool_flag(buttons.contains(ControllerButtons::JUMP), ACTION_JUMP)
        | bool_flag(
            buttons.contains(ControllerButtons::INTERACT),
            ACTION_INTERACT,
        )
        | bool_flag(buttons.contains(ControllerButtons::EXTRACT), ACTION_EXTRACT)
}

const fn bool_flag(enabled: bool, flag: u16) -> u16 {
    if enabled {
        flag
    } else {
        0
    }
}

fn navigation_state(
    keyboard: Option<KeyboardMouseInput>,
    controller: Option<ControllerInput>,
    threshold: f32,
) -> NavigationState {
    let mut state = NavigationState::default();
    if let Some(input) = keyboard {
        state.up |= input.navigate_up;
        state.down |= input.navigate_down;
        state.left |= input.navigate_left;
        state.right |= input.navigate_right;
        state.accept |= input.accept;
        state.cancel |= input.cancel;
    }
    if let Some(input) = controller {
        state.up |= input.buttons.contains(ControllerButtons::DPAD_UP)
            || input.left_stick[1] <= -threshold;
        state.down |= input.buttons.contains(ControllerButtons::DPAD_DOWN)
            || input.left_stick[1] >= threshold;
        state.left |= input.buttons.contains(ControllerButtons::DPAD_LEFT)
            || input.left_stick[0] <= -threshold;
        state.right |= input.buttons.contains(ControllerButtons::DPAD_RIGHT)
            || input.left_stick[0] >= threshold;
        state.accept |= input.buttons.contains(ControllerButtons::ACCEPT);
        state.cancel |= input.buttons.contains(ControllerButtons::CANCEL);
    }
    state
}

fn navigation_edges(
    previous: NavigationState,
    current: NavigationState,
) -> NavigationEvents {
    let mut events = Vec::new();
    if current.up && !previous.up {
        events.push(NavigationEvent::Up);
    }
    if current.down && !previous.down {
        events.push(NavigationEvent::Down);
    }
    if current.left && !previous.left {
        events.push(NavigationEvent::Left);
    }
    if current.right && !previous.right {
        events.push(NavigationEvent::Right);
    }
    if current.accept && !previous.accept {
        events.push(NavigationEvent::Accept);
    }
    if current.cancel && !previous.cancel {
        events.push(NavigationEvent::Cancel);
    }
    NavigationEvents { events }
}
