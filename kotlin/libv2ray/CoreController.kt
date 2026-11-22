package libv2ray

class CoreController(private val handler: CoreCallbackHandler) {
    @Volatile
    private var processFinder: ProcessFinder? = null

    val isRunning: Boolean
        get() = Libv2ray.nativeIsRunning()

    fun registerProcessFinder(finder: ProcessFinder?) {
        processFinder = finder
    }

    @Throws(Exception::class)
    fun startLoop(configContent: String, tunFd: Int) {
        if (isRunning) return
        val err = Libv2ray.nativeStartLoop(configContent, tunFd)
        if (err.isNotEmpty()) throw RuntimeException(err)
        handler.startup()
        handler.onEmitStatus(0L, "Started successfully, running")
    }

    fun stopLoop() {
        if (Libv2ray.nativeStopLoop()) {
            handler.shutdown()
            handler.onEmitStatus(0L, "Core stopped")
        }
    }

    fun queryStats(tag: String, direct: String): Long = Libv2ray.nativeQueryStats(tag, direct)

    fun queryAllOutboundTrafficStats(): String = Libv2ray.nativeQueryAllOutboundTrafficStats()

    fun measureDelay(testUrl: String): Long = Libv2ray.nativeMeasureDelay(testUrl)
}
