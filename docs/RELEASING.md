# Releasing

1. Set the version in `package.json`, `src-tauri/tauri.conf.json`, `Cargo.toml` and
   `extension/manifest.json`.
2. Push a tag `vX.Y.Z`. The Release workflow builds the installers, the browser extension
   zips and the Safari app into a draft GitHub release.
3. Publish the draft. Installed copies update from it.
4. Run the "Extension stores" workflow (Actions → Extension stores → Run) to publish the
   extension.

Installers are not code-signed. On first launch Windows SmartScreen shows "Windows
protected your PC": choose **More info → Run anyway**. On macOS, open the app once, then
**System Settings → Privacy & Security → Open Anyway**. Auto-updates are still verified with
the updater key (`TAURI_SIGNING_PRIVATE_KEY`), which is separate from code signing.

The store steps below are skipped until their secrets are set.

## Safari extension

Safari runs extensions inside a small Mac app. The Release workflow builds it, unsigned,
from `extension/` with Apple's converter (`node scripts/safari-extension.mjs`), adding our
native handler (`safari/SafariWebExtensionHandler.swift`), which reaches KlyxDown through a
shared folder (`~/Library/Group Containers/TEAMID.net.spacie.klyxdown`).

Unsigned Safari extensions load only with **Develop → Allow Unsigned Extensions** turned on
in Safari (Safari turns it off again when it quits). In Safari the extension can't take over
downloads (Safari has no downloads API for extensions); the right-click menus, "download all
links" and the video button work.

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
