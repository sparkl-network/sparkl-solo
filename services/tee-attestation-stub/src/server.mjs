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

async function bootstrap() {
  if (!RPC_URL) throw new Error('RPC_URL is required');
  if (!REGISTRY || !ethers.isAddress(REGISTRY)) throw new Error('PROVIDER_REGISTRY_ADDRESS must be set to a contract address');
  if (!ADMIN_PK) throw new Error('ADMIN_PRIVATE_KEY is required');

  const provider = new ethers.JsonRpcProvider(RPC_URL);
  const wallet = new ethers.Wallet(ADMIN_PK, provider);

  const abi = ['function attestationService() view returns (address)', 'function setTEEProof(address provider, bytes32 teeReportHash)'];
  const registry = new ethers.Contract(REGISTRY, abi, wallet);

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
    const providerAddressRaw = req.body?.providerAddress;
    const reportRaw = req.body?.report;
    const challengeId = req.body?.challengeId;
    const signatureRaw = req.body?.signature;

    if (!providerAddressRaw || !reportRaw || !challengeId || !signatureRaw) {
      res.status(400).json({
        ok: false,
        error: 'providerAddress, report (hex), challengeId, and signature required',
      });
      return;
    }

    let providerAddr;
    try {
      providerAddr = ethers.getAddress(providerAddressRaw);
    } catch {
      res.status(400).json({ ok: false, error: 'invalid providerAddress' });
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

    if (ethers.getAddress(signerFromSig) !== providerAddr) {
      res.status(401).json({ ok: false, error: 'signature does not match providerAddress' });
      return;
    }

    challenges.delete(challengeId);

    const teeReportHash = ethers.keccak256(report);
    const tx = await ctx.registry.setTEEProof(providerAddr, teeReportHash);
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
