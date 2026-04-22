import XCTest
@testable import CrossNotifier

final class DeviceTokenTests: XCTestCase {
    func testHexStringEncodesAllBytes() {
        let bytes: [UInt8] = [0x00, 0x0f, 0x10, 0xff]
        let data = Data(bytes)
        XCTAssertEqual(hexString(from: data), "000f10ff")
    }

    func testHexStringIsLowercase() {
        let data = Data([0xab, 0xcd, 0xef])
        let hex = hexString(from: data)
        XCTAssertEqual(hex, hex.lowercased())
    }

    func testHexStringEmptyData() {
        XCTAssertEqual(hexString(from: Data()), "")
    }

    func testHexStringLength() {
        // APNS tokens are 32 bytes → 64 hex chars.
        let data = Data(repeating: 0x42, count: 32)
        XCTAssertEqual(hexString(from: data).count, 64)
    }
}
