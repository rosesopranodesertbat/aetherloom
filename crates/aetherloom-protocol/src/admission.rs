use crate::ValidationError;

pub const MAX_REGION_ID_BYTES: usize = 128;

/// Matchmaking input pool pinned into a signed join ticket.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum InputPool {
    Browser = 1,
    MouseKeyboard = 2,
    Controller = 3,
    Mixed = 4,
}

impl InputPool {
    pub fn from_name(value: &str) -> Result<Self, ValidationError> {
        match value {
            "browser" => Ok(Self::Browser),
            "mouse-keyboard" => Ok(Self::MouseKeyboard),
            "controller" => Ok(Self::Controller),
            "mixed" => Ok(Self::Mixed),
            _ => Err(ValidationError::InvalidInputPool),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::MouseKeyboard => "mouse-keyboard",
            Self::Controller => "controller",
            Self::Mixed => "mixed",
        }
    }
}

/// Bounded canonical region identifier carried through admission.
///
/// Keeping the bytes inline makes verified connection claims fixed-size and
/// copyable while preserving the exact signed value for equality checks.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RegionId {
    bytes: [u8; MAX_REGION_ID_BYTES],
    length: u8,
}

impl RegionId {
    pub fn new(value: &str) -> Result<Self, ValidationError> {
        let source = value.as_bytes();
        if source.is_empty()
            || source.len() > MAX_REGION_ID_BYTES
            || !source.iter().enumerate().all(|(index, byte)| {
                byte.is_ascii_alphanumeric()
                    || (index > 0 && matches!(byte, b'_' | b'.' | b':' | b'-'))
            })
        {
            return Err(ValidationError::InvalidRegion);
        }

        let mut bytes = [0_u8; MAX_REGION_ID_BYTES];
        bytes[..source.len()].copy_from_slice(source);
        Ok(Self {
            bytes,
            length: source.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..usize::from(self.length)])
            .expect("RegionId construction accepts ASCII only")
    }
}

impl core::fmt::Debug for RegionId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_tuple("RegionId")
            .field(&self.as_str())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_and_input_pool_require_canonical_contract_values() {
        let region = RegionId::new("weur-1").expect("region");
        assert_eq!(region.as_str(), "weur-1");
        assert_eq!(RegionId::new("-weur"), Err(ValidationError::InvalidRegion));
        assert_eq!(
            InputPool::from_name("controller"),
            Ok(InputPool::Controller)
        );
        assert_eq!(
            InputPool::from_name("keyboard"),
            Err(ValidationError::InvalidInputPool)
        );
    }
}
