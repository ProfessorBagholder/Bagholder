# Possible features after the migration

Nothing here is built or planned. Each is an idea noted for when the migration is complete (`docs/decisions.md`, 2026-09-26), and waits for the owner's go.

- **The Wealthsimple sign-in kept in the operating system's own credential store** (the Mac's keychain, Windows' Credential Manager, a Linux desktop's Secret Service) rather than `session.json` in the data folder. For now it stays as it is.
