package libv2ray

import java.net.HttpURLConnection
import java.net.InetSocketAddress
import java.net.Proxy
import java.net.URL

/**
 * Drop-in Kotlin replacement for gomobile `libv2ray.CoreController`, backed by `libxray_android.so` (xray-rs).
 */
class CoreController(private val handler: CoreCallbackHandler) {
    @Volatile
    private var lastConfigJson: String = ""

    @Volatile
    private var processFinder: ProcessFinder? = null

    val isRunning: Boolean
        get() = Libv2ray.nativeIsRunning()

    fun registerProcessFinder(finder: ProcessFinder?) {
        processFinder = finder
    }

    @Throws(Exception::class)
    fun startLoop(configContent: String, tunFd: Int) {
        lastConfigJson = configContent
        val err = Libv2ray.nativeStartLoop(configContent, tunFd)
        if (err.isNotEmpty()) {
            throw RuntimeException(err)
        }
        handler.startup()
        handler.onEmitStatus(0L, "xray-rs started")
    }

    fun stopLoop() {
        Libv2ray.nativeStopLoop()
        handler.shutdown()
        handler.onEmitStatus(0L, "xray-rs stopped")
    }

    fun queryStats(@Suppress("UNUSED_PARAMETER") tag: String, @Suppress("UNUSED_PARAMETER") direct: String): Long {
        return 0L
    }

    fun queryAllOutboundTrafficStats(): String {
        return Libv2ray.nativeQueryAllOutboundTrafficStats()
    }

    fun measureDelay(testUrl: String): Long {
        val cfg = lastConfigJson
        if (cfg.isNotEmpty()) {
            val ms = Libv2ray.nativeMeasureOutboundDelay(cfg, testUrl)
            if (ms >= 0L) return ms
        }
        return try {
            val start = System.currentTimeMillis()
            val conn = URL(testUrl).openConnection(
                Proxy(Proxy.Type.SOCKS, InetSocketAddress("127.0.0.1", 10808))
            ) as HttpURLConnection
            conn.connectTimeout = 5000
            conn.readTimeout = 5000
            conn.connect()
            val code = conn.responseCode
            conn.disconnect()
            if (code in 200..399) (System.currentTimeMillis() - start).coerceAtLeast(1L) else -1L
        } catch (_: Exception) {
            -1L
        }
    }
}
