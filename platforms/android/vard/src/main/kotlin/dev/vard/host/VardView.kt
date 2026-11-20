package dev.vard.host

import android.annotation.SuppressLint
import android.content.Context
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PorterDuff
import android.graphics.PorterDuffColorFilter
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.Shader
import android.graphics.Typeface
import android.os.Build
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.util.Log
import android.util.TypedValue
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.ViewConfiguration
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import android.widget.FrameLayout
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sin

/**
 * Hosts one Vard app: the Rust core owns state, layout and hit-testing; this view draws the
 * frame, forwards touches, and overlays a real [EditText] on every `Input` so the platform
 * IME (composition, autocorrect, selection) works.
 *
 * Drawing: with [gpu] (default) a [SurfaceView] behind the overlays is rendered by the
 * core's GPU renderer (`crates/vard-gpu`, Vulkan or GLES) — the same pixels as `vard run
 * --do shot`, text drawn and measured with the core's fonts. Without it, or on a device
 * with no usable GPU API, the frame's records are drawn on a Canvas.
 *
 * The view fills the whole window (edge-to-edge); system bar and keyboard insets are passed
 * to the core with [setInsets] and the app decides where content goes (`safeArea`).
 *
 * @param files the project's `.vard` sources (path, text)
 * @param fontFiles project fonts as asset paths (`app/fonts/Inter-Bold.ttf`), measured by the core
 *        and drawn here with the same files
 * @param assetRoot asset folder of the Vard project, for `Image("assets/...")`
 */
@SuppressLint("ViewConstructor")
class VardView(
    context: Context,
    private val files: List<Pair<String, String>>,
    val host: VardHost,
    private val fontFiles: List<String> = emptyList(),
    private val assetRoot: String = "app",
    /** Draw with the GPU renderer (falls back to Canvas when the device has no GPU API). */
    private val gpu: Boolean = true,
) : FrameLayout(context) {

    private var gpuHandle = 0L
    private val surface: SurfaceView? = if (gpu) SurfaceView(context) else null

    private var handle = 0L
    private var frame: Frame? = null
    private var failure: String? = null
    private var loggedDiag = 0
    private val density = resources.displayMetrics.density
    private val inputs = HashMap<Int, VardInput>()
    private var insets = floatArrayOf(0f, 0f, 0f, 0f, 0f) // top bottom left right keyboard, dp

    /** Called after each frame: (dark theme, app handles back). */
    var onFrame: ((dark: Boolean, backEnabled: Boolean) -> Unit)? = null

    private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val shadowPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    private val text = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG)
    private val imagePaint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG)
    private val rect = RectF()
    private val path = Path()
    private val radii8 = FloatArray(8)

    private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop
    private var downX = 0f
    private var downY = 0f
    private var lastY = 0f
    private var scrolling = false

    init {
        setWillNotDraw(false)
        isClickable = true
        isFocusableInTouchMode = true // lets taps outside an Input take focus from it
        host.onChanged = { refresh() }
        host.fonts = VardFonts(context, fontFiles)
        surface?.let { sv ->
            addView(sv, 0, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
            sv.holder.addCallback(object : SurfaceHolder.Callback {
                override fun surfaceCreated(holder: SurfaceHolder) {}

                override fun surfaceChanged(holder: SurfaceHolder, format: Int, w: Int, h: Int) {
                    if (gpuHandle == 0L) {
                        gpuHandle = try {
                            VardNative.gpuCreate(holder.surface, w, h, density)
                        } catch (e: UnsatisfiedLinkError) {
                            0L // libvard_ffi built with vard.gpu=false
                        }
                        if (gpuHandle == 0L) Log.w(TAG, "GPU renderer unavailable: drawing with Canvas")
                    } else {
                        VardNative.gpuResize(gpuHandle, w, h, density)
                    }
                    refresh()
                }

                override fun surfaceDestroyed(holder: SurfaceHolder) {
                    if (gpuHandle != 0L) VardNative.gpuDestroy(gpuHandle)
                    gpuHandle = 0L
                }
            })
        }
    }

    private fun systemDark() =
        resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        if (w == 0 || h == 0) return
        val (dw, dh) = w / density to h / density
        if (handle == 0L && failure == null) start(dw, dh) else if (handle != 0L) VardNative.resize(handle, dw, dh)
        // We are inside a layout pass: adding/resizing Input overlays now would be dropped.
        post { refresh() }
    }

    private fun start(dw: Float, dh: Float) {
        handle = VardNative.create(files.map { it.first }.toTypedArray(), files.map { it.second }.toTypedArray(), dw, dh)
        if (handle == 0L) {
            failure = "vard: could not create the runtime"
            return
        }
        VardNative.setHost(handle, host, "android", platformText = !gpu)
        for (f in fontFiles) {
            try {
                val bytes = context.assets.open(f).use { it.readBytes() }
                if (VardNative.addFont(handle, f.substringAfterLast('/'), bytes) != 0) Log.w(TAG, "font $f could not be loaded")
            } catch (e: Exception) {
                Log.w(TAG, "font $f: $e")
            }
        }
        VardNative.setDark(handle, systemDark())
        VardNative.setLocale(handle, resources.configuration.locales[0].toLanguageTag())
        VardNative.setInsets(handle, insets[0], insets[1], insets[2], insets[3], insets[4])
        if (VardNative.mount(handle, null) != 0) {
            failure = VardNative.diagnostics(handle)
            return
        }
        host.mounted(handle)
        startInspector()
    }

    /**
     * Debuggable builds only: the action protocol of `vard run` on 127.0.0.1:8472, so
     * `adb forward tcp:8472 tcp:8472 && vard run --device 127.0.0.1:8472 --do tree` drives
     * this screen. Scripts run here on the UI thread, polled every 100 ms.
     */
    private fun startInspector() {
        val debuggable = (context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0
        if (!debuggable || VardNative.inspectListen(handle, "127.0.0.1:8472") != 0) return
        Log.i(TAG, "inspection: adb forward tcp:8472 tcp:8472 && vard run --device 127.0.0.1:8472")
        val poll = object : Runnable {
            override fun run() {
                if (handle == 0L) return
                if (VardNative.inspectPoll(handle)) refresh()
                postDelayed(this, 100)
            }
        }
        postDelayed(poll, 100)
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (handle != 0L) {
            VardNative.setDark(handle, systemDark())
            VardNative.setLocale(handle, newConfig.locales[0].toLanguageTag())
            refresh()
        }
    }

    /** System bars + cutout (px) and keyboard height (px). */
    fun setInsets(top: Int, bottom: Int, left: Int, right: Int, keyboard: Int) {
        val n = floatArrayOf(top / density, bottom / density, left / density, right / density, keyboard / density)
        if (n.contentEquals(insets)) return
        insets = n
        if (handle != 0L) {
            VardNative.setInsets(handle, n[0], n[1], n[2], n[3], n[4])
            refresh()
        }
    }

    // ---------------- back ----------------
    fun backEnabled() = handle != 0L && failure == null && VardNative.backEnabled(handle)

    fun backProgress(progress: Float) {
        if (handle != 0L) {
            VardNative.backProgress(handle, progress)
            refresh()
        }
    }

    fun backCancel() {
        if (handle != 0L) {
            VardNative.backCancel(handle)
            refresh()
        }
    }

    /** True when the app handled it; false = the caller performs the system back. */
    fun back(): Boolean {
        if (handle == 0L) return false
        val r = VardNative.back(handle) == 1
        refresh()
        return r
    }

    fun destroy() {
        if (gpuHandle != 0L) VardNative.gpuDestroy(gpuHandle)
        gpuHandle = 0L
        inputs.values.forEach { removeView(it) }
        inputs.clear()
        host.handle = 0L
        if (handle != 0L) VardNative.destroy(handle)
        handle = 0L
    }

    /** Pull a new frame after any event and sync overlays. */
    fun refresh() {
        if (handle == 0L) {
            invalidate()
            return
        }
        logDiagnostics()
        val f = if (failure == null) Frame.decode(VardNative.frame(handle)) else null
        frame = f
        if (f != null) {
            (parent as? View)?.setBackgroundColor(Frame.argb(f.background))
            onFrame?.invoke(f.dark, f.backEnabled)
            syncInputs(f)
            if (f.animating) scheduleAnimationFrame()
        }
        if (gpuHandle != 0L && failure == null) VardNative.gpuRender(gpuHandle, handle)
        invalidate()
    }

    private var lastVsync = 0L
    private var frameScheduled = false

    /** Drive `animate:`/`enter:` from vsync: advance the core's clock and pull a frame. */
    private fun scheduleAnimationFrame() {
        if (frameScheduled) return
        frameScheduled = true
        android.view.Choreographer.getInstance().postFrameCallback { now ->
            frameScheduled = false
            if (handle == 0L) return@postFrameCallback
            val dt = if (lastVsync == 0L) 16.7f else (now - lastVsync) / 1_000_000f
            lastVsync = now
            VardNative.advance(handle, dt)
            refresh()
            if (frame?.animating != true) lastVsync = 0L
        }
    }

    private fun logDiagnostics() {
        val d = VardNative.diagnostics(handle)
        if (d.length > loggedDiag) {
            d.substring(loggedDiag).trim().lines().forEach { if (it.isNotBlank()) Log.w(TAG, it) }
            loggedDiag = d.length
        }
    }

    // ---------------- fonts ----------------
    /** (typeface, whether the core measured with exactly this typeface). */
    private fun typeface(r: Frame.Record): Pair<Typeface, Boolean> = host.fonts.typeface(r.font, r.weight, r.italic) to true

    // ---------------- images ----------------
    private val bitmaps = HashMap<String, Bitmap?>()
    private val loading = HashSet<String>()
    private val io = Executors.newFixedThreadPool(2)

    private fun bitmap(src: String): Bitmap? {
        if (bitmaps.containsKey(src)) return bitmaps[src]
        if (src.startsWith("http://") || src.startsWith("https://")) {
            if (loading.add(src)) {
                io.execute {
                    val bmp = try {
                        (URL(src).openConnection() as HttpURLConnection).run {
                            connectTimeout = 10_000
                            readTimeout = 15_000
                            inputStream.use { BitmapFactory.decodeStream(it) }
                        }
                    } catch (e: Exception) {
                        Log.w(TAG, "image $src: $e")
                        null
                    }
                    post {
                        bitmaps[src] = bmp
                        invalidate()
                    }
                }
            }
            return null
        }
        val bmp = try {
            context.assets.open("$assetRoot/$src").use { BitmapFactory.decodeStream(it) }
        } catch (e: Exception) {
            Log.w(TAG, "image $src not found in assets/$assetRoot")
            null
        }
        bitmaps[src] = bmp
        return bmp
    }

    // ---------------- drawing ----------------
    private fun shape(x: Float, y: Float, w: Float, h: Float, radii: FloatArray): Path {
        val d = density
        rect.set(x * d, y * d, (x + w) * d, (y + h) * d)
        path.reset()
        for (i in 0 until 4) {
            radii8[i * 2] = radii[i] * d
            radii8[i * 2 + 1] = radii[i] * d
        }
        path.addRoundRect(rect, radii8, Path.Direction.CW)
        return path
    }

    override fun onDraw(canvas: Canvas) {
        val f = frame
        if (f == null) {
            drawFailure(canvas)
            return
        }
        if (gpuHandle != 0L) return // the SurfaceView below shows the GPU frame
        canvas.drawColor(Frame.argb(f.background))
        val d = density
        for (r in f.records) {
            canvas.save()
            if (r.roundedClip) {
                canvas.clipPath(Path(shape(r.cx, r.cy, r.cw, r.ch, r.clipRadii)))
            } else {
                canvas.clipRect(r.cx * d, r.cy * d, (r.cx + r.cw) * d, (r.cy + r.ch) * d)
            }
            if (r.shadowBlur > 0f) {
                val base = if (r.hasBackground) r.background or 0xFF else 0xFFFFFFFF.toInt()
                shadowPaint.color = Frame.argb(base, r.opacity)
                shadowPaint.setShadowLayer(max(0.1f, r.shadowBlur * 0.866f * d), 0f, r.shadowY * d, Frame.argb(r.shadowColor, r.opacity))
                canvas.drawPath(shape(r.x, r.y, r.w, r.h, r.radii), shadowPaint)
            }
            var drewImage = false
            if (r.kind == Frame.KIND_IMAGE) drewImage = drawImage(canvas, r)
            if (r.hasBackground && !drewImage) {
                fill.shader = null
                fill.color = Frame.argb(r.background, r.opacity)
                canvas.drawPath(shape(r.x, r.y, r.w, r.h, r.radii), fill)
            }
            if (r.gradient.isNotEmpty()) drawGradient(canvas, r)
            if (r.border > 0f) {
                val bw = r.border
                val inner = FloatArray(4) { max(0f, r.radii[it] - bw / 2f) }
                stroke.strokeWidth = bw * d
                stroke.color = Frame.argb(r.borderColor, r.opacity)
                canvas.drawPath(shape(r.x + bw / 2f, r.y + bw / 2f, r.w - bw, r.h - bw, inner), stroke)
            }
            if (r.sides.any { it > 0f }) {
                fill.shader = null
                fill.color = Frame.argb(r.borderColor, r.opacity)
                val (l, t, rr, b) = r.sides.toList()
                if (l > 0f) canvas.drawRect(r.x * d, r.y * d, (r.x + l) * d, (r.y + r.h) * d, fill)
                if (t > 0f) canvas.drawRect(r.x * d, r.y * d, (r.x + r.w) * d, (r.y + t) * d, fill)
                if (rr > 0f) canvas.drawRect((r.x + r.w - rr) * d, r.y * d, (r.x + r.w) * d, (r.y + r.h) * d, fill)
                if (b > 0f) canvas.drawRect(r.x * d, (r.y + r.h - b) * d, (r.x + r.w) * d, (r.y + r.h) * d, fill)
            }
            if (r.kind == Frame.KIND_TEXT) drawText(canvas, r)
            canvas.restore()
        }
    }

    private fun drawGradient(canvas: Canvas, r: Frame.Record) {
        val d = density
        val a = Math.toRadians(r.gradientAngle.toDouble())
        val (dx, dy) = sin(a).toFloat() to -cos(a).toFloat()
        val len = abs(r.w * dx) + abs(r.h * dy)
        val (cx, cy) = (r.x + r.w / 2f) to (r.y + r.h / 2f)
        val colors = IntArray(r.gradient.size) { Frame.argb(r.gradient[it], r.opacity) }
        val shader = if (colors.size == 1) null else LinearGradient(
            (cx - dx * len / 2f) * d, (cy - dy * len / 2f) * d, (cx + dx * len / 2f) * d, (cy + dy * len / 2f) * d,
            colors, null, Shader.TileMode.CLAMP,
        )
        fill.shader = shader
        if (shader == null) fill.color = colors[0]
        canvas.drawPath(shape(r.x, r.y, r.w, r.h, r.radii), fill)
        fill.shader = null
    }

    private fun drawImage(canvas: Canvas, r: Frame.Record): Boolean {
        val bmp = bitmap(r.text) ?: return false
        val d = density
        val (iw, ih) = bmp.width.toFloat() to bmp.height.toFloat()
        val (sx, sy) = (r.w * d / iw) to (r.h * d / ih)
        val (kx, ky) = when (r.fit) {
            Frame.FIT_CONTAIN -> min(sx, sy) to min(sx, sy)
            Frame.FIT_FILL -> sx to sy
            else -> max(sx, sy) to max(sx, sy)
        }
        val ox = r.x * d + (r.w * d - iw * kx) / 2f
        val oy = r.y * d + (r.h * d - ih * ky) / 2f
        val m = Matrix().apply {
            setScale(kx, ky)
            postTranslate(ox, oy)
        }
        imagePaint.shader = BitmapShader(bmp, Shader.TileMode.CLAMP, Shader.TileMode.CLAMP).apply { setLocalMatrix(m) }
        imagePaint.alpha = (r.opacity * 255).roundToInt()
        imagePaint.colorFilter = if (r.placeholderColor != 0) PorterDuffColorFilter(Frame.argb(r.placeholderColor), PorterDuff.Mode.SRC_IN) else null
        val p = if (r.fit == Frame.FIT_CONTAIN) {
            val w = min(r.w, iw * kx / d)
            val h = min(r.h, ih * ky / d)
            shape(ox / d, oy / d, w, h, r.radii)
        } else {
            shape(r.x, r.y, r.w, r.h, r.radii)
        }
        canvas.drawPath(p, imagePaint)
        return true
    }

    private fun drawText(canvas: Canvas, r: Frame.Record) {
        val d = density
        val (tf, exact) = typeface(r)
        text.typeface = tf
        text.textSize = r.fontPx * d
        text.letterSpacing = if (r.fontPx > 0f) r.letterSpacing / r.fontPx else 0f
        text.color = Frame.argb(r.color, r.opacity)
        text.isUnderlineText = r.underline
        text.isStrikeThruText = r.strike
        // a system font with synthetic italic (pre-28 or missing face)
        text.textSkewX = if (r.italic && !tf.isItalic && !exact) -0.2f else 0f
        val fm = text.fontMetrics
        val innerW = r.w - r.padLeft - r.padRight
        r.lines.forEachIndexed { i, line ->
            // Lines come wrapped by the core. With a project font its widths are exact;
            // with a system font re-measure for alignment.
            val mw = if (exact) line.width else text.measureText(line.text) / d
            val x = r.x + r.padLeft + when (r.textAlign) {
                Frame.ALIGN_CENTER -> (innerW - mw) / 2f
                Frame.ALIGN_END -> innerW - mw
                else -> 0f
            }
            val top = (r.y + r.padTop + i * r.lineHeight) * d
            val baseline = top + (r.lineHeight * d - (fm.descent - fm.ascent)) / 2f - fm.ascent
            canvas.drawText(line.text, x * d, baseline, text)
        }
    }

    private fun drawFailure(canvas: Canvas) {
        val msg = failure ?: return
        canvas.drawColor(0xFFFFFFFF.toInt())
        text.textSize = 12f * density
        text.typeface = Typeface.MONOSPACE
        text.color = 0xFFB00020.toInt()
        text.isUnderlineText = false
        text.isStrikeThruText = false
        text.letterSpacing = 0f
        var y = 24f * density + insets[0] * density
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
            setOnFocusChangeListener { _, has ->
                if (handle != 0L) {
                    VardNative.focus(handle, if (has) uid else 0)
                    refresh()
                }
            }
            setOnEditorActionListener { _, actionId, event ->
                // a multi-line Input (`lines` > 1): Enter is a newline, not a submit
                if (bound?.multiline == true) return@setOnEditorActionListener false
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
            if (prev == null || prev.padLeft != r.padLeft || prev.padRight != r.padRight || prev.padTop != r.padTop ||
                prev.padBottom != r.padBottom || prev.multiline != r.multiline
            ) {
                // single-line text is centred vertically; multi-line text starts under the top padding
                val (top, bottom) = if (r.multiline) (r.padTop * d).roundToInt() to (r.padBottom * d).roundToInt() else 0 to 0
                setPadding((r.padLeft * d).roundToInt(), top, (r.padRight * d).roundToInt(), bottom)
            }
            if (prev == null || prev.fontPx != r.fontPx || prev.weight != r.weight || prev.font != r.font || prev.letterSpacing != r.letterSpacing) {
                setTextSize(TypedValue.COMPLEX_UNIT_PX, r.fontPx * d)
                typeface = typeface(r).first
                letterSpacing = if (r.fontPx > 0f) r.letterSpacing / r.fontPx else 0f
            }
            // multi-line text keeps the core's line height, so wrapping and height agree
            if (r.multiline && android.os.Build.VERSION.SDK_INT >= 28 && (prev == null || prev.lineHeight != r.lineHeight || !prev.multiline)) {
                lineHeight = (r.lineHeight * d).roundToInt()
            }
            if (prev == null || prev.secure != r.secure || prev.keyboard != r.keyboard || prev.multiline != r.multiline) {
                isSingleLine = !r.multiline
                gravity = if (r.multiline) Gravity.TOP or Gravity.START else Gravity.CENTER_VERTICAL or Gravity.START
                imeOptions = if (r.multiline) EditorInfo.IME_ACTION_NONE else EditorInfo.IME_ACTION_DONE
                inputType = when {
                    r.multiline -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or
                        InputType.TYPE_TEXT_FLAG_CAP_SENTENCES or InputType.TYPE_TEXT_FLAG_AUTO_CORRECT
                    r.secure -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
                    r.keyboard == 1 -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS
                    r.keyboard == 2 -> InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_FLAG_DECIMAL or InputType.TYPE_NUMBER_FLAG_SIGNED
                    r.keyboard == 3 -> InputType.TYPE_CLASS_PHONE
                    r.keyboard == 4 -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
                    else -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_AUTO_CORRECT
                }
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

        /** `Inter-SemiBoldItalic` -> ("Inter", 600, true); same rules as the core. */
        fun parseFontName(stem: String): Triple<String, Int, Boolean> {
            val weights = mapOf(
                "hairline" to 100, "thin" to 100, "extralight" to 200, "ultralight" to 200, "light" to 300,
                "regular" to 400, "normal" to 400, "book" to 400, "medium" to 500, "semibold" to 600, "demibold" to 600,
                "extrabold" to 800, "ultrabold" to 800, "bold" to 700, "black" to 900, "heavy" to 900,
            )
            val cut = max(stem.lastIndexOf('-'), stem.lastIndexOf('_'))
            if (cut < 0) return Triple(stem, 400, false)
            val family = stem.substring(0, cut)
            val style = stem.substring(cut + 1).lowercase()
            val italic = "italic" in style || "oblique" in style
            val rest = style.replace("italic", "").replace("oblique", "")
            if (rest.isEmpty()) return Triple(family, 400, italic)
            val w = weights[rest] ?: return Triple(stem, 400, false)
            return Triple(family, w, italic)
        }
    }
}
