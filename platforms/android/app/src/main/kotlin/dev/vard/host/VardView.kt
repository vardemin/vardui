package dev.vard.host

import android.annotation.SuppressLint
import android.content.Context
import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.Typeface
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.util.Log
import android.util.TypedValue
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import android.widget.FrameLayout
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * Hosts one Vard app: the Rust core owns state, layout and hit-testing; this view draws the
 * frame's records on a Canvas, forwards touches, and overlays a real [EditText] on every
 * `Input` so the platform IME (composition, autocorrect, selection) works.
 */
@SuppressLint("ViewConstructor")
class VardView(context: Context, private val files: List<Pair<String, String>>) : FrameLayout(context) {

    private var handle = 0L
    private var frame: Frame? = null
    private var failure: String? = null
    private var loggedDiag = 0
    private val density = resources.displayMetrics.density
    private val inputs = HashMap<Int, VardInput>()

    /** Called after each frame with the app's theme (dark = true), e.g. to style system bars. */
    var onTheme: ((Boolean) -> Unit)? = null

    private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    private val text = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG)
    private val rect = RectF()

    private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop
    private var downX = 0f
    private var downY = 0f
    private var lastY = 0f
    private var scrolling = false

    init {
        setWillNotDraw(false)
        isClickable = true
        isFocusableInTouchMode = true // lets taps outside an Input take focus from it
    }

    private fun systemDark() =
        resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        if (w == 0 || h == 0) return
        val (dw, dh) = w / density to h / density
        if (handle == 0L && failure == null) {
            handle = VardNative.create(files.map { it.first }.toTypedArray(), files.map { it.second }.toTypedArray(), dw, dh)
            if (handle == 0L) {
                failure = "vard: could not create the runtime"
            } else {
                VardNative.setDark(handle, systemDark())
                if (VardNative.mount(handle, null) != 0) failure = VardNative.diagnostics(handle)
            }
        } else if (handle != 0L) {
            VardNative.resize(handle, dw, dh)
        }
        // We are inside a layout pass: adding/resizing Input overlays now would be dropped.
        post { refresh() }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (handle != 0L) {
            VardNative.setDark(handle, systemDark())
            refresh()
        }
    }

    fun destroy() {
        inputs.values.forEach { removeView(it) }
        inputs.clear()
        if (handle != 0L) VardNative.destroy(handle)
        handle = 0L
    }

    /** Pull a new frame after any event and sync overlays. */
    private fun refresh() {
        if (handle == 0L) {
            invalidate()
            return
        }
        logDiagnostics()
        val f = if (failure == null) Frame.decode(VardNative.frame(handle)) else null
        frame = f
        if (f != null) {
            (parent as? View)?.setBackgroundColor(Frame.argb(f.background))
            onTheme?.invoke(f.dark)
            syncInputs(f)
        }
        invalidate()
    }

    private fun logDiagnostics() {
        val d = VardNative.diagnostics(handle)
        if (d.length > loggedDiag) {
            d.substring(loggedDiag).trim().lines().forEach { if (it.isNotBlank()) Log.w(TAG, it) }
            loggedDiag = d.length
        }
    }

    // ---------------- drawing ----------------

    override fun onDraw(canvas: Canvas) {
        val f = frame
        if (f == null) {
            drawFailure(canvas)
            return
        }
        canvas.drawColor(Frame.argb(f.background))
        val d = density
        for (r in f.records) {
            canvas.save()
            canvas.clipRect(r.cx * d, r.cy * d, (r.cx + r.cw) * d, (r.cy + r.ch) * d)
            if (r.hasBackground) {
                val rad = min(r.radius, min(r.w, r.h) / 2f).coerceAtLeast(0f) * d
                rect.set(r.x * d, r.y * d, (r.x + r.w) * d, (r.y + r.h) * d)
                fill.color = Frame.argb(r.background, r.opacity)
                canvas.drawRoundRect(rect, rad, rad, fill)
            }
            if (r.border > 0f) {
                val bw = r.border
                val rad = (min(r.radius, min(r.w, r.h) / 2f) - bw / 2f).coerceAtLeast(0f) * d
                rect.set((r.x + bw / 2f) * d, (r.y + bw / 2f) * d, (r.x + r.w - bw / 2f) * d, (r.y + r.h - bw / 2f) * d)
                stroke.strokeWidth = bw * d
                stroke.color = Frame.argb(r.borderColor, r.opacity)
                canvas.drawRoundRect(rect, rad, rad, stroke)
            }
            if (r.kind == Frame.KIND_TEXT) drawText(canvas, r)
            canvas.restore()
        }
    }

    private fun drawText(canvas: Canvas, r: Frame.Record) {
        val d = density
        text.textSize = r.fontPx * d
        text.typeface = if (r.bold) Typeface.DEFAULT_BOLD else Typeface.DEFAULT
        text.color = Frame.argb(r.color, r.opacity)
        val fm = text.fontMetrics
        val innerW = r.w - r.padLeft - r.padRight
        r.lines.forEachIndexed { i, line ->
            // Lines come wrapped by the core; re-measure with the platform shaper for alignment.
            val mw = text.measureText(line.text) / d
            val x = r.x + r.padLeft + when (r.textAlign) {
                Frame.ALIGN_CENTER -> (innerW - mw) / 2f
                Frame.ALIGN_END -> innerW - mw
                else -> 0f
            }
            val top = (r.y + r.padTop + i * r.lineHeight) * d
            val baseline = top + (r.lineHeight * d - (fm.descent - fm.ascent)) / 2f - fm.ascent
            canvas.drawText(line.text, x * d, baseline, text)
            if (r.strike) {
                val sy = top + r.lineHeight * d / 2f
                fill.color = text.color
                canvas.drawRect(x * d, sy, (x + mw) * d, sy + max(1f, r.fontPx / 14f) * d, fill)
            }
        }
    }

    private fun drawFailure(canvas: Canvas) {
        val msg = failure ?: return
        canvas.drawColor(0xFFFFFFFF.toInt())
        text.textSize = 12f * density
        text.typeface = Typeface.MONOSPACE
        text.color = 0xFFB00020.toInt()
        var y = 24f * density
        for (line in msg.lines()) {
            canvas.drawText(line, 12f * density, y, text)
            y += 16f * density
        }
    }

    // ---------------- touch ----------------

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(e: MotionEvent): Boolean {
        if (handle == 0L || frame == null) return super.onTouchEvent(e)
        val d = density
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = e.x; downY = e.y; lastY = e.y
                scrolling = false
            }
            MotionEvent.ACTION_MOVE -> {
                if (!scrolling && abs(e.y - downY) > touchSlop) {
                    scrolling = true
                    lastY = e.y
                }
                if (scrolling) {
                    val dy = (lastY - e.y) / d
                    lastY = e.y
                    if (VardNative.scroll(handle, downX / d, downY / d, dy) == 1) refresh()
                }
            }
            MotionEvent.ACTION_UP -> {
                if (!scrolling) {
                    performClick()
                    if (VardNative.tap(handle, e.x / d, e.y / d) == 1) refresh()
                }
                scrolling = false
            }
            MotionEvent.ACTION_CANCEL -> scrolling = false
        }
        return true
    }

    override fun performClick(): Boolean = super.performClick()

    // ---------------- Input overlays ----------------

    private fun syncInputs(f: Frame) {
        val seen = HashSet<Int>()
        for (r in f.records) {
            if (r.kind != Frame.KIND_INPUT) continue
            seen += r.uid
            val et = inputs.getOrPut(r.uid) { VardInput(r.uid).also { addView(it) } }
            et.bind(r)
        }
        val gone = inputs.keys.filter { it !in seen }
        for (uid in gone) removeView(inputs.remove(uid))
    }

    private fun onInputChanged(uid: Int, value: String) {
        if (handle != 0L && VardNative.inputText(handle, uid, value) == 1) refresh()
    }

    private fun onInputSubmit(uid: Int) {
        if (handle != 0L && VardNative.submit(handle, uid) == 1) refresh()
    }

    private inner class VardInput(val uid: Int) : EditText(context) {
        private var updating = false
        private var bound: Frame.Record? = null

        init {
            background = null
            includeFontPadding = false
            isSingleLine = true
            imeOptions = EditorInfo.IME_ACTION_DONE
            gravity = Gravity.CENTER_VERTICAL or Gravity.START
            addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
                override fun afterTextChanged(s: Editable?) {
                    if (!updating) onInputChanged(uid, s?.toString() ?: "")
                }
            })
            setOnEditorActionListener { _, actionId, event ->
                val enter = event != null && event.keyCode == KeyEvent.KEYCODE_ENTER && event.action == KeyEvent.ACTION_DOWN
                if (actionId == EditorInfo.IME_ACTION_DONE || enter) {
                    onInputSubmit(uid)
                    true
                } else false
            }
        }

        fun bind(r: Frame.Record) {
            val d = density
            val prev = bound
            bound = r
            if (prev == null || prev.x != r.x || prev.y != r.y || prev.w != r.w || prev.h != r.h) {
                layoutParams = LayoutParams((r.w * d).roundToInt(), (r.h * d).roundToInt()).apply {
                    leftMargin = (r.x * d).roundToInt()
                    topMargin = (r.y * d).roundToInt()
                }
            }
            if (prev == null || prev.padLeft != r.padLeft || prev.padRight != r.padRight) {
                setPadding((r.padLeft * d).roundToInt(), 0, (r.padRight * d).roundToInt(), 0)
            }
            if (prev == null || prev.fontPx != r.fontPx || prev.bold != r.bold) {
                setTextSize(TypedValue.COMPLEX_UNIT_PX, r.fontPx * d)
                typeface = if (r.bold) Typeface.DEFAULT_BOLD else Typeface.DEFAULT
            }
            if (prev == null || prev.secure != r.secure) {
                inputType = InputType.TYPE_CLASS_TEXT or
                    if (r.secure) InputType.TYPE_TEXT_VARIATION_PASSWORD else InputType.TYPE_TEXT_FLAG_AUTO_CORRECT
            }
            setTextColor(Frame.argb(r.color))
            setHintTextColor(Frame.argb(r.placeholderColor))
            if (hint?.toString() != r.placeholder) hint = r.placeholder
            alpha = r.opacity
            isEnabled = !r.disabled
            clipBounds = if (r.clipsSelf) {
                Rect(
                    ((r.cx - r.x) * d).roundToInt(), ((r.cy - r.y) * d).roundToInt(),
                    ((r.cx + r.cw - r.x) * d).roundToInt(), ((r.cy + r.ch - r.y) * d).roundToInt(),
                )
            } else null
            // Mirror state-driven changes (e.g. the draft cleared after Add) into the widget.
            if (text.toString() != r.text) {
                updating = true
                setText(r.text)
                setSelection(r.text.length)
                updating = false
            }
        }
    }

    companion object {
        private const val TAG = "Vard"
    }
}
