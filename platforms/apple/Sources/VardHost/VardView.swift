import Foundation
import QuartzCore
import VardFFI
#if canImport(UIKit)
import UIKit
public typealias VardPlatformView = UIView
typealias PlatformTextField = UITextField
#else
import AppKit
public typealias VardPlatformView = NSView
typealias PlatformTextField = NSTextField
#endif

/// Shows a Vard app: frames are drawn by the Rust GPU renderer into this view's
/// `CAMetalLayer`; layout, hit-testing and `onTap` bubbling stay in the core. Native text
/// fields float over `Input` records (IME, selection, autofill), the display link runs
/// only while `animate:`/`enter:` animations or scroll inertia are in flight.
public final class VardView: VardPlatformView {
    public let engine: VardEngine
    private var gpu: OpaquePointer?
    private var link: CADisplayLink?
    private var lastTimestamp: CFTimeInterval = 0
    private var inputs: [UInt32: PlatformTextField] = [:]
    private var lastFrame: VardFrame?
    private var lastDark: Bool?
    /// scroll inertia: (point, velocity in points per second)
    private var fling: (CGPoint, CGFloat)?
    private var inspectTimer: Timer?
    /// Every drawn frame (status bar style from `dark`, tests).
    public var onFrame: ((VardFrame) -> Void)?
    /// Frames presented on the GPU so far.
    public private(set) var framesDrawn = 0
    /// Whether a Metal renderer is attached (false: no GPU, nothing is drawn).
    public var hasGPU: Bool { gpu != nil }

    public init(engine: VardEngine, frame: CGRect = .zero) {
        self.engine = engine
        super.init(frame: frame)
        #if canImport(UIKit)
        isMultipleTouchEnabled = false
        backgroundColor = .clear
        installGestures()
        #else
        wantsLayer = true
        #endif
        engine.onChange = { [weak self] in self?.requestFrame() }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    deinit {
        link?.invalidate()
        inspectTimer?.invalidate()
        if let gpu { vard_gpu_destroy(gpu) }
    }

    // MARK: layer

    #if canImport(UIKit)
    public override class var layerClass: AnyClass { CAMetalLayer.self }
    private var metalLayer: CAMetalLayer { layer as! CAMetalLayer }
    private var pixelScale: CGFloat { window?.screen.scale ?? UIScreen.main.scale }
    #else
    public override func makeBackingLayer() -> CALayer { CAMetalLayer() }
    public override var isFlipped: Bool { true }
    private var metalLayer: CAMetalLayer { layer as! CAMetalLayer }
    private var pixelScale: CGFloat { window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2 }
    #endif

    private func sizeChanged() {
        let scale = pixelScale
        let (w, h) = (bounds.width, bounds.height)
        guard w > 0, h > 0 else { return }
        metalLayer.contentsScale = scale
        metalLayer.drawableSize = CGSize(width: w * scale, height: h * scale)
        let (pw, ph) = (UInt32(w * scale), UInt32(h * scale))
        if let gpu {
            vard_gpu_resize(gpu, pw, ph, Float(scale))
        } else {
            gpu = vard_gpu_create_metal(Unmanaged.passUnretained(metalLayer).toOpaque(), pw, ph, Float(scale))
            if gpu == nil { NSLog("vard: no Metal device; nothing will be drawn") }
        }
        engine.resize(width: Float(w), height: Float(h))
        updateInsets()
        requestFrame()
    }

    #if canImport(UIKit)
    public override func layoutSubviews() {
        super.layoutSubviews()
        sizeChanged()
    }

    public override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else { return }
        syncAppearance()
        sizeChanged()
    }

    public override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        updateInsets()
    }

    public override func traitCollectionDidChange(_ previous: UITraitCollection?) {
        super.traitCollectionDidChange(previous)
        syncAppearance()
    }
    #else
    public override func layout() {
        super.layout()
        sizeChanged()
    }

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil else { return }
        syncAppearance()
        sizeChanged()
    }

    public override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        sizeChanged()
    }

    public override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        syncAppearance()
    }
    #endif

    private func syncAppearance() {
        #if canImport(UIKit)
        let dark = traitCollection.userInterfaceStyle == .dark
        #else
        let dark = effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        #endif
        engine.setDark(dark)
        requestFrame()
    }

    // MARK: insets and keyboard

    private var keyboard: CGFloat = 0

    private func updateInsets() {
        #if canImport(UIKit)
        let s = safeAreaInsets
        engine.setInsets(top: Float(s.top), bottom: Float(s.bottom), left: Float(s.left), right: Float(s.right), keyboard: Float(keyboard))
        #else
        engine.setInsets(top: 0, bottom: 0, left: 0, right: 0, keyboard: 0)
        #endif
    }

    #if canImport(UIKit)
    private func observeKeyboard() {
        let nc = NotificationCenter.default
        nc.addObserver(forName: UIResponder.keyboardWillChangeFrameNotification, object: nil, queue: .main) { [weak self] n in
            guard let self, let end = (n.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue else { return }
            let local = convert(end, from: nil)
            keyboard = max(0, bounds.maxY - local.minY)
            updateInsets()
            requestFrame()
        }
        nc.addObserver(forName: UIResponder.keyboardWillHideNotification, object: nil, queue: .main) { [weak self] _ in
            self?.keyboard = 0
            self?.updateInsets()
            self?.requestFrame()
        }
    }
    #endif

    // MARK: mounting

    /// Mount the root view and start drawing. Returns false with ``VardEngine/diagnostics``
    /// describing compile or mount errors.
    @discardableResult
    public func mount(_ view: String? = nil) -> Bool {
        let ok = engine.mount(view)
        if ok {
            engine.setLocale(Locale.preferredLanguages.first ?? "en")
            #if canImport(UIKit)
            observeKeyboard()
            #endif
            #if DEBUG
            if engine.inspectListen("127.0.0.1:8472") {
                inspectTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
                    if self?.engine.inspectPoll() == true { self?.requestFrame() }
                }
            }
            #endif
        } else {
            NSLog("vard: %@", engine.diagnostics)
        }
        requestFrame()
        return ok
    }

    // MARK: frames

    /// Draw on the next vsync (after any event).
    public func requestFrame() {
        if link == nil {
            #if canImport(UIKit)
            let l = CADisplayLink(target: self, selector: #selector(tick(_:)))
            #else
            let l = displayLink(target: self, selector: #selector(tick(_:)))
            #endif
            l.add(to: .main, forMode: .common)
            link = l
        }
        link?.isPaused = false
    }

    @objc private func tick(_ l: CADisplayLink) {
        let dt = lastTimestamp == 0 ? 1000.0 / 60 : (l.timestamp - lastTimestamp) * 1000
        lastTimestamp = l.timestamp
        engine.advance(ms: Float(dt))
        let flinging = stepFling(dt: dt)
        guard let gpu, engine.mounted else { return }
        let frame = engine.frame()
        let animating = vard_gpu_render(gpu, engine.handle)
        framesDrawn += 1
        if let frame {
            lastFrame = frame
            syncInputs(frame)
            if frame.dark != lastDark {
                lastDark = frame.dark
            }
            onFrame?(frame)
        }
        if !animating && !flinging {
            l.isPaused = true
            lastTimestamp = 0
        }
    }

    // MARK: input

    private func point(_ p: CGPoint) -> (Float, Float) { (Float(p.x), Float(p.y)) }

    #if canImport(UIKit)
    private func installGestures() {
        let tap = UITapGestureRecognizer(target: self, action: #selector(onTap(_:)))
        tap.cancelsTouchesInView = false
        addGestureRecognizer(tap)
        let pan = UIPanGestureRecognizer(target: self, action: #selector(onPan(_:)))
        addGestureRecognizer(pan)
        let edge = UIScreenEdgePanGestureRecognizer(target: self, action: #selector(onEdge(_:)))
        edge.edges = .left
        addGestureRecognizer(edge)
        pan.require(toFail: edge)
    }

    @objc private func onTap(_ g: UITapGestureRecognizer) {
        endEditing(true)
        let (x, y) = point(g.location(in: self))
        fling = nil
        engine.tap(x, y)
        requestFrame()
    }

    private var panLast: CGPoint = .zero

    @objc private func onPan(_ g: UIPanGestureRecognizer) {
        let t = g.translation(in: self)
        switch g.state {
        case .began:
            fling = nil
            panLast = t
        case .changed:
            let (x, y) = point(g.location(in: self))
            engine.scroll(x, y, dy: Float(panLast.y - t.y))
            panLast = t
            requestFrame()
        case .ended:
            fling = (g.location(in: self), -g.velocity(in: self).y)
            requestFrame()
        default:
            break
        }
    }

    @objc private func onEdge(_ g: UIScreenEdgePanGestureRecognizer) {
        guard engine.backEnabled || g.state != .began else { return }
        let progress = Float(min(1, max(0, g.translation(in: self).x / max(bounds.width, 1))))
        switch g.state {
        case .changed: engine.backProgress(progress)
        case .ended: if progress > 0.3 || g.velocity(in: self).x > 500 { engine.back() } else { engine.backCancel() }
        case .cancelled, .failed: engine.backCancel()
        default: break
        }
        requestFrame()
    }
    #else
    public override var acceptsFirstResponder: Bool { true }

    public override func mouseUp(with event: NSEvent) {
        window?.makeFirstResponder(self)
        let (x, y) = point(convert(event.locationInWindow, from: nil))
        fling = nil
        engine.tap(x, y)
        requestFrame()
    }

    public override func scrollWheel(with event: NSEvent) {
        let (x, y) = point(convert(event.locationInWindow, from: nil))
        let dy = event.hasPreciseScrollingDeltas ? event.scrollingDeltaY : event.scrollingDeltaY * 10
        engine.scroll(x, y, dy: Float(-dy))
        requestFrame()
    }

    public override func keyDown(with event: NSEvent) {
        // Escape and ⌘[ are "back"
        let isBack = event.keyCode == 53 || (event.modifierFlags.contains(.command) && event.charactersIgnoringModifiers == "[")
        if isBack, engine.backEnabled {
            engine.back()
            requestFrame()
        } else {
            super.keyDown(with: event)
        }
    }
    #endif

    /// Decaying scroll velocity after a pan ends (iOS); true while still moving.
    private func stepFling(dt: Double) -> Bool {
        guard let (p, v0) = fling else { return false }
        var v = v0
        let step = v * CGFloat(dt / 1000)
        let (x, y) = point(p)
        let moved = engine.scroll(x, y, dy: Float(step))
        v *= CGFloat(pow(0.998, dt))
        if abs(v) < 20 || !moved {
            fling = nil
            return false
        }
        fling = (p, v)
        return true
    }

    // MARK: native text fields over Input records

    private func syncInputs(_ frame: VardFrame) {
        var seen = Set<UInt32>()
        for r in frame.records where r.isInput {
            seen.insert(r.uid)
            let f = inputs[r.uid] ?? makeField(r.uid)
            let rect = CGRect(x: CGFloat(r.rect.x + r.pad[0]), y: CGFloat(r.rect.y + r.pad[1]),
                              width: CGFloat(r.rect.w - r.pad[0] - r.pad[2]), height: CGFloat(r.rect.h - r.pad[1] - r.pad[3]))
            if f.frame != rect { f.frame = rect }
            style(f, r)
        }
        for (uid, f) in inputs where !seen.contains(uid) {
            f.removeFromSuperview()
            inputs[uid] = nil
        }
    }

    private func color(_ c: UInt32, _ opacity: Float) -> CGColor {
        let (r, g, b, a) = c.rgba
        return CGColor(red: r, green: g, blue: b, alpha: a * Double(opacity))
    }

    private func font(_ r: VardFrame.Record) -> Any {
        #if canImport(UIKit)
        if !r.font.isEmpty, let f = UIFont(name: r.font, size: CGFloat(r.fontPx)) { return f }
        return UIFont.systemFont(ofSize: CGFloat(r.fontPx), weight: r.weight >= 600 ? .semibold : .regular)
        #else
        if !r.font.isEmpty, let f = NSFont(name: r.font, size: CGFloat(r.fontPx)) { return f }
        return NSFont.systemFont(ofSize: CGFloat(r.fontPx), weight: r.weight >= 600 ? .semibold : .regular)
        #endif
    }

    #if canImport(UIKit)
    private func makeField(_ uid: UInt32) -> UITextField {
        let f = UITextField()
        f.borderStyle = .none
        f.backgroundColor = .clear
        f.tag = Int(uid)
        f.addTarget(self, action: #selector(fieldChanged(_:)), for: .editingChanged)
        f.addTarget(self, action: #selector(fieldSubmit(_:)), for: .editingDidEndOnExit)
        f.addTarget(self, action: #selector(fieldFocus(_:)), for: .editingDidBegin)
        f.addTarget(self, action: #selector(fieldBlur(_:)), for: .editingDidEnd)
        addSubview(f)
        inputs[uid] = f
        return f
    }

    private func style(_ f: UITextField, _ r: VardFrame.Record) {
        if !f.isFirstResponder, f.text != r.text { f.text = r.text }
        f.textColor = UIColor(cgColor: color(r.color, r.opacity))
        f.font = font(r) as? UIFont
        f.isSecureTextEntry = r.secure
        f.isEnabled = !r.disabled
        f.alpha = CGFloat(r.opacity)
        f.textAlignment = r.textAlign == 1 ? .center : r.textAlign == 2 ? .right : .natural
        f.keyboardType = [.default, .emailAddress, .decimalPad, .phonePad, .URL][min(Int(r.keyboard), 4)]
        f.autocapitalizationType = r.keyboard == 0 ? .sentences : .none
        f.attributedPlaceholder = NSAttributedString(string: r.placeholder, attributes: [.foregroundColor: UIColor(cgColor: color(r.placeholderColor, r.opacity))])
    }

    @objc private func fieldChanged(_ f: UITextField) {
        engine.inputText(uid: UInt32(f.tag), f.text ?? "")
        requestFrame()
    }

    @objc private func fieldSubmit(_ f: UITextField) {
        engine.submit(uid: UInt32(f.tag))
        requestFrame()
    }

    @objc private func fieldFocus(_ f: UITextField) {
        engine.focus(uid: UInt32(f.tag))
        requestFrame()
    }

    @objc private func fieldBlur(_ f: UITextField) {
        engine.focus(uid: 0)
        requestFrame()
    }
    #else
    private final class Delegate: NSObject, NSTextFieldDelegate {
        weak var view: VardView?
        func controlTextDidChange(_ n: Notification) {
            guard let f = n.object as? NSTextField, let v = view else { return }
            v.engine.inputText(uid: UInt32(f.tag), f.stringValue)
            v.requestFrame()
        }
        func controlTextDidBeginEditing(_ n: Notification) {
            guard let f = n.object as? NSTextField, let v = view else { return }
            v.engine.focus(uid: UInt32(f.tag))
        }
        func controlTextDidEndEditing(_ n: Notification) {
            guard let f = n.object as? NSTextField, let v = view else { return }
            if (n.userInfo?["NSTextMovement"] as? Int) == NSTextMovement.return.rawValue {
                v.engine.submit(uid: UInt32(f.tag))
            }
            v.engine.focus(uid: 0)
            v.requestFrame()
        }
    }

    private lazy var delegate: Delegate = {
        let d = Delegate()
        d.view = self
        return d
    }()

    private func makeField(_ uid: UInt32) -> NSTextField {
        let f = NSTextField()
        f.tag = Int(uid)
        configure(f)
        addSubview(f)
        inputs[uid] = f
        return f
    }

    private func configure(_ f: NSTextField) {
        f.isBordered = false
        f.drawsBackground = false
        f.focusRingType = .none
        f.isBezeled = false
        f.delegate = delegate
        f.cell?.isScrollable = true
        f.cell?.wraps = false
    }

    private func style(_ f0: NSTextField, _ r: VardFrame.Record) {
        var f = f0
        // NSSecureTextField is its own class: swap the field when `secure` changes
        if r.secure != (f is NSSecureTextField) {
            let n: NSTextField = r.secure ? NSSecureTextField() : NSTextField()
            n.tag = f.tag
            n.frame = f.frame
            configure(n)
            f.removeFromSuperview()
            addSubview(n)
            inputs[r.uid] = n
            f = n
        }
        let editing = f.currentEditor() != nil
        if !editing, f.stringValue != r.text { f.stringValue = r.text }
        f.textColor = NSColor(cgColor: color(r.color, r.opacity))
        f.font = font(r) as? NSFont
        f.isEnabled = !r.disabled
        f.alignment = r.textAlign == 1 ? .center : r.textAlign == 2 ? .right : .natural
        f.placeholderAttributedString = NSAttributedString(string: r.placeholder, attributes: [
            .foregroundColor: NSColor(cgColor: color(r.placeholderColor, r.opacity)) ?? .placeholderTextColor,
            .font: (font(r) as? NSFont) ?? .systemFont(ofSize: 15),
        ])
    }
    #endif
}
