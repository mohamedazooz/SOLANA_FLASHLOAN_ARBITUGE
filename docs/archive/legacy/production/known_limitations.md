> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Known Limitations & Platform Constraints

## 1. Operating System & Platform Nuances
- **Windows vs Linux**:
  - The project builds cleanly on both Windows and Linux.
  - Production deployments require Linux (Ubuntu 22.04/24.04) for optimal socket performance, io_uring, and systemd integration.
- **Let-Chains Feature**:
  - Requires Rust 1.99.1 (pinned) and Edition 2024.

## 2. Protocol Boundaries
- **Transaction V1 SIMD-0385**:
  - Requires Agave nodes with SIMD-0385 runtime support activated.
  - The engine seamlessly runs V0 with Address Lookup Tables whenever cluster support for V1 is unconfirmed.
- **RPC Quotas & Rate Limits**:
  - Yellowstone gRPC requires dedicated high-throughput plan (e.g. Helius Developer/Business tier).
  - Public/shared RPC nodes will experience dropped slots and are incompatible with production arbitrage.
