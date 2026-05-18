// TEE quote verification pipeline
//
// Parses TEE quotes (SGX/EPID, SGX/ECDSA, TDX, SEV-SNP, SEV-ES, Nitro)
// and validates them against NRAS root of trust.
//
// The verification pipeline:
// 1. Parse the raw quote bytes to extract the report body
// 2. Validate the quote signature against the PCK/DCAP chain (SGX) or
//    equivalent root of trust for the TEE type
// 3. Extract the MRENCLAVE/MRCONFIGID (or equivalent) from the report
// 4. Compare against the expected MRENCLAVE for the provider's code
// 5. Return an AttestationResult with the verification outcome

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use super::nras::AttestationResponse;
use super::quote::{Quote, QuoteType};

// ---------------------------------------------------------------------------
// Verification result types
// ---------------------------------------------------------------------------

/// The level of trust established by verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Tier A: TEE-verified (NRAS or equivalent attestation service)
    TierA,
    /// Tier B: Software-only attestation (no TEE)
    TierB,
}

impl std::fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrustLevel::TierA => write!(f, "tier-a"),
            TrustLevel::TierB => write!(f, "tier-b"),
        }
    }
}

/// Result of TEE quote verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationResult {
    pub trust_level: TrustLevel,
    pub tee_report_hash: String,
    pub mrenclave: Option<String>,
    pub verified: bool,
    pub cert_chain_len: usize,
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AttestationResult {
    /// Create a successful result.
    pub fn success(
        trust_level: TrustLevel,
        tee_report_hash: String,
        mrenclave: Option<String>,
        cert_chain_len: usize,
        provider_id: Option<String>,
    ) -> Self {
        Self {
            trust_level,
            tee_report_hash,
            mrenclave,
            verified: true,
            cert_chain_len,
            provider_id,
            error: None,
        }
    }

    /// Create a failed result.
    pub fn failure(trust_level: TrustLevel, error: String, provider_id: Option<String>) -> Self {
        Self {
            trust_level,
            tee_report_hash: String::new(),
            mrenclave: None,
            verified: false,
            cert_chain_len: 0,
            provider_id,
            error: Some(error),
        }
    }

    /// Compute the SHA-256 hash of the tee_report_hash for on-chain use.
    pub fn report_hash_bytes(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.tee_report_hash.as_bytes());
        hasher.finalize().into()
    }
}

// ---------------------------------------------------------------------------
// Quote parser
// ---------------------------------------------------------------------------

/// Parse a raw TEE quote into a structured Quote.
pub fn parse_quote(raw_quote: &[u8], quote_type: QuoteType) -> Result<Quote> {
    // Validate minimum quote size
    if raw_quote.len() < 64 {
        bail!(
            "quote too small: {} bytes (expected >= 64)",
            raw_quote.len()
        );
    }

    let quote = match quote_type {
        QuoteType::SgxEpid | QuoteType::SgxEcdsa => parse_sgx_quote(raw_quote)?,
        QuoteType::Tdx => parse_tdx_quote(raw_quote)?,
        QuoteType::SevSnp | QuoteType::SevEs => parse_sev_quote(raw_quote)?,
        QuoteType::Nitro => parse_nitro_quote(raw_quote)?,
        QuoteType::Unknown => bail!("cannot parse Unknown quote_type"),
    };

    debug!(
        quote_type = %quote_type,
        quote_size = raw_quote.len(),
        mrenclave = ?quote.mrenclave,
        "parsed TEE quote"
    );

    Ok(quote)
}

/// Parse an SGX quote (EPID or ECDSA format).
///
//  SGX quote layout (simplified):
//    - quote version (u16)
//    - group type (u16)
//    - attestation key type (u16)
//    - quote format (u16)
//    - signer version (u8)
//    - quote type (u8)
//    - platform info (u64)
//    - reserved (u64)
//    - enclave hash (MRENCLAVE, 32 bytes)
//    - ... (more fields)
fn parse_sgx_quote(raw: &[u8]) -> Result<Quote> {
    if raw.len() < 64 {
        bail!("SGX quote too small: {} bytes", raw.len());
    }

    let version = u16::from_le_bytes([raw[0], raw[1]]);
    let key_type = u16::from_le_bytes([raw[2], raw[3]]);

    let mrenclave = if raw.len() >= 80 {
        let mut mren = [0u8; 32];
        mren.copy_from_slice(&raw[48..80]);
        Some(hex::encode(mren))
    } else {
        None
    };

    let qt = if key_type == 1 {
        QuoteType::SgxEpid
    } else {
        QuoteType::SgxEcdsa
    };
    let mut quote = Quote::new(qt, raw.to_vec());
    quote.version = version;
    quote.mrenclave = mrenclave;
    quote.signer_id = None;
    quote.platform_info = Some(u64::from_le_bytes([
        raw[8], raw[9], raw[10], raw[11], raw[12], raw[13], raw[14], raw[15],
    ]));
    quote.reserved = None;
    quote.raw_size = raw.len();
    Ok(quote)
}

/// Parse a TDX quote (DCAP format).
///
//  TDX quote layout (simplified):
//    - TDINFO structure
//    - REPORT body (includes MRENCLAVE, MRSEAM, etc.)
//    - MAC/signature
fn parse_tdx_quote(raw: &[u8]) -> Result<Quote> {
    if raw.len() < 128 {
        bail!("TDX quote too small: {} bytes", raw.len());
    }

    let mrenclave = if raw.len() >= 128 {
        let mut mren = [0u8; 32];
        mren.copy_from_slice(&raw[96..128]);
        Some(hex::encode(mren))
    } else {
        None
    };

    let mut quote = Quote::new(QuoteType::Tdx, raw.to_vec());
    quote.mrenclave = mrenclave;
    quote.signer_id = None;
    quote.platform_info = Some(u64::from_le_bytes([
        raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
    ]));
    quote.reserved = None;
    quote.version = 0;
    quote.raw_size = raw.len();
    Ok(quote)
}

/// Parse a SEV-SNP / SEV-ES quote.
///
//  SEV-SNP quote layout (simplified):
//    - header
//    - report_data (32 bytes)
//    - measurement (48 bytes)
//    - host_data (32 bytes)
//    - ID key digest (32 bytes)
//    - policy (8 bytes)
//    - family ID (16 bytes)
//    - guest_svn (8 bytes)
//    - signature
fn parse_sev_quote(raw: &[u8]) -> Result<Quote> {
    if raw.len() < 64 {
        bail!("SEV quote too small: {} bytes", raw.len());
    }

    let mrenclave = if raw.len() >= 112 {
        let mut meas = [0u8; 32];
        meas.copy_from_slice(&raw[32..64]);
        Some(hex::encode(meas))
    } else {
        None
    };

    let mut quote = Quote::new(QuoteType::SevSnp, raw.to_vec());
    quote.mrenclave = mrenclave;
    quote.signer_id = None;
    quote.platform_info = Some(u64::from_le_bytes([
        raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
    ]));
    quote.reserved = None;
    quote.version = 0;
    quote.raw_size = raw.len();
    Ok(quote)
}

/// Parse a AWS Nitro TEE quote.
///
//  Nitro quote layout (simplified):
//    - header
//    - report body (includes measurement hash)
//    - signature
fn parse_nitro_quote(raw: &[u8]) -> Result<Quote> {
    if raw.len() < 64 {
        bail!("Nitro quote too small: {} bytes", raw.len());
    }

    let mrenclave = if raw.len() >= 48 {
        let mut meas = [0u8; 32];
        meas.copy_from_slice(&raw[16..48]);
        Some(hex::encode(meas))
    } else {
        None
    };

    let mut quote = Quote::new(QuoteType::Nitro, raw.to_vec());
    quote.mrenclave = mrenclave;
    quote.signer_id = None;
    quote.platform_info = None;
    quote.reserved = None;
    quote.version = 0;
    quote.raw_size = raw.len();
    Ok(quote)
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// Verify a parsed TEE quote against NRAS-returned attestation data.
///
//  The verification process:
//  1. The quote's MRENCLAVE (or equivalent) is hashed
//  2. The hash is compared against the NRAS-verified tee_report_hash
//  3. The certificate chain is validated (done in nras.rs)
//  4. The result is returned with the appropriate trust level
pub fn verify_attestation(
    raw_quote: &[u8],
    quote_type: QuoteType,
    nras_response: &AttestationResponse,
    provider_id: Option<String>,
) -> Result<AttestationResult> {
    // Parse the quote
    let quote = match parse_quote(raw_quote, quote_type) {
        Ok(q) => q,
        Err(e) => {
            return Ok(AttestationResult::failure(
                TrustLevel::TierB,
                e.to_string(),
                provider_id.clone(),
            ));
        }
    };

    // Check that NRAS returned a report hash
    let tee_hash = match nras_response.tee_report_hash.as_ref() {
        Some(h) => h,
        None => {
            return Ok(AttestationResult::failure(
                TrustLevel::TierB,
                "NRAS did not return tee_report_hash".to_string(),
                provider_id.clone(),
            ));
        }
    };

    // Compute local hash of the MRENCLAVE (or equivalent)
    let local_hash = match &quote.mrenclave {
        Some(mren) => {
            let mut hasher = Sha256::new();
            hasher.update(hex::decode(mren).unwrap_or_default());
            hex::encode(hasher.finalize())
        }
        None => {
            // No MRENCLAVE — fall back to Tier B
            warn!("no MRENCLAVE in quote; downgrading to Tier B");
            return Ok(AttestationResult::failure(
                TrustLevel::TierB,
                "no MRENCLAVE in quote".to_string(),
                provider_id,
            ));
        }
    };

    // Compare with NRAS-verified hash
    let verified = local_hash == *tee_hash;

    let trust_level = if verified {
        TrustLevel::TierA
    } else {
        warn!(
            local = %local_hash,
            nras = %tee_hash,
            "MRENCLAVE hash mismatch — downgrading to Tier B"
        );
        TrustLevel::TierB
    };

    Ok(AttestationResult::success(
        trust_level,
        tee_hash.clone(),
        quote.mrenclave,
        nras_response.cert_chain.as_ref().map_or(0, |c| c.len()),
        provider_id,
    ))
}

/// Generate a deterministic MRENCLAVE hash from known good code bytes.
/// Used by providers to compute their expected MRENCLAVE for comparison.
pub fn compute_expected_mrenclave(code: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code);
    hex::encode(hasher.finalize())
}

/// Validate an MRENCLAVE against an expected value.
pub fn validate_mrenclave(provided: &str, expected: &str) -> bool {
    // Both should be hex-encoded SHA-256 hashes
    if provided.len() != 64 || expected.len() != 64 {
        return false;
    }
    provided == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sgx_quote() {
        // 128-byte synthetic SGX quote; `key_type` at bytes [2..4] le = 1 => EPID interpretion in parser
        let mut raw = vec![0u8; 128];
        raw[2..4].copy_from_slice(&1u16.to_le_bytes());
        let quote = parse_sgx_quote(&raw).unwrap();
        assert_eq!(quote.quote_type, QuoteType::SgxEpid);
        assert!(quote.mrenclave.is_some());
        assert_eq!(quote.mrenclave.as_ref().unwrap().len(), 64); // hex
    }

    #[test]
    fn test_parse_tdx_quote() {
        let raw = vec![0u8; 256];
        let quote = parse_tdx_quote(&raw).unwrap();
        assert_eq!(quote.quote_type, QuoteType::Tdx);
        assert!(quote.mrenclave.is_some());
    }

    #[test]
    fn test_parse_sev_quote() {
        let raw = vec![0u8; 128];
        let quote = parse_sev_quote(&raw).unwrap();
        assert_eq!(quote.quote_type, QuoteType::SevSnp);
        assert!(quote.mrenclave.is_some());
    }

    #[test]
    fn test_parse_nitro_quote() {
        let raw = vec![0u8; 128];
        let quote = parse_nitro_quote(&raw).unwrap();
        assert_eq!(quote.quote_type, QuoteType::Nitro);
        assert!(quote.mrenclave.is_some());
    }

    #[test]
    fn test_parse_quote_too_small() {
        let raw = vec![0u8; 32];
        assert!(parse_quote(&raw, QuoteType::SgxEpid).is_err());
    }

    #[test]
    fn test_validate_mrenclave() {
        let hash = "a".repeat(64);
        assert!(validate_mrenclave(&hash, &hash));
        assert!(!validate_mrenclave(&hash, "b".repeat(64).as_str()));
        assert!(!validate_mrenclave("short", &hash));
    }

    #[test]
    fn test_compute_expected_mrenclave() {
        let code = b"hello world";
        let hash = compute_expected_mrenclave(code);
        assert_eq!(hash.len(), 64);

        // Same code should produce same hash
        assert_eq!(hash, compute_expected_mrenclave(code));
        // Different code should produce different hash
        assert_ne!(hash, compute_expected_mrenclave(b"goodbye world"));
    }

    #[test]
    fn test_trust_level_display() {
        assert_eq!(TrustLevel::TierA.to_string(), "tier-a");
        assert_eq!(TrustLevel::TierB.to_string(), "tier-b");
    }

    #[test]
    fn test_attestation_result_hash() {
        let result =
            AttestationResult::success(TrustLevel::TierA, "test_hash".to_string(), None, 2, None);
        let bytes = result.report_hash_bytes();
        assert_eq!(bytes.len(), 32);
    }
}
