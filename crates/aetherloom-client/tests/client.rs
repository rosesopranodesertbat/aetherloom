use std::vec::Vec;

use aetherloom_client::vendor::{
    VendorApiHeader, VendorClientApiV1, AETHERLOOM_CLIENT_ABI_VERSION,
};
use aetherloom_client::{
    select_capabilities, CapabilityError, CapabilityTier, ClientFrameOrchestrator,
    ClientLifecycle, ClientPlatformServices, ClientReplica, ControllerButtons, ControllerInput,
    DeviceState, EntitlementId, HudFrame, IdentityId, InputDeviceMode, InputMapper,
    InputBatchScheduler, KeyboardMouseInput, LifecycleError, LoopbackTransport,
    MappedInput, NavigationEvent, OnlineSessionState, OrchestratorError,
    PlatformError, QuicTransport, RenderError, RendererBackend,
    RendererCapabilities, ResumeOutcome, SafeAreaInsets, SafeAreaRect, SaveNamespace,
    SelectedCapabilities, SnapshotJitterEstimator, StorageState, TextureFormat,
    TextureFormatSet, TransportMarker, Viewport, WebSocketTransport,
};
use aetherloom_core::{
    EntityKind, InterestTier, ReplicatedEntity, Snapshot, SnapshotId, SnapshotKind,
};
use aetherloom_protocol::EntityId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestRenderError {
    RecoveryFailed,
}

struct TestRenderer {
    fail_recovery: bool,
}

impl RendererBackend for TestRenderer {
    type Error = TestRenderError;

    fn capabilities(&self) -> RendererCapabilities {
        minimum_capabilities()
    }

    fn configure(&mut self, _selected: SelectedCapabilities) -> Result<(), Self::Error> {
        Ok(())
    }

    fn render(
        &mut self,
        _frame: &aetherloom_client::ClientRenderFrame,
    ) -> Result<(), RenderError<Self::Error>> {
        Ok(())
    }

    fn recover_device(&mut self) -> Result<(), Self::Error> {
        if self.fail_recovery {
            Err(TestRenderError::RecoveryFailed)
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct TestPlatform {
    identity: Option<IdentityId>,
    storage_error: Option<PlatformError>,
    stores: Vec<(SaveNamespace, std::string::String, Vec<u8>)>,
}

impl ClientPlatformServices for TestPlatform {
    fn monotonic_time_nanoseconds(&self) -> u64 {
        123
    }

    fn current_identity(&self) -> Result<Option<IdentityId>, PlatformError> {
        Ok(self.identity)
    }

    fn has_entitlement(&self, _entitlement: EntitlementId) -> Result<bool, PlatformError> {
        Ok(true)
    }

    fn load_save(
        &self,
        _namespace: SaveNamespace,
        _key: &str,
    ) -> Result<Option<Vec<u8>>, PlatformError> {
        if let Some(error) = self.storage_error {
            Err(error)
        } else {
            Ok(None)
        }
    }

    fn store_save(
        &mut self,
        namespace: SaveNamespace,
        key: &str,
        bytes: &[u8],
    ) -> Result<(), PlatformError> {
        if let Some(error) = self.storage_error {
            return Err(error);
        }
        self.stores
            .push((namespace, key.to_owned(), bytes.to_vec()));
        Ok(())
    }

    fn unlock_achievement(
        &mut self,
        _identity: IdentityId,
        _achievement: aetherloom_client::AchievementId,
    ) -> Result<(), PlatformError> {
        Ok(())
    }
}

fn identity(byte: u8) -> IdentityId {
    IdentityId::new([byte; 16])
}

fn minimum_capabilities() -> RendererCapabilities {
    RendererCapabilities {
        supports_hdr_output: false,
        supports_bloom: false,
        texture_formats: TextureFormatSet::RGBA8_SRGB,
        max_texture_dimension_2d: 4_096,
        max_shader_storage_buffers: 4,
        max_storage_buffer_bytes: 16 * 1024 * 1024,
        dedicated_video_memory_bytes: 256 * 1024 * 1024,
        unified_memory_budget_bytes: 0,
    }
}

#[test]
fn controller_only_mapping_ignores_keyboard_and_supports_edge_navigation() {
    let mut mapper = InputMapper::new(InputDeviceMode::ControllerOnly, 0.2);
    let keyboard = KeyboardMouseInput {
        move_left: true,
        accept: true,
        ..KeyboardMouseInput::default()
    };
    let controller = ControllerInput {
        left_stick: [1.0, 0.0],
        buttons: ControllerButtons::ACCEPT,
        ..ControllerInput::default()
    };

    let (mapped, first_navigation) = mapper.map(Some(keyboard), Some(controller));
    assert_eq!(mapped.move_x, 2_047);
    assert_eq!(
        first_navigation.events,
        vec![NavigationEvent::Right, NavigationEvent::Accept]
    );

    let (_, held_navigation) = mapper.map(Some(keyboard), Some(controller));
    assert!(held_navigation.events.is_empty());

    let (_, _) = mapper.map(None, Some(ControllerInput::default()));
    let (_, repeated_navigation) = mapper.map(None, Some(controller));
    assert_eq!(
        repeated_navigation.events,
        vec![NavigationEvent::Right, NavigationEvent::Accept]
    );
}

#[test]
fn controller_deadzone_suppresses_drift() {
    let mut mapper = InputMapper::new(InputDeviceMode::ControllerOnly, 0.25);
    let input = ControllerInput {
        left_stick: [0.2, -0.2],
        right_stick: [0.2, 0.2],
        ..ControllerInput::default()
    };
    let (mapped, navigation) = mapper.map(None, Some(input));
    assert_eq!(mapped.move_x, 0);
    assert_eq!(mapped.move_y, 0);
    assert_eq!(mapped.controller_yaw_per_tick, 0);
    assert_eq!(mapped.controller_pitch_per_tick, 0);
    assert!(navigation.events.is_empty());
}

#[test]
fn lifecycle_rechecks_identity_across_suspend_resume() {
    let mut platform = TestPlatform {
        identity: Some(identity(1)),
        ..TestPlatform::default()
    };
    let mut lifecycle = ClientLifecycle::online(&platform).unwrap();

    lifecycle.on_suspend();
    assert!(!lifecycle.can_advance());
    assert_eq!(
        lifecycle.on_resume(&platform).unwrap(),
        ResumeOutcome::Resumed
    );
    assert!(lifecycle.can_advance());

    lifecycle.on_suspend();
    platform.identity = Some(identity(2));
    assert_eq!(
        lifecycle.on_resume(&platform).unwrap(),
        ResumeOutcome::IdentityChanged
    );
    assert_eq!(
        lifecycle.online_state(),
        OnlineSessionState::InvalidatedIdentityChange {
            previous: identity(1),
            current: Some(identity(2)),
        }
    );
    assert!(!lifecycle.can_advance());
    assert_eq!(
        lifecycle.store_save(&mut platform, "profile", b"forbidden"),
        Err(LifecycleError::OnlineSessionInvalidated)
    );
    assert!(platform.stores.is_empty());
}

#[test]
fn online_storage_rejects_a_live_identity_switch() {
    let mut platform = TestPlatform {
        identity: Some(identity(3)),
        ..TestPlatform::default()
    };
    let mut lifecycle = ClientLifecycle::online(&platform).unwrap();
    platform.identity = Some(identity(4));

    assert_eq!(
        lifecycle.store_save(&mut platform, "profile", b"wrong account"),
        Err(LifecycleError::OnlineSessionInvalidated)
    );
    assert_eq!(
        lifecycle.online_state(),
        OnlineSessionState::InvalidatedIdentityChange {
            previous: identity(3),
            current: Some(identity(4)),
        }
    );
    assert!(platform.stores.is_empty());
}

#[test]
fn device_loss_requires_successful_backend_recovery() {
    let mut lifecycle = ClientLifecycle::offline();
    lifecycle.on_device_lost();
    assert_eq!(lifecycle.device_state(), DeviceState::Lost);
    assert!(!lifecycle.can_render());

    let mut renderer = TestRenderer {
        fail_recovery: true,
    };
    assert_eq!(
        lifecycle.recover_device(&mut renderer),
        Err(TestRenderError::RecoveryFailed)
    );
    assert_eq!(lifecycle.device_state(), DeviceState::Lost);

    renderer.fail_recovery = false;
    lifecycle.recover_device(&mut renderer).unwrap();
    assert_eq!(lifecycle.device_state(), DeviceState::Ready);
    assert!(lifecycle.can_render());
}

#[test]
fn storage_failure_is_visible_and_save_namespaces_cannot_alias() {
    let mut failing_platform = TestPlatform {
        storage_error: Some(PlatformError::StorageFull),
        ..TestPlatform::default()
    };
    let mut offline = ClientLifecycle::offline();
    assert_eq!(
        offline.store_save(&mut failing_platform, "campaign", b"save"),
        Err(LifecycleError::Platform(PlatformError::StorageFull))
    );
    assert_eq!(
        offline.storage_state(),
        StorageState::Degraded(PlatformError::StorageFull)
    );

    let mut platform = TestPlatform {
        identity: Some(identity(7)),
        ..TestPlatform::default()
    };
    offline
        .store_save(&mut platform, "slot-1", b"offline")
        .unwrap();
    let mut online = ClientLifecycle::online(&platform).unwrap();
    online
        .store_save(&mut platform, "slot-1", b"online")
        .unwrap();
    assert_eq!(platform.stores[0].0, SaveNamespace::OfflineLocal);
    assert_eq!(
        platform.stores[1].0,
        SaveNamespace::OnlineAccount(identity(7))
    );
}

#[test]
fn capability_fallback_is_deterministic() {
    let mut capabilities = minimum_capabilities();
    assert_eq!(
        select_capabilities(capabilities).unwrap().tier,
        CapabilityTier::Minimum
    );

    capabilities.max_texture_dimension_2d = 16_384;
    capabilities.max_shader_storage_buffers = 12;
    capabilities.max_storage_buffer_bytes = 128 * 1024 * 1024;
    capabilities.dedicated_video_memory_bytes = 2_048 * 1024 * 1024;
    capabilities.supports_bloom = true;
    capabilities.texture_formats = capabilities
        .texture_formats
        .union(TextureFormatSet::BGRA8_SRGB)
        .union(TextureFormatSet::BC7_SRGB)
        .union(TextureFormatSet::RGBA16_FLOAT);
    assert_eq!(
        select_capabilities(capabilities).unwrap().tier,
        CapabilityTier::High
    );

    capabilities.supports_hdr_output = true;
    let ultra = select_capabilities(capabilities).unwrap();
    assert_eq!(ultra.tier, CapabilityTier::Ultra);
    assert_eq!(ultra.color_format, TextureFormat::Bgra8Srgb);
    assert!(ultra.hdr_enabled);

    capabilities.texture_formats = TextureFormatSet::BC7_SRGB;
    assert_eq!(
        select_capabilities(capabilities),
        Err(CapabilityError::MissingSrgbColorFormat)
    );
}

#[test]
fn prediction_cadence_is_128_hz_at_common_render_rates() {
    for render_hz in [30_u64, 60, 120, 144, 240] {
        let viewer = aetherloom_client::PlayerId::new(0).unwrap();
        let mut replica = ClientReplica::new(viewer);
        let mut orchestrator = ClientFrameOrchestrator::new(10, 100);
        let mut previous_timestamp = 0;
        let mut commands = Vec::new();

        for frame in 1..=render_hz {
            let timestamp = frame * 1_000_000_000 / render_hz;
            let elapsed = timestamp - previous_timestamp;
            previous_timestamp = timestamp;
            let cadence = orchestrator
                .advance_frame(elapsed, MappedInput::default(), &mut replica)
                .unwrap();
            commands.extend(cadence.commands);
        }

        assert_eq!(commands.len(), 128, "render rate {render_hz}");
        assert_eq!(commands.first().unwrap().target_tick(), 10);
        assert_eq!(commands.last().unwrap().target_tick(), 137);
        assert_eq!(orchestrator.next_prediction_tick(), 138);
        assert_eq!(orchestrator.next_sequence(), 228);
    }
}

#[test]
fn command_sequences_normalize_and_skip_reserved_zero() {
    let viewer = aetherloom_client::PlayerId::new(0).unwrap();
    let mut replica = ClientReplica::new(viewer);
    let mut normalized = ClientFrameOrchestrator::new(0, 0);
    assert_eq!(normalized.next_sequence(), 1);
    normalized.rebase(8, 0);
    assert_eq!(normalized.next_sequence(), 1);

    let mut wrapping = ClientFrameOrchestrator::new(10, u32::MAX);
    let cadence = wrapping
        .advance_frame(15_625_000, MappedInput::default(), &mut replica)
        .unwrap();
    let sequences: Vec<_> = cadence
        .commands
        .iter()
        .map(|command| command.sequence())
        .collect();
    assert_eq!(sequences, vec![u32::MAX, 1]);
    assert_eq!(wrapping.next_sequence(), 2);
}

#[test]
fn held_controller_look_is_identical_at_every_render_rate() {
    let mut reference = None;
    for render_hz in [30_u64, 60, 240] {
        let viewer = aetherloom_client::PlayerId::new(0).unwrap();
        let mut replica = ClientReplica::new(viewer);
        let mut orchestrator = ClientFrameOrchestrator::new(0, 1);
        let mut mapper = InputMapper::new(InputDeviceMode::ControllerOnly, 0.2);
        let controller = ControllerInput {
            right_stick: [0.75, 0.0],
            ..ControllerInput::default()
        };
        let mut yaw_by_tick = Vec::new();
        let mut previous_timestamp = 0;

        for frame in 1..=render_hz {
            let timestamp = frame * 1_000_000_000 / render_hz;
            let elapsed = timestamp - previous_timestamp;
            previous_timestamp = timestamp;
            let (input, _) = mapper.map(None, Some(controller));
            let cadence = orchestrator
                .advance_frame(elapsed, input, &mut replica)
                .unwrap();
            yaw_by_tick.extend(cadence.commands.iter().map(|command| command.look_yaw()));
        }

        assert_eq!(yaw_by_tick.len(), 128);
        if let Some(expected) = &reference {
            assert_eq!(&yaw_by_tick, expected, "render rate {render_hz}");
        } else {
            reference = Some(yaw_by_tick);
        }
    }
}

#[test]
fn mouse_delta_is_applied_once_when_a_render_frame_catches_up_many_ticks() {
    let viewer = aetherloom_client::PlayerId::new(0).unwrap();
    let mut replica = ClientReplica::new(viewer);
    let mut orchestrator = ClientFrameOrchestrator::new(0, 1);
    let mut mapper = InputMapper::new(InputDeviceMode::KeyboardMouse, 0.2);
    let (input, _) = mapper.map(
        Some(KeyboardMouseInput {
            look_yaw_delta: 100,
            ..KeyboardMouseInput::default()
        }),
        None,
    );

    let cadence = orchestrator
        .advance_frame(31_250_000, input, &mut replica)
        .unwrap();
    let yaw: Vec<_> = cadence
        .commands
        .iter()
        .map(|command| command.look_yaw())
        .collect();
    assert_eq!(yaw, vec![100, 100, 100, 100]);
}

#[test]
fn local_prediction_updates_before_the_next_render_frame() {
    let viewer = aetherloom_client::PlayerId::new(0).unwrap();
    let entity_id = EntityId::new(0, 1).unwrap();
    let mut replica = ClientReplica::new(viewer);
    replica
        .apply(Snapshot {
            snapshot_id: SnapshotId::new(1),
            baseline_id: SnapshotId::NONE,
            kind: SnapshotKind::Keyframe,
            viewer,
            server_tick: 40,
            acknowledged_input_sequence: 0,
            entities: vec![ReplicatedEntity {
                entity_id,
                kind: EntityKind::Player,
                owner: Some(viewer),
                team: None,
                position_cm: [100, 0, 200],
                velocity_cm_per_tick: [0; 3],
                yaw: 0,
                pitch: 0,
                health: 100,
                flags: 0,
                interest: InterestTier::CombatCritical128Hz,
                scheduled_hz: 128,
            }],
            removed_entities: Vec::new(),
            terrain_revisions: Vec::new(),
        })
        .unwrap();

    let mut orchestrator = ClientFrameOrchestrator::new(41, 1);
    let cadence = orchestrator
        .advance_frame(
            7_812_500,
            MappedInput {
                move_x: 2_047,
                ..MappedInput::default()
            },
            &mut replica,
        )
        .unwrap();
    assert_eq!(cadence.commands.len(), 1);
    assert_eq!(replica.predicted_position_cm(), Some([108, 0, 200]));
}

#[test]
fn invalid_input_does_not_consume_prediction_time() {
    let viewer = aetherloom_client::PlayerId::new(0).unwrap();
    let mut replica = ClientReplica::new(viewer);
    let mut orchestrator = ClientFrameOrchestrator::new(5, 1);

    let error = orchestrator
        .advance_frame(
            7_812_500,
            MappedInput {
                move_x: 3_000,
                ..MappedInput::default()
            },
            &mut replica,
        )
        .unwrap_err();
    assert!(matches!(error, OrchestratorError::Command(_)));
    assert_eq!(orchestrator.next_prediction_tick(), 5);

    let cadence = orchestrator
        .advance_frame(0, MappedInput::default(), &mut replica)
        .unwrap();
    assert_eq!(cadence.commands.len(), 1);
    assert_eq!(cadence.commands[0].target_tick(), 5);
}

#[test]
fn excessive_prediction_backlog_requires_and_recovers_through_rebase() {
    let viewer = aetherloom_client::PlayerId::new(0).unwrap();
    let mut replica = ClientReplica::new(viewer);
    let mut orchestrator = ClientFrameOrchestrator::new(20, 1);

    assert_eq!(
        orchestrator
            .advance_frame(
                1_000_000_000,
                MappedInput {
                    look_yaw_delta: 500,
                    ..MappedInput::default()
                },
                &mut replica,
            )
            .unwrap_err(),
        OrchestratorError::PredictionBacklog {
            discarded_ticks: 128,
        }
    );
    assert!(orchestrator.requires_resync());
    assert_eq!(
        orchestrator
            .advance_frame(7_812_500, MappedInput::default(), &mut replica)
            .unwrap_err(),
        OrchestratorError::ResyncRequired
    );

    orchestrator.rebase(900, 44);
    assert!(!orchestrator.requires_resync());
    let cadence = orchestrator
        .advance_frame(7_812_500, MappedInput::default(), &mut replica)
        .unwrap();
    assert_eq!(cadence.commands.len(), 1);
    assert_eq!(cadence.commands[0].target_tick(), 900);
    assert_eq!(cadence.commands[0].sequence(), 44);
    assert_eq!(cadence.commands[0].look_yaw(), 0);
}

#[test]
fn suspended_orchestrator_does_not_replay_wall_clock_gap() {
    let viewer = aetherloom_client::PlayerId::new(0).unwrap();
    let mut replica = ClientReplica::new(viewer);
    let mut orchestrator = ClientFrameOrchestrator::new(20, 1);
    orchestrator.suspend();
    assert!(orchestrator
        .advance_frame(
            10_000_000_000,
            MappedInput::default(),
            &mut replica
        )
        .is_err());
    orchestrator.resume_at(50);
    let cadence = orchestrator
        .advance_frame(7_812_500, MappedInput::default(), &mut replica)
        .unwrap();
    assert_eq!(cadence.commands.len(), 1);
    assert_eq!(cadence.commands[0].target_tick(), 50);
}

#[test]
fn safe_area_layout_saturates_malformed_insets() {
    let rect = SafeAreaRect::layout(
        Viewport {
            width_px: 1_920,
            height_px: 1_080,
        },
        SafeAreaInsets {
            left_px: 100,
            top_px: 50,
            right_px: 100,
            bottom_px: 50,
        },
    );
    assert_eq!(
        rect,
        SafeAreaRect {
            x_px: 100,
            y_px: 50,
            width_px: 1_720,
            height_px: 980,
        }
    );

    let collapsed = SafeAreaRect::layout(
        Viewport {
            width_px: 100,
            height_px: 50,
        },
        SafeAreaInsets {
            left_px: 200,
            top_px: 200,
            right_px: 200,
            bottom_px: 200,
        },
    );
    assert_eq!(collapsed.width_px, 0);
    assert_eq!(collapsed.height_px, 0);
}

#[test]
fn transport_markers_expose_required_cadences() {
    assert_eq!(QuicTransport::POLICY.input_hz, 128);
    assert_eq!(QuicTransport::POLICY.snapshot_hz, 128);
    assert!(QuicTransport::POLICY.supports_unreliable_datagrams);

    assert_eq!(WebSocketTransport::POLICY.input_hz, 64);
    assert_eq!(WebSocketTransport::POLICY.snapshot_hz, 32);
    assert!(!WebSocketTransport::POLICY.supports_unreliable_datagrams);

    assert_eq!(LoopbackTransport::POLICY.input_hz, 128);
    assert_eq!(LoopbackTransport::POLICY.snapshot_hz, 128);
}

#[test]
fn input_batch_scheduler_emits_native_redundancy_and_browser_64_hz_batches() {
    fn input(tick: u64, sequence: u32) -> aetherloom_client::PlayerCommand {
        aetherloom_client::PlayerCommand::new(tick, sequence, 0, 0, 0, 0, 0, None)
            .unwrap()
    }

    let mut native = InputBatchScheduler::new(QuicTransport::POLICY).unwrap();
    assert_eq!(native.commands_per_batch(), 1);
    let first = native.push(input(0, 1)).unwrap().unwrap();
    assert_eq!(
        first
            .commands
            .iter()
            .map(|command| command.sequence())
            .collect::<Vec<_>>(),
        vec![1]
    );
    let third = native.push(input(1, 2)).unwrap().unwrap();
    assert_eq!(
        third
            .commands
            .iter()
            .map(|command| command.sequence())
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    let third = native.push(input(2, 3)).unwrap().unwrap();
    assert_eq!(
        third
            .commands
            .iter()
            .map(|command| command.sequence())
            .collect::<Vec<_>>(),
        vec![3, 2, 1]
    );

    let mut browser = InputBatchScheduler::new(WebSocketTransport::POLICY).unwrap();
    assert_eq!(browser.commands_per_batch(), 2);
    assert!(browser.push(input(0, 1)).unwrap().is_none());
    let batch = browser.push(input(1, 2)).unwrap().unwrap();
    assert_eq!(
        batch
            .commands
            .iter()
            .map(|command| command.sequence())
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert!(browser.push(input(2, 3)).unwrap().is_none());
    let flushed = browser.flush().unwrap().unwrap();
    assert_eq!(
        flushed
            .commands
            .iter()
            .map(|command| command.sequence())
            .collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
}

#[test]
fn invalid_redundant_input_does_not_corrupt_the_batch_scheduler() {
    fn input(tick: u64, sequence: u32) -> aetherloom_client::PlayerCommand {
        aetherloom_client::PlayerCommand::new(tick, sequence, 0, 0, 0, 0, 0, None)
            .unwrap()
    }

    let mut browser = InputBatchScheduler::new(WebSocketTransport::POLICY).unwrap();
    assert!(browser.push(input(5, 5)).unwrap().is_none());
    assert!(browser.push(input(4, 4)).is_err());
    assert_eq!(browser.pending_commands(), 1);
    let recovered = browser.push(input(6, 6)).unwrap().unwrap();
    assert_eq!(
        recovered
            .commands
            .iter()
            .map(|command| command.sequence())
            .collect::<Vec<_>>(),
        vec![6, 5]
    );
}

#[test]
fn snapshot_jitter_adapts_replica_interpolation_between_two_and_six_ticks() {
    let viewer = aetherloom_client::PlayerId::new(0).unwrap();
    let mut replica = ClientReplica::new(viewer);
    let mut jitter = SnapshotJitterEstimator::new();
    assert_eq!(jitter.observe_and_apply(0, 0, &mut replica), 2);
    assert_eq!(
        jitter.observe_and_apply(31_250_000, 4, &mut replica),
        2
    );

    let fixed_arrival = 31_250_000;
    for server_tick in (8..=132).step_by(4) {
        jitter.observe_and_apply(fixed_arrival, server_tick, &mut replica);
    }
    assert_eq!(jitter.interpolation_delay_ticks(), 6);
    assert_eq!(replica.interpolation_delay_ticks(), 6);

    let before = jitter;
    assert_eq!(jitter.observe(fixed_arrival - 1, 136), 6);
    assert_eq!(jitter, before);
    assert_eq!(jitter.observe(fixed_arrival, 128), 6);
    assert_eq!(jitter, before);
}

#[test]
fn c_header_and_rust_boundary_share_the_version_contract() {
    let header = VendorApiHeader::for_type::<VendorClientApiV1>();
    assert_eq!(header.abi_version, AETHERLOOM_CLIENT_ABI_VERSION);
    assert_eq!(
        header.struct_size as usize,
        core::mem::size_of::<VendorClientApiV1>()
    );

    let c_header = include_str!("../include/aetherloom_client.h");
    assert!(c_header.contains("#define AETHERLOOM_CLIENT_ABI_VERSION 1u"));
    assert!(c_header.contains("AetherloomVendorClientApiV1"));
}

#[test]
fn hud_frame_is_safe_area_aware_by_construction() {
    let safe_area = SafeAreaRect {
        x_px: 10,
        y_px: 20,
        width_px: 800,
        height_px: 600,
    };
    let hud = HudFrame {
        safe_area,
        ..HudFrame::default()
    };
    assert_eq!(hud.safe_area, safe_area);
}
