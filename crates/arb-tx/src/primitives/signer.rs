//! Keypair signer, ALT handle, Kamino bundle params and versioned transaction
//! wrapper types.

use crate::*;

/// In-memory Keypair Signer securely decoded from Base58 with real Ed25519 signing.
#[derive(Clone)]
pub struct KeypairSigner {
    pubkey: Pubkey,
    signing_key: SigningKey,
}

impl std::fmt::Debug for KeypairSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeypairSigner")
            .field("pubkey", &self.pubkey)
            .field("signing_key", &"[REDACTED_SECURE]")
            .finish()
    }
}

impl KeypairSigner {
    pub fn from_base58(secret_b58: &str) -> Result<Self> {
        let decoded = bs58::decode(secret_b58.trim())
            .into_vec()
            .context("Failed to decode base58 secret key")?;

        if decoded.len() != 64 {
            bail!(
                "Invalid Solana private key length: expected 64 bytes, got {}",
                decoded.len()
            );
        }

        let seed: [u8; 32] = decoded[0..32]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid secret seed"))?;
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey_bytes: [u8; 32] = decoded[32..64]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid pubkey"))?;
        let pubkey = Pubkey::new_from_array(pubkey_bytes);

        Ok(Self {
            pubkey,
            signing_key,
        })
    }

    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(seed);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let pubkey = Pubkey::new_from_array(pubkey_bytes);
        Self {
            pubkey,
            signing_key,
        }
    }

    /// Generate a random keypair for testing or ephemeral signing.
    pub fn new_random() -> Self {
        let seed = Pubkey::new_unique().to_bytes();
        Self::from_seed(&seed)
    }

    pub fn from_bytes(bytes: &[u8; 64]) -> Result<Self> {
        let seed: [u8; 32] = bytes[0..32]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid secret seed"))?;
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey_bytes: [u8; 32] = bytes[32..64]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid pubkey"))?;
        let pubkey = Pubkey::new_from_array(pubkey_bytes);

        Ok(Self {
            pubkey,
            signing_key,
        })
    }

    pub fn from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let trimmed = content.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let bytes: Vec<u8> = serde_json::from_str(trimmed)?;
            if bytes.len() != 64 {
                bail!(
                    "Invalid keypair JSON length: expected 64, got {}",
                    bytes.len()
                );
            }
            let mut arr = [0u8; 64];
            arr.copy_from_slice(&bytes);
            Self::from_bytes(&arr)
        } else {
            Self::from_base58(trimmed)
        }
    }

    pub fn pubkey(&self) -> Pubkey {
        self.pubkey
    }

    /// Sign payload using real cryptographic Ed25519 signature algorithm
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let sig: Signature = self.signing_key.sign(message);
        sig.to_bytes()
    }

    /// Verifying key for signature verification
    pub fn verifying_key(&self) -> ed25519_dalek::VerifyingKey {
        self.signing_key.verifying_key()
    }
}

/// Address Lookup Table structure for Solana v0 versioned transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressLookupTable {
    pub key: Pubkey,
    pub addresses: Vec<Pubkey>,
}

impl AddressLookupTable {
    pub fn new(key: Pubkey, addresses: Vec<Pubkey>) -> Self {
        Self { key, addresses }
    }
}

/// Parameters for assembling and signing an atomic Kamino flash loan bundle.
#[derive(Debug, Clone)]
pub struct KaminoBundleParams<'a> {
    pub payer: Pubkey,
    pub signer: &'a KeypairSigner,
    pub tip_account: Pubkey,
    pub tip_lamports: Lamports,
    pub priority_fee_micro_lamports: u64,
    pub opportunity: &'a ExecutionOpportunity,
    pub kamino_params: &'a KaminoFlashParams,
    pub recent_blockhash: [u8; 32],
    pub lookup_tables: &'a [AddressLookupTable],
    pub flash_loan_fee_sf: u64,
}

/// Versioned transaction container matching Solana v0 wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedTransaction {
    pub signatures: Vec<[u8; 64]>,
    pub message: Vec<u8>,
    pub wire_bytes: Vec<u8>,
}

impl VersionedTransaction {
    pub fn into_bytes(self) -> Vec<u8> {
        self.wire_bytes
    }
}

impl AsRef<[u8]> for VersionedTransaction {
    fn as_ref(&self) -> &[u8] {
        &self.wire_bytes
    }
}
