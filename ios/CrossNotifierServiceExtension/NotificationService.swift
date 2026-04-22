import UserNotifications
import UniformTypeIdentifiers

/// Runs on-device before iOS shows the banner. We use it for a single
/// purpose: if the payload carries an `iconHref`, download the image,
/// stash it in the extension's temp dir, and attach it to the
/// notification so the banner shows a meaningful icon.
///
/// The APNS 4 KB limit rules out baking the image data into the push
/// itself, so this is the only viable path to rich icons.
final class NotificationService: UNNotificationServiceExtension {
    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttempt: UNMutableNotificationContent?
    private var inflightTask: URLSessionDownloadTask?
    /// Exactly-once guard on `contentHandler`. Calling it twice is
    /// undefined behavior per Apple's docs; we can hit that if
    /// `serviceExtensionTimeWillExpire` fires while the image download
    /// is still in-flight (the download completion then races with the
    /// timeout fallback).
    private var handlerFired = false

    /// 30s is the documented extension budget. Pick something well
    /// under that to leave headroom for the on-device work (writing to
    /// disk, handing back to UNUserNotificationCenter).
    private static let fetchTimeout: TimeInterval = 10

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        self.contentHandler = contentHandler
        let mutable = request.content.mutableCopy() as? UNMutableNotificationContent
        self.bestAttempt = mutable

        guard let mutable,
              let iconHref = mutable.userInfo["iconHref"] as? String,
              let url = URL(string: iconHref) else {
            // No icon in the payload — deliver unchanged.
            deliver(request.content)
            return
        }

        fetchAttachment(from: url) { [weak self] attachment in
            guard let self else { return }
            if let attachment {
                mutable.attachments = [attachment]
            }
            self.deliver(mutable)
        }
    }

    /// iOS calls this when our budget runs out. Cancel any in-flight
    /// download first so its completion handler can't race with us,
    /// then deliver whatever we have. Losing the icon is better than
    /// losing the push entirely.
    override func serviceExtensionTimeWillExpire() {
        inflightTask?.cancel()
        inflightTask = nil
        if let bestAttempt {
            deliver(bestAttempt)
        }
    }

    // MARK: - Helpers

    /// Single entry point for `contentHandler` — guards against the
    /// dual-fire race between `fetchAttachment`'s completion and
    /// `serviceExtensionTimeWillExpire`.
    private func deliver(_ content: UNNotificationContent) {
        guard !handlerFired, let handler = contentHandler else { return }
        handlerFired = true
        handler(content)
    }

    private func fetchAttachment(
        from url: URL,
        completion: @escaping (UNNotificationAttachment?) -> Void
    ) {
        var request = URLRequest(url: url)
        request.timeoutInterval = Self.fetchTimeout

        let task = URLSession.shared.downloadTask(with: request) { [weak self] tempURL, response, _ in
            // Nil out the in-flight handle on completion so a later
            // timeout doesn't try to cancel an already-done task.
            self?.inflightTask = nil

            guard let tempURL else {
                completion(nil)
                return
            }

            // Move the downloaded file out of the session's per-task
            // scratch dir before it's wiped, and give it an extension
            // iOS can recognise. We sniff from Content-Type rather than
            // the URL path because server-side resized icons often
            // have no extension in their URL.
            let ext = preferredExtension(for: response)
            let dst = FileManager.default.temporaryDirectory
                .appendingPathComponent(UUID().uuidString)
                .appendingPathExtension(ext)
            do {
                try FileManager.default.moveItem(at: tempURL, to: dst)
                let attachment = try UNNotificationAttachment(
                    identifier: "icon",
                    url: dst,
                    options: nil
                )
                completion(attachment)
            } catch {
                completion(nil)
            }
        }
        inflightTask = task
        task.resume()
    }
}

/// Best-effort content-type → extension mapping. Defaults to .png; iOS
/// is forgiving about the mismatch as long as the bytes decode.
private func preferredExtension(for response: URLResponse?) -> String {
    guard let mime = (response as? HTTPURLResponse)?
        .value(forHTTPHeaderField: "Content-Type")?
        .split(separator: ";").first.map(String.init)
    else {
        return "png"
    }
    if let type = UTType(mimeType: mime), let ext = type.preferredFilenameExtension {
        return ext
    }
    return "png"
}
