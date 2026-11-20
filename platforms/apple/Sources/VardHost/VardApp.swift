import Foundation
#if canImport(UIKit)
import UIKit
#else
import AppKit
#endif

/// Wires app lifecycle to the std events: `appResume`, `appPause` and `deepLink`.
public final class VardLifecycle {
    private var tokens: [NSObjectProtocol] = []
    private weak var engine: VardEngine?

    public init(engine: VardEngine) {
        self.engine = engine
        let nc = NotificationCenter.default
        #if canImport(UIKit)
        let (resume, pause) = (UIApplication.didBecomeActiveNotification, UIApplication.willResignActiveNotification)
        #else
        let (resume, pause) = (NSApplication.didBecomeActiveNotification, NSApplication.willResignActiveNotification)
        #endif
        tokens.append(nc.addObserver(forName: resume, object: nil, queue: .main) { [weak self] _ in self?.engine?.emit("appResume", "[]") })
        tokens.append(nc.addObserver(forName: pause, object: nil, queue: .main) { [weak self] _ in self?.engine?.emit("appPause", "[]") })
    }

    deinit { tokens.forEach(NotificationCenter.default.removeObserver) }

    /// Forward a `vard://...` (or universal) link the app was opened with.
    public func open(_ url: URL) {
        engine?.emit("deepLink", args: url.absoluteString)
    }
}

#if canImport(UIKit)
/// A full-screen Vard app for UIKit/SwiftUI hosts: loads the bundled project (`app/`, from
/// `vard bundle`), installs the std modules plus `modules`, draws edge to edge and keeps the
/// status bar readable on the app's theme.
///
/// ```swift
/// window.rootViewController = VardViewController(modules: [MyModule()])
/// ```
open class VardViewController: UIViewController {
    public private(set) var vard: VardView!
    public private(set) var lifecycle: VardLifecycle!
    private let project: VardProject?
    private let extra: [VardModule]
    private var dark = false

    public init(project: VardProject? = try? .main(), modules: [VardModule] = []) {
        self.project = project
        extra = modules
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    public required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    open override func loadView() {
        let size = UIScreen.main.bounds.size
        guard let project else {
            let l = UILabel()
            l.text = "No Vard project in the app bundle: add the `vard bundle` output as a folder reference named app"
            l.numberOfLines = 0
            l.textAlignment = .center
            view = l
            return
        }
        let engine = project.makeEngine(width: Float(size.width), height: Float(size.height))
        CoreModules.all().forEach { engine.register($0) }
        extra.forEach { engine.register($0) }
        vard = VardView(engine: engine)
        vard.onFrame = { [weak self] f in
            guard let self, f.dark != dark else { return }
            dark = f.dark
            setNeedsStatusBarAppearanceUpdate()
        }
        lifecycle = VardLifecycle(engine: engine)
        view = vard
        if !vard.mount() {
            NSLog("vard: mount failed:\n%@", engine.diagnostics)
        }
    }

    open override var preferredStatusBarStyle: UIStatusBarStyle { dark ? .lightContent : .darkContent }
}
#endif
