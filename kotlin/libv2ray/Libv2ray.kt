package libv2ray

/**
 * Drop-in Kotlin replacement for `libv2ray.Libv2ray`, backed by native `libxray_android.so` (xray-rs).
 */
object Libv2ray {
    init {
        System.loadLibrary("xray_android")
    }

    interface SocketProtector {
        fun protect(fd: Int): Boolean
    }

    @JvmStatic
    external fun nativeInitCoreEnv(assetPath: String, deviceId: String)

    @JvmStatic
    external fun nativeCheckVersionX(): String

    @JvmStatic
    external fun nativeRegisterProtector(protector: SocketProtector?)

    @JvmStatic
    external fun nativeStartLoop(configJson: String, tunFd: Int): String

    @JvmStatic
    external fun nativeStopLoop()

    @JvmStatic
    external fun nativeIsRunning(): Boolean

    @JvmStatic
    external fun nativeMeasureOutboundDelay(configJson: String, testUrl: String): Long

    @JvmStatic
    external fun nativeQueryAllOutboundTrafficStats(): String

    @JvmStatic
    fun initCoreEnv(assetPath: String, deviceId: String) {
        nativeInitCoreEnv(assetPath, deviceId)
    }

    @JvmStatic
    fun reconcileBrowserDialer(@Suppress("UNUSED_PARAMETER") dialerAddr: String) {
        // Optional browser dialer hook
    }

    @JvmStatic
    fun checkVersionX(): String = nativeCheckVersionX()

    @JvmStatic
    fun measureOutboundDelay(config: String, testUrl: String): Long =
        nativeMeasureOutboundDelay(config, testUrl)

    @JvmStatic
    fun newCoreController(handler: CoreCallbackHandler): CoreController =
        CoreController(handler)

    @JvmStatic
    fun fetchTlsCertSha256(@Suppress("UNUSED_PARAMETER") requestJson: String): String =
        """{"sha256":"","error":"not implemented in xray-rs android bridge"}"""

    @JvmStatic
    fun fetchQuicCertSha256(@Suppress("UNUSED_PARAMETER") requestJson: String): String =
        """{"sha256":"","error":"not implemented in xray-rs android bridge"}"""
}
