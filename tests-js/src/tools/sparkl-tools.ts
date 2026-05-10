// tools/sparkl-tools.ts
export const sparklTools = {
  build: {
    cmd: 'cargo build --features mock-tpm',
    cwd: '.',
    successPattern: 'Finished',
    failurePattern: 'error[E',
  },
  test: {
    cmd: 'cargo test --features mock-tpm -- --nocapture',
    cwd: '.',
    successPattern: 'test result: ok',
  },
  e2eTest: {
    cmd: 'yarn tpm:suite',
    cwd: 'tests-js',
    successPattern: 'TPM suite passed',
  },
  checkStatus: {
    cmd: 'yarn status',
    cwd: 'tests-js',
    successPattern: 'peer_id',
  },
}