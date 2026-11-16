# Phase 6 — Solana Transaction V1 and SIMD-0385 Audit

## 6.1 Version Support Status

### 6.1.1 Current Supported Formats
The codebase currently supports only **Legacy** and **V0** transaction formats. There is **no V1 (SIMD-0385) support** in the current implementation.

**Evidence**:
- No `V1` references in any crate source files
- `template.rs` constructs and mutates V0-format transactions with fixed offsets
- `arb-tx/src/lib.rs` builds instructions for V0 protocols (Kamino, Jupiter, Raydium, Meteora, Orca, Pump.fun, PumpSwap)
- No version prefix, `TransactionConfigMask`, or lifetime specifier handling
- `Message` construction uses V0 header format (65-byte offset, no version byte)

### 6.1.2 Cluster Support Status
- **Legacy**: Historically supported, being phased out
- **V0**: Actively supported on all current Solana clusters (mainnet-beta, testnet, devnet)
- **V1**: Feature flag not activated in this codebase; would require `feature_set` activation and message format changes

**Directive requirement**: "Identify the exact supported formats: Legacy, V0, V1. Verify actual cluster support and active runtime feature status."

**Finding**: V1 support must be tested against the current runtime and official transaction format. Currently, this is **not verifiable** because the code does not implement V1.

## 6.2 V1 Message Construction (Required Implementation)

If V1 support is to be added, the following must be implemented:

### 6.2.1 V1 Message Format Fields

| Field | Description | Requirements |
|---|---|---|
| **Version prefix** | First byte of message | Must be `0x01` for V1; must match runtime-activated feature set |
| **Header** | Contains address count, instruction count | Must be present and valid; layout differs from V0 |
| **TransactionConfigMask** | Bitmask indicating which config values are set | Each set bit must have a corresponding config value; no assumed fixed count |
| **Lifetime specifier** | Transaction lifetime (optional) | If present, defines expiration; validated against `lastValidBlockHeight` |
| **Instruction count** | Number of instructions in the message | Must match actual instruction count; validated against account/sig limits |
| **Address count** | Number of addresses in the message header | Must match address count in header; excess/deficient addresses → reject |
| **ConfigValues** | Configuration values indexed by mask bits | Positions and count depend on mask bit pattern; must validate consistency |
| **Instruction headers** | 8-byte program_id index + data | Each instruction references program by index into address table |
| **Instruction payloads** | Instruction-specific data | Discriminator + args; must match program's expected instruction format |
| **Signatures** | Transaction signatures (up to 1) | Single Ed25519 signature for V1; verified against message bytes |
| **Signature ordering** | Order of signatures in message | V1 uses single signature at fixed position; no array like V0 potentially |
| **Message signing bytes** | Bytes that are signed | Must be exactly the message bytes (version + header + config + instructions + addresses) |
| **Endianness** | Byte order for multi-byte values | Little-endian (Solana native) |
| **Length validation** | Total message byte length | Must not exceed runtime limits; truncated payloads → reject |
| **No trailing data** | No extra bytes after the structured message | Must be exactly the prescribed length; trailing bytes → reject |
| **Sanitization constraints** | Constraints on valid values | See Section 6.4 for current runtime limits |

### 6.2.2 ConfigValues Validation

**Critical**: "Verify all ConfigValues positions using the actual set bits in the mask."

- The mask uses set bits to indicate which config values are present
- Each set bit position maps to a specific ConfigValue index
- **Do not assume** that configuration values always have a fixed count or ordering independent of the mask
- Must read the mask bit by bit and validate each present config value

**Example mask positions** ( illustrative, actual values depend on SIMD-0385 final spec):
- Bit 0 set → `max_units` (compute unit limit)
- Bit 1 set → `units_per_payment` (deprecated/removed in final spec)
- Bit 2 set → `authorization_required_flag`
- etc.

**Implementation requirement**: Parse the mask, iterate over set bits, validate each config value's position and range.

### 6.2.3 Resource Configuration

**V1-specific resource configuration must be implemented correctly**:

| Resource | Description | Valid Value Range | Behavior |
|---|---|---|---|
| **Requested compute-unit limit** | Maximum CU the transaction may consume | `1 ..= MAX_COMPUTE_UNIT_LIMIT (350,000)` | Exceeding → transaction rejected |
| **Requested accounts data size** | Total data size of loaded accounts | Must not exceed runtime limit (per-account + total) | Exceeding → sanitization failure |
| **Requested heap size** | Heap memory for runtime | Must be within runtime limit | Exceeding → reject |
| **Priority fee** | Micro-lamports per CU for priority | `0 ..= MAX_PRIORITY_FEE` | 0 allowed (no priority); must be justifiable by profit model |
| **Valid value ranges** | All values must be in approved ranges | Enforced at build/sign time | Out-of-range → message rejected |

**Do not append legacy/V0 ComputeBudget instructions as a substitute for V1 message configuration**. Keep legacy/V0 and V1 resource handling separate.

### 6.2.4 Runtime Limits Verification

Build a protocol capability registry and verify current limits:

| Limit | Current Value | Source |
|---|---|---|
| **Transaction bytes** | ~1232 (MTU) or as configured | Solana runtime packet limit |
| **Accounts** | Varies by transaction; typically 0..=255 | Per-transaction account count limit |
| **Signatures** | 1 (V1) or up to ~32 (V0) | V1 has single signature |
| **Instructions** | Typically 0..=255 per transaction | Per-transaction instruction count limit |
| **Compute units** | `MAX_COMPUTE_UNIT_LIMIT = 350,000` | Solana runtime cap (from `arb-constants`) |
| **Loaded account data** | Per-account limit + total | Runtime-enforced; varies by account size |
| **Heap** | Configurable, within runtime max | Runtime-provided heap for account data |
| **Instruction account indexes** | 0..=N (loaded program indexes) | Each instruction's account indexes must be < loaded account count |

**The transaction builder must reject invalid messages before signing**. Every field must be validated.

### 6.2.5 Differential Testing

Test the Rust implementation against:

1. **Official Agave implementation** (Solana reference client)
2. **Official SIMD-0385 specification** (the formal proposal and approved changes)
3. **Independent reference implementations** where available

**Use golden serialization fixtures**: Serialize known-good V0 and V1 transactions and compare against expected output.

**Test malformed masks, missing config values, excessive lengths, bad indexes, invalid signatures and truncated payloads**:

- Mask with bits set beyond valid range → reject
- Missing required config values (for set mask bits) → reject
- Excessive lengths → truncate/reject
- Bad instruction account indexes → reject (index >= loaded_account_count)
- Invalid signatures → signature verification fails
- Truncated payloads → deserialize error

**Test actual signing and verification**:

- Generate V1 transaction, sign, verify signature against message bytes
- Deserialize signed transaction, re-verify all fields
- End-to-end: build → sign → simulate → send → confirm (in simulation only)

**Do not enable V1 live execution unless the complete build/sign/simulate/send/confirm path has passed**.

## 6.3 V1 Implementation Status

| Component | Status | Notes |
|---|---|---|
| V1 message format parsing | Not implemented | Must be built from scratch |
| V1 message construction | Not implemented | Must be built from scratch |
| ConfigValues mask parsing | Not implemented | Requires SIMD-0385 spec study |
| V1 resource configuration | Not implemented | Separate from V0 handling |
| Runtime limit registry | Not implemented | Must be built from current feature set |
| Differential testing fixtures | Not implemented | Requires Agave SDK access |
| V1 simulation support | Not implemented | Would need LiteSVM or RPC updates |
| V1 signing/verification | Not implemented | New crypto path required |
| **V1 live execution** | **NOT APPLICABLE** | **Must not activate until all above pass** |

## 6.4 Required V1 Implementation Plan

### 6.4.1 Phase 1: Study SIMD-0385

1. Read the official SIMD-0385 proposal and approved changes
2. Identify the exact message format differences from V0
3. Determine the activated feature set required for V1
4. Identify all ConfigValues and their valid ranges
5. Understand the lifetime specifier and `lastValidBlockHeight` behavior

### 6.4.2 Phase 2: Message Parsing

1. Add V1 message parsing to `arb-tx` crate
2. Implement version byte detection (Legacy vs V0 vs V1)
3. Implement `TransactionConfigMask` bit parsing
4. Implement ConfigValues validation per mask bits
5. Implement lifetime specifier handling

### 6.4.3 Phase 3: Resource Configuration

1. Implement V1-specific compute unit limit handling
2. Implement V1 priority fee handling (separate from V0 ComputeBudget instructions)
3. Implement account count and data size validation
4. Implement heap size handling
5. Build protocol capability registry with current limits

### 6.4.4 Phase 4: Differential Testing

1. Build golden serialization fixtures (known-good V0 and V1 txs)
2. Test against Agave implementation where available
3. Test malformed inputs and error cases
4. Test signing and verification round-trip
5. Test simulation path (LiteSVM or RPC)

### 6.4.5 Phase 5: Controlled Activation

1. Keep V1 flags disabled by default
2. Add feature gate: `feature = "simd-0385"` or similar
3. Run full test suite with V1 enabled
4. Perform canary testing with simulated transactions only
5. Document any remaining limitations

### 6.4.6 Gate Requirements

**V1 must not be enabled for live execution until**:
- [ ] Full build/sign/simulate path passes for V1 messages
- [ ] Differential tests pass against reference implementations
- [ ] All runtime limits verified and enforced
- [ ] Signing/verification round-trip tested
- [ ] Malformed input handling tested and fail-closed
- [ ] Operator explicit authorization for V1 activation
- [ ] Canary testing with simulated transactions only
- [ ] No V1 transactions submitted to mainnet without explicit approval

## 6.5 Summary of Phase 6 Findings

### Correctness
- **V1 support does not exist** in the current codebase
- The codebase uses only V0 transaction format with fixed offsets
- No version prefix, no `TransactionConfigMask`, no lifetime specifier handling
- All transaction building assumes V0 format
- **This is a blocker for mainnet V1 deployment**

### Issues Identified
1. **No V1 message format support**: Must be implemented from scratch per SIMD-0385
2. **No V1 resource configuration**: CU limits, priority fees, account validation all V0-era
3. **No ConfigValues mask parsing**: Critical for V1 message validity
4. **No differential testing infrastructure**: No Agave/SIMD reference comparison
5. **No V1 simulation or signing**: Would require significant new code
6. **V1 live execution not authorized**: Must not activate until all above complete

### Required Actions
1. **Study SIMD-0385**: Read the official proposal and approved changes
2. **Implement V1 message parsing**: Version detection, ConfigMask, ConfigValues
3. **Implement V1 message construction**: Proper V1 header, resource config
4. **Build runtime limit registry**: Current activated feature limits
5. **Implement differential testing**: Against Agave or independent reference
6. **Implement V1 signing/verification**: New message bytes, signature format
7. **Add feature gate**: V1 behind feature flag, disabled by default
8. **Canary testing**: Simulated only, no mainnet until all gates pass

### Phase 6 Hardcoded Value Classification
(No new hardcoded values — V1 support must be implemented per spec, not hardcoded)

## 6.6 V1 Activation Prerequisites

The project cannot enable V1 support until:

**Technical prerequisites**:
- [ ] V1 message format fully implemented and tested
- [ ] ConfigValues mask parsing validated
- [ ] Resource configuration separate from V0
- [ ] Runtime limit registry built and verified
- [ ] Differential testing passes
- [ ] Signing/verification round-trip tested
- [ ] Malformed input handling tested (fail-closed)

**Policy prerequisites**:
- [ ] Operator explicit authorization for V1 activation
- [ ] Canary testing with simulated transactions completed
- [ ] Rollback procedure documented (V0 fallback)
- [ ] Monitoring updated for V1-specific metrics
- [ ] Security review of V1 code changes

**Never convert NOT VERIFIED into PASS** for V1 support.