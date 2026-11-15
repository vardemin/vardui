package dev.vard.host

/**
 * JNI surface of `crates/vard-ffi` (libvard_ffi.so). Coordinates and sizes are dp.
 * Event calls return 1 (handled), 0 (nothing reacted) or -1 (error / not mounted).
 * All calls must happen on the UI thread.
 */
object VardNative {
    init {
        System.loadLibrary("vard_ffi")
    }

    /** Compile [paths]/[texts] (parallel arrays) for a [width]×[height] dp screen. */
    @JvmStatic external fun create(paths: Array<String>, texts: Array<String>, width: Float, height: Float): Long
    @JvmStatic external fun destroy(handle: Long)
    /** Mount [view] (null = `App`, else the last view in user code). 0 ok, -1 see [diagnostics]. */
    @JvmStatic external fun mount(handle: Long, view: String?): Int
    @JvmStatic external fun tap(handle: Long, x: Float, y: Float): Int
    @JvmStatic external fun inputText(handle: Long, uid: Int, text: String): Int
    @JvmStatic external fun submit(handle: Long, uid: Int): Int
    @JvmStatic external fun scroll(handle: Long, x: Float, y: Float, dy: Float): Int
    @JvmStatic external fun setDark(handle: Long, dark: Boolean)
    @JvmStatic external fun resize(handle: Long, width: Float, height: Float)
    /** The laid-out screen; layout documented in crates/vard-ffi/src/frame.rs. */
    @JvmStatic external fun frame(handle: Long): ByteArray
    @JvmStatic external fun diagnostics(handle: Long): String
}
