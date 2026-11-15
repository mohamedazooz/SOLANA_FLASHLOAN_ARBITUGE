//! Provider Capability Model and Validation Registry.
//!
//! Enforces deterministic capability declarations per provider endpoint, preventing
//! runtime mismatches (e.g. sending JSON-RPC to a decoded/Mirage stream or expecting
//! Geyser write versions from standard WebSocket endpoints).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderTransport {
    HttpJsonRpc,
    WebSocketJsonRpc,
    WebSocketMirageProtobuf,
    WebSocketBlurJson,
    YellowstoneGrpc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SubscriptionMethod {
    AccountSubscribe,
    ProgramSubscribe,
    SlotSubscribe,
    LogsSubscribe,
    MirageSubscribeUpdate,
    BlurEventFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccountDataEncoding {
    Base64,
    Base64Zstd,
    JsonParsed,
    RawProtobuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommitmentLevel {
    Processed,
    Confirmed,
    Finalized,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconnectPolicy {
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub max_retries: Option<u32>,
    pub enable_jitter: bool,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_ms: 500,
            max_backoff_ms: 10_000,
            max_retries: None, // infinite reconnect by default
            enable_jitter: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitPolicy {
    pub max_requests_per_sec: u32,
    pub max_concurrent_subscriptions: usize,
}

impl Default for RateLimitPolicy {
    fn default() -> Self {
        Self {
            max_requests_per_sec: 100,
            max_concurrent_subscriptions: 1_024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub name: String,
    pub transport: ProviderTransport,
    pub supported_methods: HashSet<SubscriptionMethod>,
    pub supported_encodings: HashSet<AccountDataEncoding>,
    pub supported_commitments: HashSet<CommitmentLevel>,
    pub rate_limits: RateLimitPolicy,
    pub reconnect_policy: ReconnectPolicy,
    pub has_upstream_slot_metadata: bool,
    pub has_upstream_write_versions: bool,
    pub has_fork_information: bool,
}

impl ProviderCapabilities {
    /// Solami Blur Decoded Event Stream capabilities.
    pub fn solami_blur() -> Self {
        let mut supported_methods = HashSet::new();
        supported_methods.insert(SubscriptionMethod::BlurEventFilter);

        let mut supported_encodings = HashSet::new();
        supported_encodings.insert(AccountDataEncoding::JsonParsed);

        let mut supported_commitments = HashSet::new();
        supported_commitments.insert(CommitmentLevel::Processed);
        supported_commitments.insert(CommitmentLevel::Confirmed);

        Self {
            name: "Solami Blur".to_string(),
            transport: ProviderTransport::WebSocketBlurJson,
            supported_methods,
            supported_encodings,
            supported_commitments,
            rate_limits: RateLimitPolicy {
                max_requests_per_sec: 50,
                max_concurrent_subscriptions: 100,
            },
            reconnect_policy: ReconnectPolicy::default(),
            has_upstream_slot_metadata: true,
            has_upstream_write_versions: false, // Decoded stream has no raw Geyser write versions
            has_fork_information: false,
        }
    }

    /// Solami Mirage Yellowstone-over-WebSocket capabilities.
    pub fn solami_mirage() -> Self {
        let mut supported_methods = HashSet::new();
        supported_methods.insert(SubscriptionMethod::MirageSubscribeUpdate);

        let mut supported_encodings = HashSet::new();
        supported_encodings.insert(AccountDataEncoding::RawProtobuf);

        let mut supported_commitments = HashSet::new();
        supported_commitments.insert(CommitmentLevel::Processed);
        supported_commitments.insert(CommitmentLevel::Confirmed);

        Self {
            name: "Solami Mirage".to_string(),
            transport: ProviderTransport::WebSocketMirageProtobuf,
            supported_methods,
            supported_encodings,
            supported_commitments,
            rate_limits: RateLimitPolicy {
                max_requests_per_sec: 1_000,
                max_concurrent_subscriptions: 10_000,
            },
            reconnect_policy: ReconnectPolicy::default(),
            has_upstream_slot_metadata: true,
            has_upstream_write_versions: true, // Yellowstone Geyser frames preserve write versions
            has_fork_information: true,
        }
    }

    /// Standard Solana JSON-RPC WebSocket (Helius, Solami RPC WS, Triton, Shyft).
    pub fn standard_json_rpc_ws(name: &str) -> Self {
        let mut supported_methods = HashSet::new();
        supported_methods.insert(SubscriptionMethod::AccountSubscribe);
        supported_methods.insert(SubscriptionMethod::ProgramSubscribe);
        supported_methods.insert(SubscriptionMethod::SlotSubscribe);
        supported_methods.insert(SubscriptionMethod::LogsSubscribe);

        let mut supported_encodings = HashSet::new();
        supported_encodings.insert(AccountDataEncoding::Base64);
        supported_encodings.insert(AccountDataEncoding::Base64Zstd);
        supported_encodings.insert(AccountDataEncoding::JsonParsed);

        let mut supported_commitments = HashSet::new();
        supported_commitments.insert(CommitmentLevel::Processed);
        supported_commitments.insert(CommitmentLevel::Confirmed);
        supported_commitments.insert(CommitmentLevel::Finalized);

        Self {
            name: name.to_string(),
            transport: ProviderTransport::WebSocketJsonRpc,
            supported_methods,
            supported_encodings,
            supported_commitments,
            rate_limits: RateLimitPolicy {
                max_requests_per_sec: 100,
                max_concurrent_subscriptions: 256,
            },
            reconnect_policy: ReconnectPolicy::default(),
            has_upstream_slot_metadata: true,
            has_upstream_write_versions: false, // Standard RPC WS does not provide Geyser write versions
            has_fork_information: false,
        }
    }
}

#[derive(Debug, Error)]
pub enum CapabilityError {
    #[error("Provider {provider} does not support method {method:?}")]
    UnsupportedMethod {
        provider: String,
        method: SubscriptionMethod,
    },
    #[error("Provider {provider} does not support transport {transport:?}")]
    UnsupportedTransport {
        provider: String,
        transport: ProviderTransport,
    },
    #[error("Provider {provider} lacks required capability: {requirement}")]
    MissingRequiredCapability {
        provider: String,
        requirement: &'static str,
    },
}

#[derive(Debug, Default, Clone)]
pub struct ProviderCapabilityRegistry {
    providers: Vec<ProviderCapabilities>,
}

impl ProviderCapabilityRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, caps: ProviderCapabilities) {
        self.providers.push(caps);
    }

    pub fn find(&self, name: &str) -> Option<&ProviderCapabilities> {
        self.providers.iter().find(|p| p.name == name)
    }

    pub fn validate_subscription(
        &self,
        provider_name: &str,
        method: SubscriptionMethod,
    ) -> Result<(), CapabilityError> {
        if let Some(p) = self.find(provider_name) {
            if !p.supported_methods.contains(&method) {
                return Err(CapabilityError::UnsupportedMethod {
                    provider: provider_name.to_string(),
                    method,
                });
            }
        }
        Ok(())
    }

    /// Verify before trading starts that at least one provider supplies live account streaming.
    pub fn validate_execution_prerequisites(&self) -> Result<(), CapabilityError> {
        let has_account_stream = self.providers.iter().any(|p| {
            p.supported_methods
                .contains(&SubscriptionMethod::AccountSubscribe)
                || p.supported_methods
                    .contains(&SubscriptionMethod::MirageSubscribeUpdate)
        });
        if !has_account_stream {
            return Err(CapabilityError::MissingRequiredCapability {
                provider: "Registry".to_string(),
                requirement: "No provider configured that supports real-time account streaming",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solami_blur_capabilities() {
        let blur = ProviderCapabilities::solami_blur();
        assert_eq!(blur.transport, ProviderTransport::WebSocketBlurJson);
        assert!(
            blur.supported_methods
                .contains(&SubscriptionMethod::BlurEventFilter)
        );
        assert!(
            !blur
                .supported_methods
                .contains(&SubscriptionMethod::AccountSubscribe)
        );
        assert!(!blur.has_upstream_write_versions);
    }

    #[test]
    fn test_solami_mirage_capabilities() {
        let mirage = ProviderCapabilities::solami_mirage();
        assert_eq!(mirage.transport, ProviderTransport::WebSocketMirageProtobuf);
        assert!(
            mirage
                .supported_methods
                .contains(&SubscriptionMethod::MirageSubscribeUpdate)
        );
        assert!(mirage.has_upstream_write_versions);
        assert!(mirage.has_fork_information);
    }

    #[test]
    fn test_standard_rpc_capabilities() {
        let rpc = ProviderCapabilities::standard_json_rpc_ws("Helius");
        assert_eq!(rpc.transport, ProviderTransport::WebSocketJsonRpc);
        assert!(
            rpc.supported_methods
                .contains(&SubscriptionMethod::AccountSubscribe)
        );
        assert!(!rpc.has_upstream_write_versions);
    }

    #[test]
    fn test_registry_validation() {
        let mut reg = ProviderCapabilityRegistry::new();
        reg.register(ProviderCapabilities::solami_blur());
        // Blur cannot be asked for standard accountSubscribe
        assert!(
            reg.validate_subscription("Solami Blur", SubscriptionMethod::AccountSubscribe)
                .is_err()
        );
        assert!(
            reg.validate_subscription("Solami Blur", SubscriptionMethod::BlurEventFilter)
                .is_ok()
        );
    }
}
