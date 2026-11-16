package dev.vard.host

import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Decoded `vard_frame` buffer (format v2: crates/vard-ffi/src/frame.rs). Units are dp. */
class Frame(val dark: Boolean, val backEnabled: Boolean, val background: Int, val records: List<Record>) {

    class Line(val text: String, val width: Float)

    class Record(
        val uid: Int,
        val kind: Int,
        val flags: Int,
        val textAlign: Int,
        val fit: Int,
        val x: Float, val y: Float, val w: Float, val h: Float,
        val scale: Float,
        val cx: Float, val cy: Float, val cw: Float, val ch: Float,
        /** rounded clip of the innermost `clip: true` ancestor: tl tr br bl */
        val clipRadii: FloatArray,
        val opacity: Float,
        /** 0xRRGGBBAA */
        val background: Int,
        val gradient: IntArray,
        val gradientAngle: Float,
        /** tl tr br bl */
        val radii: FloatArray,
        val border: Float,
        /** l t r b */
        val sides: FloatArray,
        val borderColor: Int,
        val shadowY: Float,
        val shadowBlur: Float,
        val shadowColor: Int,
        val color: Int,
        /** Input: placeholder color; Image: tint (0 = none) */
        val placeholderColor: Int,
        val fontPx: Float,
        val lineHeight: Float,
        val weight: Int,
        val keyboard: Int,
        val letterSpacing: Float,
        val padLeft: Float, val padTop: Float, val padRight: Float, val padBottom: Float,
        val font: String,
        val text: String,
        val placeholder: String,
        val lines: List<Line>,
    ) {
        val hasBackground get() = flags and F_BACKGROUND != 0
        val strike get() = flags and F_STRIKE != 0
        val secure get() = flags and F_SECURE != 0
        val italic get() = flags and F_ITALIC != 0
        val disabled get() = flags and F_DISABLED != 0
        val underline get() = flags and F_UNDERLINE != 0
        val clipsSelf get() = cx > x || cy > y || cx + cw < x + w || cy + ch < y + h
        val roundedClip get() = clipRadii.any { it > 0f }
        val uniformRadius get() = radii.all { it == radii[0] }
    }

    companion object {
        const val KIND_SCROLL = 3
        const val KIND_TEXT = 4
        const val KIND_INPUT = 5
        const val KIND_IMAGE = 6

        const val ALIGN_CENTER = 1
        const val ALIGN_END = 2

        const val FIT_COVER = 0
        const val FIT_CONTAIN = 1
        const val FIT_FILL = 2

        const val F_BACKGROUND = 1
        const val F_STRIKE = 2
        const val F_SECURE = 4
        const val F_ITALIC = 8
        const val F_DISABLED = 16
        const val F_CHECKED = 32
        const val F_UNDERLINE = 64

        private const val VERSION = 2

        fun decode(bytes: ByteArray): Frame? {
            if (bytes.size < 16) return null
            val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
            if (b.get() != 'V'.code.toByte() || b.get() != 'R'.code.toByte() ||
                b.get() != 'D'.code.toByte() || b.get() != 'F'.code.toByte()
            ) return null
            if (b.short.toInt() != VERSION) return null
            val hflags = b.short.toInt()
            val background = b.int
            val count = b.int
            fun str(): String {
                val n = b.int
                val s = String(bytes, b.position(), n, Charsets.UTF_8)
                b.position(b.position() + n)
                return s
            }
            fun f4() = floatArrayOf(b.float, b.float, b.float, b.float)
            val records = ArrayList<Record>(count)
            repeat(count) {
                val uid = b.int
                val kind = b.get().toInt() and 0xFF
                val flags = b.get().toInt() and 0xFF
                val align = b.get().toInt() and 0xFF
                val fit = b.get().toInt() and 0xFF
                val x = b.float; val y = b.float; val w = b.float; val h = b.float
                val scale = b.float
                val cx = b.float; val cy = b.float; val cw = b.float; val ch = b.float
                val clipRadii = f4()
                val opacity = b.float
                val bg = b.int
                val gradient = IntArray(b.int) { b.int }
                val angle = b.float
                val radii = f4()
                val border = b.float
                val sides = f4()
                val borderColor = b.int
                val shadowY = b.float
                val shadowBlur = b.float
                val shadowColor = b.int
                val color = b.int
                val placeholderColor = b.int
                val fontPx = b.float
                val lineHeight = b.float
                val weight = b.short.toInt() and 0xFFFF
                val keyboard = b.get().toInt() and 0xFF
                b.get()
                val letterSpacing = b.float
                val pl = b.float; val pt = b.float; val pr = b.float; val pb = b.float
                val font = str()
                val text = str()
                val placeholder = str()
                val n = b.int
                val lines = ArrayList<Line>(n)
                repeat(n) { lines += Line(str(), b.float) }
                records += Record(
                    uid, kind, flags, align, fit, x, y, w, h, scale, cx, cy, cw, ch, clipRadii, opacity, bg, gradient, angle,
                    radii, border, sides, borderColor, shadowY, shadowBlur, shadowColor, color, placeholderColor, fontPx,
                    lineHeight, weight, keyboard, letterSpacing, pl, pt, pr, pb, font, text, placeholder, lines,
                )
            }
            return Frame(hflags and 1 != 0, hflags and 2 != 0, background, records)
        }

        /** Vard 0xRRGGBBAA with extra [opacity] → Android 0xAARRGGBB. */
        fun argb(rgba: Int, opacity: Float = 1f): Int {
            val a = (((rgba and 0xFF) * opacity) + 0.5f).toInt().coerceIn(0, 255)
            return (a shl 24) or (rgba ushr 8)
        }
    }
}
