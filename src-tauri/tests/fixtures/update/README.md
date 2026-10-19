# Updater test fixtures

A THROWAWAY minisign keypair made with `pnpm tauri signer generate` (the private key was deleted and is not in the
repository). `test.pub` is its public key, `payload.bin.sig` the signature `tauri signer sign` made for `payload.bin`,
and `other.pub` a second unrelated public key (for the wrong-key test). The production key is the constant
`UPDATE_PUBKEY` in the code and is never used by tests.
