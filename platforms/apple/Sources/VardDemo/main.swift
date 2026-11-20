// A macOS window running a Vard project with the GPU renderer:
//   swift run VardDemo PATH   (a project folder without packages, or `vard bundle` output)
import AppKit
import VardHost

final class Delegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    var lifecycle: VardLifecycle?

    func applicationDidFinishLaunching(_ note: Notification) {
        let args = CommandLine.arguments.dropFirst().filter { !$0.hasPrefix("-") }
        let path = args.first ?? ProcessInfo.processInfo.environment["VARD_PROJECT"] ?? "."
        let project: VardProject
        do {
            project = try VardProject(directory: URL(fileURLWithPath: path))
        } catch {
            fputs("VardDemo: no .vard files in \(path): \(error)\n", stderr)
            exit(2)
        }
        let size = NSSize(width: 390, height: 780)
        let engine = project.makeEngine(width: Float(size.width), height: Float(size.height))
        CoreModules.all().forEach { engine.register($0) }
        let view = VardView(engine: engine, frame: NSRect(origin: .zero, size: size))
        window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled, .closable, .resizable, .miniaturizable], backing: .buffered, defer: false)
        window.title = project.root.lastPathComponent
        window.contentView = view
        window.center()
        window.makeKeyAndOrderFront(nil)
        lifecycle = VardLifecycle(engine: engine)
        if !view.mount(args.dropFirst().first) {
            fputs(engine.diagnostics + "\n", stderr)
            exit(1)
        }
        NSApp.activate(ignoringOtherApps: true)
        if CommandLine.arguments.contains("--check") { selfCheck(view) }
    }

    /// `--check`: draw on Metal, tap the first tappable record, let animations finish, exit.
    func selfCheck(_ view: VardView) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            guard view.hasGPU, view.framesDrawn > 0, let f = view.engine.frame() else {
                fputs("check: no frames drawn (gpu: \(view.hasGPU))\n", stderr)
                exit(1)
            }
            let before = view.framesDrawn
            print("check: Metal frames \(before), \(f.records.count) records, dark \(f.dark)")
            let tapped = f.records.contains { r in
                view.engine.tap(r.rect.x + r.rect.w / 2, r.rect.y + r.rect.h / 2)
            }
            view.requestFrame()
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                let after = view.engine.frame()
                print("check: tapped \(tapped), frames +\(view.framesDrawn - before), animating \(after?.animating ?? false)")
                exit(after?.animating == false ? 0 : 1)
            }
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool { true }
}

let app = NSApplication.shared
let delegate = Delegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
