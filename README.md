# AndroidLibXrayLite-rs

Pure-Rust Android library bridging [`xray-rs`](https://github.com/abc15018045126/xray-rs) into [`v2rayNG`](https://github.com/2dust/v2rayNG) (drop-in replacement for Go `AndroidLibXrayLite` / `libv2ray.aar`), with **zero C dependencies** (no `hev-socks5-tunnel`).

## Directory Structure

```text
AndroidLibXrayLite-rs/
├── assets/                   # geoip.dat & geosite.dat
├── kotlin/
│   ├── go/Seq.kt             # Minimal drop-in stub for go.Seq
│   └── libv2ray/             # Drop-in libv2ray Kotlin API + XrayRsDnsHelper
├── rust-xray-jni/            # Rust cdylib (`libxray_android.so`) JNI bridge
└── build.ps1                 # One-click NDK build + sync + Gradle APK + ADB install
```

## Highlights (v0.0.1)

- **100% Pure-Rust Android TUN**: Native `AndroidTunDevice` (`VpnService` `tun_fd` + `AsyncFd`) + vendored pure-Rust `ipstack` (no `hev-socks5-tunnel`).
- **Socket Protection**: Full `VpnService.protect(fd)` callback wired across TCP, UDP, direct dialers, and DNS sockets.
- **Aligned DNS & Routing**: Persistent HTTP/2 multiplexed DoH (`h2`), `InitSystemDialer` built-in DNS lookup, `ext:geoip`/`geosite` rule parsing, and `xray.location.asset` support.
- **Pingora Pool**: Zero-RTT pre-warmed VLESS connection reuse on Android (`0~2 ms` connect latency).
- **Minimal Frontend Diff**: Keeps upstream `v2rayNG` Jetpack Compose UI nearly untouched; includes `XrayRsDnsHelper` for Remote / Local (`+local`) DNS mode switching and dual default Remote DNS (`dns.google` + `cloudflare-dns.com`).
- **Debug Telemetry**: Debug builds automatically expose the `xray-rs` live debug panel on `127.0.0.1:19876` (`adb forward tcp:19877 tcp:19876`).

## Build & Install

Requires Rust `aarch64-linux-android` target, `cargo-ndk`, and Android NDK (`ANDROID_NDK_HOME`):

```powershell
# Debug build + install via ADB
.\build.ps1 -Install

# Release build + install via ADB
.\build.ps1 -Release -Install
```
