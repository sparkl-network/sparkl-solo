//! Moniker validation for `[node].moniker` (operator-facing label; not stored on-chain).

use anyhow::{bail, Result};

pub const MAX_MONIKER_LEN: usize = 128;

/// Trim and validate moniker length (empty is allowed).
pub fn normalize_moniker(raw: &str) -> Result<String> {
    let m = raw.trim().to_string();
    if m.len() > MAX_MONIKER_LEN {
        bail!(
            "moniker must be at most {MAX_MONIKER_LEN} characters (got {})",
            m.len()
        );
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rejects_over_128() {
        let long = "a".repeat(129);
        assert!(normalize_moniker(&long).is_err());
    }
}
