package dev.vard.host

/**
 * JNI surface of `crates/vard-ffi` (libvard_ffi.so). Coordinates and sizes are dp.
 * Event calls return 1 (handled), 0 (nothing reacted) or -1 (error / not mounted).
 * All calls must happen on the UI thread, and never from inside [VardHost.call]
 * (the runtime is mid-evaluation there; post to the main looper instead).
 */
object VardNative {
    init {
        System.loadLibrary("vard_ffi")
    }

    /** Compile [paths]/[texts] (parallel arrays) for a [width]×[height] dp screen. */
    @JvmStatic external fun create(paths: Array<String>, texts: Array<String>, width: Float, height: Float): Long
    @JvmStatic external fun destroy(handle: Long)
    /** Install the native-module dispatcher (before [mount]); [host] implements `call`. */
    @JvmStatic external fun setHost(handle: Long, host: VardHost, platform: String)
    /** Register a project font (`Inter-SemiBold.ttf`); the core measures text with it. */
    @JvmStatic external fun addFont(handle: Long, fileName: String, bytes: ByteArray): Int
    /** Mount [view] (null = `App`, else the last view in user code). 0 ok, -1 see [diagnostics]. */
    @JvmStatic external fun mount(handle: Long, view: String?): Int
    @JvmStatic external fun tap(handle: Long, x: Float, y: Float): Int
    @JvmStatic external fun inputText(handle: Long, uid: Int, text: String): Int
    @JvmStatic external fun submit(handle: Long, uid: Int): Int
    /** Input [uid] has keyboard focus (0 = none); the core keeps it above the keyboard. */
    @JvmStatic external fun focus(handle: Long, uid: Int)
    @JvmStatic external fun scroll(handle: Long, x: Float, y: Float, dy: Float): Int
    /** The system light/dark setting; the app's `setColorScheme` may override it. */
    @JvmStatic external fun setDark(handle: Long, dark: Boolean)
    @JvmStatic external fun resize(handle: Long, width: Float, height: Float)
    /** System bars + cutout insets and the keyboard height, dp. */
    @JvmStatic external fun setInsets(handle: Long, top: Float, bottom: Float, left: Float, right: Float, keyboard: Float)
    /** Answer a `{"$callback": id}` argument with a JSON value; [keep] for repeating listeners. */
    @JvmStatic external fun callback(handle: Long, id: Long, json: String, keep: Boolean): Int
    @JvmStatic external fun release(handle: Long, id: Long)
    /** Deliver `extern event name(...)` with a JSON array of arguments. */
    @JvmStatic external fun emit(handle: Long, name: String, json: String): Int
    /** Does an enabled `on back` handler take the next back gesture? */
    @JvmStatic external fun backEnabled(handle: Long): Boolean
    @JvmStatic external fun backProgress(handle: Long, progress: Float)
    @JvmStatic external fun backCancel(handle: Long)
    /** Back committed: 1 = the app handled it, 0 = perform the system back. */
    @JvmStatic external fun back(handle: Long): Int
    /** The laid-out screen; layout documented in crates/vard-ffi/src/frame.rs. */
    @JvmStatic external fun frame(handle: Long): ByteArray
    @JvmStatic external fun diagnostics(handle: Long): String
}
