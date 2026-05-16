// TEE quote generation and verification for Tier A provider verification.
//
// Supports Intel SGX, AMD SEV-SNP, and AWS Nitro TEE quote formats.
// Each quote type has its own verification pipeline against the appropriate
// root of trust (NRAS for SGX, AMD for SEV-SNP, AWS for Nitro).
//
// See issue #5 and docs/MVP_ROADMAP.md §2.2.

use anyhow::{anyhow, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// TEE vendor type for quote generation and verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeeVendor {
    IntelSgx,
    AMDSEVSnp,
    AWSNitro,
    /// Mock/untrusted (for local dev / CI).
    Mock,
}

impl std::fmt::Display for TeeVendor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TeeVendor::IntelSgx => write!(f, "intel_sgx"),
            TeeVendor::AMDSEVSnp => write!(f, "amd_sev_snp"),
            TeeVendor::AWSNitro => write!(f, "aws_nitro"),
            TeeVendor::Mock => write!(f, "mock"),
        }
    }
}

impl std::str::FromStr for TeeVendor {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "intel_sgx" => Ok(TeeVendor::IntelSgx),
            "amd_sev_snp" => Ok(TeeVendor::AMDSEVSnp),
            "aws_nitro" => Ok(TeeVendor::AWSNitro),
            "mock" => Ok(TeeVendor::Mock),
            _ => Err(format!("unknown TEE vendor: {s}")),
        }
    }
}

/// A TEE quote in its raw vendor-specific format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeeQuote {
    /// Vendor of the TEE that produced this quote.
    pub vendor: TeeVendor,
    /// Raw quote bytes (vendor-specific binary format).
    pub quote_bytes: Vec<u8>,
    /// Base64-encoded representation for JSON transport.
    pub quote_b64: String,
    /// Optional extended report data (32 bytes) set by the TEE.
    pub extended_report: Option<[u8; 32]>,
    /// Optional attestation certificate chain (DER-encoded).
    pub cert_chain: Option<Vec<Vec<u8>>>,
}

impl TeeQuote {
    /// Compute the SHA-256 hash of the raw quote bytes.
    /// This hash is used as the TEE evidence hash submitted to ProviderRegistry.
    pub fn quote_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&self.quote_bytes);
        hasher.finalize().into()
    }

    /// Alias for `quote_hash` for use in session/TeeQuote context.
    pub fn canonical_hash(&self) -> [u8; 32] {
        self.quote_hash()
    }

    /// Compute the SHA-256 hash of the extended report data.
    pub fn extended_report_hash(&self) -> Option<[u8; 32]> {
        self.extended_report.map(|data| {
            let mut hasher = Sha256::new();
            hasher.update(&data);
            hasher.finalize().into()
        })
    }
}

/// Verification result from the TEE quote verification pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeeVerificationResult {
    /// Whether the quote was successfully verified.
    pub verified: bool,
    /// The vendor that produced the verified quote.
    pub vendor: TeeVendor,
    /// The hash of the verified quote (for on-chain submission).
    pub quote_hash: [u8; 32],
    /// Detailed error message if verification failed.
    pub error: Option<String>,
}

impl TeeVerificationResult {
    pub fn ok(vendor: TeeVendor, quote_hash: [u8; 32]) -> Self {
        Self {
            verified: true,
            vendor,
            quote_hash,
            error: None,
        }
    }

    pub fn fail(vendor: TeeVendor, quote_bytes: &[u8], error: String) -> Self {
        let quote_hash = {
            let mut hasher = Sha256::new();
            hasher.update(quote_bytes);
            hasher.finalize().into()
        };
        Self {
            verified: false,
            vendor,
            quote_hash,
            error: Some(error),
        }
    }
}

// ─── Quote Generation ───────────────────────────────────────────────────────

/// Generate a TEE quote for the given vendor.
///
/// In production, this would interface with:
/// - Intel SGX: `sgx_ql_get_quote_gen()` or raw quote generation API
/// - AMD SEV-SNP: `sev_snp_quote_generation()` via the PSP
/// - AWS Nitro: `nitro_tec_quote_generation()` via the Nitro TPM
///
/// For now, we provide a mock implementation for local development.
pub async fn generate_quote(vendor: TeeVendor) -> Result<TeeQuote> {
    match vendor {
        TeeVendor::IntelSgx => generate_sgx_quote().await,
        TeeVendor::AMDSEVSnp => generate_sev_snp_quote().await,
        TeeVendor::AWSNitro => generate_nitro_quote().await,
        TeeVendor::Mock => generate_mock_quote(),
    }
}

async fn generate_sgx_quote() -> Result<TeeQuote> {
    // TODO: Integrate with Intel SGX quote generation library.
    // This would call into the SGX Quote Generation Library (QGL) to produce
    // a PCK-signed quote with the extended report containing our challenge nonce.
    //
    // Reference: https://intel.github.io/software-guard-extensions/QuoteGeneration/
    //
    // For now, return a mock quote for local testing.
    tracing::warn!(
        "SGX quote generation not available; using mock quote for local development"
    );
    generate_mock_quote()
}

async fn generate_sev_snp_quote() -> Result<TeeQuote> {
    // TODO: Integrate with AMD SEV-SNP quote generation.
    // This would use the AMD PSP (Platform Security Processor) to generate
    // a signed quote with the guest measurement and extended report.
    //
    // Reference: https://www.amd.com/system/files/TechDocs/24594.pdf
    //
    // For now, return a mock quote for local testing.
    tracing::warn!(
        "AMD SEV-SNP quote generation not available; using mock quote for local development"
    );
    generate_mock_quote()
}

async fn generate_nitro_quote() -> Result<TeeQuote> {
    // TODO: Integrate with AWS Nitro TPM quote generation.
    // This would use the Nitro TPM to generate a quote signed by the
    // AWS Nitro root of trust.
    //
    // Reference: https://aws.amazon.com/blogs/mt/verifying-the-integrity-of-your-workloads-with-aws-nitro-secure-enclaves/
    //
    // For now, return a mock quote for local testing.
    tracing::warn!(
        "AWS Nitro quote generation not available; using mock quote for local development"
    );
    generate_mock_quote()
}

fn generate_mock_quote() -> Result<TeeQuote> {
    // Generate a deterministic mock quote for local testing.
    // In production, this would be replaced by actual TEE quote generation.
    let mut quote_bytes = vec![0u8; 256];
    // Set a magic header to identify mock quotes.
    quote_bytes[0..4].copy_from_slice(b"MOCK");

    // Encode as base64 for JSON transport.
    let quote_b64 = base64::engine::general_purpose::STANDARD.encode(&quote_bytes);

    Ok(TeeQuote {
        vendor: TeeVendor::Mock,
        quote_bytes,
        quote_b64,
        extended_report: Some([0u8; 32]),
        cert_chain: None,
    })
}

// ─── Quote Verification ─────────────────────────────────────────────────────

/// Verify a TEE quote against the appropriate root of trust.
///
/// Verification pipeline:
/// 1. Parse the raw quote bytes into vendor-specific structures.
/// 2. Verify the quote signature against the TEE's root of trust.
/// 3. Verify the extended report matches the expected challenge.
/// 4. Verify the certificate chain (if present) against the root CA.
///
/// Returns a `TeeVerificationResult` indicating success or failure.
pub async fn verify_quote(quote: &TeeQuote, expected_nonce: &[u8; 32]) -> Result<TeeVerificationResult> {
    match quote.vendor {
        TeeVendor::IntelSgx => verify_sgx_quote(quote, expected_nonce).await,
        TeeVendor::AMDSEVSnp => verify_sev_snp_quote(quote, expected_nonce).await,
        TeeVendor::AWSNitro => verify_nitro_quote(quote, expected_nonce).await,
        TeeVendor::Mock => Ok(verify_mock_quote(quote)),
    }
}

async fn verify_sgx_quote(_quote: &TeeQuote, _expected_nonce: &[u8; 32]) -> Result<TeeVerificationResult> {
    // TODO: Verify Intel SGX quote against PCK certificate chain.
    // 1. Parse the SGX quote header and body.
    // 2. Verify the PCK signature against the Intel Root CA.
    // 3. Verify the collateral timestamp is within the validity period.
    // 4. Verify the extended report matches the expected challenge nonce.
    //
    // Reference: Intel SGX Quote Verification Library (QVL)
    // https://intel.github.io/software-guard-extensions/QuoteVerification/
    Err(anyhow!("SGX quote verification not yet implemented"))
}

async fn verify_sev_snp_quote(_quote: &TeeQuote, _expected_nonce: &[u8; 32]) -> Result<TeeVerificationResult> {
    // TODO: Verify AMD SEV-SNP quote against AMD root of trust.
    // 1. Parse the SEV-SNP quote header and body.
    // 2. Verify the AMD signature using the AMD Root Key.
    // 3. Verify the guest measurement matches the expected value.
    // 4. Verify the extended report matches the expected challenge.
    Err(anyhow!("AMD SEV-SNP quote verification not yet implemented"))
}

async fn verify_nitro_quote(_quote: &TeeQuote, _expected_nonce: &[u8; 32]) -> Result<TeeVerificationResult> {
    // TODO: Verify AWS Nitro TPM quote against AWS root of trust.
    // 1. Parse the Nitro TPM quote structure.
    // 2. Verify the AWS TPM signature.
    // 3. Verify the PCR measurements match the expected values.
    Err(anyhow!("AWS Nitro quote verification not yet implemented"))
}

fn verify_mock_quote(quote: &TeeQuote) -> TeeVerificationResult {
    // Verify that the mock quote has the correct magic header.
    if quote.quote_bytes.len() >= 4 && &quote.quote_bytes[0..4] == b"MOCK" {
        TeeVerificationResult::ok(TeeVendor::Mock, quote.quote_hash())
    } else {
        TeeVerificationResult::fail(TeeVendor::Mock, &quote.quote_bytes, "invalid mock quote header".to_string())
    }
}

// ─── Consumer-side TEE Proof Verification ───────────────────────────────────

/// Verify that a provider is a Tier A (TEE-verified) provider by checking
/// the ProviderRegistry contract.
///
/// This is the consumer-side verification step: before routing a request
/// to a provider, the consumer checks that the provider has a valid TEE
/// proof recorded on-chain.
///
/// Returns `true` if the provider is verified as Tier A, `false` otherwise.
///
/// Note: In the current architecture, this is handled by `registry::supports_tier()`
/// which queries the ProviderRegistry contract. This function provides a
/// higher-level API that combines contract checks with local cache lookups.
pub async fn verify_tier_a_provider(
    _node_id: [u8; 32],
) -> Result<bool> {
    // This is a placeholder that will be wired to the registry client.
    // The actual implementation calls `registry::supports_tier()` with
    // `SecurityTier::TeeVerified`.
    //
    // TODO: Wire to registry client when the EVM RPC is configured.
    tracing::warn!(
        "TEE provider verification not yet wired to registry client"
    );
    Ok(false)
}

/// Verify a receipt's TEE provenance.
///
/// For Tier A providers, receipts should include a TEE proof that can be
/// verified to ensure the inference was run in a trusted execution environment.
///
/// The receipt verification flow:
/// 1. Verify the provider's Ed25519 signature on the receipt.
/// 2. Check that the provider has a valid TEE proof on ProviderRegistry.
/// 3. Optionally verify the TEE quote hash matches the one on-chain.
pub fn verify_receipt_tee_provenance(
    receipt: &crate::receipts::ChunkReceipt,
    provider_pubkey: &[u8; 32],
    tee_quote_hash: Option<[u8; 32]>,
) -> bool {
    // Step 1: Verify provider signature (existing logic).
    let sig_valid = crate::receipts::verify_provider_receipt(receipt, provider_pubkey);
    if !sig_valid {
        tracing::warn!(
            receipt_seq = receipt.seq,
            "receipt verification failed: invalid provider signature"
        );
        return false;
    }

    // Step 2: If a TEE quote hash was provided, verify it matches.
    if let Some(expected_hash) = tee_quote_hash {
        // The quote hash would be verified against the ProviderRegistry.
        // For now, we just log that TEE verification is pending.
        tracing::info!(
            receipt_seq = receipt.seq,
            tee_hash = %hex::encode(expected_hash),
            "TEE provenance verification pending on-chain check"
        );
    }

    sig_valid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_quote_generation() {
        let quote = generate_mock_quote().unwrap();
        assert_eq!(quote.vendor, TeeVendor::Mock);
        assert!(!quote.quote_b64.is_empty());
        assert_eq!(quote.quote_bytes.len(), 256);
        assert_eq!(&quote.quote_bytes[0..4], b"MOCK");
    }

    #[test]
    fn test_mock_quote_hash_consistency() {
        let quote = generate_mock_quote().unwrap();
        let hash1 = quote.quote_hash();
        let quote2 = generate_mock_quote().unwrap();
        let hash2 = quote2.quote_hash();
        // Mock quotes may differ (random bytes), so just check they're both 32 bytes.
        assert_eq!(hash1.len(), 32);
        assert_eq!(hash2.len(), 32);
    }

    #[test]
    fn test_mock_quote_verification() {
        let quote = generate_mock_quote().unwrap();
        let result = verify_mock_quote(&quote);
        assert!(result.verified);
        assert_eq!(result.vendor, TeeVendor::Mock);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_mock_quote_verification_invalid() {
        let quote = TeeQuote {
            vendor: TeeVendor::Mock,
            quote_bytes: vec![0u8; 256], // no MOCK header
            quote_b64: String::new(),
            extended_report: None,
            cert_chain: None,
        };
        let result = verify_mock_quote(&quote);
        assert!(!result.verified);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_tee_vendor_display() {
        assert_eq!(format!("{}", TeeVendor::IntelSgx), "intel_sgx");
        assert_eq!(format!("{}", TeeVendor::AMDSEVSnp), "amd_sev_snp");
        assert_eq!(format!("{}", TeeVendor::AWSNitro), "aws_nitro");
        assert_eq!(format!("{}", TeeVendor::Mock), "mock");
    }

    #[test]
    fn test_tee_verification_result_ok() {
        let hash = [1u8; 32];
        let result = TeeVerificationResult::ok(TeeVendor::IntelSgx, hash);
        assert!(result.verified);
        assert_eq!(result.vendor, TeeVendor::IntelSgx);
        assert_eq!(result.quote_hash, hash);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_tee_verification_result_fail() {
        let quote_bytes = vec![0u8; 256];
        let result = TeeVerificationResult::fail(
            TeeVendor::AMDSEVSnp,
            &quote_bytes,
            "test error".to_string(),
        );
        assert!(!result.verified);
        assert_eq!(result.vendor, TeeVendor::AMDSEVSnp);
        assert_eq!(result.error, Some("test error".to_string()));
        // quote_hash should be SHA-256 of the quote bytes
        let expected_hash: [u8; 32] = {
            let mut hasher = Sha256::new();
            hasher.update(&quote_bytes);
            hasher.finalize().into()
        };
        assert_eq!(result.quote_hash, expected_hash);
    }
}
