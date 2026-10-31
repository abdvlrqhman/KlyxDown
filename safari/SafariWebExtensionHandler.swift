// KlyxDown for Safari: the extension's messages go to the running KlyxDown app.
//
// Safari hands native messages to this app extension instead of a separate host program.
// It is sandboxed, so it talks to KlyxDown over a Unix socket in the shared App Group
// folder (src-tauri/src/bridge.rs listens there on macOS), with the same framing as the
// Chrome and Firefox helper: a 4-byte little-endian length, then JSON.

import Foundation
import SafariServices
import AppKit

final class SafariWebExtensionHandler: NSObject, NSExtensionRequestHandling {
    func beginRequest(with context: NSExtensionContext) {
        let request = context.inputItems.first as? NSExtensionItem
        let message = request?.userInfo?[SFExtensionMessageKey] ?? [:]
        var reply: Any = ["ok": false, "error": "KlyxDown is not running."]
        if let body = try? JSONSerialization.data(withJSONObject: message) {
            if let answer = relay(body) ?? (launchApp() ? relayAfterLaunch(body) : nil),
               let json = try? JSONSerialization.jsonObject(with: answer) {
                reply = json
            }
        }
        let response = NSExtensionItem()
        response.userInfo = [SFExtensionMessageKey: reply]
        context.completeRequest(returningItems: [response], completionHandler: nil)
    }

    private var socketPath: String? {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "KlyxDownAppGroup") as? String,
              let dir = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
        else { return nil }
        return dir.appendingPathComponent("klyxdown.sock").path
    }

    /// One request and its reply over the socket; nil when KlyxDown isn't listening.
    private func relay(_ body: Data) -> Data? {
        guard let path = socketPath else { return nil }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return nil }
        defer { close(fd) }
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: addr.sun_path)
        guard path.utf8.count < capacity else { return nil }
        withUnsafeMutablePointer(to: &addr.sun_path) { raw in
            raw.withMemoryRebound(to: CChar.self, capacity: capacity) { _ = strcpy($0, path) }
        }
        let connected = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { return nil }
        var length = UInt32(body.count).littleEndian
        let frame = Data(bytes: &length, count: 4) + body
        guard writeAll(fd, frame), let head = readExactly(fd, 4) else { return nil }
        let size = head.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }
        guard size <= 1 << 20 else { return nil }
        return readExactly(fd, Int(size))
    }

    private func relayAfterLaunch(_ body: Data) -> Data? {
        for _ in 0..<40 {
            Thread.sleep(forTimeInterval: 0.25)
            if let answer = relay(body) { return answer }
        }
        return nil
    }

    /// Starts KlyxDown in the background, like the Chrome helper does.
    private func launchApp() -> Bool {
        guard let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "net.spacie.klyxdown") else { return false }
        let config = NSWorkspace.OpenConfiguration()
        config.arguments = ["--background"]
        config.activates = false
        NSWorkspace.shared.openApplication(at: app, configuration: config)
        return true
    }

    private func writeAll(_ fd: Int32, _ data: Data) -> Bool {
        data.withUnsafeBytes { raw in
            var sent = 0
            while sent < data.count {
                let n = write(fd, raw.baseAddress!.advanced(by: sent), data.count - sent)
                if n <= 0 { return false }
                sent += n
            }
            return true
        }
    }

    private func readExactly(_ fd: Int32, _ count: Int) -> Data? {
        var out = Data(count: count)
        var got = 0
        let ok = out.withUnsafeMutableBytes { raw -> Bool in
            while got < count {
                let n = read(fd, raw.baseAddress!.advanced(by: got), count - got)
                if n <= 0 { return false }
                got += n
            }
            return true
        }
        return ok ? out : nil
    }
}
