package libv2ray

object XrayRsDnsHelper {
    const val PREF_DNS_MODE = "pref_xray_rs_dns_mode"
    const val DNS_MODE_REMOTE = "remote"
    const val DNS_MODE_LOCAL = "local"
    const val DEFAULT_REMOTE_DNS = "https://dns.google/dns-query,https://cloudflare-dns.com/dns-query"
    val DNS_MODE_ENTRIES: List<String> = listOf("Remote 模式 (经 Xray 路由/代理 DoH)", "Local 模式 (+local 物理网卡直连)")
    val DNS_MODE_VALUES: List<String> = listOf(DNS_MODE_REMOTE, DNS_MODE_LOCAL)

    fun applyDnsMode(servers: List<String>, mode: String?): List<String> {
        val local = (mode?.lowercase()?.trim() ?: DNS_MODE_REMOTE) == DNS_MODE_LOCAL
        val list = servers.ifEmpty { DEFAULT_REMOTE_DNS.split(",") }
        return list.map { raw ->
            val s = raw.trim()
            if (local) when {
                s.startsWith("https://") -> "https+local://" + s.removePrefix("https://")
                s.startsWith("h2c://") -> "h2c+local://" + s.removePrefix("h2c://")
                s.startsWith("tcp://") -> "tcp+local://" + s.removePrefix("tcp://")
                s.startsWith("udp://") -> "udp+local://" + s.removePrefix("udp://")
                else -> s
            } else when {
                s.startsWith("https+local://") -> "https://" + s.removePrefix("https+local://")
                s.startsWith("h2c+local://") -> "h2c://" + s.removePrefix("h2c+local://")
                s.startsWith("tcp+local://") -> "tcp://" + s.removePrefix("tcp+local://")
                s.startsWith("udp+local://") -> "udp://" + s.removePrefix("udp+local://")
                else -> s
            }
        }
    }
}
