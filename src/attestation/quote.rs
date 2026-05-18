use serde::{Deserialize, Serialize};

/// Type of TEE quote being generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteType {
    /// Intel SGX EPID/ADCX quote (legacy).
    SgxEpid,
    /// Intel SGX PSE/ECDSA quote.
    SgxEcdsa,
    /// Intel TDX (Trust Domain Extensions) quote.
    Tdx,
    /// AMD SEV-SNP quote.
    SevSnp,
    /// AMD SEV-ES quote.
    SevEs,
    /// NVIDIA DCX/Nitro quote.
    Nitro,
    /// Unknown/unrecognized quote format.
    Unknown,
}

impl std::fmt::Display for QuoteType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuoteType::SgxEpid => write!(f, "sgx_epid"),
            QuoteType::SgxEcdsa => write!(f, "sgx_ecdsa"),
            QuoteType::Tdx => write!(f, "tdx"),
            QuoteType::SevSnp => write!(f, "sev_snp"),
            QuoteType::SevEs => write!(f, "sev_es"),
            QuoteType::Nitro => write!(f, "nitro"),
            QuoteType::Unknown => write!(f, "unknown"),
        }
    }
}

/// Raw TEE quote payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    /// Type of TEE quote (SGX, TDX, SEV-SNP, Nitro, etc.)
    pub quote_type: QuoteType,
    /// Raw quote bytes (hex-encoded).
    pub quote_data: Vec<u8>,
    /// Optional attestation report hash (pre-computed).
    pub report_hash: Option<[u8; 32]>,
    /// Timestamp when the quote was generated (Unix epoch ms).
    pub timestamp_ms: u64,
    /// Parsed MRENCLAVE / measurement hex (implementation-specific offsets).
    #[serde(default)]
    pub mrenclave: Option<String>,
    #[serde(default)]
    pub signer_id: Option<String>,
    #[serde(default)]
    pub platform_info: Option<u64>,
    #[serde(default)]
    pub reserved: Option<u64>,
    #[serde(default)]
    pub version: u16,
    #[serde(default)]
    pub raw_size: usize,
}

impl Quote {
    pub fn new(quote_type: QuoteType, quote_data: Vec<u8>) -> Self {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            quote_type,
            quote_data,
            report_hash: None,
            timestamp_ms: ts,
            mrenclave: None,
            signer_id: None,
            platform_info: None,
            reserved: None,
            version: 0,
            raw_size: 0,
        }
    }

    pub fn with_report_hash(mut self, hash: [u8; 32]) -> Self {
        self.report_hash = Some(hash);
        self
    }

    /// Returns the raw quote bytes as a hex string.
    pub fn quote_hex(&self) -> String {
        hex::encode(&self.quote_data)
    }

    /// Returns the report hash as a hex string (if available).
    pub fn report_hash_hex(&self) -> Option<String> {
        self.report_hash.map(hex::encode)
    }
}
