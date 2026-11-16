package dev.vard.host

import android.content.Context
import android.graphics.Paint
import android.graphics.Typeface
import android.os.Build
import kotlin.math.abs

/**
 * Font resolution shared by drawing ([VardView]) and measuring ([VardHost.measure]), so the
 * core's line breaks are computed with exactly the typeface that is drawn. Project fonts
 * (`fonts/Inter-SemiBold.ttf`) win; otherwise the system font at the requested weight.
 */
class VardFonts(private val context: Context, fontFiles: List<String>) {
    private class FontFile(val asset: String, val family: String, val weight: Int, val italic: Boolean)

    private val files = fontFiles.map { f ->
        val (fam, w, it) = VardView.parseFontName(f.substringAfterLast('/').substringBeforeLast('.'))
        FontFile(f, fam.lowercase(), w, it)
    }
    private val cache = HashMap<String, Typeface>()
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG)
    private val density = context.resources.displayMetrics.density

    fun typeface(family: String, weight: Int, italic: Boolean): Typeface = cache.getOrPut("$family|$weight|$italic") {
        val fam = family.lowercase()
        val best = files.filter { it.family == fam }.minByOrNull { abs(it.weight - weight) + if (it.italic != italic) 1000 else 0 }
        when {
            best != null -> try {
                Typeface.createFromAsset(context.assets, best.asset)
            } catch (e: Exception) {
                Typeface.DEFAULT
            }
            Build.VERSION.SDK_INT >= 28 -> Typeface.create(Typeface.DEFAULT, weight, italic)
            else -> Typeface.create(Typeface.DEFAULT, (if (weight >= 600) Typeface.BOLD else 0) or (if (italic) Typeface.ITALIC else 0))
        }
    }

    /** Width in dp, measured at device pixels like it is drawn. */
    fun measure(text: String, family: String, weight: Int, italic: Boolean, px: Float): Float {
        paint.typeface = typeface(family, weight, italic)
        paint.textSize = px * density
        paint.letterSpacing = 0f
        return paint.measureText(text) / density
    }
}
