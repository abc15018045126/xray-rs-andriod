package libv2ray

object Libv2ray {
    init {
        System.loadLibrary("xray_android")
    }

    interface SocketProtector {
        fun protect(fd: Int): Boolean
    }

    @JvmStatic external fun nativeInitCoreEnv(assetPath: String, deviceId: String)
    @JvmStatic external fun nativeCheckVersionX(): String
    @JvmStatic external fun nativeRegisterProtector(protector: SocketProtector?)
    @JvmStatic external fun nativeStartLoop(configJson: String, tunFd: Int): String
    @JvmStatic external fun nativeStopLoop(): Boolean
    @JvmStatic external fun nativeIsRunning(): Boolean
    @JvmStatic external fun nativeReconcileBrowserDialer(dialerAddr: String)
    @JvmStatic external fun nativeMeasureDelay(testUrl: String): Long
    @JvmStatic external fun nativeMeasureOutboundDelay(configJson: String, testUrl: String): Long
    @JvmStatic external fun nativeQueryStats(tag: String, direct: String): Long
    @JvmStatic external fun nativeQueryAllOutboundTrafficStats(): String
    @JvmStatic external fun nativeFetchTlsCertSha256(requestJson: String): String

    @JvmStatic fun initCoreEnv(assetPath: String, deviceId: String) = nativeInitCoreEnv(assetPath, deviceId)
    @JvmStatic fun reconcileBrowserDialer(dialerAddr: String) = nativeReconcileBrowserDialer(dialerAddr)
    @JvmStatic fun checkVersionX(): String = nativeCheckVersionX()
    @JvmStatic fun measureOutboundDelay(config: String, testUrl: String): Long = nativeMeasureOutboundDelay(config, testUrl)
    @JvmStatic fun newCoreController(handler: CoreCallbackHandler): CoreController = CoreController(handler)
    @JvmStatic fun fetchTlsCertSha256(requestJson: String): String = nativeFetchTlsCertSha256(requestJson)
    @JvmStatic fun fetchQuicCertSha256(@Suppress("UNUSED_PARAMETER") requestJson: String): String =
        """{"sha256":"","error":"QUIC cert SHA256 fetch not supported yet"}"""
}
