import XCTest
@testable import VardHost

final class EchoModule: VardModule {
    var functions: [String: (VardCall) -> Any?] {
        [
            "shout": { c in c.string(0).uppercased() },
            "later": { c in
                let done = c.callback(0)
                DispatchQueue.main.async { done.resolve("ready") }
                return nil
            },
        ]
    }
}

final class VardHostTests: XCTestCase {
    let app = """
    extern fn shout(text: String) -> String
    extern fn later(done: fn(String))
    view App {
      state n = 0
      state status = "idle"
      state name = ""
      Column(gap: 8, padding: 16) {
        Text("Count {n}", id: "count")
        Column(id: "tapme", height: 44, background: .primary, onTap: { n += 1 }, animate: .easeOut, opacity: n > 0 ? 0.5 : 1) {}
        Input(value: name, placeholder: "Name")
        Text(shout("hi"))
        Text(status, id: "status")
        Column(height: 44, background: .surface, onTap: { later(s => { status = s }) }) {}
      }
    }
    """

    func engine() -> VardEngine {
        let e = VardEngine(files: [("app.vard", app)], width: 390, height: 844)
        CoreModules.all().forEach { e.register($0) }
        e.register(EchoModule())
        XCTAssertTrue(e.mount(), e.diagnostics)
        return e
    }

    func texts(_ f: VardFrame) -> [String] { f.records.filter { $0.kind == 4 }.map(\.text) }

    func testFrameDecodesAndModulesAnswer() throws {
        let e = engine()
        let f = try XCTUnwrap(e.frame())
        XCTAssertEqual(texts(f), ["Count 0", "HI", "idle"])
        XCTAssertTrue(f.records.contains { $0.isInput && $0.placeholder == "Name" })
        XCTAssertFalse(f.animating)
    }

    func testTapAnimatesAndAdvances() throws {
        let e = engine()
        let f0 = try XCTUnwrap(e.frame())
        let box = try XCTUnwrap(f0.records.first { $0.kind == 0 && $0.rect.h == 44 })
        XCTAssertTrue(e.tap(box.rect.x + 10, box.rect.y + 10))
        let f1 = try XCTUnwrap(e.frame())
        XCTAssertEqual(texts(f1).first, "Count 1")
        XCTAssertTrue(f1.animating, "opacity change with animate: runs on the frame clock")
        var running = true
        for _ in 0..<60 where running { running = e.advance(ms: 16.7) }
        XCTAssertFalse(running)
        let f2 = try XCTUnwrap(e.frame())
        XCTAssertEqual(f2.records.first { $0.uid == box.uid }?.opacity ?? 0, 0.5, accuracy: 0.001)
    }

    func testCallbacksArriveOnMain() throws {
        let e = engine()
        let f = try XCTUnwrap(e.frame())
        let target = try XCTUnwrap(f.records.last { $0.kind == 0 && $0.rect.h == 44 })
        e.tap(target.rect.x + 5, target.rect.y + 5)
        let changed = expectation(description: "callback")
        e.onChange = { changed.fulfill() }
        wait(for: [changed], timeout: 2)
        XCTAssertTrue(texts(try XCTUnwrap(e.frame())).contains("ready"))
    }

    func testInputText() throws {
        let e = engine()
        let input = try XCTUnwrap(e.frame()?.records.first { $0.isInput })
        XCTAssertTrue(e.inputText(uid: input.uid, "Ada"))
        XCTAssertEqual(e.frame()?.records.first { $0.isInput }?.text, "Ada")
    }

    func testPrefsModule() {
        let d = UserDefaults(suiteName: "vard.tests")!
        let m = PrefsModule(defaults: d)
        let e = VardEngine(files: [("app.vard", "view App { Text(\"x\") }")], width: 100, height: 100)
        let call = { (args: String) in VardCall(engine: e, name: "", json: args) }
        _ = m.functions["prefSet"]!(call(#"["k","v"]"#))
        XCTAssertEqual(m.functions["prefGet"]!(call(#"["k"]"#)) as? String, "v")
        _ = m.functions["prefRemove"]!(call(#"["k"]"#))
        XCTAssertNil(m.functions["prefGet"]!(call(#"["k"]"#)) as? String)
    }

    func testJSON() {
        struct P: Encodable { let name: String; let lat: Double }
        XCTAssertEqual(VardJSON.encode(nil), "null")
        XCTAssertEqual(VardJSON.encode("a\"b"), #""a\"b""#)
        XCTAssertEqual(VardJSON.encode(3), "3")
        XCTAssertEqual(VardJSON.encode(["x": 1]), #"{"x":1}"#)
        XCTAssertEqual(VardJSON.encode(P(name: "n", lat: 1.5)), #"{"lat":1.5,"name":"n"}"#)
    }
}
