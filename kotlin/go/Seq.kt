package go

import android.content.Context

/** Compatibility stub replacing gomobile's `go.Seq` for xray-rs Android JNI. */
object Seq {
    @JvmStatic
    fun setContext(@Suppress("UNUSED_PARAMETER") context: Context?) {
        // No-op: xray-rs initializes directly via Libv2ray.initCoreEnv
    }
}
