import XCTest
@testable import CrossNotifier

final class ClientErrorTests: XCTestCase {
    func testDescriptionsAreHumanReadable() {
        // Every error case should return a non-empty message the UI
        // can show without post-processing.
        let cases: [ClientError] = [
            .notConfigured,
            .invalidURL,
            .network("timeout"),
            .unauthorized,
            .notFound,
            .server(status: 500, body: "boom"),
            .server(status: 500, body: ""),
            .decoding("missing field"),
        ]
        for c in cases {
            XCTAssertFalse((c.errorDescription ?? "").isEmpty, "empty message for \(c)")
        }
    }

    func testUnauthorizedMentionsSecret() {
        // Helps the user self-diagnose — "secret" is the word they
        // typed, so it's the one they'll look for.
        XCTAssertTrue(
            (ClientError.unauthorized.errorDescription ?? "").lowercased().contains("secret")
        )
    }

    func testNotFoundMentionsPush() {
        // 404 from /devices specifically means the server has no
        // registry configured — phrase it that way rather than as a
        // generic "not found".
        XCTAssertTrue(
            (ClientError.notFound.errorDescription ?? "").lowercased().contains("push")
        )
    }
}
