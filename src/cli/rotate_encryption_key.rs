use std::path::PathBuf;

use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct RotateEncryptionKeyCli {
    pub config_path: Option<PathBuf>,
    /// When `--grace-period` is omitted, defaults to `0` (immediate deprecation of the previous key).
    pub grace_period_secs: u64,
    pub operator_key: Option<String>,
    pub dry_run: bool,
}

pub fn parse_grace_period(raw: &str) -> Result<u64> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(anyhow!("grace period is empty"));
    }
    if s.starts_with('-') {
        return Err(anyhow!("grace period must not be negative"));
    }

    let (num_part, mult) = match s.as_bytes().last().copied() {
        Some(b'd' | b'D') => (&s[..s.len() - 1], 86_400u64),
        Some(b'h' | b'H') => (&s[..s.len() - 1], 3_600u64),
        Some(b'm' | b'M') => (&s[..s.len() - 1], 60u64),
        Some(b's' | b'S') => (&s[..s.len() - 1], 1u64),
        _ => (s, 1u64),
    };

    let num_part = num_part.trim();
    if num_part.is_empty() {
        return Err(anyhow!("missing numeric value in grace period `{raw}`"));
    }
    let n: u64 = num_part
        .parse()
        .map_err(|_| anyhow!("invalid grace period number in `{raw}`"))?;
    n.checked_mul(mult)
        .ok_or_else(|| anyhow!("grace period overflow for `{raw}`"))
}

pub fn parse_rotate_encryption_key_args<I>(mut args: I) -> Result<RotateEncryptionKeyCli>
where
    I: Iterator<Item = String>,
{
    let mut config_path: Option<PathBuf> = None;
    let mut grace_period_secs: Option<u64> = None;
    let mut operator_key: Option<String> = None;
    let mut dry_run = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => {
                let path = args
                    .next()
                    .ok_or_else(|| anyhow!("--config requires a path"))?;
                config_path = Some(PathBuf::from(path));
            }
            "--grace-period" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("--grace-period requires a value"))?;
                grace_period_secs = Some(parse_grace_period(&raw)?);
            }
            "--operator-key" => {
                let k = args
                    .next()
                    .ok_or_else(|| anyhow!("--operator-key requires a hex value"))?;
                operator_key = Some(k);
            }
            "--dry-run" => dry_run = true,
            other => {
                return Err(anyhow!(
                    "unknown argument for rotate-encryption-key: `{other}`"
                ));
            }
        }
    }

    Ok(RotateEncryptionKeyCli {
        config_path,
        grace_period_secs: grace_period_secs.unwrap_or(0),
        operator_key,
        dry_run,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_grace_period;

    #[test]
    fn grace_period_days() {
        assert_eq!(parse_grace_period("30d").unwrap(), 30 * 86_400);
    }

    #[test]
    fn grace_period_zero() {
        assert_eq!(parse_grace_period("0").unwrap(), 0);
    }

    #[test]
    fn grace_period_hours() {
        assert_eq!(parse_grace_period("24h").unwrap(), 24 * 3_600);
    }

    #[test]
    fn grace_period_s_suffix() {
        assert_eq!(parse_grace_period("120s").unwrap(), 120);
    }

    #[test]
    fn grace_period_rejects_negative() {
        assert!(parse_grace_period("-1").is_err());
    }

    #[test]
    fn grace_period_rejects_garbage() {
        assert!(parse_grace_period("12x").is_err());
    }
}
