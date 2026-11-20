# Signing and notarising the macOS build

The release workflow (`.github/workflows/release.yml`) signs `Rat Run.app` with a
Developer ID certificate and notarises it when these repository secrets exist.
Without them it falls back to an ad-hoc signature, and people who download the
app have to confirm the first launch in System Settings.

## One-time setup

1. **Create a Developer ID Application certificate.** In Xcode: Settings →
   Accounts → your team → Manage Certificates → `+` → *Developer ID Application*.
   (An "Apple Development" certificate is not enough: it is for running your own
   builds, not for distributing outside the App Store.)
2. **Export it.** Keychain Access → My Certificates → right-click the
   *Developer ID Application* entry → Export → `.p12`, with a password.
3. **Create an App Store Connect API key** for notarisation:
   appstoreconnect.apple.com → Users and Access → Integrations → App Store
   Connect API → generate a key with the *Developer* role. Download the `.p8`
   once, and note the Key ID and the Issuer ID.
4. **Store the secrets** (run these yourself; they never pass through an agent):

   ```sh
   base64 -i DeveloperID.p12 | gh secret set MACOS_CERT_P12
   gh secret set MACOS_CERT_PASSWORD        # the .p12 password
   gh secret set APPLE_API_KEY_P8 < AuthKey_XXXXXXXXXX.p8
   gh secret set APPLE_API_KEY_ID           # e.g. XXXXXXXXXX
   gh secret set APPLE_API_ISSUER_ID        # a UUID
   ```

## What the workflow does with them

- Imports the certificate into a temporary keychain on the runner.
- Signs the universal `ratrun` binary and the `.app` with the hardened runtime
  and a secure timestamp.
- Submits both to Apple's notary service and waits, then staples the ticket to
  the `.app` so it opens offline without a warning.

The bare binary in the `.tar.gz` is notarised but cannot be stapled; Gatekeeper
checks it online the first time it runs.
