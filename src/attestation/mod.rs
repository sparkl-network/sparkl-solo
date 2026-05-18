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
pub mod runtime;
pub mod verify;

pub use nras::{AttestationRequest, AttestationResponse, NrasChallenge, NrasClient, NrasRootCert};
pub use quote::{Quote, QuoteType};
pub use verify::{
    compute_expected_mrenclave, parse_quote, validate_mrenclave, verify_attestation,
    AttestationResult, TrustLevel,
};

/// Maximum size for a raw TEE quote (SGX 2KB, TDX ~4KB, SEV-SNP ~8KB).
pub const MAX_QUOTE_SIZE: usize = 8192;

/// NRAS challenge nonce size in bytes.
pub const NRAS_NONCE_SIZE: usize = 32;

pub use runtime::{
    normalize_tee_report_hash, refresh_nras_tee_report_hash, truncate_hash_display, ttl_secs,
    unix_now_secs, NrasRuntimeState, NrasUiSnapshot,
};
