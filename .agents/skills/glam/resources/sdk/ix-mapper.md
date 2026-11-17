# SDK: ix-mapper

GLAM vaults execute DeFi operations through integration programs that enforce access control and policy checks. This SDK transforms standard Solana instructions into their GLAM-proxied equivalents by remapping accounts and instruction discriminators according to predefined mapping configurations.

## Installation

```bash
npm install @glamsystems/ix-mapper
```

## Usage

```typescript
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { getVaultPda, mapToGlamIx } from "@glamsystems/ix-mapper";

// GLAM state PDA (identifies the vault)
const glamState = new PublicKey("...");

// Signer should be the vault owner or a delegate
const glamSigner = new PublicKey("...");

// The vault PDA is derived from the state PDA internally
const glamVault = getVaultPda(glamState);

// Build a standard Solana instruction as if signing from the vault PDA
const transferIx = SystemProgram.transfer({
  fromPubkey: glamVault,
  toPubkey: recipient,
  lamports,
});

// Transform to a GLAM proxy instruction
const glamInstruction = mapToGlamIx(transferIx, glamState, glamSigner);

// glamInstruction can now be added to a transaction
```

### Exceptions

Do **not** use `ix-mapper` for:

- Jupiter swap instructions
- SOL wrapping or unwrapping instructions (`WSOL` flow)

These should be passed through as regular instructions and handled outside `mapToGlamIx(...)`.

### Staging Environment

To use staging program deployments, pass `staging = true`:

```typescript
const glamInstruction = mapToGlamIx(transferIx, glamState, glamSigner, true);
```

## API

### `mapToGlamIx(ix, glamState, glamSigner, staging?)`

Transforms a standard Solana `TransactionInstruction` into a GLAM proxy instruction.

**Parameters:**

| Parameter    | Type                     | Description                                       |
| ------------ | ------------------------ | ------------------------------------------------- |
| `ix`         | `TransactionInstruction` | The original Solana instruction to transform      |
| `glamState`  | `PublicKey`              | The GLAM state PDA that identifies the vault      |
| `glamSigner` | `PublicKey`              | The vault owner or delegate signing the operation |
| `staging`    | `boolean` (optional)     | Use staging program IDs (default: `false`)        |

**Returns:** `TransactionInstruction | null` - The transformed instruction, or `null` if the program/instruction is not supported or does not require remapping.

### `getVaultPda(statePda, staging?)`

Derives the vault PDA from a state PDA.

### `getIntegrationAuthority(integrationProgram)`

Derives the integration authority PDA for a given proxy program.

## How It Works

```mermaid
flowchart TD
    subgraph input["Standard Solana Instruction"]
        direction LR
        I1["programId: TargetProgram"]
        I2["disc: src_discriminator"]
        I3["accounts: [user, market, vault, ...]"]
        I4["data: payload"]
    end

    input -->|"mapToGlamIx(ix, glamState, glamSigner)"| lookup

    lookup["Lookup mapping config by programId"] --> match
    match["Match instruction by src_discriminator"] --> transform

    subgraph transform["Build Proxy Instruction"]
        direction LR
        subgraph disc["Discriminator"]
            T1["src_disc --> dst_disc"]
        end
        subgraph accounts["Accounts"]
            direction TB
            T2["Inject dynamic accounts<br/>state, vault, signer, integration_authority"]
            T3["Add static accounts from config"]
            T4["Reorder original accounts via index_map<br/>drop accounts where index = -1"]
            T5["Append remaining accounts as-is"]
            T2 --> T3 --> T4 --> T5
        end
    end

    transform --> output

    subgraph output["GLAM Proxy Instruction"]
        direction LR
        O1["programId: ext_integration"]
        O2["disc: dst_discriminator"]
        O3["accounts: [state, vault, signer, auth, ..., user, market, ...]"]
        O4["data: payload (unchanged)"]
    end
```
