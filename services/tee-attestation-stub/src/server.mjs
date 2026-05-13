/**
 * TEE attestation stub — see README.md and ../../DEVELOPER.md
 */
import dotenv from 'dotenv';
import express from 'express';
import crypto from 'crypto';
import { ethers } from 'ethers';

dotenv.config();

const PORT = Number(process.env.PORT ?? '8787');
const RPC_URL = process.env.RPC_URL;
const REGISTRY = process.env.PROVIDER_REGISTRY_ADDRESS;
const ADMIN_PK = process.env.ADMIN_PRIVATE_KEY?.startsWith('0x')
  ? process.env.ADMIN_PRIVATE_KEY
  : process.env.ADMIN_PRIVATE_KEY
    ? `0x${process.env.ADMIN_PRIVATE_KEY}`
    : undefined;

/** @typedef {{ challengeId: string, message: string, expiresAt: number }} Challenge */
/** @type {Map<string, Challenge>} */
const challenges = new Map();

const CHALLENGE_TTL_MS = 10 * 60 * 1000;

const registryAbi = [
  'function attestationService() view returns (address)',
  'function nodeOperator(bytes32 nodeId) view returns (address)',
  'function setTEEProof(bytes32 nodeId, bytes32 teeReportHash)',
];

/**
 * @param {unknown} raw
 * @returns {string | null} normalized 0x + 64 hex (bytes32)
 */
function parseNodeIdBytes32(raw) {
  if (typeof raw !== 'string') return null;
  const s = raw.trim();
  if (!ethers.isHexString(s, 32)) return null;
  return ethers.zeroPadValue(s, 32);
}

async function bootstrap() {
  if (!RPC_URL) throw new Error('RPC_URL is required');
  if (!REGISTRY || !ethers.isAddress(REGISTRY)) throw new Error('PROVIDER_REGISTRY_ADDRESS must be set to a contract address');
  if (!ADMIN_PK) throw new Error('ADMIN_PRIVATE_KEY is required');

  const provider = new ethers.JsonRpcProvider(RPC_URL);
  const wallet = new ethers.Wallet(ADMIN_PK, provider);

  const registry = new ethers.Contract(REGISTRY, registryAbi, wallet);

  try {
    const onChainAttestation = await registry.attestationService();
    const adminAddr = wallet.address;
    if (onChainAttestation.toLowerCase() !== adminAddr.toLowerCase()) {
      console.warn(
        `[stub] ADMIN wallet ${adminAddr} !== registry.attestationService() ${onChainAttestation}; setTEEProof will revert on-chain.`,
      );
    }
  } catch {
    console.warn('[stub] Could not read attestationService(); ensure RPC_URL and PROVIDER_REGISTRY_ADDRESS');
  }

  return { wallet, registry };
}

/** @type {Awaited<ReturnType<typeof bootstrap>>} */
const ctx = await bootstrap();

function newChallengeId() {
  return ethers.hexlify(crypto.randomBytes(16));
}

function purgeExpiredChallenges() {
  const now = Date.now();
  for (const [id, ch] of challenges.entries()) {
    if (ch.expiresAt <= now) challenges.delete(id);
  }
}

setInterval(purgeExpiredChallenges, 60_000).unref();

const app = express();
app.use(express.json({ limit: '2mb' }));

app.get('/health', (_req, res) => {
  res.json({ ok: true });
});

app.get('/v1/challenge', (_req, res) => {
  purgeExpiredChallenges();
  const challengeId = newChallengeId();
  const entropy = ethers.hexlify(crypto.randomBytes(16));
  const message = `sparkl-attest:${challengeId}:${entropy}`;
  const expiresAt = Date.now() + CHALLENGE_TTL_MS;
  challenges.set(challengeId, { challengeId, message, expiresAt });
  res.json({ challengeId, message, expiresAt });
});

app.post('/v1/attest', async (req, res) => {
  purgeExpiredChallenges();
  try {
    const nodeIdRaw = req.body?.nodeId ?? req.body?.providerAddress;
    const reportRaw = req.body?.report;
    const challengeId = req.body?.challengeId;
    const signatureRaw = req.body?.signature;

    if (!nodeIdRaw || !reportRaw || !challengeId || !signatureRaw) {
      res.status(400).json({
        ok: false,
        error: 'nodeId (bytes32 hex), report (hex), challengeId, and signature required',
      });
      return;
    }

    const nodeId = parseNodeIdBytes32(nodeIdRaw);
    if (!nodeId) {
      res.status(400).json({
        ok: false,
        error: 'nodeId must be 0x-prefixed 32-byte hex (bytes32), matching registerNode',
      });
      return;
    }

    let report;
    try {
      report = ethers.getBytes(reportRaw);
    } catch {
      res.status(400).json({ ok: false, error: 'report must be 0x-prefixed hex bytes' });
      return;
    }

    const stored = challenges.get(challengeId);
    if (!stored || stored.expiresAt <= Date.now()) {
      res.status(401).json({ ok: false, error: 'unknown or expired challengeId' });
      return;
    }

    let signerFromSig;
    try {
      signerFromSig = ethers.verifyMessage(stored.message, signatureRaw);
    } catch {
      res.status(401).json({ ok: false, error: 'invalid signature' });
      return;
    }

    let operatorOnChain;
    try {
      operatorOnChain = await ctx.registry.nodeOperator(nodeId);
    } catch (e) {
      console.error('[stub] nodeOperator call', e);
      res.status(502).json({ ok: false, error: 'nodeOperator registry read failed' });
      return;
    }

    if (!operatorOnChain || operatorOnChain === ethers.ZeroAddress) {
      res.status(400).json({ ok: false, error: 'node not registered (nodeOperator is zero)' });
      return;
    }

    if (ethers.getAddress(signerFromSig) !== ethers.getAddress(operatorOnChain)) {
      res.status(401).json({
        ok: false,
        error: 'signature must recover to registry nodeOperator(nodeId)',
      });
      return;
    }

    challenges.delete(challengeId);

    const teeReportHash = ethers.keccak256(report);
    const tx = await ctx.registry.setTEEProof(nodeId, teeReportHash);
    const receipt = await tx.wait();

    res.json({
      ok: true,
      teeReportHash,
      txHash: receipt?.hash ?? tx.hash,
      blockNumber: receipt?.blockNumber,
    });
  } catch (e) {
    console.error('[stub] /v1/attest', e);
    const msg = e?.shortMessage || e?.reason || e?.message || String(e);
    res.status(502).json({ ok: false, error: msg });
  }
});

app.listen(PORT, () => {
  console.log(`tee-attestation-stub listening on :${PORT} (admin=${ctx.wallet.address})`);
});
