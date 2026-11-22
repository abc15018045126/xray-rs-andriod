#![deny(unsafe_op_in_unsafe_fn)]

use jni::JNIEnv;
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jint, jlong, jstring};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use xray_core::common::errors::{Error, Result as XResult};
use xray_core::common::net::{BoxStream, Destination, Network};
use xray_core::common::protocol::SessionContext;
use xray_core::core::Instance;
use xray_core::features::outbound::OutboundHandler;
use xray_core::infra::conf::Config;

struct RunningCore {
    instance: Instance,
    tasks: Vec<JoinHandle<()>>,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static RUNNING: Mutex<Option<RunningCore>> = Mutex::new(None);
static ENV_LOCK: Mutex<()> = Mutex::new(());
static LOG_INIT: OnceLock<()> = OnceLock::new();
static INSECURE_TLS: OnceLock<tokio_rustls::TlsConnector> = OnceLock::new();
#[cfg(debug_assertions)]
static ASSET_DIR: Mutex<String> = Mutex::new(String::new());

fn set_env(key: &str, val: Option<&str>) {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match val {
        Some(v) if !v.is_empty() => unsafe { std::env::set_var(key, v) },
        _ => unsafe { std::env::remove_var(key) },
    }
}

fn init_logging() {
    LOG_INIT.get_or_init(|| {
        let filter = if cfg!(debug_assertions) {
            tracing_subscriber::filter::LevelFilter::INFO
        } else {
            tracing_subscriber::filter::LevelFilter::WARN
        };
        #[cfg(target_os = "android")]
        {
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;
            if let Ok(android_layer) = tracing_android::layer("xray-rs") {
                let _ = tracing_subscriber::registry()
                    .with(filter)
                    .with(android_layer)
                    .try_init();
            }
            register_android_tun();
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = tracing_subscriber::fmt()
                .with_ansi(false)
                .with_max_level(filter)
                .try_init();
        }
        #[cfg(debug_assertions)]
        drop(runtime().spawn(async {
            drop(xray_core::debug_panel::server::spawn_server("127.0.0.1:19876".to_string()));
        }));
    });
}

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("xray-rs")
            .build()
            .expect("tokio runtime")
    })
}

fn get_string(env: &mut JNIEnv<'_>, s: JString<'_>) -> String {
    env.get_string(&s).map(Into::into).unwrap_or_default()
}

fn make_jstring(env: &JNIEnv<'_>, s: &str) -> jstring {
    env.new_string(s)
        .map(|js| js.into_raw())
        .unwrap_or_else(|_| JString::default().into_raw())
}

#[cfg(debug_assertions)]
fn log_startup_debug(json_str: &str, tun_fd: i32) {
    if let Ok(dir) = ASSET_DIR.lock()
        && !dir.is_empty()
    {
        let _ = std::fs::write(format!("{}/last_config.json", dir.trim_end_matches('/')), json_str);
    }
    tracing::info!("startLoop tun_fd={}", tun_fd);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeInitCoreEnv(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    asset_path: JString<'_>,
    device_id: JString<'_>,
) {
    init_logging();
    let asset = get_string(&mut env, asset_path);
    let dev = get_string(&mut env, device_id);
    if !asset.is_empty() {
        #[cfg(debug_assertions)]
        if let Ok(mut g) = ASSET_DIR.lock() {
            *g = asset.clone();
        }
        set_env("xray.location.asset", Some(&asset));
        set_env("xray.location.cert", Some(&asset));
        set_env("XRAY_LOCATION_ASSET", Some(&asset));
    }
    if !dev.is_empty() {
        set_env("xray.xudp.basekey", Some(&dev));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeCheckVersionX(env: JNIEnv<'_>, _class: JClass<'_>) -> jstring {
    let ver = if cfg!(debug_assertions) {
        "xray-rs 3.5.0-debug (Rust + 19876)"
    } else {
        "xray-rs 3.5.0 (Rust)"
    };
    make_jstring(&env, ver)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeRegisterProtector(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    protector_obj: JObject<'_>,
) {
    init_logging();
    if protector_obj.is_null() {
        xray_core::proxy::tun::socket_helpers::register_android_protect_fn(None);
        return;
    }
    let (Ok(vm), Ok(global_ref)) = (env.get_java_vm(), env.new_global_ref(protector_obj)) else {
        return;
    };
    let global_ref: GlobalRef = global_ref;
    xray_core::proxy::tun::socket_helpers::register_android_protect_fn(Some(Arc::new(move |fd: i32| {
        let Ok(mut jenv) = vm.attach_current_thread() else {
            return false;
        };
        match jenv.call_method(global_ref.as_obj(), "protect", "(I)Z", &[JValue::Int(fd)]) {
            Ok(val) => val.z().unwrap_or(false),
            Err(_) => {
                let _ = jenv.exception_clear();
                false
            }
        }
    })));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeStartLoop(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    config_json: JString<'_>,
    tun_fd: jint,
) -> jstring {
    init_logging();
    let json_str = get_string(&mut env, config_json);
    stop_internal();
    #[cfg(debug_assertions)]
    log_startup_debug(&json_str, tun_fd);

    let tun_s = tun_fd.to_string();
    set_env("xray.tun.fd", (tun_fd > 0).then_some(tun_s.as_str()));

    let cfg: Config = match serde_json::from_str(&json_str) {
        Ok(c) => c,
        Err(e) => return make_jstring(&env, &format!("config error: {}", e)),
    };

    let res = runtime().block_on(async move {
        let instance = Instance::from_config(cfg)?;
        let tasks = instance.start().await?;
        Ok::<RunningCore, Error>(RunningCore { instance, tasks })
    });

    match res {
        Ok(core) => {
            if let Ok(mut guard) = RUNNING.lock() {
                *guard = Some(core);
            }
            make_jstring(&env, "")
        }
        Err(e) => make_jstring(&env, &format!("startup failed: {}", e)),
    }
}

fn stop_internal() -> bool {
    if let Ok(mut guard) = RUNNING.lock()
        && let Some(core) = guard.take()
    {
        let _ = core.instance.close();
        for t in core.tasks {
            t.abort();
        }
        return true;
    }
    false
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeStopLoop(_env: JNIEnv<'_>, _class: JClass<'_>) -> jboolean {
    if stop_internal() { JNI_TRUE } else { JNI_FALSE }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeIsRunning(_env: JNIEnv<'_>, _class: JClass<'_>) -> jboolean {
    let running = RUNNING
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|c| c.instance.is_running()))
        .unwrap_or(false);
    if running { JNI_TRUE } else { JNI_FALSE }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeReconcileBrowserDialer(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    dialer_addr: JString<'_>,
) {
    let addr = get_string(&mut env, dialer_addr);
    set_env("xray.browser.dialer", Some(&addr));
}

async fn read_http_response(stream: &mut BoxStream) -> XResult<(u16, bool)> {
    let mut buf = Vec::with_capacity(512);
    let mut tmp = [0u8; 512];
    let header_end = loop {
        let n = stream.read(&mut tmp).await.map_err(Error::Io)?;
        if n == 0 {
            return Err(Error::Protocol("eof".into()));
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 16384 {
            return Err(Error::Protocol("header too large".into()));
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = head.split("\r\n");
    let status_code: u16 = lines
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::Protocol("invalid status".into()))?;

    let (mut content_length, mut is_chunked, mut close_conn) = (0usize, false, false);
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let v_low = v.trim().to_ascii_lowercase();
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v_low.parse().unwrap_or(0);
            } else if k.trim().eq_ignore_ascii_case("transfer-encoding") && v_low.contains("chunked") {
                is_chunked = true;
            } else if k.trim().eq_ignore_ascii_case("connection") && v_low.contains("close") {
                close_conn = true;
            }
        }
    }

    if status_code != 204 {
        let already = buf.len() - header_end;
        if content_length > already {
            let mut rem = content_length - already;
            while rem > 0 {
                let to_read = rem.min(tmp.len());
                let n = stream.read(&mut tmp[..to_read]).await.map_err(Error::Io)?;
                if n == 0 {
                    close_conn = true;
                    break;
                }
                rem -= n;
            }
        } else if is_chunked {
            let mut body = buf[header_end..].to_vec();
            while !body.windows(5).any(|w| w == b"0\r\n\r\n") && body.len() < 65536 {
                let n = stream.read(&mut tmp).await.map_err(Error::Io)?;
                if n == 0 {
                    close_conn = true;
                    break;
                }
                body.extend_from_slice(&tmp[..n]);
            }
        }
    }
    Ok((status_code, !close_conn))
}

async fn measure_http_delay(outbound: Arc<dyn OutboundHandler>, url: &str) -> jlong {
    let (is_https, host, port, path) = parse_test_url(url);
    let Ok(dest) = Destination::parse_str(&format!("{}:{}", host, port), Network::Tcp) else {
        return -1;
    };
    let tls_client = if is_https {
        match xray_core::transport::internet::TlsClient::new(&host, false, vec![b"http/1.1".to_vec()]) {
            Ok(c) => Some(c),
            Err(_) => return -1,
        }
    } else {
        None
    };

    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: keep-alive\r\nUser-Agent: Go-http-client/1.1\r\n\r\n",
        path, host
    );

    let overall = tokio::time::timeout(Duration::from_secs(12), async move {
        let mut conn: Option<BoxStream> = None;
        let mut min_duration: jlong = -1;
        let mut success = false;

        for _ in 0..2 {
            let start = Instant::now();
            if conn.is_none() {
                let session = SessionContext::new("delay-test", dest.clone());
                let dialed = tokio::time::timeout(Duration::from_secs(6), async {
                    let s = outbound.connect(&session).await?;
                    if let Some(ref tls) = tls_client {
                        tls.connect(&host, s).await
                    } else {
                        Ok(s)
                    }
                })
                .await;
                match dialed {
                    Ok(Ok(s)) => conn = Some(s),
                    _ => continue,
                }
            }

            let Some(ref mut stream) = conn else { continue };
            if stream.write_all(req.as_bytes()).await.is_err() || stream.flush().await.is_err() {
                conn = None;
                continue;
            }

            match read_http_response(stream).await {
                Ok((status, keep_alive)) => {
                    if !keep_alive {
                        conn = None;
                    }
                    if status != 200 && status != 204 {
                        continue;
                    }
                    let ms = start.elapsed().as_millis().max(1) as jlong;
                    if !success || ms < min_duration {
                        min_duration = ms;
                    }
                    success = true;
                }
                Err(_) => conn = None,
            }
        }
        if success { min_duration } else { -1 }
    })
    .await;

    overall.unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeMeasureDelay(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    test_url: JString<'_>,
) -> jlong {
    init_logging();
    let url = get_string(&mut env, test_url);
    let Some(outbound) = RUNNING
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|c| c.instance.dispatcher.get_handler("proxy")))
    else {
        return -1;
    };
    runtime().block_on(measure_http_delay(outbound, &url))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeMeasureOutboundDelay(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    config_json: JString<'_>,
    test_url: JString<'_>,
) -> jlong {
    init_logging();
    let json_str = get_string(&mut env, config_json);
    let url = get_string(&mut env, test_url);

    let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&json_str) else {
        return -1;
    };
    if let Some(obj) = val.as_object_mut() {
        obj.insert("inbounds".to_string(), serde_json::Value::Array(Vec::new()));
        if let Some(outbounds) = obj.get_mut("outbounds").and_then(|v| v.as_array_mut()) {
            for ob in outbounds {
                if let Some(vnext) = ob.pointer_mut("/settings/vnext").and_then(|v| v.as_array_mut()) {
                    for srv in vnext {
                        if let Some(users) = srv.get_mut("users").and_then(|u| u.as_array_mut()) {
                            for user in users {
                                if let Some(uobj) = user.as_object_mut() {
                                    uobj.insert("testpre".to_string(), serde_json::Value::from(0));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let Ok(cfg) = serde_json::from_value::<Config>(val) else {
        return -1;
    };

    runtime().block_on(async move {
        let Ok(built) = cfg.build() else { return -1 };
        let Some(outbound) = built
            .outbounds
            .get("proxy")
            .or_else(|| built.outbounds.values().next())
            .cloned()
        else {
            return -1;
        };
        measure_http_delay(outbound, &url).await
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeQueryStats(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    tag: JString<'_>,
    direct: JString<'_>,
) -> jlong {
    let name = format!(
        "outbound>>>{}>>>traffic>>>{}",
        get_string(&mut env, tag),
        get_string(&mut env, direct)
    );
    RUNNING
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|c| c.instance.stats_manager.get_counter(&name)))
        .map(|c| c.set(0) as jlong)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeQueryAllOutboundTrafficStats(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jstring {
    let mut out = String::new();
    if let Ok(guard) = RUNNING.lock()
        && let Some(core) = guard.as_ref()
    {
        for (name, counter) in core.instance.stats_manager.all_counters() {
            let mut it = name.split(">>>");
            let (Some("outbound"), Some(tag), Some("traffic"), Some(dir), None) =
                (it.next(), it.next(), it.next(), it.next(), it.next())
            else {
                continue;
            };
            let value = counter.set(0);
            if value > 0 {
                let _ = write!(out, "{},{},{};", tag, dir, value);
            }
        }
    }
    make_jstring(&env, &out)
}

#[derive(Debug)]
struct NoVerifyCert;

impl rustls::client::danger::ServerCertVerifier for NoVerifyCert {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn insecure_tls_connector() -> &'static tokio_rustls::TlsConnector {
    INSECURE_TLS.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cfg = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifyCert))
            .with_no_client_auth();
        tokio_rustls::TlsConnector::from(Arc::new(cfg))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeFetchTlsCertSha256(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    request_json: JString<'_>,
) -> jstring {
    let req_str = get_string(&mut env, request_json);
    let val: serde_json::Value = match serde_json::from_str(&req_str) {
        Ok(v) => v,
        Err(e) => return make_jstring(&env, &serde_json::json!({"sha256":"","error":e.to_string()}).to_string()),
    };

    let address = val.get("address").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if address.is_empty() {
        return make_jstring(&env, r#"{"sha256":"","error":"address is empty"}"#);
    }
    let port = match val.get("port").and_then(|v| v.as_i64()).unwrap_or(443) {
        p if p > 0 => p as u16,
        _ => 443,
    };
    let server_name = val
        .get("serverName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&address)
        .to_string();
    let timeout_ms = val.get("timeoutMs").and_then(|v| v.as_u64()).unwrap_or(8000).max(1000);
    let target = if address.contains(':') && !address.starts_with('[') {
        format!("[{}]:{}", address, port)
    } else {
        format!("{}:{}", address, port)
    };

    let res: Result<String, String> = runtime().block_on(async move {
        let sni = rustls::pki_types::ServerName::try_from(server_name)
            .map_err(|e| format!("invalid serverName: {}", e))?;
        tokio::time::timeout(Duration::from_millis(timeout_ms), async move {
            let tcp = tokio::net::TcpStream::connect(&target).await.map_err(|e| e.to_string())?;
            let tls = insecure_tls_connector().connect(sni, tcp).await.map_err(|e| e.to_string())?;
            let leaf = tls
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|c| c.first())
                .ok_or_else(|| "peer certificate is empty".to_string())?;
            Ok::<String, String>(hex::encode(Sha256::digest(leaf.as_ref())))
        })
        .await
        .map_err(|_| "timeout".to_string())?
    });

    let resp = match res {
        Ok(sha) => serde_json::json!({"sha256": sha, "error": ""}).to_string(),
        Err(err) => serde_json::json!({"sha256": "", "error": err}).to_string(),
    };
    make_jstring(&env, &resp)
}

fn parse_test_url(url: &str) -> (bool, String, u16, String) {
    let t = match url.trim() {
        "" => "https://www.google.com/generate_204",
        s => s,
    };
    let (is_https, def_port, rest) = if let Some(r) = t.strip_prefix("https://") {
        (true, 443u16, r)
    } else if let Some(r) = t.strip_prefix("http://") {
        (false, 80u16, r)
    } else {
        (false, 80u16, t)
    };
    let (hp, path) = rest.find('/').map(|i| (&rest[..i], &rest[i..])).unwrap_or((rest, "/generate_204"));
    let (host, port) = match hp.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse::<u16>().unwrap()),
        _ => (hp, def_port),
    };
    let host = if host.is_empty() { "www.gstatic.com" } else { host };
    (is_https, host.to_string(), port, path.to_string())
}

#[cfg(target_os = "android")]
fn register_android_tun() {
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, BorrowedFd};
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::unix::AsyncFd;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    struct RawFdTunStream {
        inner: AsyncFd<File>,
    }

    impl RawFdTunStream {
        fn open(fd: i32) -> XResult<BoxStream> {
            let owned = unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned().map_err(Error::Io)?;
            let raw = owned.as_raw_fd();
            let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
                return Err(Error::Io(io::Error::last_os_error()));
            }
            let inner = AsyncFd::new(File::from(owned)).map_err(Error::Io)?;
            Ok(Box::pin(Self { inner }))
        }
    }

    impl AsyncRead for RawFdTunStream {
        fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
            loop {
                let mut guard = match self.inner.poll_read_ready(cx) {
                    Poll::Ready(Ok(g)) => g,
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                };
                let dst = buf.initialize_unfilled();
                match guard.try_io(|inner| inner.get_ref().read(dst)) {
                    Ok(Ok(n)) => { buf.advance(n); return Poll::Ready(Ok(())); }
                    Ok(Err(e)) => return Poll::Ready(Err(e)),
                    Err(_) => continue,
                }
            }
        }
    }

    impl AsyncWrite for RawFdTunStream {
        fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
            loop {
                let mut guard = match self.inner.poll_write_ready(cx) {
                    Poll::Ready(Ok(g)) => g,
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                };
                match guard.try_io(|inner| inner.get_ref().write(buf)) {
                    Ok(res) => return Poll::Ready(res),
                    Err(_) => continue,
                }
            }
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> { Poll::Ready(Ok(())) }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> { Poll::Ready(Ok(())) }
    }

    xray_core::proxy::tun::tun_android::register_android_tun_opener(Some(Arc::new(RawFdTunStream::open)));
}
