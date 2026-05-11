use std::ffi::CStr;
use std::os::raw::c_char;
use crossbeam_channel::{bounded, Sender};
use lazy_static::lazy_static;
use tracing::{error, info, debug, trace, warn};
use std::thread;
use std::sync::Mutex;
use quick_xml::Reader;
use quick_xml::events::Event;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

// Global lock-free queue policy: 0 = Discard, 1 = Block
static QUEUE_POLICY: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, PartialEq)]
enum RollingPolicy {
    Never,
    Hourly,
    Daily,
}

#[derive(Clone, PartialEq)]
enum LayoutType {
    Pattern,
    Json,
}

// Configuration state
struct RlogConfig {
    file_name: Option<String>,
    rolling_policy: RollingPolicy,
    level: tracing::Level,
    layout_type: LayoutType,
    max_size: Option<u64>,
    max_files: usize,
}

lazy_static! {
    static ref CONFIG: Mutex<RlogConfig> = Mutex::new(RlogConfig {
        file_name: None,
        rolling_policy: RollingPolicy::Never,
        level: tracing::Level::TRACE,
        layout_type: LayoutType::Pattern,
        max_size: None,
        max_files: 7,
    });
}

// Define a log event struct
struct LogEvent {
    level: i32,
    message: String,
    context: Option<String>,
}

lazy_static! {
    static ref LOG_SENDER: Sender<LogEvent> = {
        let (sender, receiver) = bounded::<LogEvent>(1_000_000);
        
        // Read configuration and extract values to move into the thread
        let (file_name, level, rolling_policy, layout_type, max_size, max_files) = {
            let config = CONFIG.lock().unwrap();
            (config.file_name.clone(), config.level, config.rolling_policy.clone(), config.layout_type.clone(), config.max_size, config.max_files)
        };
        
        thread::spawn(move || {
            // Helper to get non_blocking writer
            let (writer, guard) = if let Some(ref f_name) = file_name {
                // Ensure parent directory exists
                let path = Path::new(f_name);
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                }
                
                if let Some(limit) = max_size {
                    let condition = rolling_file::RollingConditionBasic::new().max_size(limit);
                    let appender = rolling_file::RollingFileAppender::new(f_name, condition, max_files).unwrap();
                    let (non_blocking, guard) = tracing_appender::non_blocking(appender);
                    (Some(non_blocking), Some(guard))
                } else {
                    let path = Path::new(f_name);
                    let dir = path.parent().unwrap_or(Path::new("."));
                    let file_prefix = path.file_name().unwrap_or_default();
                    
                    let (non_blocking, guard) = match rolling_policy {
                        RollingPolicy::Hourly => {
                            let appender = tracing_appender::rolling::hourly(dir, file_prefix);
                            tracing_appender::non_blocking(appender)
                        },
                        RollingPolicy::Daily => {
                            let appender = tracing_appender::rolling::daily(dir, file_prefix);
                            tracing_appender::non_blocking(appender)
                        },
                        RollingPolicy::Never => {
                            let appender = tracing_appender::rolling::never(dir, file_prefix);
                            tracing_appender::non_blocking(appender)
                        }
                    };
                    (Some(non_blocking), Some(guard))
                }
            } else {
                (None, None)
            };
            
            if layout_type == LayoutType::Json {
                let subscriber_builder = tracing_subscriber::fmt().json().with_max_level(level);
                if let Some(w) = writer {
                    subscriber_builder.with_writer(w).init();
                } else {
                    subscriber_builder.init();
                }
            } else {
                let subscriber_builder = tracing_subscriber::fmt().with_max_level(level);
                if let Some(w) = writer {
                    subscriber_builder.with_writer(w).init();
                } else {
                    subscriber_builder.init();
                }
            }
            
            if let Some(g) = guard {
                Box::leak(Box::new(g));
            }
            
            while let Ok(event) = receiver.recv() {
                match event.context {
                    Some(ctx) => {
                        match event.level {
                            1 => trace!(context = %ctx, "{}", event.message),
                            2 => debug!(context = %ctx, "{}", event.message),
                            3 => info!(context = %ctx, "{}", event.message),
                            4 => warn!(context = %ctx, "{}", event.message),
                            5 => error!(context = %ctx, "{}", event.message),
                            _ => info!(context = %ctx, "{}", event.message),
                        }
                    },
                    None => {
                        match event.level {
                            1 => trace!("{}", event.message),
                            2 => debug!("{}", event.message),
                            3 => info!("{}", event.message),
                            4 => warn!("{}", event.message),
                            5 => error!("{}", event.message),
                            _ => info!("{}", event.message),
                        }
                    }
                }
            }
        });
        
        sender
    };
}

#[no_mangle]
pub extern "C" fn rlog_configure(xml_ptr: *const c_char) -> i32 {
    if xml_ptr.is_null() {
        return -1;
    }
    
    let c_str = unsafe { CStr::from_ptr(xml_ptr) };
    if let Ok(xml_str) = c_str.to_str() {
        println!("Rust received XML length: {}", xml_str.len());
        let mut reader = Reader::from_str(xml_str);
        reader.trim_text(true);
        
        let mut config = CONFIG.lock().unwrap();
        
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                    let name = e.name();
                    let name_str = String::from_utf8_lossy(name.as_ref());
                    
                    if name_str.eq_ignore_ascii_case("File") {
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"fileName" {
                                    if let Ok(file_name) = String::from_utf8(attr.value.into_owned()) {
                                        println!("Rust extracted file name: {}", file_name);
                                        config.file_name = Some(file_name);
                                        config.rolling_policy = RollingPolicy::Never;
                                    }
                                }
                            }
                        }
                    } else if name_str.eq_ignore_ascii_case("RollingFile") {
                        let mut file_name = None;
                        let mut policy = RollingPolicy::Never;
                        
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"fileName" {
                                    if let Ok(f_name) = String::from_utf8(attr.value.into_owned()) {
                                        file_name = Some(f_name);
                                    }
                                } else if attr.key.as_ref() == b"filePattern" {
                                    if let Ok(pattern) = String::from_utf8(attr.value.into_owned()) {
                                        if pattern.contains("HH") {
                                            policy = RollingPolicy::Hourly;
                                        } else if pattern.contains("%d{") {
                                            policy = RollingPolicy::Daily;
                                        }
                                    }
                                }
                            }
                        }
                        
                        if let Some(f_name) = file_name {
                            println!("Rust extracted rolling file name: {:?}", f_name);
                            config.file_name = Some(f_name);
                            config.rolling_policy = policy;
                        }
                    } else if name_str.eq_ignore_ascii_case("JsonLayout") {
                        println!("Rust detected JsonLayout");
                        config.layout_type = LayoutType::Json;
                    } else if name_str.eq_ignore_ascii_case("AsyncQueueFullPolicy") {
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"type" {
                                    if let Ok(policy_type) = String::from_utf8(attr.value.into_owned()) {
                                        if policy_type.eq_ignore_ascii_case("Block") {
                                            println!("Rust configured to BLOCK on full queue");
                                            QUEUE_POLICY.store(1, Ordering::SeqCst);
                                        } else {
                                            QUEUE_POLICY.store(0, Ordering::SeqCst);
                                        }
                                    }
                                }
                            }
                        }
                    } else if name_str.eq_ignore_ascii_case("SizeBasedTriggeringPolicy") {
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"size" {
                                    if let Ok(size_str) = String::from_utf8(attr.value.into_owned()) {
                                        let size_str = size_str.to_uppercase();
                                        let mut size: u64 = Default::default();
                                        if size_str.ends_with("MB") {
                                            if let Ok(val) = size_str.trim_end_matches("MB").trim().parse::<u64>() {
                                                size = val * 1024 * 1024;
                                            }
                                        } else if size_str.ends_with("KB") {
                                            if let Ok(val) = size_str.trim_end_matches("KB").trim().parse::<u64>() {
                                                size = val * 1024;
                                            }
                                        } else {
                                            if let Ok(val) = size_str.parse::<u64>() {
                                                size = val;
                                            }
                                        }
                                        println!("Rust configured SizeBasedTriggeringPolicy: {} bytes", size);
                                        config.max_size = Some(size);
                                    }
                                }
                            }
                        }
                    } else if name_str.eq_ignore_ascii_case("DefaultRolloverStrategy") {
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"max" {
                                    if let Ok(max_str) = String::from_utf8(attr.value.into_owned()) {
                                        if let Ok(max_val) = max_str.parse::<usize>() {
                                            println!("Rust configured DefaultRolloverStrategy max: {}", max_val);
                                            config.max_files = max_val;
                                        }
                                    }
                                }
                            }
                        }
                    } else if name_str.eq_ignore_ascii_case("Root") {
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"level" {
                                    if let Ok(level_str) = String::from_utf8(attr.value.into_owned()) {
                                        config.level = match level_str.to_uppercase().as_str() {
                                            "TRACE" => tracing::Level::TRACE,
                                            "DEBUG" => tracing::Level::DEBUG,
                                            "INFO" => tracing::Level::INFO,
                                            "WARN" => tracing::Level::WARN,
                                            "ERROR" | "FATAL" => tracing::Level::ERROR,
                                            _ => tracing::Level::INFO,
                                        };
                                    }
                                }
                            }
                        }
                    }
                },
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => (),
            }
            buf.clear();
        }
        return 0; // Success
    }
    
    -1 // Error
}

#[no_mangle]
pub extern "C" fn rlog_init() -> i32 {
    let _ = LOG_SENDER.len();
    0
}

#[no_mangle]
pub extern "C" fn rlog_log(level: i32, msg_ptr: *const c_char) {
    rlog_log_with_context(level, msg_ptr, std::ptr::null());
}

#[no_mangle]
pub extern "C" fn rlog_log_with_context(level: i32, msg_ptr: *const c_char, ctx_ptr: *const c_char) {
    if msg_ptr.is_null() {
        return;
    }
    
    let c_str = unsafe { CStr::from_ptr(msg_ptr) };
    if let Ok(str_slice) = c_str.to_str() {
        let mut context = None;
        if !ctx_ptr.is_null() {
            let ctx_str = unsafe { CStr::from_ptr(ctx_ptr) };
            if let Ok(ctx_slice) = ctx_str.to_str() {
                context = Some(ctx_slice.to_owned());
            }
        }

        let event = LogEvent {
            level,
            message: str_slice.to_owned(),
            context,
        };
        
        if QUEUE_POLICY.load(Ordering::Relaxed) == 1 {
            let _ = LOG_SENDER.send(event);
        } else {
            let _ = LOG_SENDER.try_send(event);
        }
    }
}

#[no_mangle]
pub extern "C" fn rlog_flush() {
    // Wait for the queue to empty
    while !LOG_SENDER.is_empty() {
        thread::sleep(std::time::Duration::from_millis(10));
    }
    // Give the tracing appender background thread a moment to flush to disk
    thread::sleep(std::time::Duration::from_millis(100));
}

// --- JNI WRAPPERS FOR UNIVERSAL JAVA SUPPORT (Java 8+) ---
use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jint};

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1init(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    rlog_init()
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1configure(
    mut env: JNIEnv,
    _class: JClass,
    xml_content: JString,
) -> jint {
    if let Ok(xml_str) = env.get_string(&xml_content) {
        let xml_ptr = xml_str.as_ptr();
        return rlog_configure(xml_ptr);
    }
    -1
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1log(
    mut env: JNIEnv,
    _class: JClass,
    level: jint,
    message: JString,
) {
    if let Ok(msg_str) = env.get_string(&message) {
        rlog_log(level, msg_str.as_ptr());
    }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1log_1with_1context(
    mut env: JNIEnv,
    _class: JClass,
    level: jint,
    message: JString,
    context: JString,
) {
    if let Ok(msg_str) = env.get_string(&message) {
        if let Ok(ctx_str) = env.get_string(&context) {
            rlog_log_with_context(level, msg_str.as_ptr(), ctx_str.as_ptr());
        } else {
            rlog_log(level, msg_str.as_ptr());
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1flush(
    _env: JNIEnv,
    _class: JClass,
) {
    rlog_flush();
}
