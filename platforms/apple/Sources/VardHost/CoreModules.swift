import AVFoundation
import Contacts
import CoreLocation
import Foundation
import Photos
import Security
import UserNotifications
#if canImport(UIKit)
import UIKit
#else
import AppKit
#endif

/// The std `extern fn`s every host implements (`crates/vard-lang/src/std.vard`).
public enum CoreModules {
    public static func all() -> [VardModule] {
        [SystemModule(), PrefsModule(), SecureModule(), PermissionsModule(), NotificationsModule()]
    }
}

/// openUrl, shareText, haptic, openAppSettings.
public final class SystemModule: VardModule {
    public init() {}
    public var functions: [String: (VardCall) -> Any?] {
        [
            "openUrl": { c in
                if let url = URL(string: c.string(0)) { Self.open(url) }
                return nil
            },
            "shareText": { c in
                let text = c.string(0)
                DispatchQueue.main.async { Self.share(text) }
                return nil
            },
            "openAppSettings": { _ in
                #if canImport(UIKit)
                if let url = URL(string: UIApplication.openSettingsURLString) { Self.open(url) }
                #else
                if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security") { Self.open(url) }
                #endif
                return nil
            },
            "haptic": { c in
                #if os(iOS)
                let kind = c.string(0)
                DispatchQueue.main.async {
                    switch kind {
                    case "success": UINotificationFeedbackGenerator().notificationOccurred(.success)
                    case "warning": UINotificationFeedbackGenerator().notificationOccurred(.warning)
                    case "error": UINotificationFeedbackGenerator().notificationOccurred(.error)
                    case "heavy": UIImpactFeedbackGenerator(style: .heavy).impactOccurred()
                    case "light": UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    default: UISelectionFeedbackGenerator().selectionChanged()
                    }
                }
                #else
                _ = c
                DispatchQueue.main.async { NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now) }
                #endif
                return nil
            },
        ]
    }

    static func open(_ url: URL) {
        DispatchQueue.main.async {
            #if canImport(UIKit)
            UIApplication.shared.open(url)
            #else
            NSWorkspace.shared.open(url)
            #endif
        }
    }

    static func share(_ text: String) {
        #if canImport(UIKit)
        let vc = UIActivityViewController(activityItems: [text], applicationActivities: nil)
        let root = UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first?.rootViewController
        var top = root
        while let p = top?.presentedViewController { top = p }
        if let pop = vc.popoverPresentationController, let v = top?.view {
            pop.sourceView = v
            pop.sourceRect = CGRect(x: v.bounds.midX, y: v.bounds.midY, width: 1, height: 1)
        }
        top?.present(vc, animated: true)
        #else
        guard let view = NSApp.keyWindow?.contentView else { return }
        NSSharingServicePicker(items: [text]).show(relativeTo: .zero, of: view, preferredEdge: .minY)
        #endif
    }
}

/// prefGet/prefSet/prefRemove on UserDefaults (also backs `pref` members).
public final class PrefsModule: VardModule {
    let defaults: UserDefaults
    public init(defaults: UserDefaults = .standard) { self.defaults = defaults }
    public var functions: [String: (VardCall) -> Any?] {
        [
            "prefGet": { [defaults] c in defaults.string(forKey: c.string(0)) },
            "prefSet": { [defaults] c in defaults.set(c.string(1), forKey: c.string(0)); return nil },
            "prefRemove": { [defaults] c in defaults.removeObject(forKey: c.string(0)); return nil },
        ]
    }
}

/// secureGet/secureSet/secureRemove in the Keychain (this device only, after first unlock).
public final class SecureModule: VardModule {
    let service: String
    public init(service: String = (Bundle.main.bundleIdentifier ?? "vard") + ".secure") { self.service = service }

    private func query(_ key: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: key]
    }

    public func get(_ key: String) -> String? {
        var q = query(key)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: AnyObject?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let d = out as? Data else { return nil }
        return String(data: d, encoding: .utf8)
    }

    public func set(_ key: String, _ value: String) -> Bool {
        SecItemDelete(query(key) as CFDictionary)
        var q = query(key)
        q[kSecValueData as String] = Data(value.utf8)
        q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(q as CFDictionary, nil) == errSecSuccess
    }

    public func remove(_ key: String) { SecItemDelete(query(key) as CFDictionary) }

    public var functions: [String: (VardCall) -> Any?] {
        [
            "secureGet": { [unowned self] c in get(c.string(0)) },
            "secureSet": { [unowned self] c in set(c.string(0), c.string(1)) },
            "secureRemove": { [unowned self] c in remove(c.string(0)); return nil },
        ]
    }
}

/// permissionStatus / requestPermission for the std `Permission` enum. Status: granted |
/// denied | blocked (the system will not ask again: open settings) | notDetermined. The app's
/// Info.plist needs the usage strings (NSCameraUsageDescription, ...).
public final class PermissionsModule: NSObject, VardModule, CLLocationManagerDelegate {
    private var location: CLLocationManager?
    private var locationWaiting: [VardCallback] = []

    public override init() {}

    public var functions: [String: (VardCall) -> Any?] {
        [
            "permissionStatus": { [unowned self] c in status(c.string(0)) },
            "requestPermission": { [unowned self] c in
                request(c.string(0), c.callback(1))
                return nil
            },
        ]
    }

    public func status(_ p: String) -> String {
        switch p {
        case "camera": return av(AVCaptureDevice.authorizationStatus(for: .video))
        case "microphone": return av(AVCaptureDevice.authorizationStatus(for: .audio))
        case "photos":
            switch PHPhotoLibrary.authorizationStatus(for: .readWrite) {
            case .authorized, .limited: return "granted"
            case .denied, .restricted: return "blocked"
            default: return "notDetermined"
            }
        case "contacts":
            switch CNContactStore.authorizationStatus(for: .contacts) {
            case .notDetermined: return "notDetermined"
            case .denied, .restricted: return "blocked"
            default: return "granted"
            }
        case "location":
            switch CLLocationManager().authorizationStatus {
            case .notDetermined: return "notDetermined"
            case .denied, .restricted: return "blocked"
            default: return "granted"
            }
        case "notifications":
            // answered synchronously from the last known state; requestPermission refreshes it
            return NotificationsModule.lastStatus
        default: return "denied"
        }
    }

    private func av(_ s: AVAuthorizationStatus) -> String {
        switch s {
        case .authorized: return "granted"
        case .denied, .restricted: return "blocked"
        default: return "notDetermined"
        }
    }

    private func request(_ p: String, _ done: VardCallback) {
        let now = status(p)
        if now == "granted" || now == "blocked", p != "notifications" {
            done.resolve(now)
            return
        }
        switch p {
        case "camera", "microphone":
            AVCaptureDevice.requestAccess(for: p == "camera" ? .video : .audio) { ok in done.resolve(ok ? "granted" : "blocked") }
        case "photos":
            PHPhotoLibrary.requestAuthorization(for: .readWrite) { [unowned self] _ in done.resolve(status("photos")) }
        case "contacts":
            CNContactStore().requestAccess(for: .contacts) { ok, _ in done.resolve(ok ? "granted" : "blocked") }
        case "location":
            DispatchQueue.main.async { [unowned self] in
                if location == nil {
                    location = CLLocationManager()
                    location?.delegate = self
                }
                locationWaiting.append(done)
                location?.requestWhenInUseAuthorization()
            }
        case "notifications" where NotificationsModule.available:
            UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { ok, _ in
                NotificationsModule.refreshStatus { done.resolve(ok ? "granted" : $0) }
            }
        default:
            done.resolve("denied")
        }
    }

    public func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        guard manager.authorizationStatus != .notDetermined else { return }
        let s = status("location")
        let waiting = locationWaiting
        locationWaiting.removeAll()
        waiting.forEach { $0.resolve(s) }
    }
}

/// showNotification and getPushToken. The app delegate forwards the APNs token with
/// `NotificationsModule.setPushToken(_:)`; notification taps arrive as `pushOpened`.
public final class NotificationsModule: VardModule {
    static var lastStatus = "notDetermined"
    static var token: String?
    static var waitingToken: [VardCallback] = []
    weak var engine: VardEngine?

    /// UserNotifications needs an app bundle (not `swift run`/`swift test` binaries).
    static var available: Bool { Bundle.main.bundleURL.pathExtension == "app" }

    public init() { Self.refreshStatus { _ in } }

    public func install(_ engine: VardEngine) { self.engine = engine }

    static func refreshStatus(_ done: @escaping (String) -> Void) {
        guard available else {
            lastStatus = "denied"
            return done(lastStatus)
        }
        UNUserNotificationCenter.current().getNotificationSettings { s in
            switch s.authorizationStatus {
            case .authorized, .provisional, .ephemeral: lastStatus = "granted"
            case .denied: lastStatus = "blocked"
            default: lastStatus = "notDetermined"
            }
            done(lastStatus)
        }
    }

    /// From `application(_:didRegisterForRemoteNotificationsWithDeviceToken:)`.
    public static func setPushToken(_ deviceToken: Data) {
        let t = deviceToken.map { String(format: "%02x", $0) }.joined()
        token = t
        let w = waitingToken
        waitingToken.removeAll()
        w.forEach { $0.resolve(t) }
    }

    public var functions: [String: (VardCall) -> Any?] {
        [
            "showNotification": { c in
                guard Self.available else { return nil }
                let content = UNMutableNotificationContent()
                content.title = c.string(0)
                content.body = c.string(1)
                content.userInfo = c.stringMap(2)
                let req = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
                UNUserNotificationCenter.current().add(req)
                return nil
            },
            "getPushToken": { c in
                let done = c.callback(0)
                if let t = Self.token {
                    done.resolve(t)
                } else {
                    Self.waitingToken.append(done)
                    #if canImport(UIKit)
                    DispatchQueue.main.async { UIApplication.shared.registerForRemoteNotifications() }
                    #else
                    DispatchQueue.main.async { NSApplication.shared.registerForRemoteNotifications() }
                    #endif
                }
                return nil
            },
        ]
    }
}
