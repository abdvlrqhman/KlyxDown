# Releasing

1. Set the version in `package.json`, `src-tauri/tauri.conf.json`, `Cargo.toml` and
   `extension/manifest.json`.
2. Push a tag `vX.Y.Z`. The Release workflow builds the installers, the browser extension
   zips and the Safari app into a draft GitHub release.
3. Publish the draft. Installed copies update from it.
4. Run the "Extension stores" workflow (Actions → Extension stores → Run) to publish the
   extension.

Everything below is optional: without it, installers are unsigned (Windows SmartScreen and
macOS Gatekeeper warn) and the store steps are skipped.

## Windows code signing (Azure Trusted Signing)

Certificates for code signing now live in hardware or a cloud HSM, so a `.pfx` file in a
secret is no longer an option. Azure Trusted Signing is the cheapest route (about US$10 a
month) and SmartScreen trusts it right away.

1. Create a Trusted Signing account and a certificate profile (Public Trust) in Azure, and
   finish the identity validation.
2. Create an app registration with a client secret, and give it the "Trusted Signing
   Certificate Profile Signer" role on the account.
3. In the GitHub repository settings:
   - Secrets: `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TENANT_ID`
   - Variables: `AZURE_SIGNING_ENDPOINT` (for example `https://wus2.codesigning.azure.net`),
     `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE`

The Release workflow then signs the app, the browser helper and the installers with
`trusted-signing-cli`.

## macOS signing and notarization

Needs an Apple Developer Program membership (US$99 a year).

1. Create a "Developer ID Application" certificate, export it with its key as a `.p12`.
2. Create an app-specific password for your Apple ID.
3. Secrets: `APPLE_CERTIFICATE` (the `.p12`, base64), `APPLE_CERTIFICATE_PASSWORD`,
   `APPLE_SIGNING_IDENTITY` (for example `Developer ID Application: Spacie (TEAMID)`),
   `APPLE_ID`, `APPLE_PASSWORD` (the app-specific password), `APPLE_TEAM_ID`.

Tauri signs the app and helper, notarizes the `.dmg` and staples the ticket.

## Safari extension

Safari runs extensions inside a small Mac app. The Release workflow builds it from
`extension/` with Apple's converter (`node scripts/safari-extension.mjs`), adding our native
handler (`safari/SafariWebExtensionHandler.swift`) and the App Group
`<TEAMID>.net.spacie.klyxdown`, through which it reaches KlyxDown. With `APPLE_TEAM_ID` set
it is signed; distribute it on the Mac App Store (App Store Connect, new macOS app with
bundle id `net.spacie.klyxdown.safari`) or notarized next to the installers.

KlyxDown itself must be signed with the same team for the App Group folder to be shared.

In Safari the extension can't take over downloads (Safari has no downloads API for
extensions); the right-click menus, "download all links" and the video button work.

## Browser extension stores

### Chrome Web Store

1. Register as a Chrome Web Store developer (one-time US$5).
2. First upload by hand: zip `extension/` **with the private key as `key.pem` in the root
   of the zip** (the key whose public half is the `key` in the manifest). The store then
   keeps the id `aohhcdaipkgppefknfnindgdacepogeb`, which the app's browser helper allows.
3. Fill the listing from [docs/store/listing.md](store/listing.md).
4. For later updates by the workflow, create an OAuth client (Google Cloud console, Chrome
   Web Store API) and a refresh token, then set the secrets `CWS_CLIENT_ID`,
   `CWS_CLIENT_SECRET`, `CWS_REFRESH_TOKEN`.

### Microsoft Edge Add-ons

1. Register in Microsoft Partner Center (free) and submit the zip from the workflow by hand
   once, with the same listing text.
2. Edge assigns its own extension id. Build the app with
   `KLYXDOWN_EDGE_EXTENSION_ID=<that id>` (a repository variable passed to the Release
   workflow's environment) so the browser helper allows it.
3. For updates by the workflow, create API credentials in Partner Center (Publish API) and
   set `EDGE_PRODUCT_ID`, `EDGE_CLIENT_ID`, `EDGE_API_KEY`.

### Firefox Add-ons

1. Create an account on addons.mozilla.org and API credentials (Tools → Manage API Keys).
2. Set `AMO_JWT_ISSUER` and `AMO_JWT_SECRET`. The workflow submits the Firefox build as a
   listed add-on with the metadata in `docs/store/amo-metadata.json`; the add-on id
   `klyxdown@spacie.net` is fixed in the build, so it matches the app.
