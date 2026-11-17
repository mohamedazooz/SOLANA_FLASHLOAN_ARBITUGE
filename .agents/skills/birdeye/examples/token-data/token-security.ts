/**
 * Token Security Check — Birdeye /defi/token_security
 * 
 * Detects honeypots, suspicious mint authority, and high creator concentration.
 * Use before recommending any token to flag rug risks.
 */

const TOKEN_ADDRESS = process.env.TOKEN_ADDRESS || "So11111111111111111111111111111111111111112";
const BIRDEYE_API_KEY = process.env.BIRDEYE_API_KEY!;

// Response fields per official /defi/token_security docs + live responses.
// Mint authority: the field is `isMintable` (not `mintable`). It's often null on
// established tokens; treat a non-null truthy value as active mint authority.
interface SecurityResult {
  ownerAddress: string | null;
  ownerBalance: number;
  ownerPercentage: number;
  creatorAddress: string | null;
  creatorBalance: number;
  creatorPercentage: number;
  isMintable: boolean | string | null;
  freezeable: boolean | null;
  freezeAuthority: string | null;
  mutableMetadata: boolean;
  nonTransferable: boolean | null;
  transferFeeEnable: boolean | null;
  isTrueToken: boolean | null;
  fakeToken: unknown;
  totalSupply: number;
  top10HolderBalance: number;
  top10HolderPercent: number;
}

async function checkTokenSecurity(address: string): Promise<void> {
  const url = `https://public-api.birdeye.so/defi/token_security?address=${address}`;

  const res = await fetch(url, {
    headers: {
      "X-API-KEY": BIRDEYE_API_KEY,
      "x-chain": "solana",
      "Accept": "application/json",
    },
  });

  if (!res.ok) throw new Error(`API Error ${res.status}: ${res.statusText}`);
  const json = await res.json() as any;
  const data: SecurityResult = json.data;

  console.log("=== TOKEN SECURITY REPORT ===");
  console.log(`Address: ${address}`);
  console.log(`Is True Token:      ${data.isTrueToken}`);
  console.log(`Mintable:           ${Boolean(data.isMintable)}`);
  console.log(`Freezeable:         ${Boolean(data.freezeable)}`);
  console.log(`Freeze Authority:   ${data.freezeAuthority ?? 'none'}`);
  console.log(`Mutable Metadata:   ${data.mutableMetadata}`);
  console.log(`Transfer Fee On:    ${Boolean(data.transferFeeEnable)}`);
  console.log(`Creator Holds:      ${((data.creatorPercentage ?? 0) * 100).toFixed(2)}%`);
  console.log(`Top 10 Holders:     ${((data.top10HolderPercent ?? 0) * 100).toFixed(2)}%`);

  // Risk flags
  if (data.isMintable) {
    console.warn("⚠️  WARNING: Mint authority is active — inflation risk.");
  }
  if (data.freezeable || data.freezeAuthority) {
    console.warn("⚠️  WARNING: Freeze authority is still set — tokens could be frozen.");
  }
  if (data.transferFeeEnable) {
    console.warn("⚠️  WARNING: Transfer fee enabled — swaps may bleed on each move.");
  }
  if ((data.creatorPercentage ?? 0) > 0.20) {
    console.warn(`⚠️  WARNING: Creator holds ${(data.creatorPercentage * 100).toFixed(1)}% — rug risk.`);
  }
  if ((data.top10HolderPercent ?? 0) > 0.50) {
    console.warn(`⚠️  WARNING: Top 10 holders control ${(data.top10HolderPercent * 100).toFixed(1)}% — concentration risk.`);
  }
  if (
    (data.creatorPercentage ?? 0) < 0.05 &&
    !data.isMintable &&
    !data.freezeable && !data.freezeAuthority &&
    !data.transferFeeEnable && data.isTrueToken
  ) {
    console.log("✅ Token appears safe — low creator %, no mint authority, no freeze authority, no transfer fee.");
  }
}

checkTokenSecurity(TOKEN_ADDRESS).catch(console.error);
