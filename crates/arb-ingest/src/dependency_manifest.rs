//! Explicit per-pool account dependency manifests used by subscriptions, bootstrap,
//! quote readiness and execution guards. No protocol-specific account is guessed.
use arb_types::{DexProtocol, PoolId};
use solana_pubkey::Pubkey;
use std::collections::{HashMap, HashSet};

pub const MAX_RPC_SNAPSHOT_ACCOUNTS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AccountRole {
    PoolState,
    VaultA,
    VaultB,
    MintA,
    MintB,
    TokenProgram,
    FeeConfig,
    Oracle,
    Observation,
    TickArray,
    BinArray,
    BitmapExtension,
    Config,
    Other,
    GlobalConfig,
    BondingCurve,
    CreatorVault,
    SharingConfig,
    MintExtension,
    TransferHook,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDependency {
    pub pubkey: Pubkey,
    pub expected_owner: Pubkey,
    pub role: AccountRole,
    /// Required for a quote. Optional dependencies are subscribed but do not block readiness.
    pub required_for_quote: bool,
}

#[derive(Debug, Clone)]
pub struct PoolDependencyManifest {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    dependencies: Vec<AccountDependency>,
}

impl PoolDependencyManifest {
    pub fn new(pool_id: PoolId, protocol: DexProtocol) -> Self {
        Self {
            pool_id,
            protocol,
            dependencies: Vec::new(),
        }
    }

    pub fn dependencies(&self) -> &[AccountDependency] {
        &self.dependencies
    }

    /// Seed the invariant base accounts from the verified registry record. Owner IDs
    /// are explicit inputs because vault/mint owners may be SPL Token or Token-2022.
    /// CLMM tick arrays, DLMM bin arrays, fee/config PDAs and observations must still
    /// be added by the protocol adapter that knows the active swap direction/range.
    pub fn from_registered_pool(
        pool: &crate::RegisteredPool,
        pool_owner: Pubkey,
        token_account_owner: Pubkey,
        mint_owner: Pubkey,
    ) -> Result<Self, DependencyManifestError> {
        let mut manifest = Self::new(pool.pool_id, pool.protocol);
        let pool_role = if pool.protocol == DexProtocol::PumpFun {
            AccountRole::BondingCurve
        } else {
            AccountRole::PoolState
        };
        for (pubkey, expected_owner, role) in [
            (pool.pool_id.0, pool_owner, pool_role),
            (pool.base_vault, token_account_owner, AccountRole::VaultA),
            (pool.quote_vault, token_account_owner, AccountRole::VaultB),
            (pool.base_mint.0, mint_owner, AccountRole::MintA),
            (pool.quote_mint.0, mint_owner, AccountRole::MintB),
        ] {
            manifest.add(AccountDependency {
                pubkey,
                expected_owner,
                role,
                required_for_quote: true,
            })?;
        }
        Ok(manifest)
    }

    pub fn add(&mut self, dependency: AccountDependency) -> Result<(), DependencyManifestError> {
        if dependency.pubkey == Pubkey::default() || dependency.expected_owner == Pubkey::default()
        {
            return Err(DependencyManifestError::DefaultPubkey);
        }
        if let Some(existing) = self
            .dependencies
            .iter()
            .find(|d| d.pubkey == dependency.pubkey)
        {
            return if existing == &dependency {
                Ok(())
            } else {
                Err(DependencyManifestError::ConflictingDependency)
            };
        }
        self.dependencies.push(dependency);
        self.dependencies
            .sort_by_key(|d| (d.role, d.pubkey.to_bytes()));
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyManifestError {
    DefaultPubkey,
    ConflictingDependency,
    MissingPool,
    DuplicatePool,
    EmptyManifest,
    RouteSnapshotTooLarge { count: usize, max: usize },
    ConflictingOwner,
}

#[derive(Debug, Default, Clone)]
pub struct DependencyManifestRegistry {
    pools: HashMap<PoolId, PoolDependencyManifest>,
}

impl DependencyManifestRegistry {
    pub fn register(
        &mut self,
        manifest: PoolDependencyManifest,
    ) -> Result<(), DependencyManifestError> {
        if manifest.dependencies.is_empty() {
            return Err(DependencyManifestError::EmptyManifest);
        }
        if self.pools.contains_key(&manifest.pool_id) {
            return Err(DependencyManifestError::DuplicatePool);
        }
        self.pools.insert(manifest.pool_id, manifest);
        Ok(())
    }

    pub fn get(&self, pool_id: &PoolId) -> Option<&PoolDependencyManifest> {
        self.pools.get(pool_id)
    }

    /// Merge dependencies for a route, deduplicating shared accounts. Yellowstone
    /// address subscriptions are not subject to the HTTP getMultipleAccounts limit.
    pub fn route_requirements(
        &self,
        pool_ids: &[PoolId],
    ) -> Result<(Vec<Pubkey>, HashMap<Pubkey, Pubkey>), DependencyManifestError> {
        if pool_ids.is_empty() {
            return Err(DependencyManifestError::EmptyManifest);
        }
        let mut owners = HashMap::new();
        let mut required = HashSet::new();
        for pool_id in pool_ids {
            let manifest = self
                .pools
                .get(pool_id)
                .ok_or(DependencyManifestError::MissingPool)?;
            for dependency in manifest
                .dependencies
                .iter()
                .filter(|d| d.required_for_quote)
            {
                if owners
                    .insert(dependency.pubkey, dependency.expected_owner)
                    .is_some_and(|old| old != dependency.expected_owner)
                {
                    return Err(DependencyManifestError::ConflictingOwner);
                }
                required.insert(dependency.pubkey);
            }
        }
        if required.is_empty() {
            return Err(DependencyManifestError::EmptyManifest);
        }
        let mut keys = required.into_iter().collect::<Vec<_>>();
        keys.sort_unstable_by_key(|key| key.to_bytes());
        Ok((keys, owners))
    }

    /// Return a single HTTP bootstrap group only when it fits Solana's 100-account
    /// getMultipleAccounts limit. Do not split a coherent dependency group across calls.
    pub fn rpc_snapshot_requirements(
        &self,
        pool_ids: &[PoolId],
    ) -> Result<(Vec<Pubkey>, HashMap<Pubkey, Pubkey>), DependencyManifestError> {
        let requirements = self.route_requirements(pool_ids)?;
        if requirements.0.len() > MAX_RPC_SNAPSHOT_ACCOUNTS {
            return Err(DependencyManifestError::RouteSnapshotTooLarge {
                count: requirements.0.len(),
                max: MAX_RPC_SNAPSHOT_ACCOUNTS,
            });
        }
        Ok(requirements)
    }

    /// All subscribed accounts, including optional dependencies, sorted and deduplicated.
    pub fn tracked_accounts(&self) -> Vec<Pubkey> {
        let mut keys = self
            .pools
            .values()
            .flat_map(|m| m.dependencies.iter().map(|d| d.pubkey))
            .collect::<Vec<_>>();
        keys.sort_unstable_by_key(|key| key.to_bytes());
        keys.dedup();
        keys
    }
}

/// Protocol-specific dependency resolver input. Every address must come from a
/// verified on-chain decoder or the official protocol SDK; this module deliberately
/// does not re-derive PDAs from guessed seeds.
#[derive(Debug, Clone)]
pub struct ProtocolDependencyInput {
    pub dependencies: Vec<AccountDependency>,
    /// Minimum number of direction/range-specific arrays required by the SDK quote path.
    /// Zero means the resolver applies the protocol's conservative minimum of one.
    pub required_tick_arrays: usize,
    pub required_bin_arrays: usize,
    pub requires_bitmap_extension: bool,
    pub requires_oracle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolDependencyError {
    MissingRole(AccountRole),
    MissingDynamicAccounts(AccountRole),
    UnexpectedRole(AccountRole),
    DuplicateRoleConflict(AccountRole),
    Manifest(DependencyManifestError),
}

/// Validate that the protocol-specific account set is sufficient for quote state.
/// Execution-only metas (user ATAs, signers, sysvars, program IDs) are intentionally
/// not represented as market-state dependencies unless their account data affects price.
pub fn resolve_protocol_dependencies(
    pool_id: PoolId,
    protocol: DexProtocol,
    input: ProtocolDependencyInput,
) -> Result<PoolDependencyManifest, ProtocolDependencyError> {
    let mut manifest = PoolDependencyManifest::new(pool_id, protocol);
    let required_tick_arrays = input.required_tick_arrays;
    let required_bin_arrays = input.required_bin_arrays;
    let requires_bitmap_extension = input.requires_bitmap_extension;
    let requires_oracle = input.requires_oracle;
    for dependency in input.dependencies {
        let role = dependency.role;
        manifest.add(dependency).map_err(|err| match err {
            DependencyManifestError::ConflictingDependency => {
                ProtocolDependencyError::DuplicateRoleConflict(role)
            }
            other => ProtocolDependencyError::Manifest(other),
        })?;
    }
    let has = |role: AccountRole| {
        manifest
            .dependencies
            .iter()
            .any(|d| d.role == role && d.required_for_quote)
    };
    let has_any = |role: AccountRole| manifest.dependencies.iter().any(|d| d.role == role);
    let count = |role: AccountRole| {
        manifest
            .dependencies
            .iter()
            .filter(|d| d.role == role && d.required_for_quote)
            .count()
    };
    let require = |role: AccountRole| {
        if has(role) {
            Ok(())
        } else {
            Err(ProtocolDependencyError::MissingRole(role))
        }
    };
    require(AccountRole::MintA)?;
    require(AccountRole::MintB)?;
    match protocol {
        DexProtocol::RaydiumV4 => {
            require(AccountRole::PoolState)?;
            require(AccountRole::VaultA)?;
            require(AccountRole::VaultB)?;
            // Legacy AMM v4 execution also requires OpenBook market/orderbook metas;
            // those are execution metas, not quote-state dependencies.
        }
        DexProtocol::RaydiumCpmm => {
            require(AccountRole::PoolState)?;
            require(AccountRole::VaultA)?;
            require(AccountRole::VaultB)?;
            require(AccountRole::FeeConfig)?;
        }
        DexProtocol::RaydiumClmm => {
            require(AccountRole::PoolState)?;
            require(AccountRole::VaultA)?;
            require(AccountRole::VaultB)?;
            require(AccountRole::FeeConfig)?;
            require(AccountRole::Observation)?;
            if count(AccountRole::TickArray) < required_tick_arrays.max(1) {
                return Err(ProtocolDependencyError::MissingDynamicAccounts(
                    AccountRole::TickArray,
                ));
            }
        }
        DexProtocol::OrcaWhirlpool => {
            require(AccountRole::PoolState)?;
            require(AccountRole::VaultA)?;
            require(AccountRole::VaultB)?;
            if count(AccountRole::TickArray) < required_tick_arrays.max(1) {
                return Err(ProtocolDependencyError::MissingDynamicAccounts(
                    AccountRole::TickArray,
                ));
            }
            // Oracle is conditional for adaptive-fee pools; caller sets requires_oracle
            // from the decoded Whirlpool configuration.
            if requires_oracle {
                require(AccountRole::Oracle)?;
            }
        }
        DexProtocol::MeteoraDlmm => {
            require(AccountRole::PoolState)?;
            require(AccountRole::VaultA)?;
            require(AccountRole::VaultB)?;
            if count(AccountRole::BinArray) < required_bin_arrays.max(1) {
                return Err(ProtocolDependencyError::MissingDynamicAccounts(
                    AccountRole::BinArray,
                ));
            }
            // The active swap path sets this flag when it crosses the default bitmap range.
            if requires_bitmap_extension {
                require(AccountRole::BitmapExtension)?;
            }
        }
        DexProtocol::PumpFun => {
            require(AccountRole::BondingCurve)?;
            require(AccountRole::GlobalConfig)?;
            require(AccountRole::FeeConfig)?;
            if !has_any(AccountRole::CreatorVault) {
                return Err(ProtocolDependencyError::MissingRole(
                    AccountRole::CreatorVault,
                ));
            }
            // Curve reserves are encoded in the BondingCurve state; one associated
            // token vault may be tracked as VaultA. A second generic vault is not assumed.
        }
        DexProtocol::PumpSwap => {
            require(AccountRole::PoolState)?;
            require(AccountRole::VaultA)?;
            require(AccountRole::VaultB)?;
            require(AccountRole::GlobalConfig)?;
            require(AccountRole::FeeConfig)?;
            if !has_any(AccountRole::CreatorVault) {
                return Err(ProtocolDependencyError::MissingRole(
                    AccountRole::CreatorVault,
                ));
            }
        }
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pk(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }
    fn manifest(pool: u8, account: u8, owner: u8) -> PoolDependencyManifest {
        let mut m = PoolDependencyManifest::new(PoolId(pk(pool)), DexProtocol::RaydiumV4);
        m.add(AccountDependency {
            pubkey: pk(account),
            expected_owner: pk(owner),
            role: AccountRole::PoolState,
            required_for_quote: true,
        })
        .unwrap();
        m
    }

    #[test]
    fn route_manifest_deduplicates_shared_dependencies_and_is_deterministic() {
        let mut registry = DependencyManifestRegistry::default();
        registry.register(manifest(1, 10, 90)).unwrap();
        let mut second = manifest(2, 11, 90);
        second
            .add(AccountDependency {
                pubkey: pk(10),
                expected_owner: pk(90),
                role: AccountRole::MintA,
                required_for_quote: true,
            })
            .unwrap();
        registry.register(second).unwrap();
        let (keys, owners) = registry
            .route_requirements(&[PoolId(pk(1)), PoolId(pk(2))])
            .unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(owners.len(), 2);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn conflicting_owner_for_same_dependency_fails_closed() {
        let mut registry = DependencyManifestRegistry::default();
        registry.register(manifest(1, 10, 90)).unwrap();
        registry.register(manifest(2, 10, 91)).unwrap();
        assert_eq!(
            registry.route_requirements(&[PoolId(pk(1)), PoolId(pk(2))]),
            Err(DependencyManifestError::ConflictingOwner)
        );
    }

    #[test]
    fn stream_route_can_exceed_100_but_single_rpc_snapshot_cannot() {
        let mut registry = DependencyManifestRegistry::default();
        for n in 1..=101u8 {
            registry.register(manifest(n, n, 200)).unwrap();
        }
        let ids = (1..=101u8).map(|n| PoolId(pk(n))).collect::<Vec<_>>();
        assert_eq!(registry.route_requirements(&ids).unwrap().0.len(), 101);
        assert!(matches!(
            registry.rpc_snapshot_requirements(&ids),
            Err(DependencyManifestError::RouteSnapshotTooLarge {
                count: 101,
                max: 100
            })
        ));
    }

    fn dep(key: u8, owner: u8, role: AccountRole) -> AccountDependency {
        AccountDependency {
            pubkey: pk(key),
            expected_owner: pk(owner),
            role,
            required_for_quote: true,
        }
    }

    #[test]
    fn protocol_resolver_requires_clmm_tick_arrays_and_fee_config() {
        let input = ProtocolDependencyInput {
            required_tick_arrays: 1,
            required_bin_arrays: 1,
            requires_bitmap_extension: false,
            requires_oracle: false,
            dependencies: vec![
                dep(1, 90, AccountRole::PoolState),
                dep(2, 91, AccountRole::VaultA),
                dep(3, 91, AccountRole::VaultB),
                dep(4, 92, AccountRole::MintA),
                dep(5, 92, AccountRole::MintB),
                dep(6, 90, AccountRole::FeeConfig),
            ],
        };
        assert!(matches!(
            resolve_protocol_dependencies(PoolId(pk(1)), DexProtocol::RaydiumClmm, input),
            Err(ProtocolDependencyError::MissingRole(
                AccountRole::Observation
            ))
        ));
    }

    #[test]
    fn protocol_resolver_accepts_dlmm_with_explicit_active_bin_arrays() {
        let input = ProtocolDependencyInput {
            required_tick_arrays: 1,
            required_bin_arrays: 1,
            requires_bitmap_extension: false,
            requires_oracle: false,
            dependencies: vec![
                dep(1, 90, AccountRole::PoolState),
                dep(2, 91, AccountRole::VaultA),
                dep(3, 91, AccountRole::VaultB),
                dep(4, 92, AccountRole::MintA),
                dep(5, 92, AccountRole::MintB),
                dep(6, 90, AccountRole::BinArray),
            ],
        };
        let manifest =
            resolve_protocol_dependencies(PoolId(pk(1)), DexProtocol::MeteoraDlmm, input).unwrap();
        assert!(
            manifest
                .dependencies()
                .iter()
                .any(|d| d.role == AccountRole::BinArray)
        );
    }

    #[test]
    fn pump_swap_requires_live_fee_and_creator_accounts() {
        let input = ProtocolDependencyInput {
            required_tick_arrays: 1,
            required_bin_arrays: 1,
            requires_bitmap_extension: false,
            requires_oracle: false,
            dependencies: vec![
                dep(1, 90, AccountRole::PoolState),
                dep(2, 91, AccountRole::VaultA),
                dep(3, 91, AccountRole::VaultB),
                dep(4, 92, AccountRole::MintA),
                dep(5, 92, AccountRole::MintB),
                dep(6, 90, AccountRole::GlobalConfig),
                dep(7, 93, AccountRole::FeeConfig),
            ],
        };
        assert!(matches!(
            resolve_protocol_dependencies(PoolId(pk(1)), DexProtocol::PumpSwap, input),
            Err(ProtocolDependencyError::MissingRole(
                AccountRole::CreatorVault
            ))
        ));
    }

    #[test]
    fn pump_fun_curve_manifest_does_not_assume_two_reserve_vaults() {
        let input = ProtocolDependencyInput {
            required_tick_arrays: 0,
            required_bin_arrays: 0,
            requires_bitmap_extension: false,
            requires_oracle: false,
            dependencies: vec![
                dep(1, 90, AccountRole::BondingCurve),
                dep(2, 92, AccountRole::MintA),
                dep(3, 92, AccountRole::MintB),
                dep(4, 90, AccountRole::GlobalConfig),
                dep(5, 93, AccountRole::FeeConfig),
                AccountDependency {
                    pubkey: pk(6),
                    expected_owner: pk(90),
                    role: AccountRole::CreatorVault,
                    required_for_quote: false,
                },
            ],
        };
        assert!(resolve_protocol_dependencies(PoolId(pk(1)), DexProtocol::PumpFun, input).is_ok());
    }

    #[test]
    fn orca_adaptive_fee_requires_oracle_when_flagged() {
        let input = ProtocolDependencyInput {
            required_tick_arrays: 2,
            required_bin_arrays: 0,
            requires_bitmap_extension: false,
            requires_oracle: true,
            dependencies: vec![
                dep(1, 90, AccountRole::PoolState),
                dep(2, 91, AccountRole::VaultA),
                dep(3, 91, AccountRole::VaultB),
                dep(4, 92, AccountRole::MintA),
                dep(5, 92, AccountRole::MintB),
                dep(6, 90, AccountRole::TickArray),
                dep(7, 90, AccountRole::TickArray),
            ],
        };
        assert!(matches!(
            resolve_protocol_dependencies(PoolId(pk(1)), DexProtocol::OrcaWhirlpool, input),
            Err(ProtocolDependencyError::MissingRole(AccountRole::Oracle))
        ));
    }

    #[test]
    fn meteora_requires_bitmap_extension_only_when_active_path_needs_it() {
        let input = ProtocolDependencyInput {
            required_tick_arrays: 0,
            required_bin_arrays: 2,
            requires_bitmap_extension: true,
            requires_oracle: false,
            dependencies: vec![
                dep(1, 90, AccountRole::PoolState),
                dep(2, 91, AccountRole::VaultA),
                dep(3, 91, AccountRole::VaultB),
                dep(4, 92, AccountRole::MintA),
                dep(5, 92, AccountRole::MintB),
                dep(6, 90, AccountRole::BinArray),
                dep(7, 90, AccountRole::BinArray),
            ],
        };
        assert!(matches!(
            resolve_protocol_dependencies(PoolId(pk(1)), DexProtocol::MeteoraDlmm, input),
            Err(ProtocolDependencyError::MissingRole(
                AccountRole::BitmapExtension
            ))
        ));
    }
}
