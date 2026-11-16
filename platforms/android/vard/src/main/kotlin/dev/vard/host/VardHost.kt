package dev.vard.host

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject

/**
 * A native module: Kotlin implementations of Vard `extern fn`s, keyed by their Vard name.
 *
 * ```kotlin
 * class Haptics : VardModule {
 *     override val functions = mapOf<String, (VardCall) -> Any?>(
 *         "vibrate" to { call -> vibrator.vibrate(call.long(0)); null },
 *         "loadProfile" to { call -> api.load(call.string(0)) { p -> call.callback(1).resolve(p) }; null },
 *     )
 * }
 * ```
 * Return values and callback values are converted to JSON: `null`, String, Number, Boolean,
 * Map, List, JSONObject/JSONArray, or anything implementing [VardJson]. The runtime decodes
 * them to the declared Vard type (struct fields by name, enums by variant name).
 * `vard bindings --kotlin` generates a typed base class from the `.vard` declarations.
 */
interface VardModule {
    val functions: Map<String, (VardCall) -> Any?>

    /**
     * Functions resolved by name pattern, for externs whose Vard signature the app chooses:
     * e.g. every `firestoreGet*` with the app's own document struct. Checked after [functions].
     */
    fun resolve(name: String): ((VardCall) -> Any?)? = null

    /** Called once when the host starts (activity available via [VardHost.activity]). */
    fun install(host: VardHost) {}

    /** Return true when the request code was this module's. */
    fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, results: IntArray): Boolean = false

    fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?): Boolean = false
}

/** Objects that know their own JSON form (e.g. generated binding data classes). */
interface VardJson {
    fun toJson(): Any?
}

/** One `extern fn` invocation: positional JSON arguments. */
class VardCall(val host: VardHost, val name: String, val args: JSONArray) {
    val size get() = args.length()
    fun isNull(i: Int) = args.isNull(i)
    fun string(i: Int): String = if (args.isNull(i)) "" else args.optString(i, "")
    fun stringOrNull(i: Int): String? = if (args.isNull(i)) null else args.optString(i)
    fun int(i: Int): Int = args.optInt(i)
    fun long(i: Int): Long = args.optLong(i)
    fun double(i: Int): Double = args.optDouble(i, 0.0)
    fun bool(i: Int): Boolean = args.optBoolean(i)
    fun obj(i: Int): JSONObject = args.optJSONObject(i) ?: JSONObject()
    fun array(i: Int): JSONArray = args.optJSONArray(i) ?: JSONArray()
    /** A `[String: String]` argument. */
    fun stringMap(i: Int): Map<String, String> {
        val o = obj(i)
        return o.keys().asSequence().associateWith { o.optString(it) }
    }
    /** A `fn(T)` argument: answer it now or later, from any thread. */
    fun callback(i: Int): VardCallback = VardCallback(host, args.optJSONObject(i)?.optLong("\$callback") ?: 0L)
}

/** A Vard function handed to native code. Delivery always hops to the UI thread. */
class VardCallback(private val host: VardHost, val id: Long) {
    /** Deliver the result; the callback is consumed. */
    fun resolve(value: Any? = null) = host.deliver(id, value, keep = false)
    /** Deliver one of many values (listeners, streams); call [release] when done. */
    fun send(value: Any?) = host.deliver(id, value, keep = true)
    fun release() = host.post { if (host.handle != 0L) VardNative.release(host.handle, id) }
}

/**
 * Dispatches `extern fn` calls from the Rust core to registered [VardModule]s and carries
 * results, callbacks and platform events back. One per screen; owned by [VardView].
 */
class VardHost(val context: Context) {
    var activity: Activity? = null
    /** Runtime handle; 0 until mounted. Set by [VardView]. */
    var handle = 0L
        internal set
    /** Called after anything changed the UI outside a touch (callbacks, events). */
    var onChanged: (() -> Unit)? = null
    /** Typefaces for measuring and drawing; set by [VardView]. */
    var fonts = VardFonts(context, emptyList())

    val modules = ArrayList<VardModule>()
    private val table = HashMap<String, (VardCall) -> Any?>()
    private val main = Handler(Looper.getMainLooper())
    private val queued = ArrayList<() -> Unit>()

    fun register(module: VardModule): VardHost {
        modules += module
        table.putAll(module.functions)
        module.install(this)
        return this
    }

    fun implements(name: String) = lookup(name) != null

    private fun lookup(name: String): ((VardCall) -> Any?)? =
        table[name] ?: modules.firstNotNullOfOrNull { it.resolve(name) }?.also { table[name] = it }

    /** Called from Rust through JNI. Returns JSON, or null = not implemented. */
    @Suppress("unused")
    fun call(name: String, argsJson: String): String? {
        val f = lookup(name) ?: return null
        return try {
            toJson(f(VardCall(this, name, JSONArray(argsJson))))
        } catch (e: Throwable) {
            Log.e(TAG, "native function $name failed", e)
            "null"
        }
    }

    /** Always queued, never run inline: [call] may be on the stack inside the runtime. */
    /** Called from Rust through JNI: the core measures platform-font text with our shaper. */
    @Suppress("unused")
    fun measure(text: String, family: String, weight: Int, italic: Boolean, px: Float): Float =
        fonts.measure(text, family, weight, italic, px)

    fun post(block: () -> Unit) {
        main.post(block)
    }

    internal fun deliver(id: Long, value: Any?, keep: Boolean) {
        val json = toJson(value)
        post {
            if (handle != 0L && VardNative.callback(handle, id, json, keep) == 1) onChanged?.invoke()
        }
    }

    /** Deliver `extern event name(args...)`. Events before mount are queued. */
    fun emit(name: String, vararg args: Any?) {
        val json = toJson(args.toList())
        post {
            if (handle == 0L) {
                queued += { emitNow(name, json) }
            } else {
                emitNow(name, json)
            }
        }
    }

    private fun emitNow(name: String, json: String) {
        val r = VardNative.emit(handle, name, json)
        if (r == 1) onChanged?.invoke()
        if (r == 0 && name != "appResume" && name != "appPause") Log.i(TAG, "event $name: no handler is listening")
    }

    internal fun mounted(h: Long) {
        handle = h
        val pending = ArrayList(queued)
        queued.clear()
        pending.forEach { it() }
    }

    fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, results: IntArray) {
        modules.any { it.onRequestPermissionsResult(requestCode, permissions, results) }
    }

    fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        modules.any { it.onActivityResult(requestCode, resultCode, data) }
    }

    companion object {
        const val TAG = "Vard"

        /** Kotlin value -> JSON text understood by the runtime. */
        fun toJson(v: Any?): String = when (v) {
            null, Unit -> "null"
            is String -> JSONObject.quote(v)
            is Boolean, is Int, is Long -> v.toString()
            is Double -> if (v.isFinite()) v.toString() else "null"
            is Float -> if (v.isFinite()) v.toString() else "null"
            is Number -> v.toString()
            is Enum<*> -> JSONObject.quote(v.name)
            else -> (wrap(v) ?: JSONObject.NULL).toString()
        }

        fun wrap(v: Any?): Any? = when (v) {
            null, Unit -> JSONObject.NULL
            is VardJson -> wrap(v.toJson())
            is JSONObject, is JSONArray, is String, is Boolean, is Number -> v
            is Enum<*> -> v.name
            is Map<*, *> -> JSONObject().also { o -> v.forEach { (k, x) -> o.put(k.toString(), wrap(x)) } }
            is Iterable<*> -> JSONArray().also { a -> v.forEach { a.put(wrap(it)) } }
            is Array<*> -> JSONArray().also { a -> v.forEach { a.put(wrap(it)) } }
            else -> v.toString()
        }
    }
}

/** A typed `fn(T)` callback (used by generated bindings). */
class VardResult<T>(val raw: VardCallback) {
    fun resolve(value: T) = raw.resolve(value)
    fun send(value: T) = raw.send(value)
    fun release() = raw.release()
}

/** A typed `fn(A, B)` callback: both values arrive as one call. */
class VardResult2<A, B>(val raw: VardCallback) {
    fun resolve(a: A, b: B) = raw.resolve(listOf(a, b))
    fun send(a: A, b: B) = raw.send(listOf(a, b))
    fun release() = raw.release()
}

/** A `fn()` callback. */
class VardSignal(val raw: VardCallback) {
    fun resolve() = raw.resolve(null)
    fun send() = raw.send(null)
    fun release() = raw.release()
}

/** JSON -> Kotlin helpers for generated bindings (lenient, like the runtime's decoder). */
object VardDecode {
    fun str(v: Any?): String = if (v == null || v == JSONObject.NULL) "" else v.toString()
    fun long(v: Any?): Long = when (v) {
        is Number -> v.toLong()
        is String -> v.toLongOrNull() ?: 0L
        else -> 0L
    }
    fun double(v: Any?): Double = when (v) {
        is Number -> v.toDouble()
        is String -> v.toDoubleOrNull() ?: 0.0
        else -> 0.0
    }
    fun bool(v: Any?): Boolean = v == true || v == "true"
    fun obj(v: Any?): JSONObject = v as? JSONObject ?: JSONObject()
    fun isNull(v: Any?) = v == null || v == JSONObject.NULL
    fun <T> opt(v: Any?, f: (Any?) -> T): T? = if (isNull(v)) null else f(v)
    fun <T> list(v: Any?, f: (Any?) -> T): List<T> {
        val a = v as? JSONArray ?: return emptyList()
        return (0 until a.length()).map { f(a.opt(it)) }
    }
    fun <T> map(v: Any?, f: (Any?) -> T): Map<String, T> {
        val o = v as? JSONObject ?: return emptyMap()
        return o.keys().asSequence().associateWith { f(o.opt(it)) }
    }
}
