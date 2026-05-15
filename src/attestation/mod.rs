// TEE Attestation Module — NRAS production attestation flow
//
// This module handles:
// 1. TEE quote generation (SGX/TDX/SEV/Nitro)
// 2. NRAS (NVIDIA Remote Attestation Service) client for quote verification
// 3. Certificate chain validation against NRAS root of trust
// 4. Attestation challenge flow: provider ↔ NRAS ↔ ProviderRegistry
//
// See issue #4 for full requirements.

pub mod nras;
pub mod quote;
pub mod verify;

pub use nras::{NrasClient, AttestationRequest, AttestationResponse, NrasChallenge, NrasRootCert};
pub use quote::{Quote, QuoteType};
pub use verify::{AttestationResult, TrustLevel, parse_quote, verify_attestation, compute_expected_mrenclave, validate_mrenclave};

/// Maximum size for a raw TEE quote (SGX 2KB, TDX ~4KB, SEV-SNP ~8KB).
pub const MAX_QUOTE_SIZE: usize = 8192;

/// NRAS challenge nonce size in bytes.
pub const NRAS_NONCE_SIZE: usize = 32;
