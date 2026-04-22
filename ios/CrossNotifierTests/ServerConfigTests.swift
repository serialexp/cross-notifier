import XCTest
@testable import CrossNotifier

final class ServerConfigTests: XCTestCase {
    func testIsConfiguredIsFalseForEmptyURL() {
        XCTAssertFalse(ServerConfig.empty.isConfigured)
    }

    func testIsConfiguredIsTrueWithURL() {
        let c = ServerConfig(baseURL: "https://x.example", deviceLabel: "l")
        XCTAssertTrue(c.isConfigured)
    }

    func testEndpointJoinsPath() {
        let c = ServerConfig(baseURL: "https://x.example", deviceLabel: "l")
        XCTAssertEqual(c.endpoint("/devices")?.absoluteString, "https://x.example/devices")
    }

    func testEndpointNormalisesTrailingSlashes() {
        let c = ServerConfig(baseURL: "https://x.example///", deviceLabel: "l")
        XCTAssertEqual(c.endpoint("/devices")?.absoluteString, "https://x.example/devices")
    }

    func testEndpointAddsLeadingSlashIfMissing() {
        let c = ServerConfig(baseURL: "https://x.example", deviceLabel: "l")
        XCTAssertEqual(c.endpoint("devices")?.absoluteString, "https://x.example/devices")
    }

    func testEndpointHandlesWhitespaceAroundURL() {
        let c = ServerConfig(baseURL: "  https://x.example  ", deviceLabel: "l")
        XCTAssertEqual(c.endpoint("/devices")?.absoluteString, "https://x.example/devices")
    }

    func testEndpointReturnsNilForEmptyURL() {
        XCTAssertNil(ServerConfig(baseURL: "   ", deviceLabel: "l").endpoint("/devices"))
        XCTAssertNil(ServerConfig(baseURL: "", deviceLabel: "l").endpoint("/devices"))
    }

    func testEndpointReturnsNilForRelativeURL() {
        // Foundation parses bare paths as relative URLs (nil scheme).
        // URLSession needs an absolute URL, so we reject these.
        XCTAssertNil(ServerConfig(baseURL: "/devices", deviceLabel: "l").endpoint("/x"))
    }

    func testEndpointReturnsNilForUnsupportedScheme() {
        XCTAssertNil(
            ServerConfig(baseURL: "ftp://x.example", deviceLabel: "l").endpoint("/devices")
        )
    }

    func testEndpointPreservesPort() {
        let c = ServerConfig(baseURL: "http://localhost:9876", deviceLabel: "l")
        XCTAssertEqual(c.endpoint("/devices")?.absoluteString, "http://localhost:9876/devices")
    }

    func testCodableRoundtrip() throws {
        let c = ServerConfig(baseURL: "https://x.example", deviceLabel: "My Phone")
        let data = try JSONEncoder().encode(c)
        let decoded = try JSONDecoder().decode(ServerConfig.self, from: data)
        XCTAssertEqual(c, decoded)
    }
}
