# Privacy

KlyxDown and its browser extension collect nothing about you and send nothing to Spacie or
anyone else. There is no account, no analytics, no crash reporting, no ads.

What does leave your computer, and only because you asked for it:

- **Downloads.** KlyxDown talks to the servers of the links you download, like a browser.
- **Updates.** A few seconds after launch (if "Check for updates automatically" is on),
  KlyxDown asks GitHub for the latest release. Nothing about you is sent.
- **Video helpers.** The first time you download a video, and after you agree, KlyxDown
  downloads yt-dlp from GitHub and ffmpeg from its official builds. yt-dlp updates itself
  from GitHub once a day.
- **Torrents.** BitTorrent shares with other people by design: they see your IP address
  and which pieces you have. Sharing finished torrents can be turned off in Settings.

What the extension does with your browsing:

- It looks at downloads you start, to hand them to KlyxDown (or not, as you choose).
- It notes which video and audio files a page plays, to offer them in its popup. That list
  stays in the browser's session storage and is cleared when you leave the page.
- When it hands a download to KlyxDown, it includes the page address and the cookies of the
  download's own site, so downloads that need you to be signed in work. Cookies of other
  sites are never read or sent. Everything goes to KlyxDown on your computer through the
  browser's native messaging, never over the network.

Sign-in passwords you save in KlyxDown stay in your system's keychain (Windows Credential
Manager, macOS Keychain, or the Secret Service on Linux).

Remote control is off unless you turn it on; then it answers only devices on your own
network that have the secret link.

Questions: https://github.com/abdvlrqhman/KlyxDown/issues
