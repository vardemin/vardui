import Foundation
import VardFFI

/// One running Vard app over the C ABI (`crates/vard-ffi/include/vard.h`).
///
/// All methods must be called on the main thread, and never from inside a native module
/// function (the runtime is mid-evaluation there): modules answer later through
/// ``VardCallback`` or ``callback(_:_:keep:)``, which hop to the main thread themselves.
///
/// Generated Swift bindings (`vard bindings --swift --class PlacesModule`) talk to the host
/// through a `PlacesModuleHost` protocol whose two methods ``callback(_:_:keep:)`` and
/// ``emit(_:_:)`` this class already has: `extension VardEngine: PlacesModuleHost {}`, then
/// `engine.addDispatcher { name, json in places.call(name, json, host: engine) }`.
public final class VardEngine {
    public let handle: OpaquePointer
    public private(set) var mounted = false
    /// Called whenever the screen may have changed (a callback answered, an event delivered).
    public var onChange: (() -> Void)?

    private var modules: [String: (VardCall) -> Any?] = [:]
    private var resolvers: [(String) -> ((VardCall) -> Any?)?] = []
    private var dispatchers: [(String, String) -> String?] = []
    private var installed: [VardModule] = []
    private var lastResult: UnsafeMutablePointer<CChar>?
    private var queued: [() -> Void] = []

    /// Compile `files` (`path`, `text`: app sources, `@package/...` sources, `vard.lock`)
    /// for a `size` in points. Check ``diagnostics`` when ``mount(_:)`` fails.
    public init(files: [(String, String)], width: Float, height: Float) {
        var cstrings: [UnsafeMutablePointer<CChar>] = []
        var sources: [VardSource] = []
        for (path, text) in files {
            let p = strdup(path)!, t = strdup(text)!
            cstrings += [p, t]
            sources.append(VardSource(path: p, text: t))
        }
        handle = vard_create(sources, sources.count, width, height)
        cstrings.forEach { free($0) }
    }

    deinit {
        vard_destroy(handle)
        free(lastResult)
    }

    /// The platform name `platform` reports in Vard (`"ios"` or `"macos"`).
    public static var platformName: String {
        #if os(iOS)
        "ios"
        #else
        "macos"
        #endif
    }

    // MARK: native modules

    /// Install a module. Its functions are looked up by their Vard `extern fn` name.
    @discardableResult
    public func register(_ module: VardModule) -> VardEngine {
        for (name, f) in module.functions { modules[name] = f }
        resolvers.append(module.resolve)
        installed.append(module)
        module.install(self)
        return self
    }

    /// A raw dispatcher (`name`, JSON arguments) → JSON result or nil — the shape of
    /// `call(_:_:host:)` in generated bindings.
    public func addDispatcher(_ d: @escaping (String, String) -> String?) {
        dispatchers.append(d)
    }

    /// Answer a `fn(...)` argument (any thread). `keep` = a listener that stays subscribed.
    public func callback(_ id: UInt64, _ json: String, keep: Bool) {
        onMain { [weak self] in
            guard let self else { return }
            if vard_callback(self.handle, id, json, keep) == 1 { self.onChange?() }
        }
    }

    /// Deliver `extern event name(...)`; `argsJSON` is a JSON array (any thread).
    public func emit(_ event: String, _ argsJSON: String) {
        onMain { [weak self] in
            guard let self else { return }
            if !self.mounted {
                self.queued.append { _ = vard_emit(self.handle, event, argsJSON) }
                return
            }
            _ = vard_emit(self.handle, event, argsJSON)
            self.onChange?()
        }
    }

    /// Emit with Swift values (converted like module results).
    public func emit(_ event: String, args: Any?...) {
        emit(event, VardJSON.encode(args.map { $0 ?? NSNull() }))
    }

    func release(_ id: UInt64) {
        onMain { [weak self] in if let self { vard_release(self.handle, id) } }
    }

    private func onMain(_ f: @escaping () -> Void) {
        if Thread.isMainThread { f() } else { DispatchQueue.main.async(execute: f) }
    }

    /// Wire the dispatcher into the core; call once, before ``mount(_:)``.
    private func installHost() {
        let ctx = Unmanaged.passUnretained(self).toOpaque()
        vard_set_host(handle, { ctx, name, args in
            guard let ctx, let name, let args else { return nil }
            let engine = Unmanaged<VardEngine>.fromOpaque(ctx).takeUnretainedValue()
            return engine.dispatch(String(cString: name), String(cString: args))
        }, ctx, VardEngine.platformName)
    }

    private func dispatch(_ name: String, _ args: String) -> UnsafePointer<CChar>? {
        var out: String?
        if let f = modules[name] ?? resolvers.lazy.compactMap({ $0(name) }).first {
            let call = VardCall(engine: self, name: name, json: args)
            out = VardJSON.encode(f(call))
        } else {
            for d in dispatchers {
                if let r = d(name, args) { out = r; break }
            }
        }
        guard let out else { return nil }
        free(lastResult)
        lastResult = strdup(out)
        return UnsafePointer(lastResult)
    }

    // MARK: lifecycle

    /// Register a project font file (`Inter-SemiBold.ttf`).
    @discardableResult
    public func addFont(fileName: String, data: Data) -> Bool {
        data.withUnsafeBytes { raw in
            vard_add_font(handle, fileName, raw.bindMemory(to: UInt8.self).baseAddress, raw.count) == 0
        }
    }

    /// Folder `Image("assets/...")` paths are relative to.
    public func setAssetDirectory(_ url: URL) {
        vard_set_asset_dir(handle, url.path)
    }

    /// Mount `view` (nil: `App`, else the last view in user code).
    @discardableResult
    public func mount(_ view: String? = nil) -> Bool {
        installHost()
        let ok = view.map { vard_mount(handle, $0) } ?? vard_mount(handle, nil)
        mounted = ok == 0
        if mounted {
            let q = queued
            queued.removeAll()
            q.forEach { $0() }
        }
        return mounted
    }

    public var diagnostics: String {
        vard_diagnostics(handle).map { String(cString: $0) } ?? ""
    }

    // MARK: events (points)

    @discardableResult public func tap(_ x: Float, _ y: Float) -> Bool { vard_tap(handle, x, y) == 1 }
    @discardableResult public func scroll(_ x: Float, _ y: Float, dy: Float) -> Bool { vard_scroll(handle, x, y, dy) == 1 }
    @discardableResult public func inputText(uid: UInt32, _ text: String) -> Bool { vard_input_text(handle, uid, text) == 1 }
    @discardableResult public func submit(uid: UInt32) -> Bool { vard_submit(handle, uid) == 1 }
    public func focus(uid: UInt32) { vard_focus(handle, uid) }
    public func setDark(_ dark: Bool) { vard_set_dark(handle, dark) }
    public func setLocale(_ tag: String) { vard_set_locale(handle, tag) }
    public func resize(width: Float, height: Float) { vard_resize(handle, width, height) }
    public func setInsets(top: Float, bottom: Float, left: Float, right: Float, keyboard: Float) {
        vard_set_insets(handle, top, bottom, left, right, keyboard)
    }
    public var backEnabled: Bool { vard_back_enabled(handle) }
    public func backProgress(_ p: Float) { vard_back_progress(handle, p) }
    public func backCancel() { vard_back_cancel(handle) }
    /// Back committed: true when the app handled it.
    @discardableResult public func back() -> Bool { vard_back(handle) == 1 }

    /// Advance `animate:`/`enter:` animations; true while they run.
    @discardableResult public func advance(ms: Float) -> Bool { vard_advance(handle, ms) }

    /// The current frame (see `Frame`). Settles pending work first.
    public func frame() -> VardFrame? {
        var len = 0
        guard let p = vard_frame(handle, &len), len > 0 else { return nil }
        return VardFrame.decode(Data(bytes: p, count: len))
    }

    /// Debug builds: accept `vard run --device HOST:PORT` scripts on `addr`.
    public func inspectListen(_ addr: String) -> Bool { vard_inspect_listen(handle, addr) == 0 }
    /// Run queued `vard run --device` scripts; true when something ran (redraw).
    public func inspectPoll() -> Bool { vard_inspect_poll(handle) == 1 }
}

/// A native module: Swift implementations of `extern fn`s, keyed by their Vard name.
///
/// ```swift
/// final class Haptics: VardModule {
///     var functions: [String: (VardCall) -> Any?] { [
///         "vibrate": { _ in UIImpactFeedbackGenerator().impactOccurred(); return nil },
///         "loadProfile": { c in api.load(c.string(0)) { p in c.callback(1).resolve(p) }; return nil },
///     ] }
/// }
/// ```
/// Results and callback values become JSON: nil, String, numbers, Bool, arrays,
/// dictionaries, `Encodable` values.
public protocol VardModule: AnyObject {
    var functions: [String: (VardCall) -> Any?] { get }
    /// Functions resolved by name pattern (checked after ``functions``).
    func resolve(_ name: String) -> ((VardCall) -> Any?)?
    func install(_ engine: VardEngine)
}

public extension VardModule {
    func resolve(_ name: String) -> ((VardCall) -> Any?)? { nil }
    func install(_ engine: VardEngine) {}
}

/// One `extern fn` invocation: positional JSON arguments.
public struct VardCall {
    public unowned let engine: VardEngine
    public let name: String
    public let args: [Any]

    init(engine: VardEngine, name: String, json: String) {
        self.engine = engine
        self.name = name
        args = (try? JSONSerialization.jsonObject(with: Data(json.utf8), options: [.fragmentsAllowed])) as? [Any] ?? []
    }

    public var count: Int { args.count }
    public func isNull(_ i: Int) -> Bool { i >= args.count || args[i] is NSNull }
    public func string(_ i: Int) -> String { (i < args.count ? args[i] as? String : nil) ?? "" }
    public func stringOrNil(_ i: Int) -> String? { i < args.count ? args[i] as? String : nil }
    public func double(_ i: Int) -> Double { (i < args.count ? (args[i] as? NSNumber)?.doubleValue : nil) ?? 0 }
    public func int(_ i: Int) -> Int { (i < args.count ? (args[i] as? NSNumber)?.intValue : nil) ?? 0 }
    public func bool(_ i: Int) -> Bool { (i < args.count ? (args[i] as? NSNumber)?.boolValue : nil) ?? false }
    public func stringMap(_ i: Int) -> [String: String] {
        ((i < args.count ? args[i] as? [String: Any] : nil) ?? [:]).compactMapValues { $0 as? String ?? "\($0)" }
    }
    /// A `fn(T)` argument: answer it now or later, from any thread.
    public func callback(_ i: Int) -> VardCallback {
        let id = ((i < args.count ? args[i] as? [String: Any] : nil)?["$callback"] as? NSNumber)?.uint64Value ?? 0
        return VardCallback(engine: engine, id: id)
    }
}

/// A Vard function handed to native code.
public struct VardCallback {
    weak var engine: VardEngine?
    public let id: UInt64
    /// Deliver the result; the callback is consumed.
    public func resolve(_ value: Any? = nil) { engine?.callback(id, VardJSON.encode(value), keep: false) }
    /// Deliver one of many values (listeners); ``release()`` when done.
    public func send(_ value: Any?) { engine?.callback(id, VardJSON.encode(value), keep: true) }
    public func release() { engine?.release(id) }
}

/// Swift value → JSON text understood by the runtime.
public enum VardJSON {
    public static func encode(_ v: Any?) -> String {
        guard let v, !(v is NSNull), !(v is Void) else { return "null" }
        if let e = v as? Encodable, !(v is String), !(v is NSNumber), !(v is [Any]), !(v is [String: Any]) {
            let enc = JSONEncoder()
            enc.outputFormatting = .sortedKeys
            if let d = try? enc.encode(AnyEncodable(e)) { return String(decoding: d, as: UTF8.self) }
        }
        let wrapped = wrap(v)
        guard JSONSerialization.isValidJSONObject([wrapped]),
              let d = try? JSONSerialization.data(withJSONObject: wrapped, options: [.fragmentsAllowed]) else { return "null" }
        return String(decoding: d, as: UTF8.self)
    }

    static func wrap(_ v: Any?) -> Any {
        switch v {
        case nil: return NSNull()
        case let s as String: return s
        case let b as Bool: return b
        case let n as NSNumber: return n
        case let a as [Any?]: return a.map(wrap)
        case let m as [String: Any?]: return m.mapValues(wrap)
        case let e as Encodable:
            if let d = try? JSONEncoder().encode(AnyEncodable(e)),
               let o = try? JSONSerialization.jsonObject(with: d, options: [.fragmentsAllowed]) { return o }
            return NSNull()
        default: return "\(v!)"
        }
    }

    private struct AnyEncodable: Encodable {
        let e: Encodable
        init(_ e: Encodable) { self.e = e }
        func encode(to encoder: Encoder) throws { try e.encode(to: encoder) }
    }
}
