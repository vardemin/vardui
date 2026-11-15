package dev.vard.host

import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Decoded `vard_frame` buffer (format: crates/vard-ffi/src/frame.rs). Units are dp. */
class Frame(val dark: Boolean, val background: Int, val records: List<Record>) {

    class Line(val text: String, val width: Float)

    class Record(
        val uid: Int,
        val kind: Int,
        val flags: Int,
        val textAlign: Int,
        val x: Float, val y: Float, val w: Float, val h: Float,
        val cx: Float, val cy: Float, val cw: Float, val ch: Float,
        val opacity: Float,
        /** 0xRRGGBBAA */
        val background: Int,
        val radius: Float,
        val border: Float,
        val borderColor: Int,
        val color: Int,
        val placeholderColor: Int,
        val fontPx: Float,
        val lineHeight: Float,
        val padLeft: Float, val padTop: Float, val padRight: Float, val padBottom: Float,
        val text: String,
        val placeholder: String,
        val lines: List<Line>,
    ) {
        val hasBackground get() = flags and F_BACKGROUND != 0
        val strike get() = flags and F_STRIKE != 0
        val secure get() = flags and F_SECURE != 0
        val bold get() = flags and F_BOLD != 0
        val disabled get() = flags and F_DISABLED != 0
        val clipsSelf get() = cx > x || cy > y || cx + cw < x + w || cy + ch < y + h
    }

    companion object {
        const val KIND_SCROLL = 3
        const val KIND_TEXT = 4
        const val KIND_INPUT = 5
        const val KIND_IMAGE = 6

        const val ALIGN_CENTER = 1
        const val ALIGN_END = 2

        const val F_BACKGROUND = 1
        const val F_STRIKE = 2
        const val F_SECURE = 4
        const val F_BOLD = 8
        const val F_DISABLED = 16

        private const val VERSION = 1

        fun decode(bytes: ByteArray): Frame? {
            if (bytes.size < 16) return null
            val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
            if (b.get() != 'V'.code.toByte() || b.get() != 'R'.code.toByte() ||
                b.get() != 'D'.code.toByte() || b.get() != 'F'.code.toByte()
            ) return null
            if (b.short.toInt() != VERSION) return null
            val dark = b.short.toInt() and 1 == 1
            val background = b.int
            val count = b.int
            fun str(): String {
                val n = b.int
                val s = String(bytes, b.position(), n, Charsets.UTF_8)
                b.position(b.position() + n)
                return s
            }
            val records = ArrayList<Record>(count)
            repeat(count) {
                val uid = b.int
                val kind = b.get().toInt() and 0xFF
                val flags = b.get().toInt() and 0xFF
                val align = b.get().toInt() and 0xFF
                b.get()
                val x = b.float; val y = b.float; val w = b.float; val h = b.float
                val cx = b.float; val cy = b.float; val cw = b.float; val ch = b.float
                val opacity = b.float
                val bg = b.int
                val radius = b.float
                val border = b.float
                val borderColor = b.int
                val color = b.int
                val placeholderColor = b.int
                val fontPx = b.float
                val lineHeight = b.float
                val pl = b.float; val pt = b.float; val pr = b.float; val pb = b.float
                val text = str()
                val placeholder = str()
                val n = b.int
                val lines = ArrayList<Line>(n)
                repeat(n) { lines += Line(str(), b.float) }
                records += Record(
                    uid, kind, flags, align, x, y, w, h, cx, cy, cw, ch, opacity, bg, radius, border,
                    borderColor, color, placeholderColor, fontPx, lineHeight, pl, pt, pr, pb, text, placeholder, lines,
                )
            }
            return Frame(dark, background, records)
        }

        /** Vard 0xRRGGBBAA with extra [opacity] → Android 0xAARRGGBB. */
        fun argb(rgba: Int, opacity: Float = 1f): Int {
            val a = (((rgba and 0xFF) * opacity) + 0.5f).toInt().coerceIn(0, 255)
            return (a shl 24) or (rgba ushr 8)
        }
    }
}
