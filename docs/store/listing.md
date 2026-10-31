# Store listing (Chrome Web Store, Microsoft Edge Add-ons, Firefox Add-ons)

Copy these into each store's form. Screenshots: 1280 × 800 PNGs; take them from the app
and the popup (docs/images has app shots that can be cropped).

## Name

KlyxDown

## Summary (132 characters max)

Send downloads, video and links from your browser to KlyxDown, the free, open-source download manager. No ads, no tracking.

## Description

KlyxDown is a free, open-source download manager for Windows, macOS and Linux. This
extension connects your browser to it.

- Downloads you start in the browser go to KlyxDown, which splits them into parts that
  download at the same time and picks up where it left off after crashes or lost
  connections.
- Signed-in downloads work: the extension passes along the page address and the cookies of
  the download's own site.
- Video and audio: the toolbar shows how many media files a page plays, and one click hands
  the page to KlyxDown, which finds the best quality.
- Right-click to download a link, every link on a page, or the links in a selection.
- Hold Alt while clicking a link to let the browser keep that download.
- Images opened in a new tab stay in the browser.

Requires the KlyxDown app: https://spacie.net/klyxdown
Source code (GPL-3.0): https://github.com/abdvlrqhman/KlyxDown

## Category

Chrome / Edge: Productivity (Tools). Firefox: Download Management.

## Single purpose (Chrome)

Hand downloads, links and media found on web pages to the KlyxDown download manager app on
the same computer.

## Permission justifications (Chrome)

| Permission | Why |
|---|---|
| downloads | To see downloads the user starts and hand them to KlyxDown instead (and give them back to the browser when the user declines). |
| nativeMessaging | The only channel to the KlyxDown app on the same computer. Nothing goes over the network. |
| contextMenus | "Download with KlyxDown", "Download all links" and "Download selected links" menu items. |
| cookies | Signed-in downloads: the cookies of the download's own site are passed to KlyxDown when a download is handed over. No other cookies are read. |
| storage | The extension's two settings, and the per-tab list of media a page played (session storage, cleared when the page changes). |
| scripting | Reads the links on the page (or in the selection) when the user picks "Download all links" or "Download selected links". |
| webRequest | Notices video and audio responses (by content type) to show the count on the toolbar. Observation only; nothing is blocked or changed. |
| Host access (all sites) | Downloads, links and media can be on any site. |

## Data use (Chrome privacy form)

- Collects: none. (Web history, cookies and page content stay on the computer and are only
  passed to the user's own KlyxDown app through native messaging.)
- Sold or transferred: no.
- Used for unrelated purposes: no.

Privacy policy URL: https://github.com/abdvlrqhman/KlyxDown/blob/main/docs/PRIVACY.md
