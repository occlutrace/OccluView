use crate::ThumbnailError;
use occluview_render::{AdapterPolicy, Offscreen, RenderDeadline};

/// The production Shell policy: verify hardware output before it is allowed
/// to enter Explorer's cache, then use the software adapter when it cannot.
pub(crate) const fn shell_adapter_policy() -> AdapterPolicy {
    AdapterPolicy::HardwareThenFallback
}

/// Deterministic GPU fixtures never depend on a workstation's hardware.
pub(crate) const fn test_adapter_policy() -> AdapterPolicy {
    AdapterPolicy::FallbackOnly
}

pub(crate) const fn default_thumbnail_adapter_policy() -> AdapterPolicy {
    if cfg!(test) {
        test_adapter_policy()
    } else {
        shell_adapter_policy()
    }
}

pub(crate) fn create_thumbnail_offscreen(
    deadline: RenderDeadline,
    adapter_policy: AdapterPolicy,
) -> Result<Offscreen, ThumbnailError> {
    pollster::block_on(Offscreen::new_with_adapter_policy(adapter_policy, deadline))
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_factory_requests_hardware_then_fallback() {
        assert_eq!(shell_adapter_policy(), AdapterPolicy::HardwareThenFallback);
    }

    #[test]
    fn test_fixture_uses_only_the_fallback_adapter() {
        assert_eq!(test_adapter_policy(), AdapterPolicy::FallbackOnly);
    }
}
