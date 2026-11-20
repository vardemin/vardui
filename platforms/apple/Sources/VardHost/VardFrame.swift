import Foundation

/// Decoded `vard_frame` buffer (format v3, `crates/vard-runtime/src/frame.rs`). Units are points.
/// The GPU renderer draws the frame itself; the host reads it for the header flags and to
/// place native text fields over `Input` records.
public struct VardFrame {
    public struct Rect: Equatable {
        public var x, y, w, h: Float
    }

    public struct Record {
        public var uid: UInt32
        public var kind: UInt8
        public var flags: UInt8
        public var textAlign: UInt8
        public var fit: UInt8
        public var rect: Rect
        public var scale: Float
        public var clip: Rect
        public var opacity: Float
        public var color: UInt32
        public var placeholderColor: UInt32
        public var fontPx: Float
        public var lineHeight: Float
        public var weight: UInt16
        public var keyboard: UInt8
        public var letterSpacing: Float
        public var pad: [Float]
        public var font: String
        public var text: String
        public var placeholder: String
        public var lines: [(String, Float)]

        public var isInput: Bool { kind == 5 }
        public var secure: Bool { flags & 4 != 0 }
        public var disabled: Bool { flags & 16 != 0 }
        public var italic: Bool { flags & 8 != 0 }
        /// Input with `lines` > 1: wraps, Enter inserts a newline.
        public var multiline: Bool { flags & 128 != 0 }
    }

    public var dark: Bool
    public var backEnabled: Bool
    public var animating: Bool
    public var background: UInt32
    public var records: [Record]

    public static func decode(_ data: Data) -> VardFrame? {
        var r = Reader(data: [UInt8](data))
        guard r.bytes(4) == Array("VRDF".utf8), r.u16() == 3 else { return nil }
        let flags = r.u16()
        let background = r.u32()
        let count = Int(r.u32())
        var records: [Record] = []
        records.reserveCapacity(count)
        for _ in 0..<count {
            let uid = r.u32()
            let kind = r.u8(), f = r.u8(), align = r.u8(), fit = r.u8()
            let rect = r.rect()
            let scale = r.f32()
            let clip = r.rect()
            _ = r.f4() // clip radii
            let opacity = r.f32()
            _ = r.u32() // background
            let n = Int(r.u32())
            for _ in 0..<n { _ = r.u32() }
            _ = r.f32() // gradient angle
            _ = r.f4() // radii
            _ = r.f32(); _ = r.f4(); _ = r.u32() // border, sides, border color
            _ = r.f32(); _ = r.f32(); _ = r.u32() // shadow
            let color = r.u32(), placeholderColor = r.u32()
            let px = r.f32(), lh = r.f32()
            let weight = r.u16()
            let keyboard = r.u8()
            _ = r.u8()
            let spacing = r.f32()
            let pad = r.f4()
            let font = r.str(), text = r.str(), placeholder = r.str()
            let ln = Int(r.u32())
            var lines: [(String, Float)] = []
            for _ in 0..<ln { lines.append((r.str(), r.f32())) }
            guard !r.failed else { return nil }
            records.append(Record(uid: uid, kind: kind, flags: f, textAlign: align, fit: fit, rect: rect, scale: scale, clip: clip,
                                  opacity: opacity, color: color, placeholderColor: placeholderColor, fontPx: px, lineHeight: lh,
                                  weight: weight, keyboard: keyboard, letterSpacing: spacing, pad: pad, font: font, text: text,
                                  placeholder: placeholder, lines: lines))
        }
        guard !r.failed, r.pos == r.data.count else { return nil }
        return VardFrame(dark: flags & 1 != 0, backEnabled: flags & 2 != 0, animating: flags & 4 != 0, background: background, records: records)
    }

    private struct Reader {
        let data: [UInt8]
        var pos = 0
        var failed = false

        mutating func bytes(_ n: Int) -> [UInt8] {
            guard pos + n <= data.count else { failed = true; return [UInt8](repeating: 0, count: n) }
            defer { pos += n }
            return Array(data[pos..<pos + n])
        }
        mutating func u8() -> UInt8 { bytes(1)[0] }
        mutating func u16() -> UInt16 { let b = bytes(2); return UInt16(b[0]) | UInt16(b[1]) << 8 }
        mutating func u32() -> UInt32 {
            let b = bytes(4)
            return UInt32(b[0]) | UInt32(b[1]) << 8 | UInt32(b[2]) << 16 | UInt32(b[3]) << 24
        }
        mutating func f32() -> Float { Float(bitPattern: u32()) }
        mutating func f4() -> [Float] { [f32(), f32(), f32(), f32()] }
        mutating func rect() -> Rect { Rect(x: f32(), y: f32(), w: f32(), h: f32()) }
        mutating func str() -> String {
            let n = Int(u32())
            return String(decoding: bytes(n), as: UTF8.self)
        }
    }
}

extension UInt32 {
    /// 0xRRGGBBAA components in 0...1.
    var rgba: (Double, Double, Double, Double) {
        (Double(self >> 24) / 255, Double((self >> 16) & 0xFF) / 255, Double((self >> 8) & 0xFF) / 255, Double(self & 0xFF) / 255)
    }
}
