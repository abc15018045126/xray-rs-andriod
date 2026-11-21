//! Minimal Android JNI bridge exposing `xray-rs` (`xray_core`) to v2rayNG's `libv2ray` Kotlin layer.
//! Includes Android `logcat` tracing bridge (`tag: xray-rs` / `xray-rs-cfg`) and config dump for ADB debugging.

use jni::JNIEnv;
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jint, jlong, jstring};
use std::ffi::CString;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tracing_subscriber::fmt::MakeWriter;
use xray_core::core::Instance;
use xray_core::infra::conf::Config;

#[cfg(target_os = "android")]
#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(
        prio: i32,
        tag: *const std::ffi::c_char,
        text: *const std::ffi::c_char,
    ) -> i32;
}

fn logcat_write(tag: &str, msg: &str) {
    #[cfg(target_os = "android")]
    {
        const ANDROID_LOG_INFO: i32 = 4;
        let c_tag = CString::new(tag).unwrap_or_else(|_| CString::new("xray-rs").unwrap());
        // Split long messages into <= 3000-byte chunks so Android logcat never truncates
        for line in msg.lines() {
            let bytes = line.as_bytes();
            let mut offset = 0;
            while offset < bytes.len() {
                let mut end = (offset + 3000).min(bytes.len());
                while end > offset && !line.is_char_boundary(end) {
                    end -= 1;
                }
                let slice = &line[offset..end];
                if let Ok(c_msg) = CString::new(slice) {
                    unsafe {
                        __android_log_write(ANDROID_LOG_INFO, c_tag.as_ptr(), c_msg.as_ptr());
                    }
                }
                offset = end;
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (tag, msg);
    }
}

struct LogcatWriter {
    buf: Vec<u8>,
}

impl io::Write for LogcatWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            let s = String::from_utf8_lossy(&self.buf);
            logcat_write("xray-rs", s.trim_end());
            self.buf.clear();
        }
        Ok(())
    }
}

impl Drop for LogcatWriter {
    fn drop(&mut self) {
        let _ = io::Write::flush(self);
    }
}

#[derive(Clone, Copy)]
struct LogcatMakeWriter;

impl<'a> MakeWriter<'a> for LogcatMakeWriter {
    type Writer = LogcatWriter;
    fn make_writer(&'a self) -> Self::Writer {
        LogcatWriter {
            buf: Vec::with_capacity(256),
        }
    }
}

struct RunningCore {
    instance: Instance,
    tasks: Vec<JoinHandle<()>>,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static RUNNING: OnceLock<Mutex<Option<RunningCore>>> = OnceLock::new();
static LAST_ERROR: OnceLock<Mutex<String>> = OnceLock::new();
static ASSET_DIR: OnceLock<Mutex<String>> = OnceLock::new();
static LOG_INIT: OnceLock<()> = OnceLock::new();

fn init_logging() {
    LOG_INIT.get_or_init(|| {
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(LogcatMakeWriter)
            .try_init();
        #[cfg(debug_assertions)]
        {
            let _ = runtime().spawn(async {
                let _ = xray_core::debug_panel::server::spawn_server("127.0.0.1:19876".to_string());
            });
        }
        logcat_write(
            "xray-rs",
            &format!(
                "=== xray-rs Android JNI logger initialized (debug_panel={}) ===",
                cfg!(debug_assertions)
            ),
        );
    });
}

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(4)
            .thread_name("xray-rs-android")
            .build()
            .expect("failed to build Tokio runtime")
    })
}

fn running_slot() -> &'static Mutex<Option<RunningCore>> {
    RUNNING.get_or_init(|| Mutex::new(None))
}

fn set_last_error(msg: impl Into<String>) {
    let lock = LAST_ERROR.get_or_init(|| Mutex::new(String::new()));
    if let Ok(mut g) = lock.lock() {
        *g = msg.into();
    }
}

fn get_string(env: &mut JNIEnv, s: JString) -> String {
    env.get_string(&s)
        .map(|js| js.into())
        .unwrap_or_default()
}

fn make_jstring(env: &JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|js| js.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn log_startup_summary(json_str: &str, tun_fd: i32) {
    // Save full config to app asset dir for easy `adb shell run-as ... cat`
    if let Some(lock) = ASSET_DIR.get()
        && let Ok(dir) = lock.lock()
        && !dir.is_empty()
    {
        let path = format!("{}/last_config.json", dir.trim_end_matches('/'));
        if std::fs::write(&path, json_str).is_ok() {
            logcat_write("xray-rs", &format!("Saved full config to {}", path));
        }
    }

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
        let mut summary = format!("=== STARTUP SUMMARY (tun_fd={}) ===\n", tun_fd);
        if let Some(inbounds) = val.get("inbounds").and_then(|v| v.as_array()) {
            for ib in inbounds {
                let tag = ib.get("tag").and_then(|v| v.as_str()).unwrap_or("-");
                let proto = ib.get("protocol").and_then(|v| v.as_str()).unwrap_or("-");
                let port = ib.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
                summary.push_str(&format!("  [INBOUND]  tag={} proto={} port={}\n", tag, proto, port));
            }
        }
        if let Some(outbounds) = val.get("outbounds").and_then(|v| v.as_array()) {
            for ob in outbounds {
                let tag = ob.get("tag").and_then(|v| v.as_str()).unwrap_or("-");
                let proto = ob.get("protocol").and_then(|v| v.as_str()).unwrap_or("-");
                let net = ob
                    .pointer("/streamSettings/network")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let sec = ob
                    .pointer("/streamSettings/security")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let addr = ob
                    .pointer("/settings/address")
                    .or_else(|| ob.pointer("/settings/vnext/0/address"))
                    .or_else(|| ob.pointer("/settings/servers/0/address"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let port = ob
                    .pointer("/settings/port")
                    .or_else(|| ob.pointer("/settings/vnext/0/port"))
                    .or_else(|| ob.pointer("/settings/servers/0/port"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                summary.push_str(&format!(
                    "  [OUTBOUND] tag={} proto={} target={}:{} net={} sec={}\n",
                    tag, proto, addr, port, net, sec
                ));
            }
        }
        if let Some(dns) = val.get("dns") {
            summary.push_str(&format!("  [DNS]      {}\n", dns));
        }
        logcat_write("xray-rs", &summary);
    }

    // Also dump full raw JSON under dedicated tag `xray-rs-cfg` for `adb logcat -s xray-rs-cfg`
    logcat_write("xray-rs-cfg", "=== BEGIN CONFIG JSON ===");
    logcat_write("xray-rs-cfg", json_str);
    logcat_write("xray-rs-cfg", "=== END CONFIG JSON ===");
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeInitCoreEnv(
    mut env: JNIEnv,
    _class: JClass,
    asset_path: JString,
    device_id: JString,
) {
    init_logging();
    let asset = get_string(&mut env, asset_path);
    let dev = get_string(&mut env, device_id);
    logcat_write(
        "xray-rs",
        &format!("nativeInitCoreEnv asset_path='{}' device_id_len={}", asset, dev.len()),
    );
    if !asset.is_empty() {
        let lock = ASSET_DIR.get_or_init(|| Mutex::new(String::new()));
        if let Ok(mut g) = lock.lock() {
            *g = asset.clone();
        }
        unsafe {
            std::env::set_var("xray.location.asset", &asset);
            std::env::set_var("XRAY_LOCATION_ASSET", &asset);
        };
    }
    if !dev.is_empty() {
        unsafe { std::env::set_var("xray.xudp.basekey", &dev) };
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeCheckVersionX(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let ver = if cfg!(debug_assertions) {
        "xray-rs 3.0.7-debug (Rust + 19876)"
    } else {
        "xray-rs 3.0.7 (Rust)"
    };
    make_jstring(&env, ver)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeRegisterProtector(
    env: JNIEnv,
    _class: JClass,
    protector_obj: JObject,
) {
    init_logging();
    if protector_obj.is_null() {
        xray_core::proxy::tun::socket_helpers::register_android_protect_fn(None);
        return;
    }
    let Ok(vm) = env.get_java_vm() else {
        return;
    };
    let Ok(global_ref): Result<GlobalRef, _> = env.new_global_ref(protector_obj) else {
        return;
    };
    logcat_write("xray-rs", "Registered Android VpnService.protect(fd) callback");
    xray_core::proxy::tun::socket_helpers::register_android_protect_fn(Some(Arc::new(
        move |fd: i32| -> bool {
            let Ok(mut jenv) = vm.attach_current_thread() else {
                return false;
            };
            match jenv.call_method(
                global_ref.as_obj(),
                "protect",
                "(I)Z",
                &[JValue::Int(fd)],
            ) {
                Ok(val) => val.z().unwrap_or(false),
                Err(_) => {
                    let _ = jenv.exception_clear();
                    false
                }
            }
        },
    )));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeStartLoop(
    mut env: JNIEnv,
    _class: JClass,
    config_json: JString,
    tun_fd: jint,
) -> jstring {
    init_logging();
    let json_str = get_string(&mut env, config_json);

    stop_internal();
    log_startup_summary(&json_str, tun_fd);

    if tun_fd > 0 {
        unsafe { std::env::set_var("xray.tun.fd", tun_fd.to_string()) };
    } else {
        unsafe { std::env::remove_var("xray.tun.fd") };
    }

    let cfg: Config = match serde_json::from_str(&json_str) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("JSON config parse error: {}", e);
            logcat_write("xray-rs", &msg);
            set_last_error(&msg);
            return make_jstring(&env, &msg);
        }
    };

    let rt = runtime();
    let res = rt.block_on(async move {
        let instance = Instance::from_config(cfg)?;
        let tasks = instance.start().await?;
        Ok::<RunningCore, xray_core::common::errors::Error>(RunningCore { instance, tasks })
    });

    match res {
        Ok(core) => {
            logcat_write(
                "xray-rs",
                &format!(
                    "xray-rs started successfully (tasks={}, tun_fd={}, debug_panel=http://127.0.0.1:19876)",
                    core.tasks.len(),
                    tun_fd
                ),
            );
            if let Ok(mut guard) = running_slot().lock() {
                *guard = Some(core);
            }
            make_jstring(&env, "")
        }
        Err(e) => {
            let msg = format!("xray-rs start error: {}", e);
            logcat_write("xray-rs", &msg);
            set_last_error(&msg);
            make_jstring(&env, &msg)
        }
    }
}

fn stop_internal() {
    if let Ok(mut guard) = running_slot().lock()
        && let Some(core) = guard.take()
    {
        logcat_write("xray-rs", "Stopping xray-rs instance...");
        let _ = core.instance.close();
        for t in core.tasks {
            t.abort();
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeStopLoop(_env: JNIEnv, _class: JClass) {
    stop_internal();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeIsRunning(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let running = running_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|c| c.instance.is_running()))
        .unwrap_or(false);
    if running { JNI_TRUE } else { JNI_FALSE }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeMeasureOutboundDelay(
    mut env: JNIEnv,
    _class: JClass,
    config_json: JString,
    test_url: JString,
) -> jlong {
    init_logging();
    let json_str = get_string(&mut env, config_json);
    let url = get_string(&mut env, test_url);
    let cfg: Config = match serde_json::from_str(&json_str) {
        Ok(c) => c,
        Err(_) => return -1,
    };

    let rt = runtime();
    rt.block_on(async move {
        let built = match cfg.build() {
            Ok(b) => b,
            Err(_) => return -1,
        };
        let Some(outbound) = built
            .outbounds
            .get("proxy")
            .or_else(|| built.outbounds.values().next())
            .cloned()
        else {
            return -1;
        };

        let host_port = parse_url_host_port(&url);
        let dest = match xray_core::common::net::Destination::parse_str(
            &host_port,
            xray_core::common::net::Network::Tcp,
        ) {
            Ok(d) => d,
            Err(_) => return -1,
        };
        let session = xray_core::common::protocol::SessionContext::new("delay-test", dest);
        let t0 = Instant::now();
        match tokio::time::timeout(Duration::from_secs(6), outbound.connect(&session)).await {
            Ok(Ok(_stream)) => {
                let ms = t0.elapsed().as_millis().max(1) as jlong;
                logcat_write("xray-rs", &format!("measureOutboundDelay({}) => {}ms", host_port, ms));
                ms
            }
            _ => -1,
        }
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_libv2ray_Libv2ray_nativeQueryAllOutboundTrafficStats(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    make_jstring(&env, "[]")
}

fn parse_url_host_port(url: &str) -> String {
    let trimmed = url.trim();
    let (default_port, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        (443u16, r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        (80u16, r)
    } else {
        (80u16, trimmed)
    };
    let host_part = rest.split('/').next().unwrap_or("www.gstatic.com");
    if host_part.contains(':') {
        host_part.to_string()
    } else {
        format!("{}:{}", host_part, default_port)
    }
}
