package libv2ray

/**
 * Custom DNS mode & default remote DNS helper for xray-rs, isolated in AndroidLibXrayLite-rs
 * so upstream v2rayNG-master requires only minimal glue lines.
 */
object XrayRsDnsHelper {
    const val PREF_DNS_MODE = "pref_xray_rs_dns_mode"
    const val DNS_MODE_REMOTE = "remote"
    const val DNS_MODE_LOCAL = "local"

    const val DEFAULT_REMOTE_DNS =
        "https://dns.google/dns-query,https://cloudflare-dns.com/dns-query"

    val DNS_MODE_ENTRIES: List<String> = listOf(
        "Remote 模式 (经 Xray 路由/代理 DoH)",
        "Local 模式 (+local 物理网卡直连)"
    )

    val DNS_MODE_VALUES: List<String> = listOf(
        DNS_MODE_REMOTE,
        DNS_MODE_LOCAL
    )

    fun applyDnsMode(servers: List<String>, mode: String?): List<String> {
        val m = mode?.lowercase()?.trim() ?: DNS_MODE_REMOTE
        val list = if (servers.isEmpty()) {
            DEFAULT_REMOTE_DNS.split(",")
        } else {
            servers
        }
        return list.map { raw ->
            val s = raw.trim()
            if (m == DNS_MODE_LOCAL) {
                when {
                    s.startsWith("https://") -> "https+local://" + s.removePrefix("https://")
                    s.startsWith("h2c://") -> "h2c+local://" + s.removePrefix("h2c://")
                    s.startsWith("tcp://") -> "tcp+local://" + s.removePrefix("tcp://")
                    s.startsWith("udp://") -> "udp+local://" + s.removePrefix("udp://")
                    else -> s
                }
            } else {
                when {
                    s.startsWith("https+local://") -> "https://" + s.removePrefix("https+local://")
                    s.startsWith("h2c+local://") -> "h2c://" + s.removePrefix("h2c+local://")
                    s.startsWith("tcp+local://") -> "tcp://" + s.removePrefix("tcp+local://")
                    s.startsWith("udp+local://") -> "udp://" + s.removePrefix("udp+local://")
                    else -> s
                }
            }
        }
    }
}
