## cargo test --features mock-tpm (expected)

Typical successful summary:

```text
test result: ok. 6 passed; 0 failed; 0 ignored
```

You may also see additional `test result: ok` lines for unit/doc tests; the key integration anchor is that all integration tests pass with zero failures.

## yarn tpm:suite (expected)

Typical successful anchor:

```text
TPM suite passed { node1_peer_id: 'mock-...', cert_type: 'mock-software', ... }
```

The exact peer IDs and model counts vary by run, but success requires:

- line starts with `TPM suite passed`
- `receipt_verify: 'signature_ok'`
- no uncaught errors or non-zero exit code
