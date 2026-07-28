use alloc::vec::Vec;

use crate::RendererBackend;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdentityId([u8; 16]);

impl IdentityId {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntitlementId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AchievementId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveNamespace {
    /// Local campaign data. It is never uploaded as progression or ratings.
    OfflineLocal,
    /// Account-bound data. The identity is part of the storage keyspace.
    OnlineAccount(IdentityId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformError {
    Unavailable,
    PermissionDenied,
    NotEntitled,
    StorageFull,
    CorruptData,
    IdentityChanged,
}

/// Browser, desktop, and console SDK adapters implement this interface. SDK
/// handles and vendor types do not cross into the shared client.
pub trait ClientPlatformServices {
    fn monotonic_time_nanoseconds(&self) -> u64;

    fn current_identity(&self) -> Result<Option<IdentityId>, PlatformError>;

    fn has_entitlement(&self, entitlement: EntitlementId) -> Result<bool, PlatformError>;

    fn load_save(
        &self,
        namespace: SaveNamespace,
        key: &str,
    ) -> Result<Option<Vec<u8>>, PlatformError>;

    fn store_save(
        &mut self,
        namespace: SaveNamespace,
        key: &str,
        bytes: &[u8],
    ) -> Result<(), PlatformError>;

    fn unlock_achievement(
        &mut self,
        identity: IdentityId,
        achievement: AchievementId,
    ) -> Result<(), PlatformError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Offline,
    Online,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    Ready,
    Lost,
    Recovering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageState {
    Ready,
    Degraded(PlatformError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnlineSessionState {
    NotApplicable,
    Active(IdentityId),
    InvalidatedIdentityChange {
        previous: IdentityId,
        current: Option<IdentityId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeOutcome {
    Resumed,
    IdentityChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleError {
    Platform(PlatformError),
    Suspended,
    OnlineSessionInvalidated,
    InvalidSaveKey,
}

/// Owns client lifecycle state and derives storage namespaces from the active
/// session, preventing an online save from being opened as an offline save (or
/// vice versa).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientLifecycle {
    session_kind: SessionKind,
    online_state: OnlineSessionState,
    suspended: bool,
    device_state: DeviceState,
    storage_state: StorageState,
}

impl ClientLifecycle {
    pub const fn offline() -> Self {
        Self {
            session_kind: SessionKind::Offline,
            online_state: OnlineSessionState::NotApplicable,
            suspended: false,
            device_state: DeviceState::Ready,
            storage_state: StorageState::Ready,
        }
    }

    pub fn online<S: ClientPlatformServices>(services: &S) -> Result<Self, LifecycleError> {
        let identity = services
            .current_identity()
            .map_err(LifecycleError::Platform)?
            .ok_or(LifecycleError::Platform(PlatformError::Unavailable))?;
        Ok(Self {
            session_kind: SessionKind::Online,
            online_state: OnlineSessionState::Active(identity),
            suspended: false,
            device_state: DeviceState::Ready,
            storage_state: StorageState::Ready,
        })
    }

    pub const fn session_kind(&self) -> SessionKind {
        self.session_kind
    }

    pub const fn online_state(&self) -> OnlineSessionState {
        self.online_state
    }

    pub const fn is_suspended(&self) -> bool {
        self.suspended
    }

    pub const fn device_state(&self) -> DeviceState {
        self.device_state
    }

    pub const fn storage_state(&self) -> StorageState {
        self.storage_state
    }

    pub const fn can_advance(&self) -> bool {
        !self.suspended
            && !matches!(
                self.online_state,
                OnlineSessionState::InvalidatedIdentityChange { .. }
            )
    }

    pub const fn can_render(&self) -> bool {
        self.can_advance() && matches!(self.device_state, DeviceState::Ready)
    }

    pub fn on_suspend(&mut self) {
        self.suspended = true;
    }

    /// Revalidates the account before permitting an online session to resume.
    /// A different or signed-out identity permanently invalidates that session;
    /// callers must discard transient credentials and create a new lifecycle.
    pub fn on_resume<S: ClientPlatformServices>(
        &mut self,
        services: &S,
    ) -> Result<ResumeOutcome, LifecycleError> {
        match self.online_state {
            OnlineSessionState::Active(previous) => {
                let current = services
                    .current_identity()
                    .map_err(LifecycleError::Platform)?;
                if current != Some(previous) {
                    self.online_state = OnlineSessionState::InvalidatedIdentityChange {
                        previous,
                        current,
                    };
                    self.suspended = false;
                    return Ok(ResumeOutcome::IdentityChanged);
                }
            }
            OnlineSessionState::InvalidatedIdentityChange { .. } => {
                self.suspended = false;
                return Err(LifecycleError::OnlineSessionInvalidated);
            }
            OnlineSessionState::NotApplicable => {}
        }
        self.suspended = false;
        Ok(ResumeOutcome::Resumed)
    }

    pub fn on_device_lost(&mut self) {
        self.device_state = DeviceState::Lost;
    }

    pub fn recover_device<R: RendererBackend>(
        &mut self,
        renderer: &mut R,
    ) -> Result<(), R::Error> {
        self.device_state = DeviceState::Recovering;
        match renderer.recover_device() {
            Ok(()) => {
                self.device_state = DeviceState::Ready;
                Ok(())
            }
            Err(error) => {
                self.device_state = DeviceState::Lost;
                Err(error)
            }
        }
    }

    pub fn load_save<S: ClientPlatformServices>(
        &mut self,
        services: &S,
        key: &str,
    ) -> Result<Option<Vec<u8>>, LifecycleError> {
        validate_save_key(key)?;
        let namespace = self.validated_namespace(services)?;
        match services.load_save(namespace, key) {
            Ok(bytes) => {
                self.storage_state = StorageState::Ready;
                Ok(bytes)
            }
            Err(error) => {
                self.storage_state = StorageState::Degraded(error);
                Err(LifecycleError::Platform(error))
            }
        }
    }

    pub fn store_save<S: ClientPlatformServices>(
        &mut self,
        services: &mut S,
        key: &str,
        bytes: &[u8],
    ) -> Result<(), LifecycleError> {
        validate_save_key(key)?;
        let namespace = self.validated_namespace(services)?;
        match services.store_save(namespace, key, bytes) {
            Ok(()) => {
                self.storage_state = StorageState::Ready;
                Ok(())
            }
            Err(error) => {
                self.storage_state = StorageState::Degraded(error);
                Err(LifecycleError::Platform(error))
            }
        }
    }

    pub fn unlock_achievement<S: ClientPlatformServices>(
        &mut self,
        services: &mut S,
        achievement: AchievementId,
    ) -> Result<(), LifecycleError> {
        if self.suspended {
            return Err(LifecycleError::Suspended);
        }
        let identity = match self.online_state {
            OnlineSessionState::Active(identity) => {
                let current = services
                    .current_identity()
                    .map_err(LifecycleError::Platform)?;
                if current != Some(identity) {
                    self.online_state = OnlineSessionState::InvalidatedIdentityChange {
                        previous: identity,
                        current,
                    };
                    return Err(LifecycleError::OnlineSessionInvalidated);
                }
                identity
            }
            OnlineSessionState::NotApplicable => {
                return Err(LifecycleError::Platform(PlatformError::Unavailable))
            }
            OnlineSessionState::InvalidatedIdentityChange { .. } => {
                return Err(LifecycleError::OnlineSessionInvalidated)
            }
        };
        services
            .unlock_achievement(identity, achievement)
            .map_err(LifecycleError::Platform)
    }

    pub fn require_entitlement<S: ClientPlatformServices>(
        &self,
        services: &S,
        entitlement: EntitlementId,
    ) -> Result<(), LifecycleError> {
        if services
            .has_entitlement(entitlement)
            .map_err(LifecycleError::Platform)?
        {
            Ok(())
        } else {
            Err(LifecycleError::Platform(PlatformError::NotEntitled))
        }
    }

    fn validated_namespace<S: ClientPlatformServices>(
        &mut self,
        services: &S,
    ) -> Result<SaveNamespace, LifecycleError> {
        if self.suspended {
            return Err(LifecycleError::Suspended);
        }
        match self.online_state {
            OnlineSessionState::NotApplicable => Ok(SaveNamespace::OfflineLocal),
            OnlineSessionState::Active(identity) => {
                let current = services
                    .current_identity()
                    .map_err(LifecycleError::Platform)?;
                if current == Some(identity) {
                    Ok(SaveNamespace::OnlineAccount(identity))
                } else {
                    self.online_state = OnlineSessionState::InvalidatedIdentityChange {
                        previous: identity,
                        current,
                    };
                    Err(LifecycleError::OnlineSessionInvalidated)
                }
            }
            OnlineSessionState::InvalidatedIdentityChange { .. } => {
                Err(LifecycleError::OnlineSessionInvalidated)
            }
        }
    }
}

fn validate_save_key(key: &str) -> Result<(), LifecycleError> {
    if key.is_empty() || key.len() > 96 || key.bytes().any(|byte| byte == 0 || byte == b'/') {
        Err(LifecycleError::InvalidSaveKey)
    } else {
        Ok(())
    }
}
