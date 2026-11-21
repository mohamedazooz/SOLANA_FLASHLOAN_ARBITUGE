//! LiteSVM end-to-end harness milestone A for `arb-executor` (S2.5 unblock,
//! stage 1 of ISSUE-BLOCKED LiteSVM borrow→swap→repay).
//!
//! What these tests PROVE, with real SBF/mainnet artifacts:
//! 1. The workspace-built `arb_executor.so` SBPF ELF loads and executes inside
//!    LiteSVM (in-process SVM, BPFLoaderUpgradeable, matching mainnet), and its
//!    typed dispatcher errors are observable from the VM
//!    (`InvalidInstructionData = 6000`).
//! 2. The VM itself boots and executes system-program semantics.
//! 3. Mainnet `Kamino klend` and `Meteora lb_clmm` program blobs (fetched from
//!    mainnet and hash-pinned, see
//!    `docs/research/EXTERNAL_BEST_PRACTICES.md §S2.5`) deploy into the same VM.
//!
//! What these tests do NOT claim: a full flash-borrow→swap→repay economic
//! proof. That requires a consistent mainnet account snapshot (Kamino
//! reserve/market + DLMM pool/bin arrays + CPMM pool + vaults) at one slot;
//! the fixture list is enumerated in the audit register.
//!
//! Artifacts are resolved from:
//! - `ARB_EXECUTOR_SO`       (default: `<workspace>/target/deploy/arb_executor.so`)
//! - `LITESVM_PROGRAM_BLOBS` (optional dir containing `klend.so`, `dlmm.so`)
//!
//! Run: `cargo test -p arb-sim --features litesvm --test litesvm_arb_executor_e2e`
//! (CI runs cargo test without the `litesvm` feature → this file compiles to an
//! empty test binary and is skipped, by design, never a fabricated pass).
#![cfg(feature = "litesvm")]

use litesvm::LiteSVM;
use solana_keypair::{Keypair, Signer};
use solana_transaction::{
    AccountMeta, Address, Instruction, Message, VersionedMessage, versioned::VersionedTransaction,
};

/// `ArbExec1111111111111111111111111111111111111` — canonical arb-executor ID.
const ARB_EXECUTOR_ID: Address =
    Address::from_str_const("ArbExec1111111111111111111111111111111111111");
/// Kamino klend mainnet program ID.
const KLEND_ID: Address = Address::from_str_const("KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD");
/// Meteora DLMM lb_clmm mainnet program ID.
const DLMM_ID: Address = Address::from_str_const("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");

/// Solana System Program ID.
const SYSTEM_PROGRAM_ID: Address = Address::from_str_const("11111111111111111111111111111111");

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(std::path::Path::to_path_buf)
        .expect("arb-sim lives at <workspace>/crates/arb-sim")
}

fn arb_executor_so_path() -> std::path::PathBuf {
    std::env::var_os("ARB_EXECUTOR_SO")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target/deploy/arb_executor.so"))
}

fn program_blobs_dir() -> std::path::PathBuf {
    std::env::var_os("LITESVM_PROGRAM_BLOBS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target/deploy/program-blobs"))
}

fn read_bytes_or_skip(path: &std::path::Path, what: &str) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            eprintln!(
                "SKIP: {what} artifact not found at {} ({e}); run the SBF unblock \
                 commands in docs/research/EXTERNAL_BEST_PRACTICES.md §S2.5 to produce it",
                path.display()
            );
            None
        }
    }
}

fn svm_with_payer() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 1_000_000_000).unwrap();
    (svm, payer)
}

fn airdrop(svm: &mut LiteSVM, address: &Address) {
    svm.airdrop(address, 1_000_000_000).unwrap();
}

fn signed_legacy_tx(
    svm: &LiteSVM,
    payer: &Keypair,
    instructions: &[Instruction],
) -> VersionedTransaction {
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(instructions, Some(&payer.pubkey()), &blockhash);
    VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer])
        .expect("transaction signing must succeed")
}

/// Deterministic System-Program `Transfer` instruction (instruction enum index 2,
/// `lamports` payload) — no extra dependency needed for the VM sanity check.
fn system_transfer(from: &Address, to: &Address, lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction::new_with_bytes(
        SYSTEM_PROGRAM_ID,
        &data,
        vec![AccountMeta::new(*from, true), AccountMeta::new(*to, false)],
    )
}

#[test]
fn system_transfer_proves_vm_boots() {
    let (mut svm, payer) = svm_with_payer();
    let recipient = Keypair::new();
    airdrop(&mut svm, &recipient.pubkey());

    let tx = signed_legacy_tx(
        &svm,
        &payer,
        &[system_transfer(
            &payer.pubkey(),
            &recipient.pubkey(),
            123_456,
        )],
    );

    let sim = svm.simulate_transaction(tx).unwrap_or_else(|failed| {
        panic!("system transfer must succeed in LiteSVM: {:?}", failed.err)
    });
    assert!(
        sim.meta.compute_units_consumed > 0,
        "system transfer must consume compute units"
    );
}

#[test]
fn arb_executor_sbf_elf_loads_and_dispatches_typed_error() {
    let Some(program_bytes) = read_bytes_or_skip(&arb_executor_so_path(), "arb_executor.so") else {
        return;
    };

    let (mut svm, payer) = svm_with_payer();
    svm.add_program(ARB_EXECUTOR_ID, &program_bytes)
        .expect("arb_executor.so SBPF ELF must load into LiteSVM");

    // Garbage 8-byte discriminator: the dispatcher's borsh parse of an unknown
    // discriminator falls through to `ExecutorError::InvalidInstructionData`
    // (custom error 6000) — see programs/arb-executor/src/lib.rs dispatch.
    let ix = Instruction::new_with_bytes(ARB_EXECUTOR_ID, &[1u8, 2, 3, 4, 5, 6, 7, 8], vec![]);
    let tx = signed_legacy_tx(&svm, &payer, &[ix]);

    let failed = svm
        .simulate_transaction(tx)
        .expect_err("garbage discriminator must be rejected, not accepted");
    let err_debug = format!("{:?}", failed.err);
    assert!(
        err_debug.contains("Custom(6000)"),
        "expected arb-executor InvalidInstructionData (Custom(6000)), got: {err_debug}"
    );
    eprintln!(
        "VERIFIED: arb_executor.so dispatched typed error in LiteSVM \
         (logs captured: {} lines, CU: {})",
        failed.meta.logs.len(),
        failed.meta.compute_units_consumed
    );
}

#[test]
fn arb_executor_rejects_short_instruction_data_with_invalid_instruction_data() {
    let Some(program_bytes) = read_bytes_or_skip(&arb_executor_so_path(), "arb_executor.so") else {
        return;
    };

    let (mut svm, payer) = svm_with_payer();
    svm.add_program(ARB_EXECUTOR_ID, &program_bytes)
        .expect("arb_executor.so SBPF ELF must load into LiteSVM");

    // Below 8 bytes: the dispatcher's early `len() < 8` guard must fire before
    // any account handling, returning custom error 6000.
    let ix = Instruction::new_with_bytes(ARB_EXECUTOR_ID, &[0u8; 4], vec![]);
    let tx = signed_legacy_tx(&svm, &payer, &[ix]);

    let failed = svm
        .simulate_transaction(tx)
        .expect_err("short instruction data must be rejected, not accepted");
    let err_debug = format!("{:?}", failed.err);
    assert!(
        err_debug.contains("Custom(6000)"),
        "expected arb-executor InvalidInstructionData (Custom(6000)), got: {err_debug}"
    );
}

#[test]
fn mainnet_klend_and_dlmm_blobs_deploy_in_litesvm() {
    let blobs_dir = program_blobs_dir();
    let klend_path = blobs_dir.join("klend.so");
    let dlmm_path = blobs_dir.join("dlmm.so");

    let Some(klend_bytes) = read_bytes_or_skip(&klend_path, "klend mainnet blob") else {
        return;
    };
    let Some(dlmm_bytes) = read_bytes_or_skip(&dlmm_path, "dlmm mainnet blob") else {
        return;
    };

    let mut svm = LiteSVM::new();
    svm.add_program(KLEND_ID, &klend_bytes)
        .expect("mainnet Kamino klend ELF must deploy into LiteSVM");
    svm.add_program(DLMM_ID, &dlmm_bytes)
        .expect("mainnet Meteora DLMM lb_clmm ELF must deploy into LiteSVM");

    assert!(
        svm.get_account(&KLEND_ID).is_some(),
        "deployed klend program account must be present in the VM"
    );
    assert!(
        svm.get_account(&DLMM_ID).is_some(),
        "deployed dlmm program account must be present in the VM"
    );
    eprintln!(
        "VERIFIED: mainnet blobs deploy in LiteSVM (klend: {} bytes, dlmm: {} bytes)",
        klend_bytes.len(),
        dlmm_bytes.len()
    );
}
