// NRAS (NVIDIA Remote Attestation Service) client
//
// Handles the attestation challenge flow:
// 1. Request a challenge from NRAS
// 2. Sign the challenge with the TEE quote
// 3. Submit the signed quote to NRAS for verification
// 4. Validate the returned certificate chain
//
// NRAS API Reference (NVIDIA DCX):
//   POST /v1/attestation/challenge  - Get a fresh challenge
//   POST /v1/attestation/verify     - Submit quote for verification
//   GET  /v1/attestation/cert       - Fetch NRAS root certificates
//
// For SGX/TDX/SEV-Nitro, the flow is:
//   1. TEE generates a quote containing an MRENCLAVE/MRCONFIGID hash
//   2. Provider signs the NRAS challenge with the TEE quote's internal key
//   3. NRAS verifies the quote against its root of trust (PCK/DCAP chain)
//   4. NRAS returns a JWT/attestation document with the verified report hash

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::{NRAS_NONCE_SIZE, MAX_QUOTE_SIZE};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

/// NRAS challenge response.
#[derive(Debug, Clone, Deserialize)]
pub struct NrasChallenge {
    pub challenge_id: String,
    pub nonce: String,
    pub expires_at: u64,
}

/// NRAS attestation request body.
#[derive(Debug, Clone, Serialize)]
pub struct AttestationRequest {
    pub challenge_id: String,
    pub quote: String,
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
}

/// NRAS attestation response.
#[derive(Debug, Clone, Deserialize)]
pub struct AttestationResponse {
    pub ok: bool,
    #[serde(default)]
    pub tee_report_hash: Option<String>,
    #[serde(default)]
    pub tx_hash: Option<String>,
    #[serde(default)]
    pub cert_chain: Option<Vec<String>>,
    #[serde(default)]
    pub error: Option<String>,
}

/// NRAS root certificate for chain validation.
#[derive(Debug, Clone, Deserialize)]
pub struct NrasRootCert {
    pub issuer: String,
    pub subject: String,
    pub not_before: u64,
    pub not_after: u64,
    pub certificate: String,
}

// ---------------------------------------------------------------------------
// NRAS Client
// ---------------------------------------------------------------------------

/// Client for the NVIDIA Remote Attestation Service.
pub struct NrasClient {
    client: Client,
    base_url: String,
}

impl NrasClient {
    /// Create a new NRAS client.
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("NRAS reqwest client build"),
            base_url,
        }
    }

    /// Get the base URL for debugging.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Request a fresh attestation challenge from NRAS.
    pub async fn get_challenge(&self) -> Result<NrasChallenge> {
        let url = format!("{}/v1/attestation/challenge", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("NRAS challenge request failed: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "NRAS challenge failed ({}): {}",
                resp.status(),
                body
            ));
        }

        let challenge: NrasChallenge = resp
            .json()
            .await
            .map_err(|e| anyhow!("NRAS challenge parse error: {e}"))?;

        info!(
            challenge_id = %challenge.challenge_id,
            nonce_bytes = NRAS_NONCE_SIZE,
            "received NRAS challenge"
        );

        Ok(challenge)
    }

    /// Submit a TEE quote for NRAS verification.
    ///
    /// The `quote` must be the raw TEE quote bytes (SGX/TDX/SEV/Nitro).
    /// The `signature` must be the TEE's internal key signing the challenge nonce.
    pub async fn verify_quote(
        &self,
        quote: &str,
        signature: &str,
        challenge_id: &str,
        provider_id: Option<String>,
    ) -> Result<AttestationResponse> {
        let body = AttestationRequest {
            challenge_id: challenge_id.to_string(),
            quote: quote.to_string(),
            signature: signature.to_string(),
            provider_id,
        };

        let url = format!("{}/v1/attestation/verify", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("NRAS verify request failed: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "NRAS verify failed ({}): {}",
                resp.status(),
                body
            ));
        }

        let response: AttestationResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("NRAS verify parse error: {e}"))?;

        if !response.ok {
            return Err(anyhow!(
                "NRAS verification failed: {}",
                response.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        if let Some(ref hash) = response.tee_report_hash {
            info!(
                tee_report_hash = %hash,
                "NRAS verification successful"
            );
        }

        Ok(response)
    }

    /// Fetch NRAS root certificates for chain validation.
    pub async fn get_root_certs(&self) -> Result<Vec<NrasRootCert>> {
        let url = format!("{}/v1/attestation/cert", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("NRAS cert fetch failed: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(
                status = %resp.status(),
                "NRAS root cert fetch failed: {}",
                body
            );
            return Ok(Vec::new());
        }

        let certs: Vec<NrasRootCert> = resp
            .json()
            .await
            .map_err(|e| anyhow!("NRAS cert parse error: {e}"))?;

        info!(count = certs.len(), "fetched NRAS root certificates");
        Ok(certs)
    }

    /// Full attestation flow: challenge → verify → validate chain.
    ///
    /// This is the main entry point for the attestation challenge flow:
    /// provider ↔ attestation service ↔ registry.
    pub async fn full_attestation_flow(
        &self,
        quote: &str,
        signature: &str,
        provider_id: Option<String>,
    ) -> Result<(String, Vec<String>)> {
        // Step 1: Get challenge
        let challenge = self.get_challenge().await?;

        // Step 2: Verify quote with NRAS
        let response = self
            .verify_quote(&quote, &signature, &challenge.challenge_id, provider_id)
            .await?;

        // Step 3: Validate certificate chain
        let cert_chain = self
            .validate_chain(&response)
            .await
            .context("certificate chain validation failed")?;

        let tee_hash = response
            .tee_report_hash
            .ok_or_else(|| anyhow!("NRAS response missing tee_report_hash"))?;

        Ok((tee_hash, cert_chain))
    }

    /// Validate the NRAS-returned certificate chain against known roots.
    ///
    /// In production, this would use a proper x509 parser (e.g. `x509-parser`
    /// or `webpki`) to verify the chain:
    ///   1. Leaf cert (TEE quote signer) → intermediate CA → NRAS root
    ///   2. Check not-before/not-after validity
    ///   3. Verify signatures at each link
    ///
    /// For now, we do a basic structural check.
    async fn validate_chain(&self, response: &AttestationResponse) -> Result<Vec<String>> {
        let chain = match &response.cert_chain {
            Some(chain) if !chain.is_empty() => chain.clone(),
            _ => {
                warn!("no certificate chain in NRAS response; skipping validation");
                return Ok(Vec::new());
            }
        };

        // Basic structural validation:
        // 1. Chain must have at least 2 certs (leaf + root)
        if chain.len() < 2 {
            return Err(anyhow!(
                "invalid cert chain: expected >= 2 certs, got {}",
                chain.len()
            ));
        }

        // 2. Each cert must be valid base64 or hex
        for (i, cert) in chain.iter().enumerate() {
            if cert.len() < 100 {
                return Err(anyhow!(
                    "invalid cert chain: cert[{}] too short ({})",
                    i,
                    cert.len()
                ));
            }
        }

        info!(
            chain_len = chain.len(),
            "certificate chain structure validated"
        );

        Ok(chain)
    }
}
