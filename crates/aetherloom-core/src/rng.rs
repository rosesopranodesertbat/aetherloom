/// A small, completely specified integer PRNG.
///
/// SplitMix64 is used because its behavior is identical on every Rust target
/// and needs no floating-point arithmetic. Streams are derived with distinct
/// tags so adding a cosmetic draw can never perturb gameplay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub(crate) const fn from_state(state: u64) -> Self {
        Self { state }
    }

    pub(crate) fn seeded(seed: u64, stream_tag: u64) -> Self {
        let mut rng = Self {
            state: seed ^ stream_tag,
        };
        // Avoid closely related initial outputs for similarly numbered seeds.
        let _ = rng.next_u64();
        rng
    }

    pub(crate) const fn state(self) -> u64 {
        self.state
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    pub(crate) fn range_i32(&mut self, minimum: i32, maximum_inclusive: i32) -> i32 {
        debug_assert!(minimum <= maximum_inclusive);
        let width = (maximum_inclusive as i64 - minimum as i64 + 1) as u64;
        minimum + (self.next_u64() % width) as i32
    }
}

pub(crate) const GENERATION_STREAM: u64 = 0x4745_4E45_5241_5445;
pub(crate) const AI_STREAM: u64 = 0x4149_5F53_5452_4541;
pub(crate) const COMBAT_STREAM: u64 = 0x434F_4D42_4154_5F52;
pub(crate) const ENVIRONMENT_STREAM: u64 = 0x454E_5649_524F_4E4D;

