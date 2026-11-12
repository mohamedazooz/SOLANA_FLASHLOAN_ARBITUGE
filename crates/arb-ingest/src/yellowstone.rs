//! Native Yellowstone Geyser gRPC ingestion.
//!
//! This module intentionally does not emulate Yellowstone over Solana JSON-RPC WebSocket.
//! Solami's Yellowstone endpoint is consumed through the official Yellowstone gRPC client.
//! Account payloads are journaled individually with slot/write-version ordering. DEX-specific
//! promotion into quote state is a separate, validated stage.

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::geyser::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterAccounts, SubscribeRequestFilterEntry,
    SubscribeRequestFilterSlots, subscribe_update::UpdateOneof,
};

pub use crate::YellowstoneClient;

fn build_request(
    owner_programs: &[solana_pubkey::Pubkey],
    tracked_accounts: &[String],
    include_entry_parent_updates: bool,
) -> SubscribeRequest {
    let mut request = SubscribeRequest::default();
    let mut accounts = HashMap::new();
    accounts.insert(
        "dex_program_accounts".to_string(),
        SubscribeRequestFilterAccounts {
            account: Vec::new(),
            owner: owner_programs.iter().map(ToString::to_string).collect(),
            filters: Vec::new(),
            nonempty_txn_signature: None,
            cuckoo_accounts_filter: None,
        },
    );
    if !tracked_accounts.is_empty() {
        accounts.insert(
            "tracked_accounts".to_string(),
            SubscribeRequestFilterAccounts {
                account: tracked_accounts.to_vec(),
                owner: Vec::new(),
                filters: Vec::new(),
                nonempty_txn_signature: None,
                cuckoo_accounts_filter: None,
            },
        );
    }
    request.accounts = accounts;
    // Yellowstone's entry filter emits entry updates as well as EntryUpdateParent.
    // Keep this opt-in: entries are high-volume and the bot does not otherwise consume them.
    if include_entry_parent_updates {
        request.entry = HashMap::from([(
            "entry_parent_reconciliation".to_string(),
            SubscribeRequestFilterEntry {
                include_update_parent: Some(true),
            },
        )]);
    }
    request.slots = HashMap::from([(
        "slot_watermark".to_string(),
        SubscribeRequestFilterSlots {
            filter_by_commitment: Some(false),
            interslot_updates: Some(true),
        },
    )]);
    request.commitment = Some(CommitmentLevel::Processed as i32);
    request
}

impl crate::YellowstoneClient {
    /// Start a real Yellowstone gRPC account/slot stream.
    ///
    /// Startup snapshots are retained as raw account records. They are not treated as
    /// quote-ready until protocol decoders validate owner, discriminator, PDA and state coverage.
    pub async fn start_stream(
        self: std::sync::Arc<Self>,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        if self.endpoint_url.trim().is_empty() || self.token.trim().is_empty() {
            self.mark_stream_disconnected();
            warn!(
                "Solami Yellowstone gRPC endpoint/token missing; ingestion stays unavailable (no HTTP fallback)"
            );
            return;
        }

        let owner_programs = [
            arb_constants::RAYDIUM_V4_PROGRAM_ID,
            arb_constants::RAYDIUM_CPMM_PROGRAM_ID,
            arb_constants::RAYDIUM_CLMM_PROGRAM_ID,
            arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID,
            arb_constants::METEORA_DLMM_PROGRAM_ID,
            arb_constants::PUMP_FUN_PROGRAM_ID,
            arb_constants::PUMP_SWAP_PROGRAM_ID,
            arb_constants::PUMP_FEES_PROGRAM_ID,
            arb_constants::KAMINO_KLEND_PROGRAM_ID,
        ]
        .iter()
        .filter_map(|value| solana_pubkey::Pubkey::from_str(value).ok())
        .collect::<Vec<_>>();

        let mut tracked_rx = self.tracked_accounts_tx.subscribe();
        let include_entry_parent_updates = std::env::var("YELLOWSTONE_ENTRY_PARENT_UPDATES")
            .is_ok_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes"
                )
            });
        if include_entry_parent_updates {
            warn!(
                "YELLOWSTONE_ENTRY_PARENT_UPDATES enabled: Yellowstone entry filter also streams high-volume entry updates"
            );
        }
        let mut attempt = 0u32;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            attempt = attempt.saturating_add(1);
            let endpoint = self.endpoint_url.clone();
            let token = self.token.clone();
            let generation = self.begin_stream_generation();
            let connect = async {
                let builder = GeyserGrpcClient::build_from_shared(endpoint)
                    .context("invalid Yellowstone gRPC endpoint")?
                    .x_token(Some(token))
                    .context("invalid Yellowstone x-token")?
                    .connect_timeout(Duration::from_secs(10))
                    .tcp_nodelay(true)
                    .http2_keep_alive_interval(Duration::from_secs(15))
                    .keep_alive_timeout(Duration::from_secs(10))
                    .keep_alive_while_idle(true);
                let mut client = builder
                    .connect()
                    .await
                    .context("Yellowstone gRPC connect failed")?;
                let (mut sink, mut stream) = client
                    .subscribe()
                    .await
                    .context("Yellowstone subscribe failed")?;
                let request = build_request(
                    &owner_programs,
                    &tracked_rx.borrow().clone(),
                    include_entry_parent_updates,
                );
                sink.send(request)
                    .await
                    .context("Yellowstone initial subscription failed")?;
                self.mark_stream_connected();
                info!(
                    generation,
                    "Connected to Solami Yellowstone gRPC; account and slot streams active"
                );

                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => return Ok::<(), anyhow::Error>(()),
                        changed = tracked_rx.changed() => {
                            if changed.is_err() {
                                return Err(anyhow::anyhow!("tracked-account subscription channel closed"));
                            }
                            let request = build_request(&owner_programs, &tracked_rx.borrow().clone(), include_entry_parent_updates);
                            sink.send(request).await.context("Yellowstone dynamic subscription update failed")?;
                        }
                        message = stream.next() => {
                            let Some(message) = message else {
                                return Err(anyhow::anyhow!("Yellowstone gRPC stream ended"));
                            };
                            let update = message.context("Yellowstone gRPC stream returned error")?;
                            match update.update_oneof {
                                Some(UpdateOneof::Slot(slot_update)) => {
                                    let outcome = self.record_slot_update_with_bank(slot_update.slot, slot_update.parent, slot_update.status, slot_update.bank_id);
                                    if let crate::SlotUpdateOutcome::Dead { invalidated_slots } = outcome {
                                        warn!(slot = slot_update.slot, invalidated = invalidated_slots.len(), "Yellowstone dead fork invalidated cached state");
                                    }
                                }
                                Some(UpdateOneof::EntryUpdateParent(parent_update)) => {
                                    let removed = self.record_entry_update_parent(parent_update.slot, parent_update.cleared_bank_id);
                                    warn!(slot = parent_update.slot, bank_id = parent_update.cleared_bank_id, removed_accounts = removed,
                                        "Yellowstone EntryUpdateParent invalidated cleared bank state");
                                }
                                Some(UpdateOneof::Account(account_update)) => {
                                    if let Some(account) = account_update.account {
                                        let pubkey_bytes: std::result::Result<[u8; 32], _> = account.pubkey.as_ref().try_into();
                                        let owner_bytes: std::result::Result<[u8; 32], _> = account.owner.as_ref().try_into();
                                        if let (Ok(pubkey_bytes), Ok(owner_bytes)) = (pubkey_bytes, owner_bytes) {
                                            let pubkey = solana_pubkey::Pubkey::new_from_array(pubkey_bytes);
                                            let owner = solana_pubkey::Pubkey::new_from_array(owner_bytes);
                                            self.record_slot(account_update.slot);
                                            self.record_account_bank_id(account_update.slot, account_update.bank_id);
                                            if self.slot_is_dead(account_update.slot) || self.bank_is_dead(account_update.slot, account_update.bank_id) { continue; }
                                            self.account_store.apply(crate::AccountSnapshot {
                                                pubkey,
                                                owner,
                                                slot: account_update.slot,
                                                write_version: account.write_version,
                                                is_startup: account_update.is_startup,
                                                provider_id: self.provider_id(),
                                                bank_id: account_update.bank_id,
                                                generation,
                                                data: std::sync::Arc::new(account.data.to_vec()),
                                            });
                                        } else {
                                            warn!("Discarded Yellowstone account update with invalid pubkey/owner bytes");
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            };

            match connect.await {
                Ok(()) if cancel.is_cancelled() => {
                    self.mark_stream_disconnected();
                    break;
                }
                Ok(()) => {
                    self.mark_stream_disconnected();
                    attempt = 0;
                }
                Err(error) => {
                    self.mark_stream_disconnected();
                    warn!(attempt, error = %error, "Yellowstone gRPC stream disconnected; reconnecting");
                    let delay = crate::YellowstoneClient::backoff_delay(attempt);
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = sleep(delay) => {}
                    }
                }
            }
        }
    }
}
